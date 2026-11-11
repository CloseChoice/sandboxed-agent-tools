//! Embedded component runner. Guest authority comes only from effective settings.

use crate::{
    files::{self, FilesystemImport},
    metadata::valid_name,
};
use anyhow::{Context, Result, bail, ensure};
use clap::{Arg, ArgAction, ArgGroup, Command};
use serde_json::{Value as Json, json};
use std::{collections::BTreeSet, path::Path};
use wasmtime::component::{Component, Linker, ResourceTable, Type, Val, types::ComponentItem};
use wasmtime::{Config, Engine, Store};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
use wasmtime_wasi_http::{WasiHttpCtx, WasiHttpCtxView, WasiHttpView};

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
}

impl LoadedTool {
    pub fn load(bytes: &[u8], metadata: crate::metadata::ToolMetadata) -> Result<Self> {
        let mut config = Config::new();
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
                "MVP arguments support string, bool, scalar numbers and options of those types"
            );
        }
        let filesystem_imports = files::discover_filesystem_imports(&engine, &component)?;
        let mut names: BTreeSet<String> = ["help", "json"].map(str::to_owned).into();
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
        for argument in &metadata.arguments {
            let ty = &params.iter().find(|(n, _)| n == &argument.name).unwrap().1;
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
        for (i, spec) in self.metadata.arguments.iter().enumerate() {
            cmd = cmd.arg(
                Arg::new(spec.name.clone())
                    .index(i + 1)
                    .required(!self.is_optional_parameter(&spec.name))
                    .help(spec.help.clone()),
            );
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
        if args.iter().any(|a| a == "--help" || a == "-h") && args.iter().any(|a| a == "--json") {
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
        let matches = match self.command()?.try_get_matches_from(
            std::iter::once(self.metadata.name.clone()).chain(args.iter().cloned()),
        ) {
            Ok(m) => m,
            Err(e) if e.kind() == clap::error::ErrorKind::DisplayHelp => {
                print!("{e}");
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        };
        let params = self
            .params
            .iter()
            .map(|(n, t)| match matches.get_one::<String>(n) {
                Some(v) => parse_argument_value(t, v),
                None if matches!(t, Type::Option(_)) => Ok(Val::Option(None)),
                None => bail!("missing parameter {n}"),
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
        let single_file_preopens = crate::grants::configure_preopens(&mut wasi, &access)?;
        let mut store = Store::new(
            &self.engine,
            RuntimeState {
                wasi: wasi.build(),
                single_file_preopens,
                guest_cwd: None,
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
        let instance = self
            .linker
            .instantiate_async(&mut store, &self.component)
            .await?;
        let f = instance
            .get_func(&mut store, &self.metadata.entrypoint)
            .context("missing entrypoint")?;
        let mut results = vec![Val::Bool(false); f.ty(&store).results().len()];
        f.call_async(&mut store, &params, &mut results).await?;
        f.post_return_async(&mut store).await?;
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
            println!("{text}");
        } else {
            println!("{}", serde_json::to_string_pretty(&result)?);
        }
        Ok(())
    }
}

fn is_supported_argument_type(t: &Type) -> bool {
    if let Type::Option(inner) = t {
        return !matches!(inner.ty(), Type::Option(_)) && is_supported_argument_type(&inner.ty());
    }
    matches!(
        t,
        Type::String
            | Type::Bool
            | Type::U32
            | Type::U64
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
        Type::U32 => Val::U32(text.parse()?),
        Type::U64 => Val::U64(text.parse()?),
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
