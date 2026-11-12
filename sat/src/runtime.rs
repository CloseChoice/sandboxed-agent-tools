//! Embedded component runner. Guest authority comes only from effective settings.

use crate::{
    files::{self, FilesystemImport},
    metadata::valid_name,
};
use anyhow::{Context, Result, bail, ensure};
use clap::{Arg, ArgAction, ArgGroup, Command};
use serde_json::{Value as Json, json};
use std::{
    collections::BTreeSet,
    io::{Read, Write},
    path::Path,
};
use wasmtime::component::{Component, Linker, ResourceTable, Type, Val, types::ComponentItem};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::p2::pipe::{MemoryInputPipe, MemoryOutputPipe};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpCtxView, WasiHttpView};

#[derive(Debug)]
pub struct ToolExit(pub i32);
impl std::fmt::Display for ToolExit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "tool exited with status {}", self.0)
    }
}
impl std::error::Error for ToolExit {}

pub struct RuntimeState {
    pub(crate) wasi: WasiCtx,
    http: WasiHttpCtx,
    pub(crate) table: ResourceTable,
    pub(crate) granted_descriptors: Vec<(String, crate::grants::GrantedDescriptor)>,
    pub(crate) descriptor_paths: std::collections::BTreeMap<u32, String>,
    pub(crate) guest_cwd: Option<String>,
    pub(crate) single_file_preopens: Vec<(wasmtime_wasi::filesystem::File, String)>,
    http_permissions: crate::config::HttpPermissions,
    pub files: files::FilesystemBindings,
    pub(crate) access: crate::access::Access,
    pub(crate) directory_stream_paths: std::collections::BTreeMap<u32, String>,
}

impl WasiView for RuntimeState {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}

impl WasiHttpView for RuntimeState {
    fn http(&mut self) -> WasiHttpCtxView<'_> {
        WasiHttpCtxView {
            ctx: &mut self.http,
            table: &mut self.table,
            hooks: &mut self.http_permissions,
        }
    }
}

pub struct LoadedTool {
    pub metadata: crate::metadata::ToolMetadata,
    engine: Engine,
    component: Component,
    linker: Linker<RuntimeState>,
    pub params: Vec<(String, Type)>,
    pub filesystem_imports: Vec<FilesystemImport>,
    pub uses_http: bool,
    #[cfg(test)]
    pub(crate) compilation_cache: Option<wasmtime::Cache>,
}

impl LoadedTool {
    pub fn load(bytes: &[u8], metadata: crate::metadata::ToolMetadata) -> Result<Self> {
        Self::load_with_cache(bytes, metadata, None)
    }

    pub fn load_with_cache(
        bytes: &[u8],
        metadata: crate::metadata::ToolMetadata,
        cache_directory: Option<&Path>,
    ) -> Result<Self> {
        let mut config = Config::new();
        let compilation_cache = cache_directory
            .map(|directory| {
                let mut cache_config = wasmtime::CacheConfig::new();
                cache_config.with_directory(directory);
                wasmtime::Cache::new(cache_config)
            })
            .transpose()?;
        config.cache(compilation_cache.clone());
        config.consume_fuel(true);
        config.cranelift_opt_level(wasmtime::OptLevel::None);
        let engine = Engine::new(&config)?;
        let component = Component::new(&engine, bytes)?;
        let component_type = component.component_type();
        let export = component_type
            .get_export(&engine, &metadata.entrypoint)
            .context("entrypoint is not exported")?;
        let ComponentItem::ComponentFunc(func) = export.ty else {
            bail!("entrypoint must be a root function");
        };
        ensure!(!func.async_(), "MVP supports P2 synchronous exports");
        let params: Vec<_> = func.params().map(|(n, t)| (n.to_owned(), t)).collect();
        for (_, ty) in &params {
            ensure!(
                is_supported_argument_type(ty),
                "arguments support scalars, string lists, and options of those types"
            );
        }
        let filesystem_imports = files::discover_filesystem_imports(&engine, &component)?;
        if let Some(name) = &metadata.stdin {
            ensure!(
                params.iter().any(|(n, t)| n == name
                    && matches!(t, Type::Option(o) if matches!(o.ty(), Type::String))),
                "stdin must name an optional string parameter"
            );
        }
        ensure!(
            !(metadata.command_output && metadata.verbatim_output),
            "command_output conflicts with verbatim_output"
        );
        if metadata.command_output {
            ensure!(
                matches!(func.results().collect::<Vec<_>>().as_slice(), [Type::S32]),
                "command_output requires an s32 exit status"
            );
        }
        if metadata.cli_passthrough {
            ensure!(
                metadata.command_output
                    && metadata.stdin.is_none()
                    && matches!(params.as_slice(), [(name, Type::List(l))] if name == "args" && matches!(l.ty(), Type::String))
                    && filesystem_imports.is_empty(),
                "cli_passthrough requires run(args: list<string>) -> s32 and standard WASI imports only"
            );
        }
        let mut names: BTreeSet<String> = ["help", "json", "stdin"].map(str::to_owned).into();
        let mut shorts = BTreeSet::from(['h']);
        for option in metadata.options.values() {
            if let Some(short) = option.short {
                ensure!(
                    short.is_ascii_alphabetic() && shorts.insert(short),
                    "invalid or duplicate short option -{short}"
                );
            }
        }
        let mut mapped = BTreeSet::new();
        for name in metadata
            .arguments
            .iter()
            .map(|a| &a.name)
            .chain(metadata.options.keys())
        {
            ensure!(
                valid_name(name) && names.insert(name.clone()),
                "invalid or duplicate CLI name: {name}"
            );
            let (_, ty) = params
                .iter()
                .find(|(n, _)| n == name)
                .with_context(|| format!("unknown WIT parameter {name}"))?;
            if let Some(default) = metadata.options.get(name).and_then(|o| o.default.as_ref()) {
                parse_argument_value(ty, &format_default_value(default)?)?;
            }
            mapped.insert(name.clone());
        }
        ensure!(
            mapped.len() == params.len(),
            "metadata must describe every entrypoint parameter"
        );
        let mut optional_seen = false;
        for (index, argument) in metadata.arguments.iter().enumerate() {
            let ty = &params.iter().find(|(n, _)| n == &argument.name).unwrap().1;
            ensure!(
                !string_list_type(ty) || index + 1 == metadata.arguments.len(),
                "positional list must be last"
            );
            let is_optional_parameter = matches!(ty, Type::Option(_));
            ensure!(
                !optional_seen || is_optional_parameter,
                "required positional arguments must precede optional ones"
            );
            optional_seen |= is_optional_parameter;
        }
        let choice = &metadata.exactly_one_of;
        if !choice.is_empty() {
            ensure!(
                choice.len() >= 2,
                "exactly_one_of requires at least two parameters"
            );
            let mut unique = BTreeSet::new();
            for name in choice {
                ensure!(
                    unique.insert(name)
                        && params
                            .iter()
                            .any(|(n, t)| n == name && matches!(t, Type::Option(_))),
                    "exactly_one_of must name distinct optional WIT parameters: {name}"
                );
                ensure!(
                    metadata
                        .options
                        .get(name)
                        .is_none_or(|o| o.default.is_none()),
                    "exactly_one_of parameters cannot have defaults"
                );
            }
        }
        for filesystem_import in &filesystem_imports {
            ensure!(
                valid_name(&filesystem_import.name) && names.insert(filesystem_import.name.clone()),
                "invalid or conflicting resource name {}",
                filesystem_import.name
            );
        }
        let uses_http = component_type
            .imports(&engine)
            .any(|(n, _)| n.starts_with("wasi:http/outgoing-handler@"));
        let mut linker = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
        wasmtime_wasi_http::p2::add_only_http_to_linker_async(&mut linker)?;
        files::link_filesystem_imports(&mut linker, &filesystem_imports)?;
        crate::grants::link_filesystem_permissions(&mut linker)?;
        // Resolves imports and checks their types without executing guest code.
        linker.instantiate_pre(&component)?;
        Ok(Self {
            metadata,
            engine,
            component,
            linker,
            params,
            filesystem_imports,
            uses_http,
            #[cfg(test)]
            compilation_cache,
        })
    }

    fn is_optional_parameter(&self, name: &str) -> bool {
        self.params
            .iter()
            .any(|(n, t)| n == name && matches!(t, Type::Option(_)))
    }

    pub fn command(&self) -> Result<Command> {
        let mut cmd = Command::new(self.metadata.name.clone())
            .about(self.metadata.description.clone())
            .version(self.metadata.version.clone())
            .disable_version_flag(true)
            .arg(
                Arg::new("json")
                    .long("json")
                    .action(ArgAction::SetTrue)
                    .help("Print structured output/help"),
            );
        if self.metadata.stdin.is_some() {
            cmd = cmd.arg(
                Arg::new("stdin")
                    .long("stdin")
                    .action(ArgAction::SetTrue)
                    .conflicts_with(self.metadata.stdin.clone().unwrap())
                    .help("Read UTF-8 input from host stdin"),
            );
        }
        for (i, spec) in self.metadata.arguments.iter().enumerate() {
            let mut arg = Arg::new(spec.name.clone())
                .index(i + 1)
                .required(!self.is_optional_parameter(&spec.name))
                .help(spec.help.clone());
            if string_list_type(&self.params.iter().find(|(n, _)| n == &spec.name).unwrap().1) {
                arg = arg
                    .num_args(1..)
                    .allow_hyphen_values(true)
                    .trailing_var_arg(true);
            }
            cmd = cmd.arg(arg);
        }
        for (name, spec) in &self.metadata.options {
            let mut arg = Arg::new(name.clone())
                .long(name.clone())
                .help(spec.help.clone());
            if let Some(short) = spec.short {
                arg = arg.short(short);
            }
            if let Some(value) = &spec.default {
                arg = arg.default_value(format_default_value(value)?);
            } else {
                arg = arg.required(!self.is_optional_parameter(name));
            }
            if string_list_type(&self.params.iter().find(|(n, _)| n == name).unwrap().1) {
                arg = arg.action(ArgAction::Append);
            }
            cmd = cmd.arg(arg);
        }
        if !self.metadata.exactly_one_of.is_empty() {
            cmd = cmd.group(
                ArgGroup::new("@sat:exactly-one")
                    .args(self.metadata.exactly_one_of.clone())
                    .required(true)
                    .multiple(false),
            );
        }
        for filesystem_import in &self.filesystem_imports {
            cmd = cmd.arg(
                Arg::new(filesystem_import.name.clone())
                    .long(filesystem_import.name.clone())
                    .required(true)
                    .value_name("PATH")
                    .help(format!(
                        "Bind a {} ({})",
                        if filesystem_import.directory {
                            "directory"
                        } else {
                            "file"
                        },
                        if filesystem_import.write {
                            "requires a matching write grant"
                        } else {
                            "read-only"
                        }
                    )),
            );
        }
        if !self.metadata.examples.is_empty() {
            cmd = cmd.after_help(format!(
                "Examples:\n{}",
                self.metadata
                    .examples
                    .iter()
                    .map(|e| format!("  {}", e.command))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
        Ok(cmd)
    }

    pub async fn run(
        &self,
        args: &[String],
        settings: &crate::config::ToolSettings,
        home: &Path,
        cwd: &Path,
    ) -> Result<()> {
        if !self.metadata.cli_passthrough
            && args.iter().any(|a| a == "--help" || a == "-h")
            && args.iter().any(|a| a == "--json")
        {
            println!(
                "{}",
                serde_json::to_string_pretty(&json!({
                    "tool": self.metadata,
                    "parameters": self.params.iter().map(|(name,t)| json!({"name":name,"type":format!("{t:?}")})).collect::<Vec<_>>(),
                    "resources": self.filesystem_imports, "uses_http": self.uses_http,
                    "permissions": ["--read", "--write", "--deny-read", "--deny-write", "--allow-http", "--deny-http"],
                }))?
            );
            return Ok(());
        }
        let matches = if self.metadata.cli_passthrough {
            // Upstream parses every tool argument, including --help and --version.
            Command::new("internal")
                .arg(Arg::new("json").long("json").action(ArgAction::SetTrue))
                .try_get_matches_from(["internal"])?
        } else {
            match self.command()?.try_get_matches_from(
                std::iter::once(self.metadata.name.clone()).chain(args.iter().cloned()),
            ) {
                Ok(m) => m,
                Err(e) if e.kind() == clap::error::ErrorKind::DisplayHelp => {
                    print!("{e}");
                    return Ok(());
                }
                Err(e) => return Err(e.into()),
            }
        };
        let stdin_text = if self.metadata.stdin.is_some() && matches.get_flag("stdin") {
            let mut text = String::new();
            std::io::stdin()
                .take(16 * 1024 * 1024 + 1)
                .read_to_string(&mut text)?;
            ensure!(text.len() <= 16 * 1024 * 1024, "stdin exceeds 16 MiB");
            Some(text)
        } else {
            None
        };
        let input = self.metadata.stdin.as_ref().and_then(|name| {
            stdin_text
                .clone()
                .or_else(|| matches.get_one::<String>(name).cloned())
        });
        let params = self
            .params
            .iter()
            .map(|(n, t)| {
                if self.metadata.cli_passthrough {
                    return Ok(Val::List(args.iter().cloned().map(Val::String).collect()));
                }
                if self.metadata.stdin.as_ref() == Some(n)
                    && let Some(text) = &input
                {
                    return parse_argument_value(t, text);
                }
                if string_list_type(t) {
                    let values = matches
                        .get_many::<String>(n)
                        .map(|v| v.map(|s| Val::String(s.clone())).collect::<Vec<_>>());
                    return match (t, values) {
                        (Type::Option(_), Some(v)) => Ok(Val::Option(Some(Box::new(Val::List(v))))),
                        (Type::Option(_), None) => Ok(Val::Option(None)),
                        (_, Some(v)) => Ok(Val::List(v)),
                        _ => bail!("missing parameter {n}"),
                    };
                }
                match matches.get_one::<String>(n) {
                    Some(v) => parse_argument_value(t, v),
                    None if matches!(t, Type::Option(_)) => Ok(Val::Option(None)),
                    None => bail!("missing parameter {n}"),
                }
            })
            .collect::<Result<Vec<_>>>()?;
        let access = crate::access::Access::new(&settings.file_permissions, home, cwd)?;
        let mut bindings = files::FilesystemBindings::new();
        for filesystem_import in &self.filesystem_imports {
            bindings.insert(
                filesystem_import.name.clone(),
                files::bind_filesystem_import(
                    filesystem_import,
                    matches.get_one::<String>(&filesystem_import.name).unwrap(),
                    &access,
                    home,
                    cwd,
                )?,
            );
        }
        let mut wasi = WasiCtxBuilder::new();
        wasi.allow_tcp(false)
            .allow_udp(false)
            .allow_ip_name_lookup(false);
        let stdout = MemoryOutputPipe::new(16 * 1024 * 1024);
        let stderr = MemoryOutputPipe::new(1024 * 1024);
        if self.metadata.command_output {
            // Upstream coreutils also consult WASI argv for the utility name.
            wasi.arg(&self.metadata.name);
            if self.metadata.cli_passthrough {
                wasi.args(args);
            }
            for argument in self
                .metadata
                .arguments
                .iter()
                .filter(|_| !self.metadata.cli_passthrough)
            {
                if string_list_type(
                    &self
                        .params
                        .iter()
                        .find(|(n, _)| n == &argument.name)
                        .unwrap()
                        .1,
                ) && let Some(values) = matches.get_many::<String>(&argument.name)
                {
                    wasi.args(&values.cloned().collect::<Vec<_>>());
                }
            }
            wasi.stdin(MemoryInputPipe::new(input.unwrap_or_default().into_bytes()))
                .stdout(stdout.clone())
                .stderr(stderr.clone());
        }
        let guest_cwd = if self.metadata.cli_passthrough {
            // A synthetic cwd supplies path resolution, never ambient directory access.
            let path = crate::access::convert_to_guest_path(cwd)?;
            wasi.env("PWD", &path).inherit_stdin();
            Some(path)
        } else {
            None
        };
        let single_file_preopens = crate::grants::configure_preopens(&mut wasi, &access)?;
        let mut store = Store::new(
            &self.engine,
            RuntimeState {
                wasi: wasi.build(),
                single_file_preopens,
                guest_cwd,
                granted_descriptors: Vec::new(),
                descriptor_paths: Default::default(),
                directory_stream_paths: Default::default(),
                http: WasiHttpCtx::new(),
                table: ResourceTable::new(),
                http_permissions: settings.http_permissions.clone(),
                files: bindings,
                access,
            },
        );
        // Package metadata may lower the budget, never raise the host ceiling.
        store.set_fuel(
            self.metadata
                .fuel
                .unwrap_or(1_000_000_000)
                .min(1_000_000_000),
        )?;
        let instance = self
            .linker
            .instantiate_async(&mut store, &self.component)
            .await?;
        let f = instance
            .get_func(&mut store, &self.metadata.entrypoint)
            .context("missing entrypoint")?;
        let mut results = vec![Val::Bool(false); f.ty(&store).results().len()];
        if let Err(error) = f.call_async(&mut store, &params, &mut results).await {
            if self.metadata.command_output {
                std::io::stderr().write_all(&stderr.contents())?;
            }
            return Err(error.into());
        }
        f.post_return_async(&mut store).await?;
        if self.metadata.command_output {
            std::io::stderr().write_all(&stderr.contents())?;
            let [Val::S32(status)] = results.as_slice() else {
                bail!("expected command exit status")
            };

            if matches.get_flag("json") {
                println!(
                    "{}",
                    serde_json::to_string(&String::from_utf8(stdout.contents().to_vec())?)?
                );
            } else {
                std::io::stdout().write_all(&stdout.contents())?;
            }
            if *status != 0 {
                return Err(ToolExit(*status).into());
            }
            return Ok(());
        }
        let mut output = results
            .iter()
            .map(component_value_to_json)
            .collect::<Result<Vec<_>>>()?;
        let result = if output.len() == 1 {
            output.remove(0)
        } else {
            Json::Array(output)
        };
        if !matches.get_flag("json")
            && let Json::String(text) = &result
        {
            if self.metadata.verbatim_output {
                print!("{text}");
            } else {
                println!("{text}");
            }
        } else {
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Ok(())
    }
}

fn string_list_type(t: &Type) -> bool {
    match t {
        Type::List(l) => matches!(l.ty(), Type::String),
        Type::Option(o) => matches!(o.ty(), Type::List(l) if matches!(l.ty(), Type::String)),
        _ => false,
    }
}

fn is_supported_argument_type(t: &Type) -> bool {
    if string_list_type(t) {
        return true;
    }
    if let Type::Option(inner) = t {
        return !matches!(inner.ty(), Type::Option(_)) && is_supported_argument_type(&inner.ty());
    }
    matches!(
        t,
        Type::String
            | Type::Bool
            | Type::U8
            | Type::U16
            | Type::U32
            | Type::U64
            | Type::S8
            | Type::S16
            | Type::S32
            | Type::S64
            | Type::Float32
            | Type::Float64
    )
}

fn format_default_value(v: &toml::Value) -> Result<String> {
    Ok(match v {
        toml::Value::String(s) => s.clone(),
        toml::Value::Integer(_) | toml::Value::Boolean(_) | toml::Value::Float(_) => v.to_string(),
        _ => bail!("default must be a scalar"),
    })
}

fn parse_argument_value(ty: &Type, text: &str) -> Result<Val> {
    Ok(match ty {
        Type::Option(inner) => {
            Val::Option(Some(Box::new(parse_argument_value(&inner.ty(), text)?)))
        }
        Type::String => Val::String(text.into()),
        Type::Bool => Val::Bool(text.parse()?),
        Type::U8 => Val::U8(text.parse()?),
        Type::U16 => Val::U16(text.parse()?),
        Type::U32 => Val::U32(text.parse()?),
        Type::U64 => Val::U64(text.parse()?),
        Type::S8 => Val::S8(text.parse()?),
        Type::S16 => Val::S16(text.parse()?),
        Type::S32 => Val::S32(text.parse()?),
        Type::S64 => Val::S64(text.parse()?),
        Type::Float32 => Val::Float32(text.parse()?),
        Type::Float64 => Val::Float64(text.parse()?),
        _ => bail!("unsupported CLI argument type"),
    })
}

fn component_value_to_json(v: &Val) -> Result<Json> {
    Ok(match v {
        Val::String(s) => json!(s),
        Val::Bool(v) => json!(v),
        Val::U8(v) => json!(v),
        Val::U16(v) => json!(v),
        Val::U32(v) => json!(v),
        Val::U64(v) => json!(v),
        Val::S8(v) => json!(v),
        Val::S16(v) => json!(v),
        Val::S32(v) => json!(v),
        Val::S64(v) => json!(v),
        Val::Float32(v) => json!(v),
        Val::Float64(v) => json!(v),
        Val::List(items) | Val::Tuple(items) => Json::Array(
            items
                .iter()
                .map(component_value_to_json)
                .collect::<Result<_>>()?,
        ),
        Val::Record(fields) => Json::Object(
            fields
                .iter()
                .map(|(k, v)| Ok((k.clone(), component_value_to_json(v)?)))
                .collect::<Result<_>>()?,
        ),
        Val::Result(Ok(Some(v))) | Val::Option(Some(v)) => component_value_to_json(v)?,
        Val::Result(Ok(None)) | Val::Option(None) => Json::Null,
        Val::Result(Err(v)) => bail!(
            "tool error: {}",
            v.as_ref()
                .map(|v| component_value_to_json(v))
                .transpose()?
                .unwrap_or(Json::Null)
        ),
        _ => bail!("unsupported output type"),
    })
}

#[cfg(test)]
mod imported_tool_tests {
    use super::*;

    fn metadata(extra: &str) -> crate::metadata::ToolMetadata {
        crate::metadata::ToolMetadata::parse(&format!(
            "name = 'probe'\nversion = '1'\ndescription = 'probe'\nentrypoint = 'run'\n{extra}"
        ))
        .unwrap()
    }

    fn component(body: &str) -> Vec<u8> {
        wat::parse_str(format!(
            r#"(component
            (core module $m (func (export "run") (result i32) {body}))
            (core instance $i (instantiate $m))
            (func (export "run") (result s32) (canon lift (core func $i "run")))
        )"#
        ))
        .unwrap()
    }

    #[test]
    fn small_integer_arguments_reject_overflow() {
        for (ty, valid, invalid) in [
            (Type::U8, "255", "256"),
            (Type::U16, "65535", "65536"),
            (Type::S8, "-128", "-129"),
            (Type::S16, "-32768", "-32769"),
        ] {
            assert!(is_supported_argument_type(&ty));
            assert!(parse_argument_value(&ty, valid).is_ok());
            assert!(parse_argument_value(&ty, invalid).is_err());
        }
    }

    #[test]
    fn rejects_invalid_stdin_and_conflicting_output_modes() {
        let bytes = component("i32.const 0");
        assert!(LoadedTool::load(&bytes, metadata("stdin = 'missing'")).is_err());
        assert!(
            LoadedTool::load(
                &bytes,
                metadata("command_output = true\nverbatim_output = true")
            )
            .is_err()
        );
        assert!(LoadedTool::load(b"invalid component", metadata("")).is_err());
        assert!(
            LoadedTool::load(
                &bytes,
                metadata("command_output = true\ncli_passthrough = true")
            )
            .is_err()
        );
    }

    #[tokio::test]
    async fn guest_loop_exhausts_host_enforced_fuel() {
        let bytes = component("(loop $spin (br $spin)) i32.const 0");
        let tool = LoadedTool::load(&bytes, metadata("fuel = 1000")).unwrap();
        let directory = tempfile::tempdir().unwrap();
        let error = tool
            .run(&[], &Default::default(), directory.path(), directory.path())
            .await
            .unwrap_err();
        assert!(format!("{error:#}").contains("fuel"), "{error:#}");
    }
}
