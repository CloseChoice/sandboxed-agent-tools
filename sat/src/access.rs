//! Handle the file access. This file is mostly about resolving and cleaning paths
use crate::config::FilePermissions;
use anyhow::{Context, Result, ensure};
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use std::path::{Component, Path, PathBuf};

#[derive(Clone)]
pub struct FilesystemGrant {
    pub path: PathBuf,
    pub guest: String,
    pub write: bool,
    pub directory: bool,
}

#[derive(Clone)]
pub struct Access {
    pub grants: Vec<FilesystemGrant>,
    deny_read: GlobSet,
    deny_write: GlobSet,
    protected_home: PathBuf,
}

pub fn match_patterns(patterns: &[String]) -> Result<GlobSet> {
    // match file/folder patterns
    let mut builder = GlobSetBuilder::new();
    for p in patterns {
        ensure!(
            !p.is_empty() && !p.contains('\\'),
            "deny patterns must be nonempty globs with / separators"
        );
        builder.add(
            GlobBuilder::new(p)
                .literal_separator(true)
                .backslash_escape(false)
                .build()?,
        );
    }
    Ok(builder.build()?)
}

pub fn expand_path(path_str: &str, home: &Path, cwd: &Path) -> Result<PathBuf> {
    // todo: shouldn't we resolve this using os variables?
    let expanded_path = if path_str == "$PWD" {
        cwd.to_path_buf()
    } else if let Some(rest) = path_str.strip_prefix("$PWD/") {
        cwd.join(rest)
    } else if path_str == "$SAT_HOME" {
        home.to_path_buf()
    } else if let Some(rest) = path_str.strip_prefix("$SAT_HOME/") {
        home.join(rest)
    } else {
        PathBuf::from(path_str)
    };
    ensure!(
        expanded_path.is_absolute(),
        "grant paths must be absolute or start with $PWD/ or $SAT_HOME/"
    );
    let mut clean = PathBuf::new();
    for c in expanded_path.components() {
        match c {
            Component::ParentDir => {
                ensure!(clean.pop(), "path escapes root");
            }
            Component::CurDir => {}
            c => clean.push(c.as_os_str()),
        }
    }
    Ok(clean)
}

pub fn convert_to_guest_path(path: &Path) -> Result<String> {
    let path_str = path
        .to_str()
        .context("paths must be UTF-8")?
        .replace('\\', "/");
    Ok(if path_str.starts_with('/') {
        path_str
    } else {
        format!("/{path_str}")
    })
}

impl Access {
    pub fn new(settings: &FilePermissions, home: &Path, cwd: &Path) -> Result<Self> {
        let mut grants: Vec<FilesystemGrant> = vec![];
        // wtf, why text? isn't permission the better name?
        for (permission_list, write) in [(&settings.read, false), (&settings.write, true)] {
            for file_dir_path in permission_list {
                let path = expand_path(file_dir_path, home, cwd)?;
                let meta = std::fs::symlink_metadata(&path).context("grant path must exist")?;
                ensure!(
                    !meta.file_type().is_symlink(),
                    "symlink grants are not supported"
                );
                ensure!(
                    meta.is_file() || meta.is_dir(),
                    "grant must be a regular file or directory"
                );
                // If some files overlap, e.g. have read and write permissions, through ancestors,
                // we resolve this correctly
                let canonical_path = std::fs::canonicalize(&path)?;
                if let Some(grant) = grants.iter_mut().find(|r| r.path == canonical_path) {
                    grant.write |= write;
                    continue;
                }
                grants.push(FilesystemGrant {
                    guest: convert_to_guest_path(&path)?,
                    path: canonical_path,
                    write,
                    directory: meta.is_dir(),
                });
            }
        }
        Ok(Self {
            grants,
            protected_home: std::fs::canonicalize(home).unwrap_or_else(|_| home.to_path_buf()),
            deny_read: match_patterns(&settings.deny_read)?,
            deny_write: match_patterns(&settings.deny_write)?,
        })
    }

    pub fn denied(&self, path: &Path, write: bool) -> bool {
        // Registry metadata and compiled native cache entries are trusted host state.
        // Even a broad ancestor grant must never expose them to a component.
        if path.starts_with(&self.protected_home) {
            return true;
        }
        let patterns = if write {
            &self.deny_write
        } else {
            &self.deny_read
        };
        self.grants.iter().any(|grant| {
            let relative = if grant.directory {
                let Ok(relative) = path.strip_prefix(&grant.path) else {
                    return false;
                };
                relative.to_path_buf()
            } else if path == grant.path {
                PathBuf::from(path.file_name().unwrap())
            } else {
                return false;
            };
            // Denying a directory also denies its descendants.
            let mut prefix = PathBuf::new();
            relative.components().any(|c| {
                prefix.push(c);
                patterns.is_match(&prefix)
            })
        })
    }

    pub fn authorize(&self, path: &Path, write: bool) -> Result<()> {
        ensure!(
            self.grants.iter().any(|r| (path == r.path
                || r.directory && path.starts_with(&r.path))
                && (!write || r.write)),
            "filesystem access denied"
        );
        ensure!(!self.denied(path, write), "filesystem access denied");
        Ok(())
    }

    pub fn resolve_guest_path_to_host_path(&self, guest_path_str: &str) -> Option<PathBuf> {
        self.grants
            .iter()
            .filter_map(|r| {
                if guest_path_str == r.guest {
                    Some((r.guest.len(), r.path.clone()))
                } else if r.directory {
                    guest_path_str
                        .strip_prefix(&format!("{}/", r.guest.trim_end_matches('/')))
                        .map(|rest| (r.guest.len(), r.path.join(rest)))
                } else {
                    None
                }
            })
            .max_by_key(|(n, _)| *n)
            .map(|(_, p)| p)
    }
}

/// Walk one component at a time with no-follow semantics, pinning each parent.
pub fn open_parent_directory_without_symlinks(
    root: &Dir,
    relative: &str,
) -> std::io::Result<(Dir, String)> {
    let mut dir = root.try_clone()?;
    let components: Vec<_> = Path::new(relative).components().collect();
    if components
        .iter()
        .any(|c| !matches!(c, Component::Normal(_) | Component::CurDir))
    {
        return Err(std::io::ErrorKind::PermissionDenied.into());
    }
    let names: Vec<_> = components
        .iter()
        .filter_map(|c| {
            if let Component::Normal(n) = c {
                Some(n)
            } else {
                None
            }
        })
        .collect();
    for name in names.iter().take(names.len().saturating_sub(1)) {
        dir = dir.open_dir_nofollow(name)?;
    }
    Ok((
        dir,
        names
            .last()
            .map(|n| n.to_string_lossy().into_owned())
            .unwrap_or_else(|| ".".into()),
    ))
}

pub fn open_file(
    dir: &Dir,
    name: &str,
    write: bool,
    create: bool,
) -> std::io::Result<std::fs::File> {
    let mut options = OpenOptions::new();
    options
        .read(true)
        .write(write)
        .create(create)
        .follow(FollowSymlinks::No);
    let file = dir.open_with(name, &options)?.into_std();
    check_file(&file)?;
    Ok(file)
}

pub fn check_file(file: &std::fs::File) -> std::io::Result<()> {
    let meta = file.metadata()?;
    if !meta.is_file() {
        return Err(std::io::ErrorKind::PermissionDenied.into());
    }
    // Reject files with multiple hard-linked names: another name could
    // refer to the same file outside the permitted paths.
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.nlink() > 1 {
            return Err(std::io::ErrorKind::PermissionDenied.into());
        }
    }

    #[cfg(windows)]
    {
        use cap_fs_ext::MetadataExt;
        // std's Windows link-count API is nightly-only. Capability metadata
        // obtains the same information from the already-open file on stable Rust.
        let file = cap_std::fs::File::from_std(file.try_clone()?);
        if file.metadata()?.nlink() > 1 {
            return Err(std::io::ErrorKind::PermissionDenied.into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    #[test]
    fn hardlinks_cannot_bypass_path_permissions() {
        let directory = tempfile::tempdir().unwrap();
        let original = directory.path().join("original");
        let alias = directory.path().join("alias");
        let file = std::fs::File::create(&original).unwrap();
        super::check_file(&file).unwrap();
        std::fs::hard_link(&original, &alias).unwrap();
        assert_eq!(
            super::check_file(&file).unwrap_err().kind(),
            std::io::ErrorKind::PermissionDenied
        );
        std::fs::remove_file(alias).unwrap();
        super::check_file(&file).unwrap();
    }
}
