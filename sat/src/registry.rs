//! Three-file registry; archives are data and are never executed at registration.

use crate::{
    config::{self, Configuration, ToolSettings},
    metadata::{self, ToolMetadata},
    runtime,
};
use anyhow::{Context, Result, bail, ensure};
use std::{
    collections::BTreeMap,
    fs,
    io::{BufRead, Read, Write},
    path::{Path, PathBuf},
};

const REQUIRED_PACKAGE_FILES: [&str; 3] = ["tool.wasm", "tool.toml", "defaults.toml"];

pub fn registered_tool_names(home: &Path) -> Result<Vec<String>> {
    let root = home.join("tools");
    if !root.exists() {
        return Ok(vec![]);
    }
    let mut names = vec![];
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        if entry.file_type()?.is_dir() && entry.path().join("tool.toml").is_file() {
            let name = entry.file_name().to_string_lossy().into_owned();
            ensure!(metadata::valid_name(&name), "invalid registered tool name");
            names.push(name);
        }
    }
    names.sort();
    Ok(names)
}

pub fn registered_tool_directory(home: &Path, name: &str) -> Result<PathBuf> {
    ensure!(
        metadata::valid_name(name) && !metadata::RESERVED.contains(&name),
        "invalid tool name"
    );
    let root = home.join("tools");
    let dir = root.join(name);
    let canonical =
        fs::canonicalize(&dir).with_context(|| format!("tool {name} is not registered"))?;
    ensure!(
        canonical.parent() == Some(fs::canonicalize(root)?.as_path())
            && !fs::symlink_metadata(&dir)?.file_type().is_symlink(),
        "invalid tool directory"
    );
    Ok(dir)
}

pub fn load(home: &Path, name: &str) -> Result<runtime::LoadedTool> {
    let dir = registered_tool_directory(home, name)?;
    let metadata = ToolMetadata::parse(&fs::read_to_string(dir.join("tool.toml"))?)?;
    ensure!(metadata.name == name, "registered tool name mismatch");
    runtime::LoadedTool::load_with_cache(
        &fs::read(dir.join("tool.wasm"))?,
        metadata,
        Some(&home.join(".cache")),
    )
}

fn read_archive_entry(
    files: &mut BTreeMap<String, Vec<u8>>,
    name: &str,
    mut entry: impl Read,
) -> Result<()> {
    ensure!(!files.contains_key(name), "duplicate archive entry {name}");
    let mut bytes = vec![];
    entry.read_to_end(&mut bytes)?;
    files.insert(name.into(), bytes);
    Ok(())
}

pub fn extract(path: &Path, zip: bool, destination: &Path) -> Result<()> {
    let mut files = BTreeMap::new();
    if zip {
        let mut archive = zip::ZipArchive::new(fs::File::open(path)?)?;
        for i in 0..archive.len() {
            let entry = archive.by_index(i)?;
            if REQUIRED_PACKAGE_FILES.contains(&entry.name()) {
                let mode = entry.unix_mode().unwrap_or(0o100644) & 0o170000;
                ensure!(
                    !entry.is_dir() && (mode == 0 || mode == 0o100000),
                    "required archive entries must be regular files"
                );
                let name = entry.name().to_owned();
                read_archive_entry(&mut files, &name, entry)?;
            }
        }
    } else {
        let mut archive = tar::Archive::new(fs::File::open(path)?);
        for entry in archive.entries()? {
            let entry = entry?;
            let name = entry.path_bytes();
            if let Some(expected) = REQUIRED_PACKAGE_FILES
                .iter()
                .find(|n| n.as_bytes() == name.as_ref())
            {
                ensure!(
                    entry.header().entry_type().is_file(),
                    "required archive entries must be regular files"
                );
                read_archive_entry(&mut files, expected, entry)?;
            }
        }
    }
    for name in REQUIRED_PACKAGE_FILES {
        ensure!(
            files.contains_key(name),
            "archive missing root-level {name}"
        );
    }
    for (name, bytes) in files {
        fs::write(destination.join(name), bytes)?;
    }
    Ok(())
}

fn parse_suggested_permissions(text: &str) -> Result<ToolSettings> {
    let mut value: toml::Value = toml::from_str(text)?;
    let root = value
        .as_table_mut()
        .context("defaults must be a TOML table")?;
    ensure!(
        root.keys()
            .all(|k| ["file_permissions", "http_permissions"].contains(&k.as_str())),
        "defaults only support permission tables"
    );
    let file = root
        .entry("file_permissions")
        .or_insert_with(|| toml::Value::Table(Default::default()))
        .as_table_mut()
        .context("file_permissions must be a table")?;
    for name in ["read", "write", "deny-read", "deny-write"] {
        file.entry(name)
            .or_insert_with(|| toml::Value::Array(vec![]));
    }
    root.entry("http_permissions").or_insert_with(|| {
        toml::Value::Table(toml::map::Map::from_iter([(
            "allow-http".into(),
            toml::Value::Array(vec![]),
        )]))
    });
    let settings: ToolSettings = value.try_into()?;
    settings.http_permissions.validate()?;
    Ok(settings)
}

fn format_permission_list(value: &Option<Vec<String>>) -> String {
    value
        .as_ref()
        .map(|v| serde_json::to_string(v).unwrap())
        .unwrap_or_else(|| "unset".into())
}

fn prompt_permission_list(
    input: &mut impl BufRead,
    output: &mut impl Write,
    name: &str,
    suggestion: &Option<Vec<String>>,
    http: bool,
) -> Result<Option<Vec<String>>> {
    loop {
        write!(
            output,
            "{name} = {} [Y to accept, array to replace{}]: ",
            format_permission_list(suggestion),
            if http {
                ", unset to disable this list"
            } else {
                ""
            }
        )?;
        output.flush()?;
        let mut line = String::new();
        ensure!(
            input.read_line(&mut line)? != 0,
            "registration cancelled: input ended"
        );
        let line = line.trim();
        if line.eq_ignore_ascii_case("y") {
            return Ok(suggestion.clone());
        }
        if http && line == "unset" {
            return Ok(None);
        }
        if let Ok(table) = toml::from_str::<toml::Table>(&format!("value = {line}")) {
            if let Some(toml::Value::Array(array)) = table.get("value") {
                if array.iter().all(|v| v.is_str()) && table.len() == 1 {
                    return Ok(Some(
                        array
                            .iter()
                            .map(|v| v.as_str().unwrap().to_owned())
                            .collect(),
                    ));
                }
            }
        }
        writeln!(
            output,
            "Enter Y, a TOML array of strings{}, or cancel with Ctrl-C.",
            if http { ", or unset" } else { "" }
        )?;
    }
}

fn review_suggested_permissions(
    mut settings: ToolSettings,
    home: &Path,
    cwd: &Path,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<ToolSettings> {
    writeln!(
        output,
        "Suggested permissions:\nread = {}\nwrite = {}\ndeny-read = {}\ndeny-write = {}\nallow-http = {}\ndeny-http = {}",
        format_permission_list(&Some(settings.file_permissions.read.clone())),
        format_permission_list(&Some(settings.file_permissions.write.clone())),
        format_permission_list(&Some(settings.file_permissions.deny_read.clone())),
        format_permission_list(&Some(settings.file_permissions.deny_write.clone())),
        format_permission_list(&settings.http_permissions.allow_http),
        format_permission_list(&settings.http_permissions.deny_http)
    )?;
    for (name, value, deny) in [
        ("read", &mut settings.file_permissions.read, false),
        ("write", &mut settings.file_permissions.write, false),
        ("deny-read", &mut settings.file_permissions.deny_read, true),
        (
            "deny-write",
            &mut settings.file_permissions.deny_write,
            true,
        ),
    ] {
        loop {
            let chosen =
                prompt_permission_list(input, output, name, &Some(value.clone()), false)?.unwrap();
            let valid = if deny {
                crate::access::match_patterns(&chosen).map(|_| ())
            } else {
                chosen.iter().try_for_each(|p| {
                    let path = crate::access::expand_path(p, home, cwd)?;
                    ensure!(path.exists(), "grant path must exist");
                    Ok(())
                })
            };
            match valid {
                Ok(()) => {
                    *value = chosen;
                    break;
                }
                Err(e) => writeln!(output, "Invalid {name}: {e}")?,
            }
        }
    }
    loop {
        settings.http_permissions.allow_http = prompt_permission_list(
            input,
            output,
            "allow-http",
            &settings.http_permissions.allow_http,
            true,
        )?;
        settings.http_permissions.deny_http = prompt_permission_list(
            input,
            output,
            "deny-http",
            &settings.http_permissions.deny_http,
            true,
        )?;
        match settings.http_permissions.validate() {
            Ok(()) => break,
            Err(e) => writeln!(
                output,
                "Invalid HTTP permissions: {e}; revise both HTTP entries."
            )?,
        }
    }
    settings.validate(home, cwd)?;
    Ok(settings)
}

pub async fn register(
    source: &str,
    force: bool,
    home: &Path,
    cwd: &Path,
    input: &mut impl BufRead,
    output: &mut impl Write,
) -> Result<()> {
    let temporary = tempfile::tempdir()?;
    let url = reqwest::Url::parse(source)
        .ok()
        .filter(|u| matches!(u.scheme(), "http" | "https"));
    let (archive, extension) = if let Some(url) = url {
        let extension = Path::new(url.path())
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_owned();
        ensure!(
            ["tar", "zip"].contains(&extension.as_str()),
            "archive must be .tar or .zip"
        );
        let path = temporary.path().join("download");
        let mut response = reqwest::Client::new()
            .get(url)
            .send()
            .await?
            .error_for_status()?;
        let mut file = fs::File::create(&path)?;
        while let Some(chunk) = response.chunk().await? {
            file.write_all(&chunk)?;
        }
        (path, extension)
    } else {
        let path = PathBuf::from(source);
        ensure!(
            path.is_file(),
            "archive must be a local regular file or HTTP(S) URL"
        );
        let extension = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_owned();
        (path, extension)
    };
    ensure!(
        ["tar", "zip"].contains(&extension.as_str()),
        "archive must be .tar or .zip"
    );
    extract(&archive, extension == "zip", temporary.path())?;
    let meta = ToolMetadata::parse(&fs::read_to_string(temporary.path().join("tool.toml"))?)?;
    let name = meta.name.clone();
    let tools = home.join("tools");
    let destination = tools.join(&name);
    ensure!(
        !destination.try_exists()? || force,
        "tool {name} already exists; use --force (-f) to overwrite"
    );
    writeln!(output, "Compiling and validating {name} for this host...")?;
    output.flush()?;
    runtime::LoadedTool::load_with_cache(
        &fs::read(temporary.path().join("tool.wasm"))?,
        meta,
        Some(&home.join(".cache")),
    )?;
    let suggested =
        parse_suggested_permissions(&fs::read_to_string(temporary.path().join("defaults.toml"))?)?;
    // Validate configuration before prompting or changing any installed state.
    let mut local = Configuration::read(&config::project_config(cwd))?;
    let mut effective = Configuration::effective(home, cwd)?;
    let chosen = review_suggested_permissions(suggested, home, cwd, input, output)?;
    effective.tools.insert(name.clone(), chosen.clone());
    let mut installed = registered_tool_names(home)?;
    if !installed.contains(&name) {
        installed.push(name.clone());
    }
    effective.check_aliases(&installed)?;
    local.tools.insert(name.clone(), chosen);
    fs::create_dir_all(&tools)?;
    let stage = tempfile::tempdir_in(&tools)?;
    let staged_tool = stage.path().join("new");
    fs::create_dir(&staged_tool)?;
    for file in REQUIRED_PACKAGE_FILES {
        fs::copy(temporary.path().join(file), staged_tool.join(file))?;
    }
    let backup = stage.path().join("old");
    let existed = destination.try_exists()?;
    if existed {
        fs::rename(&destination, &backup)?;
    }
    if let Err(e) = fs::rename(&staged_tool, &destination) {
        if existed {
            fs::rename(&backup, &destination)?;
        }
        return Err(e.into());
    }
    if let Err(e) = config::save(&config::project_config(cwd), &local) {
        fs::remove_dir_all(&destination)?;
        if existed {
            fs::rename(&backup, &destination)?;
        }
        return Err(e);
    }
    writeln!(output, "Registered {name} (compiled and ready to run)")?;
    Ok(())
}

pub fn unregister(name: &str, home: &Path, cwd: &Path) -> Result<()> {
    let dir = registered_tool_directory(home, name)?;
    let paths = [home.join("config.toml"), config::project_config(cwd)];
    let mut changes = vec![];
    for path in &paths {
        let mut c = Configuration::read(path)?;
        if c.tools.remove(name).is_some() {
            changes.push((path.clone(), fs::read(path)?, c));
        }
    }
    let temporary = tempfile::tempdir_in(home.join("tools"))?;
    let backup = temporary.path().join("tool");
    fs::rename(&dir, &backup)?;
    for (i, (path, _, c)) in changes.iter().enumerate() {
        if let Err(e) = config::save(path, c) {
            for (path, bytes, _) in changes.iter().take(i) {
                fs::write(path, bytes)?;
            }
            fs::rename(&backup, &dir)?;
            return Err(e);
        }
    }
    Ok(())
}

#[cfg(test)]
mod compilation_tests {
    use super::*;

    #[tokio::test]
    async fn registration_compiles_before_first_execution() {
        let temporary = tempfile::tempdir().unwrap();
        let home = temporary.path().join("registry");
        let cwd = temporary.path().join("project");
        fs::create_dir(&cwd).unwrap();
        let archive = temporary.path().join("probe.tar");
        let wasm = wat::parse_str(
            r#"(component
            (core module $m (func (export "run") (result i32) i32.const 0))
            (core instance $i (instantiate $m))
            (func (export "run") (result s32) (canon lift (core func $i "run")))
        )"#,
        )
        .unwrap();
        let mut builder = tar::Builder::new(fs::File::create(&archive).unwrap());
        for (name, bytes) in [
            ("tool.wasm", wasm.as_slice()),
            (
                "tool.toml",
                b"name='probe'\nversion='1'\ndescription='probe'\nentrypoint='run'".as_slice(),
            ),
            ("defaults.toml", b"".as_slice()),
        ] {
            let mut header = tar::Header::new_gnu();
            header.set_size(bytes.len() as u64);
            header.set_mode(0o644);
            header.set_cksum();
            builder.append_data(&mut header, name, bytes).unwrap();
        }
        builder.finish().unwrap();
        let mut output = Vec::new();
        register(
            archive.to_str().unwrap(),
            false,
            &home,
            &cwd,
            &mut std::io::Cursor::new(b"y\ny\ny\ny\ny\ny\n"),
            &mut output,
        )
        .await
        .unwrap();
        assert!(
            String::from_utf8(output)
                .unwrap()
                .contains("compiled and ready to run")
        );
        // A fresh engine loads from the on-disk cache populated by registration.
        let tool = load(&home, "probe").unwrap();
        let cache = tool.compilation_cache.as_ref().unwrap();
        assert!(cache.cache_hits() > 0);
        assert_eq!(cache.cache_misses(), 0);
        tool.run(&[], &ToolSettings::default(), &home, &cwd)
            .await
            .unwrap();
    }
}
