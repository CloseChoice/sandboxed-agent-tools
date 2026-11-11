//! Filesystem authority shared by WASI and named text interfaces.
//! Symlinks are never followed; rename/link are rejected by the WASI adapter.

use crate::config::FilePermissions;
use anyhow::{Context, Result, ensure};
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::{Dir, OpenOptions};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use std::path::{Component, Path, PathBuf};

#[derive(Clone)]
pub struct Root {
    pub path: PathBuf,
    pub guest: String,
    pub write: bool,
    pub directory: bool,
}

#[derive(Clone)]
pub struct Access {
    pub roots: Vec<Root>,
    deny_read: GlobSet,
    deny_write: GlobSet,
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

pub fn path(text: &str, home: &Path, cwd: &Path) -> Result<PathBuf> {
    // todo: shouldn't we resolve this using os variables?
    let expanded = if text == "$PWD" {
        cwd.to_path_buf()
    } else if let Some(rest) = text.strip_prefix("$PWD/") {
        cwd.join(rest)
    } else if text == "$SAT_HOME" {
        home.to_path_buf()
    } else if let Some(rest) = text.strip_prefix("$SAT_HOME/") {
        home.join(rest)
    } else {
        PathBuf::from(text)
    };
    ensure!(
        expanded.is_absolute(),
        "grant paths must be absolute or start with $PWD/ or $SAT_HOME/"
    );
    let mut clean = PathBuf::new();
    for c in expanded.components() {
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

pub fn guest_path(path: &Path) -> Result<String> {
    let text = path
        .to_str()
        .context("paths must be UTF-8")?
        .replace('\\', "/");
    Ok(if text.starts_with('/') {
        text
    } else {
        format!("/{text}")
    })
}

impl Access {
    pub fn new(settings: &FilePermissions, home: &Path, cwd: &Path) -> Result<Self> {
        let mut roots: Vec<Root> = vec![];
        for (list, write) in [(&settings.read, false), (&settings.write, true)] {
            for text in list {
                let path = path(text, home, cwd)?;
                let meta = std::fs::symlink_metadata(&path).context("grant path must exist")?;
                ensure!(
                    !meta.file_type().is_symlink(),
                    "symlink grants are not supported"
                );
                ensure!(
                    meta.is_file() || meta.is_dir(),
                    "grant must be a regular file or directory"
                );
                // Canonical ancestors prevent aliases of a grant from changing deny coordinates.
                let canonical = std::fs::canonicalize(&path)?;
                if let Some(root) = roots.iter_mut().find(|r| r.path == canonical) {
                    root.write |= write;
                    continue;
                }
                roots.push(Root {
                    guest: guest_path(&path)?,
                    path: canonical,
                    write,
                    directory: meta.is_dir(),
                });
            }
        }
        Ok(Self {
            roots,
            deny_read: match_patterns(&settings.deny_read)?,
            deny_write: match_patterns(&settings.deny_write)?,
        })
    }

    pub fn denied(&self, path: &Path, write: bool) -> bool {
        let patterns = if write {
            &self.deny_write
        } else {
            &self.deny_read
        };
        self.roots.iter().any(|root| {
            let relative = if root.directory {
                let Ok(relative) = path.strip_prefix(&root.path) else {
                    return false;
                };
                relative.to_path_buf()
            } else if path == root.path {
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
            self.roots.iter().any(
                |r| (path == r.path || r.directory && path.starts_with(&r.path))
                    && (!write || r.write)
            ),
            "filesystem access denied"
        );
        ensure!(!self.denied(path, write), "filesystem access denied");
        Ok(())
    }

    pub fn host_for_guest(&self, guest: &str) -> Option<PathBuf> {
        self.roots
            .iter()
            .filter_map(|r| {
                if guest == r.guest {
                    Some((r.guest.len(), r.path.clone()))
                } else if r.directory {
                    guest
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
pub fn secure_parent(root: &Dir, relative: &str) -> std::io::Result<(Dir, String)> {
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
    // Hardlinks alias inode contents and could bypass a path-based exclusion.
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        if meta.nlink() > 1 {
            return Err(std::io::ErrorKind::PermissionDenied.into());
        }
    }

    #[cfg(windows)]
    {
        use std::os::windows::fs::MetadataExt;
        if meta.number_of_links().is_none_or(|n| n > 1) {
            return Err(std::io::ErrorKind::PermissionDenied.into());
        }
    }
    Ok(())
}
