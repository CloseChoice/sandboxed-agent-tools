//! Unified-diff application. Filesystem authority is enforced by the SAT host.
use clap::Parser;
use diffy::patch_set::{FileMode, FileOperation, ParseOptions, PatchKind, PatchSet};
use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{self, Read, Write},
    path::{Component, Path, PathBuf},
};

wit_bindgen::generate!({path: "wit", world: "patch-cli"});
struct ComponentImpl;
impl Guest for ComponentImpl {
    fn run(args: Vec<String>) -> i32 {
        run_cli(args)
    }
}
export!(ComponentImpl);

const FILE_LIMIT: usize = 16 * 1024 * 1024;
const TOTAL_LIMIT: usize = 64 * 1024 * 1024;
const FILE_COUNT: usize = 256;

#[derive(Parser, Debug)]
#[command(
    name = "patch",
    version,
    about = "Apply unified diffs (SAT profile; diffy 0.5.2). No fuzz, prompts, backups or reject files."
)]
struct Cli {
    /// Strip NUM leading components; default: use the header's basename.
    #[arg(short = 'p', long = "strip")]
    strip: Option<usize>,
    /// Read a patch file instead of standard input; '-' also means stdin.
    #[arg(short = 'i', long = "input")]
    input: Option<PathBuf>,
    /// Target directory (default: invocation directory).
    #[arg(short = 'd', long = "directory")]
    directory: Option<PathBuf>,
    /// Validate all files without writing anything.
    #[arg(long)]
    dry_run: bool,
    /// Apply the patch in reverse.
    #[arg(short = 'R', long)]
    reverse: bool,
    #[arg(short = 's', long = "quiet", alias = "silent")]
    quiet: bool,
    /// Accepted for GNU CLI compatibility; always noninteractive.
    #[arg(short = 't', long)]
    batch: bool,
    /// Accepted for GNU CLI compatibility; only unified diffs are supported.
    #[arg(short = 'u', long)]
    unified: bool,
    /// Preserve bytes; not support for Git binary diffs.
    #[arg(long)]
    binary: bool,
    /// Only zero is supported: all context must match.
    #[arg(short='F', long="fuzz", default_value="0", value_parser=clap::value_parser!(u8).range(0..=0))]
    fuzz: u8,
    /// Explicit target for a single-file diff, relative to -d or absolute.
    original: Option<PathBuf>,
    /// Alternative to -i (requires ORIGINAL).
    patchfile: Option<PathBuf>,
}

struct Failure {
    status: i32,
    message: String,
}
type Result<T> = std::result::Result<T, Failure>;
fn error(message: impl Into<String>) -> Failure {
    Failure {
        status: 2,
        message: message.into(),
    }
}
fn io_error(path: &Path, err: io::Error) -> Failure {
    error(format!("{}: {err}", path.display()))
}

fn read_limited(reader: impl Read, label: &str) -> Result<Vec<u8>> {
    let mut bytes = Vec::new();
    reader
        .take(FILE_LIMIT as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|e| error(format!("{label}: {e}")))?;
    if bytes.len() > FILE_LIMIT {
        return Err(error(format!("{label}: exceeds 16 MiB limit")));
    }
    Ok(bytes)
}

// Validate before stripping; explicit user-selected target paths are separate.
fn header_path(bytes: &[u8], strip: Option<usize>) -> Result<PathBuf> {
    let path =
        std::str::from_utf8(bytes).map_err(|_| error("non-UTF-8 filenames are unsupported"))?;
    if path.is_empty()
        || path.starts_with('/')
        || path.contains(['\\', ':'])
        || path.chars().any(char::is_control)
    {
        return Err(error(format!("unsafe patch filename: {path:?}")));
    }
    let parts: Vec<_> = path.split('/').collect();
    if parts
        .iter()
        .any(|p| p.is_empty() || *p == "." || *p == "..")
    {
        return Err(error(format!("unsafe patch filename: {path:?}")));
    }
    let count = strip.unwrap_or(parts.len() - 1);
    if count >= parts.len() {
        return Err(error(format!(
            "cannot strip {count} components from {path:?}"
        )));
    }
    Ok(parts[count..].iter().collect())
}

fn regular_path(path: &Path) -> Result<()> {
    let mut prefix = PathBuf::new();
    for part in path.components() {
        if part == Component::ParentDir {
            return Err(error("parent traversal is unsupported"));
        }
        prefix.push(part);
        match fs::symlink_metadata(&prefix) {
            Ok(meta) => {
                if meta.file_type().is_symlink() {
                    return Err(error(format!(
                        "symlink paths are unsupported: {}",
                        prefix.display()
                    )));
                }
                if prefix == path && !meta.is_file() {
                    return Err(error(format!("not a regular file: {}", path.display())));
                }
                if prefix != path && !meta.is_dir() {
                    return Err(error(format!("not a directory: {}", prefix.display())));
                }
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => return Err(io_error(&prefix, e)),
        }
    }
    Ok(())
}

#[derive(Clone, Copy, PartialEq)]
enum Operation {
    Create,
    Modify,
    Delete,
}
struct Change {
    path: PathBuf,
    label: PathBuf,
    old: Vec<u8>,
    new: Vec<u8>,
    op: Operation,
}

fn prepare(cli: &Cli, input: &[u8]) -> Result<Vec<Change>> {
    if input.iter().all(u8::is_ascii_whitespace) {
        return Ok(Vec::new());
    }
    let cwd = std::env::var_os("PWD")
        .map(PathBuf::from)
        .or_else(|| std::env::current_dir().ok())
        .unwrap_or_else(|| PathBuf::from("/"));
    let directory = cli
        .directory
        .as_ref()
        .map(|p| {
            if p.is_absolute() {
                p.clone()
            } else {
                cwd.join(p)
            }
        })
        .unwrap_or(cwd);
    let git = input
        .split(|c| *c == b'\n')
        .any(|l| l.starts_with(b"diff --git "));
    let options = if git {
        ParseOptions::gitdiff()
    } else {
        ParseOptions::unidiff()
    };
    let mut result = Vec::new();
    let mut targets = HashSet::new();
    let mut total = input.len();
    for item in PatchSet::parse_bytes(input, options) {
        let item = item.map_err(|e| error(format!("invalid patch: {e}")))?;
        if result.len() >= FILE_COUNT {
            return Err(error("patch exceeds 256 files"));
        }
        if [item.old_mode(), item.new_mode()]
            .into_iter()
            .flatten()
            .any(|m| !matches!(m, FileMode::Regular))
        {
            return Err(error(
                "executable modes, symlinks and submodules are unsupported",
            ));
        }
        let patch = match item.patch() {
            PatchKind::Text(patch) if !patch.hunks().is_empty() => patch,
            _ => return Err(error("binary and metadata-only patches are unsupported")),
        };
        let (op, relative) = match item.operation() {
            FileOperation::Create(name) => (Operation::Create, header_path(name, cli.strip)?),
            FileOperation::Delete(name) => (Operation::Delete, header_path(name, cli.strip)?),
            FileOperation::Modify { original, modified } => {
                let old = header_path(original, cli.strip)?;
                let new = header_path(modified, cli.strip)?;
                if old != new && cli.original.is_none() {
                    return Err(error(
                        "different old/new filenames require an explicit ORIGINAL target",
                    ));
                }
                (Operation::Modify, old)
            }
            _ => return Err(error("rename and copy patches are unsupported")),
        };
        if cli.original.is_some() && !result.is_empty() {
            return Err(error("ORIGINAL requires a single-file patch"));
        }
        let op = if cli.reverse {
            match op {
                Operation::Create => Operation::Delete,
                Operation::Delete => Operation::Create,
                other => other,
            }
        } else {
            op
        };
        let path = directory.join(cli.original.as_ref().unwrap_or(&relative));
        regular_path(&path)?;
        if !targets.insert(path.to_string_lossy().to_lowercase()) {
            return Err(error("duplicate patch target"));
        }
        let old = match File::open(&path) {
            Ok(file) => {
                if op == Operation::Create {
                    return Err(error(format!("target already exists: {}", path.display())));
                }
                read_limited(file, &path.display().to_string())?
            }
            Err(e) if e.kind() == io::ErrorKind::NotFound && op == Operation::Create => Vec::new(),
            Err(e) => return Err(io_error(&path, e)),
        };
        let reversed;
        let patch = if cli.reverse {
            reversed = patch.reverse();
            &reversed
        } else {
            patch
        };
        let new = diffy::apply_bytes(&old, patch).map_err(|e| Failure {
            status: 1,
            message: format!("{}: {e}; no changes written", path.display()),
        })?;
        if new.len() > FILE_LIMIT {
            return Err(error("patched file exceeds 16 MiB"));
        }
        if op == Operation::Delete && !new.is_empty() {
            return Err(error("deletion patch leaves nonempty content"));
        }
        total += old.len() + new.len();
        if total > TOTAL_LIMIT {
            return Err(error("patch exceeds 64 MiB working-data limit"));
        }
        result.push(Change {
            path,
            label: cli.original.clone().unwrap_or(relative),
            old,
            new,
            op,
        });
    }
    Ok(result)
}

fn commit(changes: &[Change]) -> Result<()> {
    // Preflight existing write targets before modifying content.
    let mut handles = Vec::with_capacity(changes.len());
    for change in changes {
        regular_path(&change.path)?;
        if change.op == Operation::Create {
            handles.push(None);
            continue;
        }
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&change.path)
            .map_err(|e| io_error(&change.path, e))?;
        let current = read_limited(&mut file, &change.path.display().to_string())?;
        if current != change.old {
            return Err(error("target changed during patch preparation"));
        }
        handles.push(Some(file));
    }
    for (change, handle) in changes.iter().zip(handles.iter_mut()) {
        match change.op {
            Operation::Create => {
                // Existing parent required; no implicit directory creation.
                let mut file = OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(&change.path)
                    .map_err(|e| io_error(&change.path, e))?;
                file.write_all(&change.new)
                    .map_err(|e| io_error(&change.path, e))?;
            }
            Operation::Modify => {
                use std::io::{Seek, SeekFrom};
                let file = handle.as_mut().unwrap();
                file.seek(SeekFrom::Start(0))
                    .map_err(|e| io_error(&change.path, e))?;
                file.write_all(&change.new)
                    .and_then(|_| file.set_len(change.new.len() as u64))
                    .map_err(|e| io_error(&change.path, e))?;
            }
            Operation::Delete => {
                handle.take(); // Close first for Windows.
                fs::remove_file(&change.path).map_err(|e| io_error(&change.path, e))?;
            }
        }
    }
    Ok(())
}

fn execute(cli: &Cli) -> Result<()> {
    if cli.input.is_some() && cli.patchfile.is_some() {
        return Err(error("use either -i or PATCHFILE"));
    }
    let input = match cli.input.as_ref().or(cli.patchfile.as_ref()) {
        Some(path) if path != Path::new("-") => read_limited(
            File::open(path).map_err(|e| io_error(path, e))?,
            &path.display().to_string(),
        )?,
        _ => read_limited(io::stdin().lock(), "stdin")?,
    };
    let changes = prepare(cli, &input)?;
    if !cli.dry_run {
        commit(&changes)?;
    }
    if !cli.quiet {
        for change in changes {
            println!(
                "{} file {}",
                if cli.dry_run { "checking" } else { "patching" },
                change.label.display()
            );
        }
    }
    Ok(())
}

pub fn run_cli(args: Vec<String>) -> i32 {
    let cli = match Cli::try_parse_from(std::iter::once("patch".to_string()).chain(args)) {
        Ok(cli) => cli,
        Err(e) => {
            let code = e.exit_code();
            let _ = e.print();
            return code;
        }
    };
    match execute(&cli) {
        Ok(()) => 0,
        Err(e) => {
            eprintln!("patch: {}", e.message);
            e.status
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn filenames_checked_before_stripping() {
        for path in [
            "../foo",
            "a/../foo",
            "/etc/passwd",
            "C:/foo",
            "a\\foo",
            "a//foo",
            "a/./foo",
            "a/\nfoo",
        ] {
            assert!(header_path(path.as_bytes(), Some(1)).is_err(), "{path}");
        }
        assert_eq!(
            header_path(b"a/src/lib.rs", Some(1)).ok().unwrap(),
            Path::new("src/lib.rs")
        );
        assert!(header_path(b"foo", Some(1)).is_err());
    }
}
