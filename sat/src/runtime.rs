use crate::{
    files::{self, Capability},
    metadata::{Package, Policy, valid_name},
};
use anyhow::{Context, Result, bail, ensure};
use clap::{Arg, ArgAction, ArgGroup, Command};
use http::{Request, Response};
use http_body_util::BodyExt;
use serde_json::{Value as Json, json};
use std::{collections::BTreeSet, future::Future, path::Path, time::Duration};
use wasmtime::component::{Component, Linker, ResourceTable, Type, Val, types::ComponentItem};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
use wasmtime_wasi_http::{
    Error, RequestOptions, WasiBody, WasiHttpCtx, WasiHttpCtxView, WasiHttpHooks, WasiHttpView,
};

type IoFuture = Box<dyn Future<Output = Result<(), Error>> + Send>;
type ResponseFuture =
    Box<dyn Future<Output = Result<(Response<WasiBody>, IoFuture), Error>> + Send>;

#[derive(Debug)]
pub struct CommandExit(pub u8);
impl std::fmt::Display for CommandExit {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "command exited with status {}", self.0)
    }
}
impl std::error::Error for CommandExit {}

const MAX_COMMAND_OUTPUT: usize = 8 * 1024 * 1024;
const EXECUTION_TIMEOUT: Duration = Duration::from_secs(30);
const EPOCH_INTERVAL: Duration = Duration::from_millis(10);
const MAX_STDIN_BYTES: usize = 8 * 1024 * 1024;

fn read_stdin_text(reader: impl std::io::Read) -> Result<String> {
    use std::io::Read as _;
    let mut bytes = Vec::new();
    reader
        .take((MAX_STDIN_BYTES + 1) as u64)
        .read_to_end(&mut bytes)?;
    ensure!(bytes.len() <= MAX_STDIN_BYTES, "stdin exceeds 8 MiB limit");
    String::from_utf8(bytes).context("stdin must be UTF-8")
}

// A separate OS thread keeps ticking even if a CPU-bound guest occupies the
// executor's only thread. Dropping an invocation wakes and joins its ticker.
struct EpochTicker {
    stop: std::sync::mpsc::Sender<()>,
    thread: Option<std::thread::JoinHandle<()>>,
}
impl EpochTicker {
    fn start(engine: &Engine) -> Result<Self> {
        let (stop, receiver) = std::sync::mpsc::channel();
        let engine = engine.clone();
        let thread = std::thread::Builder::new()
            .name("sat-epoch-clock".into())
            .spawn(move || {
                while matches!(
                    receiver.recv_timeout(EPOCH_INTERVAL),
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout)
                ) {
                    engine.increment_epoch();
                }
            })
            .context("cannot start guest execution clock")?;
        Ok(Self {
            stop,
            thread: Some(thread),
        })
    }
}
impl Drop for EpochTicker {
    fn drop(&mut self) {
        let _ = self.stop.send(());
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

fn configure_epoch_deadline<T: 'static>(store: &mut Store<T>) {
    store.set_epoch_deadline(1);
    // Yield at every tick so Tokio can enforce the wall-clock deadline even
    // when guest code never calls a host function or otherwise suspends.
    store.epoch_deadline_async_yield_and_update(1);
}

async fn execute_with_deadline<T>(
    engine: &Engine,
    limit: Duration,
    execution: impl Future<Output = Result<T>>,
) -> Result<T> {
    let _ticker = EpochTicker::start(engine)?;
    tokio::time::timeout(limit, execution)
        .await
        .with_context(|| format!("tool exceeded {} seconds", limit.as_secs_f64()))?
}

pub struct HttpPolicy {
    pub allowed: bool,
    pub package: Policy,
    pub(crate) credential: Option<crate::credentials::Credential>,
    #[cfg(test)]
    fixture: Option<&'static [u8]>,
}
impl HttpPolicy {
    pub fn permits(&self, method: &http::Method, uri: &http::Uri) -> bool {
        if !self.allowed
            || !matches!(uri.scheme_str(), Some("http" | "https"))
            || uri.authority().is_none_or(|a| a.as_str().contains('@'))
        {
            return false;
        }
        let port = |u: &http::Uri| {
            u.port_u16().unwrap_or(if u.scheme_str() == Some("https") {
                443
            } else {
                80
            })
        };
        self.package.http.as_ref().is_none_or(|rules| {
            rules.iter().any(|r| {
                let origin: http::Uri = r.origin.parse().expect("validated policy");
                r.methods.iter().any(|m| m == method.as_str())
                    && uri.scheme_str() == origin.scheme_str()
                    && uri
                        .host()
                        .zip(origin.host())
                        .is_some_and(|(a, b)| a.eq_ignore_ascii_case(b))
                    && port(uri) == port(&origin)
                    && r.paths.iter().any(|p| p == uri.path())
            })
        })
    }
}
impl HttpPolicy {
    fn authorize<B>(&self, request: &mut Request<B>) -> Result<(), Error> {
        if !self.permits(request.method(), request.uri()) {
            return Err(Error::HttpRequestDenied);
        }
        if let Some(credential) = &self.credential {
            let scope = HttpPolicy {
                allowed: true,
                package: credential.scope.clone(),
                credential: None,
                #[cfg(test)]
                fixture: None,
            };
            if !scope.permits(request.method(), request.uri()) {
                return Err(Error::HttpRequestDenied);
            }
            request
                .headers_mut()
                .insert(credential.header.clone(), credential.value.clone());
        }
        let host = request.uri().authority().unwrap().as_str().parse().unwrap();
        request.headers_mut().insert(http::header::HOST, host);
        Ok(())
    }
}
impl WasiHttpHooks for HttpPolicy {
    fn is_forbidden_header(&mut self, name: &http::HeaderName) -> bool {
        self.credential.as_ref().is_some_and(|c| &c.header == name)
            || wasmtime_wasi_http::DEFAULT_FORBIDDEN_HEADERS.contains(name)
    }

    fn send_request(
        &mut self,
        mut request: Request<WasiBody>,
        _: Option<RequestOptions>,
        _: IoFuture,
    ) -> ResponseFuture {
        if let Err(error) = self.authorize(&mut request) {
            return Box::new(async move { Err(error) });
        }
        #[cfg(test)]
        if let Some(fixture) = self.fixture {
            return Box::new(async move {
                let body: WasiBody = http_body_util::Full::new(fixture.into())
                    .map_err(Error::from)
                    .boxed_unsync();
                Ok((Response::new(body), Box::new(async { Ok(()) }) as IoFuture))
            });
        }
        Box::new(async move {
            let options = RequestOptions {
                connect_timeout: Some(Duration::from_secs(10)),
                first_byte_timeout: Some(Duration::from_secs(20)),
                between_bytes_timeout: Some(Duration::from_secs(10)),
            };
            // Wasmtime's transport sends one request; it does not follow redirects.
            let (response, io) =
                wasmtime_wasi_http::default_send_request(request, Some(options)).await?;
            Ok((
                response.map(BodyExt::boxed_unsync),
                Box::new(io) as IoFuture,
            ))
        })
    }
}

pub struct State {
    pub(crate) wasi: WasiCtx,
    http: WasiHttpCtx,
    pub(crate) table: ResourceTable,
    pub(crate) grant_roots: Vec<(String, crate::grants::GrantRoot)>,
    pub(crate) descriptor_paths: std::collections::BTreeMap<u32, String>,
    pub(crate) guest_cwd: Option<String>,
    pub(crate) single_files: Vec<(wasmtime_wasi::filesystem::File, String)>,
    policy: HttpPolicy,
    pub files: files::Bindings,
    limits: StoreLimits,
}
impl WasiView for State {
    fn ctx(&mut self) -> WasiCtxView<'_> {
        WasiCtxView {
            ctx: &mut self.wasi,
            table: &mut self.table,
        }
    }
}
impl WasiHttpView for State {
    fn http(&mut self) -> WasiHttpCtxView<'_> {
        WasiHttpCtxView {
            ctx: &mut self.http,
            table: &mut self.table,
            hooks: &mut self.policy,
        }
    }
}

pub struct Tool {
    pub package: Package,
    engine: Engine,
    component: Component,
    linker: Linker<State>,
    pub params: Vec<(String, Type)>,
    pub caps: Vec<Capability>,
    pub uses_http: bool,
}
impl Tool {
    pub fn load(bytes: &[u8]) -> Result<Self> {
        let package = Package::read(bytes)?;
        let mut config = Config::new();
        config.epoch_interruption(true);
        // Optimize guest execution; the cache amortizes compilation across invocations.
        config.cranelift_opt_level(wasmtime::OptLevel::Speed);
        // Cache compiled code in the same trusted local state as the registry.
        let cache_config: wasmtime::CacheConfig = serde_json::from_value(json!({
            "directory": crate::home()?.join(".cache")
        }))?;
        config.cache(Some(wasmtime::Cache::new(cache_config)?));
        let engine = Engine::new(&config)?;
        let component = Component::new(&engine, bytes)?;
        let component_type = component.component_type();
        let export = component_type
            .get_export(&engine, &package.tool.entrypoint)
            .context("entrypoint is not exported")?;
        let ComponentItem::ComponentFunc(func) = export.ty else {
            bail!("entrypoint must be a root function");
        };
        ensure!(!func.async_(), "MVP supports P2 synchronous exports");
        let params: Vec<_> = func.params().map(|(n, t)| (n.to_owned(), t)).collect();
        for (_, ty) in &params {
            ensure!(
                supported(ty),
                "arguments support strings, string lists, bool, scalar numbers and options of those types"
            );
        }
        if package.tool.command_output {
            ensure!(
                func.results().collect::<Vec<_>>() == vec![Type::S32],
                "command_output requires one s32 exit status result"
            );
            ensure!(
                !package.tool.verbatim_output,
                "command_output conflicts with verbatim_output"
            );
        }
        let caps = files::discover(&engine, &component)?;
        let mut names: BTreeSet<String> = ["help", "json", "allow-http", "read", "read-write"]
            .map(str::to_owned)
            .into();
        let mut shorts = BTreeSet::from(['h']);
        if let Some(name) = &package.tool.stdin {
            ensure!(
                params.iter().any(|(n, ty)| n == name
                    && matches!(ty,
                Type::Option(inner) if matches!(inner.ty(), Type::String))),
                "stdin must name an optional string WIT parameter"
            );
            ensure!(
                package
                    .tool
                    .options
                    .get(name)
                    .is_some_and(|option| option.default.is_none()),
                "stdin parameter must be an option without a default"
            );
            names.insert("stdin".into());
        }
        for option in package.tool.options.values() {
            if let Some(short) = option.short {
                ensure!(
                    short.is_ascii_alphabetic() && shorts.insert(short),
                    "invalid or duplicate short option -{short}"
                );
            }
        }
        let mut mapped = BTreeSet::new();
        for name in package
            .tool
            .arguments
            .iter()
            .map(|a| &a.name)
            .chain(package.tool.options.keys())
        {
            ensure!(
                valid_name(name) && names.insert(name.clone()),
                "invalid or duplicate CLI name: {name}"
            );
            let (_, ty) = params
                .iter()
                .find(|(n, _)| n == name)
                .with_context(|| format!("unknown WIT parameter {name}"))?;
            if let Some(default) = package
                .tool
                .options
                .get(name)
                .and_then(|o| o.default.as_ref())
            {
                parse_value(ty, &default_text(default)?)?;
            }
            mapped.insert(name.clone());
        }
        ensure!(
            mapped.len() == params.len(),
            "metadata must describe every entrypoint parameter"
        );
        let mut optional_seen = false;
        for (index, argument) in package.tool.arguments.iter().enumerate() {
            let ty = &params.iter().find(|(n, _)| n == &argument.name).unwrap().1;
            ensure!(
                !is_string_list(ty) || index + 1 == package.tool.arguments.len(),
                "a positional list must be last"
            );
            let optional = matches!(ty, Type::Option(_));
            ensure!(
                !optional_seen || optional,
                "required positional arguments must precede optional ones"
            );
            optional_seen |= optional;
        }
        let choice = &package.tool.exactly_one_of;
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
                    package
                        .tool
                        .options
                        .get(name)
                        .is_none_or(|o| o.default.is_none()),
                    "exactly_one_of parameters cannot have defaults"
                );
            }
        }
        for cap in &caps {
            ensure!(
                valid_name(&cap.name) && names.insert(cap.name.clone()),
                "invalid or conflicting resource name {}",
                cap.name
            );
        }
        for (rules, directory) in [
            (&package.policy.files, false),
            (&package.policy.folders, true),
        ] {
            for rule in rules.iter().flatten() {
                ensure!(
                    caps.iter()
                        .any(|c| c.name == rule.name && c.directory == directory),
                    "policy names an unknown resource {}",
                    rule.name
                );
            }
        }
        let uses_http = component_type
            .imports(&engine)
            .any(|(n, _)| n.starts_with("wasi:http/outgoing-handler@"));
        ensure!(
            uses_http || package.policy.http.as_ref().is_none_or(Vec::is_empty),
            "HTTP policy supplied for a component without outgoing HTTP"
        );
        let mut linker = Linker::new(&engine);
        wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
        wasmtime_wasi_http::p2::add_only_http_to_linker_async(&mut linker)?;
        files::link(&mut linker, &caps)?;
        crate::grants::link(&mut linker)?;
        // Resolves imports and checks their types without executing guest code.
        linker.instantiate_pre(&component)?;
        Ok(Self {
            package,
            engine,
            component,
            linker,
            params,
            caps,
            uses_http,
        })
    }

    fn optional(&self, name: &str) -> bool {
        self.params
            .iter()
            .any(|(n, t)| n == name && matches!(t, Type::Option(_)))
    }

    pub fn command(&self) -> Result<Command> {
        let mut cmd = Command::new(self.package.tool.name.clone())
            .about(self.package.tool.description.clone())
            .version(self.package.tool.version.clone())
            .disable_version_flag(true)
            .arg(
                Arg::new("json")
                    .long("json")
                    .action(ArgAction::SetTrue)
                    .help("Print structured output/help"),
            );
        if let Some(name) = &self.package.tool.stdin {
            cmd = cmd.arg(Arg::new("stdin").long("stdin").action(ArgAction::SetTrue)
                .conflicts_with(name.clone())
                .help("Pass UTF-8 stdin to the tool (maximum 8 MiB; does not inherit guest stdio)"));
        }
        for name in ["read", "read-write"] {
            cmd = cmd.arg(
                Arg::new(name)
                    .long(name)
                    .value_name("HOST[::GUEST]")
                    .action(ArgAction::Append)
                    .help(if name == "read" {
                        "Grant read-only access to a file or directory"
                    } else {
                        "Grant read/write access to a file or directory"
                    }),
            );
        }
        if self.uses_http {
            cmd = cmd.arg(
                Arg::new("allow-http")
                    .long("allow-http")
                    .action(ArgAction::SetTrue)
                    .help("Grant outgoing HTTP within the embedded package policy"),
            );
        }
        for (i, spec) in self.package.tool.arguments.iter().enumerate() {
            let mut arg = Arg::new(spec.name.clone())
                .index(i + 1)
                .required(!self.optional(&spec.name))
                .help(spec.help.clone());
            let ty = &self.params.iter().find(|(n, _)| n == &spec.name).unwrap().1;
            if is_string_list(ty) {
                arg = arg
                    .action(ArgAction::Append)
                    .num_args(if self.optional(&spec.name) { 0.. } else { 1.. })
                    .trailing_var_arg(true)
                    .allow_hyphen_values(true);
            }
            cmd = cmd.arg(arg);
        }
        for (name, spec) in &self.package.tool.options {
            let mut arg = Arg::new(name.clone())
                .long(name.clone())
                .help(spec.help.clone());
            let ty = &self.params.iter().find(|(n, _)| n == name).unwrap().1;
            if is_string_list(ty) {
                arg = arg.action(ArgAction::Append).value_name("VALUE");
            }
            if let Some(short) = spec.short {
                arg = arg.short(short);
            }
            if let Some(value) = &spec.default {
                arg = arg.default_value(default_text(value)?);
            } else {
                arg = arg.required(!self.optional(name));
            }
            cmd = cmd.arg(arg);
        }
        if !self.package.tool.exactly_one_of.is_empty() {
            cmd = cmd.group(
                ArgGroup::new("@sat:exactly-one")
                    .args(self.package.tool.exactly_one_of.clone())
                    .required(true)
                    .multiple(false),
            );
        }
        for cap in &self.caps {
            cmd = cmd.arg(
                Arg::new(cap.name.clone())
                    .long(cap.name.clone())
                    .required(true)
                    .value_name("PATH")
                    .help(format!(
                        "Bind a {} ({})",
                        if cap.directory { "directory" } else { "file" },
                        if cap.write {
                            "writable unless restricted by policy"
                        } else {
                            "read-only"
                        }
                    )),
            );
        }
        if !self.package.tool.examples.is_empty() {
            cmd = cmd.after_help(format!(
                "Examples:\n{}",
                self.package
                    .tool
                    .examples
                    .iter()
                    .map(|e| format!("  {}", e.command))
                    .collect::<Vec<_>>()
                    .join("\n")
            ));
        }
        Ok(cmd)
    }

    pub async fn run(&self, args: &[String]) -> Result<()> {
        // SAT and generated tool options share one parser. Accept a separator
        // between host grants and tool options, without changing option values.
        let mut args = args.to_vec();
        if !self.package.tool.command_output
            && let Some(index) = args.iter().position(|a| a == "--")
        {
            args.remove(index);
        }
        let matches = match self.command()?.try_get_matches_from(
            std::iter::once(self.package.tool.name.clone()).chain(args.iter().cloned()),
        ) {
            Ok(m) => m,
            Err(e) if e.kind() == clap::error::ErrorKind::DisplayHelp => {
                if args.iter().any(|a| a == "--json") {
                    println!(
                        "{}",
                        serde_json::to_string_pretty(&json!({
                            "tool": self.package.tool, "policy": self.package.policy,
                            "parameters": self.params.iter().map(|(name,t)| json!({"name":name,"type":format!("{t:?}")})).collect::<Vec<_>>(),
                            "resources": self.caps, "requires_http_grant": self.uses_http,
                            "filesystem_grants": ["--read PATH", "--read-write PATH"],
                        }))?
                    );
                } else {
                    print!("{e}");
                }
                return Ok(());
            }
            Err(e) => return Err(e.into()),
        };
        let mut params = self
            .params
            .iter()
            .map(|(n, t)| parse_matches(t, n, &matches))
            .collect::<Result<Vec<_>>>()?;
        if let Some(name) = &self.package.tool.stdin
            && matches.get_flag("stdin")
        {
            let text = read_stdin_text(std::io::stdin().lock())?;
            let index = self.params.iter().position(|(n, _)| n == name).unwrap();
            params[index] = Val::Option(Some(Box::new(Val::String(text))));
        }
        let mut bindings = files::Bindings::new();
        for cap in &self.caps {
            bindings.insert(
                cap.name.clone(),
                files::open(
                    cap,
                    Path::new(matches.get_one::<String>(&cap.name).unwrap()),
                    &self.package.policy,
                )
                .with_context(|| format!("cannot bind {}", cap.name))?,
            );
        }
        let mut wasi = WasiCtxBuilder::new();
        wasi.allow_tcp(false)
            .allow_udp(false)
            .allow_ip_name_lookup(false);
        let (single_files, guest_cwd) =
            crate::grants::configure(&mut wasi, &matches, &self.package.policy)?;
        let stdout = wasmtime_wasi::p2::pipe::MemoryOutputPipe::new(MAX_COMMAND_OUTPUT);
        let stderr = wasmtime_wasi::p2::pipe::MemoryOutputPipe::new(MAX_COMMAND_OUTPUT);
        if self.package.tool.command_output {
            let mut argv = vec![self.package.tool.name.clone()];
            for argument in &self.package.tool.arguments {
                if let Some(values) = matches.get_many::<String>(&argument.name) {
                    argv.extend(values.cloned());
                }
            }
            let input = self
                .package
                .tool
                .stdin
                .as_ref()
                .and_then(|name| {
                    let index = self.params.iter().position(|(n, _)| n == name)?;
                    match &params[index] {
                        Val::Option(Some(value)) => match value.as_ref() {
                            Val::String(text) => Some(text.clone()),
                            _ => None,
                        },
                        _ => None,
                    }
                })
                .unwrap_or_default();
            ensure!(input.len() <= MAX_STDIN_BYTES, "input exceeds 8 MiB limit");
            wasi.args(&argv)
                .stdin(wasmtime_wasi::p2::pipe::MemoryInputPipe::new(
                    input.into_bytes(),
                ))
                .stdout(stdout.clone())
                .stderr(stderr.clone());
        }
        // No inherited stdio/environment or raw sockets; clocks/random use WASI defaults.
        let mut store = Store::new(
            &self.engine,
            State {
                wasi: wasi.build(),
                single_files,
                guest_cwd,
                grant_roots: Vec::new(),
                descriptor_paths: Default::default(),
                http: WasiHttpCtx::new(),
                table: ResourceTable::new(),
                policy: HttpPolicy {
                    allowed: self.uses_http && matches.get_flag("allow-http"),
                    package: self.package.policy.clone(),
                    #[cfg(test)]
                    fixture: None,
                    credential: if self.uses_http && matches.get_flag("allow-http") {
                        crate::credentials::load(&self.package.tool.name)?
                    } else {
                        None
                    },
                },
                files: bindings,
                limits: StoreLimitsBuilder::new()
                    .memory_size(64 * 1024 * 1024)
                    .build(),
            },
        );
        store.limiter(|s| &mut s.limits);
        configure_epoch_deadline(&mut store);
        let result = execute_with_deadline(&self.engine, EXECUTION_TIMEOUT, async {
            let instance = self
                .linker
                .instantiate_async(&mut store, &self.component)
                .await?;
            let f = instance
                .get_func(&mut store, &self.package.tool.entrypoint)
                .context("missing entrypoint")?;
            let mut results = vec![Val::Bool(false); f.ty(&store).results().len()];
            f.call_async(&mut store, &params, &mut results).await?;
            let mut output = results.iter().map(to_json).collect::<Result<Vec<_>>>()?;
            Ok::<_, anyhow::Error>(if output.len() == 1 {
                output.remove(0)
            } else {
                Json::Array(output)
            })
        })
        .await?;
        if self.package.tool.command_output {
            use std::io::Write;
            let out = stdout.contents();
            let err = stderr.contents();
            ensure!(
                out.len() < MAX_COMMAND_OUTPUT && err.len() < MAX_COMMAND_OUTPUT,
                "command output reached 8 MiB limit; narrow the input"
            );
            let status = result.as_i64().context("invalid command exit status")?;
            ensure!(
                (0..=255).contains(&status),
                "invalid command exit status: {status}"
            );
            if matches.get_flag("json") {
                let encode = |bytes: &[u8]| match std::str::from_utf8(bytes) {
                    Ok(text) => json!(text),
                    Err(_) => json!(bytes),
                };
                println!(
                    "{}",
                    serde_json::to_string(&json!({
                        "stdout": encode(&out), "stderr": encode(&err), "exit_code": status
                    }))?
                );
            } else {
                std::io::stdout().lock().write_all(&out)?;
                std::io::stderr().lock().write_all(&err)?;
            }
            if status != 0 {
                return Err(CommandExit(status as u8).into());
            }
            return Ok(());
        }
        if !matches.get_flag("json")
            && let Json::String(text) = &result
        {
            if self.package.tool.verbatim_output {
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

fn supported(t: &Type) -> bool {
    if let Type::Option(inner) = t {
        let inner = inner.ty();
        return !matches!(inner, Type::Option(_)) && (is_string_list(&inner) || supported(&inner));
    }
    is_string_list(t)
        || matches!(
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
fn is_string_list(t: &Type) -> bool {
    match t {
        Type::List(list) => matches!(list.ty(), Type::String),
        Type::Option(inner) => is_string_list(&inner.ty()),
        _ => false,
    }
}
fn parse_matches(ty: &Type, name: &str, matches: &clap::ArgMatches) -> Result<Val> {
    if is_string_list(ty) {
        let values = matches
            .get_many::<String>(name)
            .map(|values| values.cloned().map(Val::String).collect::<Vec<_>>());
        return Ok(if matches!(ty, Type::Option(_)) {
            Val::Option(values.map(|items| Box::new(Val::List(items))))
        } else {
            Val::List(values.unwrap_or_default())
        });
    }
    match matches.get_one::<String>(name) {
        Some(value) => parse_value(ty, value),
        None if matches!(ty, Type::Option(_)) => Ok(Val::Option(None)),
        None => bail!("missing parameter {name}"),
    }
}
fn default_text(v: &toml::Value) -> Result<String> {
    Ok(match v {
        toml::Value::String(s) => s.clone(),
        toml::Value::Integer(_) | toml::Value::Boolean(_) | toml::Value::Float(_) => v.to_string(),
        _ => bail!("default must be a scalar"),
    })
}
fn parse_value(ty: &Type, text: &str) -> Result<Val> {
    Ok(match ty {
        Type::Option(inner) => Val::Option(Some(Box::new(parse_value(&inner.ty(), text)?))),
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
fn to_json(v: &Val) -> Result<Json> {
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
        Val::List(items) | Val::Tuple(items) => {
            Json::Array(items.iter().map(to_json).collect::<Result<_>>()?)
        }
        Val::Record(fields) => Json::Object(
            fields
                .iter()
                .map(|(k, v)| Ok((k.clone(), to_json(v)?)))
                .collect::<Result<_>>()?,
        ),
        Val::Result(Ok(Some(v))) | Val::Option(Some(v)) => to_json(v)?,
        Val::Result(Ok(None)) | Val::Option(None) => Json::Null,
        Val::Result(Err(v)) => bail!(
            "tool error: {}",
            v.as_ref()
                .map(|v| to_json(v))
                .transpose()?
                .unwrap_or(Json::Null)
        ),
        _ => bail!("unsupported output type"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metadata::HttpRule;

    #[test]
    fn stdin_text_is_bounded_and_utf8() {
        use std::io::Read as _;
        assert_eq!(read_stdin_text("日本語".as_bytes()).unwrap(), "日本語");
        assert!(
            read_stdin_text(&b"\xff"[..])
                .unwrap_err()
                .to_string()
                .contains("UTF-8")
        );
        let too_large = std::io::repeat(b'x').take((MAX_STDIN_BYTES + 1) as u64);
        assert!(
            read_stdin_text(too_large)
                .unwrap_err()
                .to_string()
                .contains("8 MiB")
        );
    }

    async fn epoch_test_guest(spin: bool, limit: Duration) -> Result<()> {
        use wasm_encoder::{
            BlockType, CodeSection, ExportKind, ExportSection, Function, FunctionSection,
            Instruction, Module, TypeSection,
        };
        let mut types = TypeSection::new();
        types.ty().function([], []);
        let mut functions = FunctionSection::new();
        functions.function(0);
        let mut exports = ExportSection::new();
        exports.export("run", ExportKind::Func, 0);
        let mut function = Function::new([]);
        if spin {
            function.instruction(&Instruction::Loop(BlockType::Empty));
            function.instruction(&Instruction::Br(0));
            function.instruction(&Instruction::End);
        }
        function.instruction(&Instruction::End);
        let mut code = CodeSection::new();
        code.function(&function);
        let mut module = Module::new();
        module
            .section(&types)
            .section(&functions)
            .section(&exports)
            .section(&code);
        let mut config = Config::new();
        config.epoch_interruption(true);
        config.cranelift_opt_level(wasmtime::OptLevel::Speed);
        let engine = Engine::new(&config)?;
        let module = wasmtime::Module::new(&engine, module.finish())?;
        let mut store = Store::new(&engine, ());
        assert!(store.get_fuel().is_err(), "fuel metering must be disabled");
        configure_epoch_deadline(&mut store);
        execute_with_deadline(&engine, limit, async {
            let instance = wasmtime::Instance::new_async(&mut store, &module, &[]).await?;
            instance
                .get_typed_func::<(), ()>(&mut store, "run")?
                .call_async(&mut store, ())
                .await?;
            Ok(())
        })
        .await
    }

    #[tokio::test(flavor = "current_thread")]
    async fn epoch_deadline_stops_cpu_bound_guest_without_host_calls() {
        let started = std::time::Instant::now();
        let error = epoch_test_guest(true, Duration::from_millis(50))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("tool exceeded 0.05 seconds"));
        assert!(started.elapsed() < Duration::from_secs(2));
    }

    #[tokio::test(flavor = "current_thread")]
    async fn epoch_deadline_allows_completed_guest() {
        epoch_test_guest(false, Duration::from_secs(1))
            .await
            .unwrap();
    }

    #[tokio::test(flavor = "current_thread")]
    async fn execution_deadline_also_stops_pending_host_calls() {
        let engine = Engine::default();
        let error = execute_with_deadline(
            &engine,
            Duration::from_millis(50),
            std::future::pending::<Result<()>>(),
        )
        .await
        .unwrap_err();
        assert!(error.to_string().contains("tool exceeded 0.05 seconds"));
    }

    #[test]
    fn http_grant_and_package_ceiling_are_both_required() {
        let package = Policy {
            http: Some(vec![HttpRule {
                methods: vec!["GET".into()],
                origin: "https://httpbin.org".into(),
                paths: vec!["/get".into()],
            }]),
            ..Default::default()
        };
        let mut policy = HttpPolicy {
            allowed: false,
            package,
            credential: None,
            fixture: None,
        };
        let uri = "https://httpbin.org/get?q=hello".parse().unwrap();
        assert!(!policy.permits(&http::Method::GET, &uri));
        policy.allowed = true;
        assert!(policy.permits(&http::Method::GET, &uri));
        assert!(!policy.permits(&http::Method::POST, &uri));
        for url in [
            "https://example.com/get",
            "https://httpbin.org.evil/get",
            "http://httpbin.org/get",
            "https://httpbin.org:444/get",
            "https://httpbin.org/other",
            "https://user@httpbin.org/get",
        ] {
            assert!(
                !policy.permits(&http::Method::GET, &url.parse().unwrap()),
                "{url}"
            );
        }
        policy.package = Policy::default();
        assert!(policy.permits(&http::Method::GET, &uri));
        policy.allowed = false;
        assert!(!policy.permits(&http::Method::GET, &uri));
    }
    fn brave_policy() -> HttpPolicy {
        let scope = Policy {
            http: Some(vec![HttpRule {
                methods: vec!["GET".into()],
                origin: "https://api.search.brave.com".into(),
                paths: vec!["/res/v1/web/search".into()],
            }]),
            ..Default::default()
        };
        HttpPolicy {
            allowed: true,
            package: scope.clone(),
            credential: Some(crate::credentials::Credential {
                header: http::HeaderName::from_static("x-subscription-token"),
                value: http::HeaderValue::from_static("test-only-secret"),
                scope,
            }),
            fixture: None,
        }
    }

    #[test]
    fn credentials_require_both_grant_and_destination_scope() {
        let mut policy = brave_policy();
        let mut request = Request::builder()
            .uri("https://api.search.brave.com/res/v1/web/search?q=hello")
            .header("host", "evil.example")
            .body(())
            .unwrap();
        policy.allowed = false;
        assert!(policy.authorize(&mut request).is_err());
        assert!(!request.headers().contains_key("x-subscription-token"));
        policy.allowed = true;
        policy.authorize(&mut request).unwrap();
        assert_eq!(
            request.headers()["x-subscription-token"],
            "test-only-secret"
        );
        assert_eq!(request.headers()["host"], "api.search.brave.com");
        assert!(policy.is_forbidden_header(&http::HeaderName::from_static("x-subscription-token")));
        // Even an unrestricted component policy cannot widen the host's key scope.
        policy.package = Policy::default();
        for uri in [
            "https://example.com/res/v1/web/search",
            "http://api.search.brave.com/res/v1/web/search",
            "https://api.search.brave.com/other",
            "https://api.search.brave.com:444/res/v1/web/search",
        ] {
            let mut request = Request::builder().uri(uri).body(()).unwrap();
            assert!(policy.authorize(&mut request).is_err());
            assert!(!request.headers().contains_key("x-subscription-token"));
        }
        let mut post = Request::builder()
            .method("POST")
            .uri("https://api.search.brave.com/res/v1/web/search")
            .body(())
            .unwrap();
        assert!(policy.authorize(&mut post).is_err());
    }

    #[tokio::test]
    #[ignore = "run python3 pi-extension/setup.py first"]
    async fn brave_component_offline() {
        let path =
            Path::new(env!("CARGO_MANIFEST_DIR")).join("../pi-extension/target/brave-search.wasm");
        let tool = Tool::load(&std::fs::read(path).unwrap()).unwrap();
        let mut policy = brave_policy();
        policy.fixture = Some(include_bytes!("../../brave-search-example/fixture.json"));
        let mut wasi = WasiCtxBuilder::new();
        wasi.allow_tcp(false)
            .allow_udp(false)
            .allow_ip_name_lookup(false);
        let mut store = Store::new(
            &tool.engine,
            State {
                wasi: wasi.build(),
                http: WasiHttpCtx::new(),
                table: ResourceTable::new(),
                grant_roots: Vec::new(),
                descriptor_paths: Default::default(),
                guest_cwd: None,
                single_files: Vec::new(),
                files: files::Bindings::new(),
                policy,
                limits: StoreLimitsBuilder::new()
                    .memory_size(64 * 1024 * 1024)
                    .build(),
            },
        );
        store.limiter(|s| &mut s.limits);
        configure_epoch_deadline(&mut store);
        let _ticker = EpochTicker::start(&tool.engine).unwrap();
        let instance = tool
            .linker
            .instantiate_async(&mut store, &tool.component)
            .await
            .unwrap();
        let run = instance.get_func(&mut store, "run").unwrap();
        let mut result = [Val::Bool(false)];
        run.call_async(
            &mut store,
            &[Val::String("WASI components".into())],
            &mut result,
        )
        .await
        .unwrap();
        let value = to_json(&result[0]).unwrap();
        assert_eq!(value[0]["title"], "Offline example result");
        assert!(!value.to_string().contains("test-only-secret"));
        let probe = instance.get_func(&mut store, "sandbox-probe").unwrap();
        probe
            .call_async(&mut store, &[], &mut result)
            .await
            .unwrap();
        assert_eq!(to_json(&result[0]).unwrap(), Json::Null);
    }
}
