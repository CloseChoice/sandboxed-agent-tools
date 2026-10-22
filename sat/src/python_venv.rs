//! Build Python components from wheels using componentize-py's Wasmtime guest.
use anyhow::{Context, Result, bail, ensure};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::{
    fs,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Config {
    registries: std::collections::BTreeMap<String, Registry>,
}
#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Registry {
    index_url: Option<String>,
    index_url_env: Option<String>,
    #[serde(default)]
    extra_index_urls: Vec<String>,
}
struct Scratch(PathBuf);
impl Drop for Scratch {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.0);
    }
}

fn scratch() -> Result<Scratch> {
    let base = std::env::var_os("SAT_BUILD_TMP")
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    fs::create_dir_all(&base)?;
    for n in 0..20 {
        let now = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let dir = base.join(format!("sat-python-{}-{now}-{n}", std::process::id()));
        match fs::create_dir(&dir) {
            Ok(()) => {
                #[cfg(unix)]
                {
                    use std::os::unix::fs::PermissionsExt;
                    fs::set_permissions(&dir, fs::Permissions::from_mode(0o700))?;
                }
                return Ok(Scratch(dir));
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(e.into()),
        }
    }
    bail!("could not allocate a unique Python build directory")
}

fn registry(name: &str) -> Result<Registry> {
    let path = crate::home()?.join("python-registries.toml");
    let text = fs::read_to_string(&path).with_context(|| {
        format!(
            "read {}; configure Python registries in SAT_HOME/python-registries.toml",
            path.display()
        )
    })?;
    let config: Config = toml::from_str(&text).context("invalid python-registries.toml")?;
    let mut item = config
        .registries
        .get(name)
        .context("Python registry is not configured")?
        .clone();
    if let Some(env) = &item.index_url_env {
        item.index_url = Some(
            std::env::var(env).with_context(|| format!("set {env} for Python registry {name}"))?,
        );
    }
    for url in item.index_url.iter().chain(item.extra_index_urls.iter()) {
        ensure!(
            url.starts_with("https://"),
            "Python registry URLs must use HTTPS"
        );
    }
    ensure!(
        item.index_url.is_some(),
        "registry needs index_url or index_url_env"
    );
    Ok(item)
}

fn builder() -> Result<(PathBuf, PathBuf)> {
    if let Some(path) = std::env::var_os("SAT_COMPONENTIZE_PY") {
        let exe = PathBuf::from(path);
        ensure!(exe.is_file(), "SAT_COMPONENTIZE_PY is not a file");
        let root = exe
            .parent()
            .and_then(Path::parent)
            .context("componentize-py must be under a builder bin directory")?
            .to_path_buf();
        return Ok((exe, root));
    }
    for ancestor in std::env::current_exe()?.ancestors() {
        let root = ancestor.join("python-example/target/builder");
        #[cfg(windows)]
        let exe = root.join("Scripts/componentize-py.exe");
        #[cfg(not(windows))]
        let exe = root.join("bin/componentize-py");
        if exe.is_file() {
            return Ok((exe, root));
        }
    }
    bail!("componentize-py not found; first run python-example/build.py or set SAT_COMPONENTIZE_PY")
}

fn run(command: &mut Command, label: &str) -> Result<()> {
    let timeout = std::env::var("SAT_PYTHON_BUILD_TIMEOUT_SECS")
        .ok()
        .and_then(|s| s.parse::<u64>().ok())
        .unwrap_or(600);
    let mut child = command
        .stdin(Stdio::null())
        .spawn()
        .with_context(|| format!("start {label}"))?;
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            ensure!(status.success(), "{label} failed with {status}");
            return Ok(());
        }
        if started.elapsed() >= Duration::from_secs(timeout) {
            let _ = child.kill();
            let _ = child.wait();
            bail!("{label} exceeded the {timeout}s build timeout");
        }
        thread::sleep(Duration::from_millis(100));
    }
}

fn valid_module(s: &str) -> bool {
    !s.is_empty()
        && s.chars()
            .next()
            .is_some_and(|c| c == '_' || c.is_ascii_alphabetic())
        && s.chars().all(|c| c == '_' || c.is_ascii_alphanumeric())
}
fn import_names(site: &Path) -> Result<Vec<String>> {
    let mut names = std::collections::BTreeSet::new();
    for entry in fs::read_dir(site)? {
        let entry = entry?;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().into_owned();
        if name.ends_with(".dist-info") {
            if let Ok(contents) = fs::read_to_string(path.join("top_level.txt")) {
                for n in contents.lines().map(str::trim).filter(|n| !n.is_empty()) {
                    if valid_module(n) {
                        names.insert(n.to_string());
                    }
                }
            }
        } else if (path.is_dir() && path.join("__init__.py").is_file()) || name.ends_with(".py") {
            let n = name.strip_suffix(".py").unwrap_or(&name);
            if valid_module(n) {
                names.insert(n.to_string());
            }
        }
    }
    Ok(names.into_iter().collect())
}
fn has_python_startup_hook(site: &Path) -> Result<bool> {
    for entry in fs::read_dir(site)? {
        let entry = entry?;
        let name = entry.file_name();
        if name.to_string_lossy().ends_with(".pth") {
            return Ok(true);
        }
    }
    Ok(false)
}

fn wheel_fingerprint(dir: &Path) -> Result<String> {
    let mut entries = fs::read_dir(dir)?.collect::<std::result::Result<Vec<_>, _>>()?;
    entries.sort_by_key(|entry| entry.file_name());
    let mut hash = Sha256::new();
    for entry in entries {
        ensure!(
            entry.path().is_file(),
            "unexpected directory in wheel download output"
        );
        hash.update(entry.file_name().to_string_lossy().as_bytes());
        hash.update(Sha256::digest(fs::read(entry.path())?));
    }
    Ok(format!("{:x}", hash.finalize()))
}

fn cache_key(
    requirements: &[String],
    registry_name: &str,
    index: &str,
    wheels: &str,
    name: &str,
) -> String {
    let mut normalized = requirements.to_vec();
    normalized.sort();
    let mut hash = Sha256::new();
    hash.update(b"sat-python-venv-v1;python=3.14;componentize-py=0.25.0;");
    hash.update(name.as_bytes());
    hash.update(registry_name.as_bytes());
    hash.update(Sha256::digest(index.as_bytes()));
    for req in normalized {
        hash.update(req.as_bytes());
        hash.update([0]);
    }
    hash.update(wheels.as_bytes());
    hash.update(include_bytes!("../../python-example/app.py"));
    hash.update(include_bytes!("../../python-example/wit/world.wit"));
    hash.update(include_bytes!("../../python-example/policy.toml"));
    format!("{:x}", hash.finalize())
}

pub fn build(requirements: &[String], name: &str, registry_name: &str) -> Result<Vec<u8>> {
    ensure!(
        crate::metadata::valid_name(name),
        "invalid Python environment name"
    );
    ensure!(!requirements.is_empty(), "provide at least one requirement");
    for requirement in requirements {
        ensure!(
            !requirement.starts_with('-'),
            "requirements must be package specifiers, not pip options"
        );
        ensure!(
            !requirement.contains("://") && !requirement.contains(" @ "),
            "direct package URLs are not allowed; use a configured registry"
        );
        ensure!(
            !requirement.contains(['\n', '\r']),
            "each requirement must be a single line"
        );
    }
    let registry = registry(registry_name)?;
    let (componentizer, builder_root) = builder()?;
    let lib = fs::read_dir(builder_root.join("lib")).context("inspect componentize-py builder")?;
    let pyver = lib
        .filter_map(std::result::Result::ok)
        .find(|e| e.file_name().to_string_lossy().starts_with("python3.") && e.path().is_dir())
        .context("builder has no lib/python3.x")?
        .file_name();
    ensure!(
        builder_root
            .join("lib")
            .join(&pyver)
            .join("site-packages/componentize_py-0.25.0.dist-info/METADATA")
            .is_file(),
        "Python environment builds require the pinned componentize-py 0.25.0 builder"
    );
    let root = scratch()?;
    let wheels = root.0.join("wheels");
    let venv = root.0.join("venv");
    let site = venv.join("lib").join(pyver).join("site-packages");
    let work = root.0.join("work");
    fs::create_dir_all(&wheels)?;
    fs::create_dir_all(&site)?;
    fs::create_dir_all(&work)?;

    let mut download = Command::new(if cfg!(windows) { "python" } else { "python3" });
    download
        .args([
            "-m",
            "pip",
            "--isolated",
            "download",
            "--only-binary=:all:",
            "--platform",
            "any",
            "--python-version",
            "3.14",
            "--implementation",
            "py",
            "--abi",
            "none",
            "--dest",
        ])
        .arg(&wheels)
        .arg("--index-url")
        .arg(registry.index_url.as_deref().unwrap());
    for url in registry.extra_index_urls {
        download.arg("--extra-index-url").arg(url);
    }
    let mut normalized_requirements = requirements.to_vec();
    normalized_requirements.sort();
    download.args(&normalized_requirements);
    run(&mut download, "wheel-only Python dependency download")?;

    let wheels_hash = wheel_fingerprint(&wheels)?;
    let cache = crate::home()?.join(".cache/python-venv");
    fs::create_dir_all(&cache)?;
    let key = cache_key(
        &normalized_requirements,
        registry_name,
        registry.index_url.as_deref().unwrap(),
        &wheels_hash,
        name,
    );
    let cached = cache.join(format!("{key}.wasm"));
    if let Ok(bytes) = fs::read(&cached) {
        if let Ok(tool) = crate::runtime::Tool::load(&bytes) {
            if tool.package.tool.name == name {
                return Ok(bytes);
            }
        }
    }

    let mut install = Command::new(if cfg!(windows) { "python" } else { "python3" });
    install
        .args([
            "-m",
            "pip",
            "--isolated",
            "install",
            "--no-index",
            "--no-deps",
            "--only-binary=:all:",
            "--no-compile",
            "--target",
        ])
        .arg(&site)
        .arg("--find-links")
        .arg(&wheels)
        .args(&normalized_requirements);
    run(&mut install, "Python wheel installation")?;

    let modules = import_names(&site)?;
    ensure!(
        !modules.is_empty() || has_python_startup_hook(&site)?,
        "downloaded wheels expose no importable modules or Python startup hooks"
    );
    let app = include_str!("../../python-example/app.py");
    let imports = modules
        .iter()
        .map(|m| format!("import {m}"))
        .collect::<Vec<_>>()
        .join("\n");
    let app = app.replace("import wit_world", "# Process Python startup hooks from mounted package paths inside the WASI guest.\nimport site\nfor _sat_path in list(sys.path):\n    if _sat_path and __import__('os').path.isdir(_sat_path):\n        site.addsitedir(_sat_path)\n\nimport wit_world");
    fs::write(
        work.join("app.py"),
        format!(
            "{app}\n# Installed package imports are initialized inside the WASI guest.\n{imports}\n"
        ),
    )?;
    fs::create_dir_all(work.join("wit"))?;
    fs::write(
        work.join("wit/world.wit"),
        include_str!("../../python-example/wit/world.wit"),
    )?;
    let metadata = format!(
        "name = \"{name}\"\nversion = \"0.1.0\"\ndescription = \"Python environment built for WASI.\"\nentrypoint = \"run\"\nexactly_one_of = [\"code\", \"script\"]\n\n[[arguments]]\nname = \"script\"\nhelp = \"Python file to execute (requires a read grant).\"\n\n[options.code]\nshort = \"c\"\nhelp = \"Python code to execute.\"\n"
    );
    let raw = root.0.join("python.raw.wasm");
    let mut command = Command::new(componentizer);
    command
        .env_clear()
        .env("PATH", std::env::var_os("PATH").unwrap_or_default())
        .env("VIRTUAL_ENV", &venv)
        .current_dir(&work)
        .args([
            "-d",
            "wit",
            "-w",
            "python-tool",
            "componentize",
            "-p",
            ".",
            "app",
            "-o",
        ])
        .arg(&raw);
    run(&mut command, "WASI Python componentization")?;
    let bytes = crate::metadata::embed(
        fs::read(raw)?,
        &metadata,
        Some(include_str!("../../python-example/policy.toml")),
    )?;
    crate::runtime::Tool::load(&bytes)
        .context("generated Python component failed SAT validation")?;
    fs::write(cached, &bytes).context("cache built Python environment")?;
    Ok(bytes)
}
