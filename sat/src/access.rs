//! Handle the file access. This file is mostly about resolving and cleaning paths
use crate::config::{FilePermissions, ToolSettings};
use anyhow::{Context, Result, ensure};
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::fs::{Dir, OpenOptions};
use globset::{GlobBuilder, GlobSet, GlobSetBuilder};
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;

const MAX_GLOB_FILES: usize = 128;
const MAX_GLOB_ENTRIES: usize = 65_536;

pub fn is_read_glob(path: &Path) -> bool {
    path.components().any(
        |part| matches!(part, Component::Normal(s) if s.to_string_lossy().contains(['*', '?'])),
    )
}

#[derive(Clone)]
pub struct FilesystemGrant {
    pub path: PathBuf,
    pub guest: String,
    pub write: bool,
    pub directory: bool,
    // Keep the file selected during enumeration pinned for both WASI and named imports.
    glob_file: Option<Arc<std::fs::File>>,
}

impl FilesystemGrant {
    pub fn open_file(&self, write: bool) -> Result<std::fs::File> {
        let file = match &self.glob_file {
            Some(file) => {
                ensure!(!write, "glob grants are read-only");
                file.try_clone()?
            }
            None => std::fs::OpenOptions::new()
                .read(true)
                .write(write)
                .open(&self.path)?,
        };
        check_file(&file)?;
        Ok(file)
    }
}

#[derive(Clone)]
pub struct Access {
    pub grants: Vec<FilesystemGrant>,
    deny_read: GlobSet,
    deny_write: GlobSet,
    protected_home: PathBuf,
    exclusion_roots: Vec<(PathBuf, bool)>,
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
                    glob_file: None,
                });
            }
        }
        Ok(Self {
            grants,
            exclusion_roots: Vec::new(),
            protected_home: std::fs::canonicalize(home).unwrap_or_else(|_| home.to_path_buf()),
            deny_read: match_patterns(&settings.deny_read)?,
            deny_write: match_patterns(&settings.deny_write)?,
        })
    }

    pub fn for_invocation(settings: &ToolSettings, home: &Path, cwd: &Path) -> Result<Self> {
        let mut access = Self::new(&settings.file_permissions, home, cwd)?;
        if !access.deny_read.is_empty() || !access.deny_write.is_empty() {
            // Narrowing a configured directory to a file must preserve the directory's
            // coordinate system for exclusion patterns such as "private/*.py".
            for value in &settings.replaced_grants {
                let path = expand_path(value, home, cwd)?;
                match std::fs::metadata(&path) {
                    Ok(meta) => access
                        .exclusion_roots
                        .push((std::fs::canonicalize(path)?, meta.is_dir())),
                    Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                    Err(err) => return Err(err.into()),
                }
            }
        }
        access.add_read_globs(&settings.read_globs, home, cwd)?;
        Ok(access)
    }

    fn add_read_globs(&mut self, patterns: &[String], home: &Path, cwd: &Path) -> Result<()> {
        let mut scanned = 0;
        let mut matched = 0;
        for value in patterns {
            let raw = Path::new(value);
            let pattern = raw
                .file_name()
                .and_then(|s| s.to_str())
                .context("expected a filename glob")?;
            ensure!(
                pattern.contains(['*', '?'])
                    && !pattern.contains("**")
                    && !is_read_glob(raw.parent().context("expected a glob parent")?),
                "read globs support * and ? only in the filename"
            );
            let path = expand_path(value, home, cwd)?;
            let parent = path.parent().context("read glob requires a directory")?;
            let root = parent.ancestors().last().unwrap();
            let relative = parent.strip_prefix(root)?;
            // Derive the policy path from the same no-follow traversal as the handle.
            let canonical_parent = std::fs::canonicalize(root)?.join(relative);
            ensure!(
                canonical_parent == std::fs::canonicalize(parent)?,
                "use the canonical directory for a read glob"
            );
            let mut dir = Dir::open_ambient_dir(root, cap_std::ambient_authority())?;
            for component in relative.components() {
                dir = dir.open_dir_nofollow(component.as_os_str())?;
            }
            let escaped = globset::escape(pattern)
                .replace("[*]", "*")
                .replace("[?]", "?");
            let matcher = GlobBuilder::new(&escaped)
                .backslash_escape(false)
                .build()?
                .compile_matcher();
            let before = matched;
            for entry in dir.entries()? {
                scanned += 1;
                ensure!(
                    scanned <= MAX_GLOB_ENTRIES,
                    "read globs exceed the directory entry limit ({MAX_GLOB_ENTRIES})"
                );
                let entry = entry?;
                let name = entry.file_name();
                let Some(name) = name.to_str() else { continue };
                if name.contains('\\') || !matcher.is_match(name) || !entry.file_type()?.is_file() {
                    continue;
                }
                let path = canonical_parent.join(name);
                if self.deny_read.is_match(name) || self.denied(&path, false) {
                    continue;
                }
                matched += 1;
                ensure!(
                    matched <= MAX_GLOB_FILES,
                    "read globs exceed the match limit ({MAX_GLOB_FILES})"
                );
                if self.grants.iter().any(|grant| grant.path == path) {
                    continue;
                }
                self.grants.push(FilesystemGrant {
                    path,
                    guest: convert_to_guest_path(&parent.join(name))?,
                    directory: false,
                    write: false,
                    glob_file: Some(Arc::new(open_file(&dir, name, false, false)?)),
                });
            }
            ensure!(
                matched > before,
                "read glob matched no permitted regular files: {value}"
            );
        }
        Ok(())
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
        let matches_relative = |relative: &Path| {
            relative
                .ancestors()
                .any(|p| !p.as_os_str().is_empty() && patterns.is_match(p))
        };
        let matches_root = |root: &Path, directory: bool| {
            if directory {
                path.strip_prefix(root).is_ok_and(matches_relative)
            } else {
                root == path && path.file_name().is_some_and(|name| patterns.is_match(name))
            }
        };
        self.exclusion_roots
            .iter()
            .any(|(root, dir)| matches_root(root, *dir))
            || self
                .grants
                .iter()
                .any(|grant| matches_root(&grant.path, grant.directory))
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
        .follow(FollowSymlinks::No)
        .nonblock(true);
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
    use super::*;
    use crate::cli::apply_permission_overrides;

    fn read_globs(cwd: &Path, patterns: &[&str]) -> Result<Access> {
        let mut settings = ToolSettings::default();
        settings.file_permissions.read.push("$PWD".into());
        settings
            .file_permissions
            .deny_read
            .push("src/private*.py".into());
        let mut args = vec!["--".to_owned()];
        for pattern in patterns {
            args.extend(["--read".to_owned(), (*pattern).to_owned()]);
        }
        apply_permission_overrides(&args, &mut settings)?;
        Access::for_invocation(&settings, &cwd.join("registry"), cwd)
    }

    #[test]
    fn glob_grants_only_matching_permitted_files() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let cwd = temp.path().canonicalize()?;
        let src = cwd.join("src");
        std::fs::create_dir(&src)?;
        for name in ["main.py", "notes.txt", "private1.py"] {
            std::fs::write(src.join(name), name)?;
        }
        std::fs::create_dir(src.join("folder.py"))?;
        #[cfg(unix)]
        std::os::unix::fs::symlink(src.join("notes.txt"), src.join("linked.py"))?;

        let access = read_globs(&cwd, &["$PWD/src/*.py", "$PWD/src/mai?.py"])?;
        assert_eq!(access.grants.len(), 1);
        let main = src.join("main.py");
        access.authorize(&main, false)?;
        assert!(access.authorize(&main, true).is_err());
        assert!(access.authorize(&src, false).is_err());
        std::fs::write(src.join("late.py"), "")?;
        for name in [
            "notes.txt",
            "private1.py",
            "linked.py",
            "folder.py",
            "late.py",
        ] {
            assert!(access.authorize(&src.join(name), false).is_err(), "{name}");
        }
        #[cfg(unix)]
        {
            // Both filesystem interfaces consume the handle selected at expansion.
            use std::io::Read;
            std::fs::remove_file(&main)?;
            std::os::unix::fs::symlink(src.join("notes.txt"), &main)?;
            let mut contents = String::new();
            access.grants[0]
                .open_file(false)?
                .read_to_string(&mut contents)?;
            assert_eq!(contents, "main.py");
        }
        Ok(())
    }

    #[test]
    fn invalid_or_excessive_globs_fail_the_invocation() -> Result<()> {
        let temp = tempfile::tempdir()?;
        let cwd = temp.path().canonicalize()?;
        for pattern in ["$PWD/*.py", "$PWD/**/*.py", "$PWD/*/name.py", "$PWD/**.py"] {
            assert!(read_globs(&cwd, &[pattern]).is_err(), "{pattern}");
        }
        for i in 0..=MAX_GLOB_FILES {
            std::fs::write(cwd.join(format!("{i}.py")), "")?;
        }
        let result = read_globs(&cwd, &["$PWD/*.py"]);
        assert!(result.err().unwrap().to_string().contains("match limit"));
        Ok(())
    }

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
