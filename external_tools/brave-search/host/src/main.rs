use bytes::Bytes;
use http::{HeaderValue, Request, Response};
use http_body_util::{BodyExt, Full};
use std::{future::Future, path::Path, time::Duration};
use wasmtime::component::{Component, Linker, ResourceTable, Val};
use wasmtime::{Config, Engine, Store, StoreLimits, StoreLimitsBuilder};
use wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiCtxView, WasiView};
use wasmtime_wasi_http::{
    Error, RequestOptions, WasiBody, WasiHttpCtx, WasiHttpCtxView, WasiHttpHooks, WasiHttpView,
};

mod metadata;
use metadata::{HttpPermission, ToolMetadata};

type IoFuture = Box<dyn Future<Output = Result<(), Error>> + Send>;
type ResponseFuture =
    Box<dyn Future<Output = Result<(Response<WasiBody>, IoFuture), Error>> + Send>;

struct HttpPolicy {
    permission: HttpPermission,
    key: HeaderValue,
    offline: bool,
    remaining_requests: usize,
}

impl HttpPolicy {
    fn authorize(&mut self, request: &mut Request<WasiBody>) -> Result<(), Error> {
        let uri = request.uri();
        let origin: http::Uri = self
            .permission
            .origin
            .parse()
            .map_err(|_| Error::HttpRequestDenied)?;
        if !self
            .permission
            .methods
            .iter()
            .any(|m| m == request.method().as_str())
            || uri.scheme_str() != Some("https")
            || uri.host() != origin.host()
            || uri.port_u16().unwrap_or(443) != origin.port_u16().unwrap_or(443)
            || uri.authority().is_none_or(|a| a.as_str().contains('@'))
            || !self.permission.paths.iter().any(|path| path == uri.path())
            || self.remaining_requests == 0
        {
            return Err(Error::HttpRequestDenied);
        }
        self.remaining_requests -= 1;
        // Discard guest headers, then inject the credential only after authorization.
        // The guest never receives a reference to this header map.
        request.headers_mut().clear();
        request.headers_mut().insert(
            http::header::HOST,
            HeaderValue::from_str(origin.authority().unwrap().as_str())
                .map_err(|_| Error::HttpRequestDenied)?,
        );
        request.headers_mut().insert(
            http::header::ACCEPT,
            HeaderValue::from_static("application/json"),
        );
        request.headers_mut().insert(
            self.permission
                .header
                .parse::<http::HeaderName>()
                .map_err(|_| Error::HttpRequestDenied)?,
            self.key.clone(),
        );
        Ok(())
    }
}

impl WasiHttpHooks for HttpPolicy {
    fn is_forbidden_header(&mut self, name: &http::HeaderName) -> bool {
        name.as_str() == self.permission.header
            || wasmtime_wasi_http::DEFAULT_FORBIDDEN_HEADERS.contains(name)
    }

    fn send_request(
        &mut self,
        mut request: Request<WasiBody>,
        _options: Option<RequestOptions>,
        _completion: IoFuture,
    ) -> ResponseFuture {
        if let Err(error) = self.authorize(&mut request) {
            return Box::new(async move { Err(error) });
        }
        let offline = self.offline;
        Box::new(async move {
            if offline {
                let body = Full::new(Bytes::from_static(include_bytes!("../../fixture.json")))
                    .map_err(Error::from)
                    .boxed_unsync();
                return Ok((Response::new(body), Box::new(async { Ok(()) }) as IoFuture));
            }
            let options = RequestOptions {
                connect_timeout: Some(Duration::from_secs(10)),
                first_byte_timeout: Some(Duration::from_secs(20)),
                between_bytes_timeout: Some(Duration::from_secs(10)),
            };
            // This transport performs one request; it does not follow redirects.
            let (response, io) =
                wasmtime_wasi_http::default_send_request(request, Some(options)).await?;
            if response.status().is_redirection() {
                return Err(Error::HttpRequestDenied);
            }
            Ok((
                response.map(BodyExt::boxed_unsync),
                Box::new(io) as IoFuture,
            ))
        })
    }
}

struct State {
    wasi: WasiCtx,
    http: WasiHttpCtx,
    table: ResourceTable,
    policy: HttpPolicy,
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

async fn run(
    component_path: &Path,
    query: &str,
    key: HeaderValue,
    offline: bool,
    metadata: &ToolMetadata,
    probe: bool,
) -> wasmtime::Result<serde_json::Value> {
    let mut config = Config::new();
    config.consume_fuel(true);
    let engine = Engine::new(&config)?;
    let component = Component::from_file(&engine, component_path)?;
    let mut linker = Linker::<State>::new(&engine);
    wasmtime_wasi::p2::add_to_linker_async(&mut linker)?;
    wasmtime_wasi_http::p2::add_only_http_to_linker_async(&mut linker)?;

    let mut wasi = WasiCtxBuilder::new();
    // No preopens, environment inheritance, stdin, or raw network permissions.
    wasi.allow_tcp(false)
        .allow_udp(false)
        .allow_ip_name_lookup(false);
    let state = State {
        wasi: wasi.build(),
        http: WasiHttpCtx::new(),
        table: ResourceTable::new(),
        policy: HttpPolicy {
            permission: metadata.permissions.http.clone(),
            key,
            offline,
            remaining_requests: 1,
        },
        limits: StoreLimitsBuilder::new()
            .memory_size(64 * 1024 * 1024)
            .build(),
    };
    let mut store = Store::new(&engine, state);
    store.limiter(|state| &mut state.limits);
    store.set_fuel(100_000_000)?;
    let execution = async {
        let instance = linker.instantiate_async(&mut store, &component).await?;
        let (parent, function, args) = if probe {
            (None, "sandbox-probe", Vec::new())
        } else {
            let (_, index) = instance
                .get_export(&mut store, None, &metadata.entrypoint.interface)
                .ok_or_else(|| wasmtime::Error::msg("declared interface is not exported"))?;
            (
                Some(index),
                metadata.entrypoint.function.as_str(),
                vec![Val::String(query.into())],
            )
        };
        let (_, index) = instance
            .get_export(&mut store, parent.as_ref(), function)
            .ok_or_else(|| wasmtime::Error::msg("declared function is not exported"))?;
        let function = instance
            .get_func(&mut store, index)
            .ok_or_else(|| wasmtime::Error::msg("entrypoint is not a function"))?;
        // The demo supports one string argument and one result; Wasmtime checks the actual types.
        let mut results = [Val::Bool(false)];
        function.call_async(&mut store, &args, &mut results).await?;
        to_json(&results[0])
    };
    tokio::time::timeout(Duration::from_secs(30), execution)
        .await
        .map_err(|_| wasmtime::Error::msg("component exceeded 30 second deadline"))?
}

#[tokio::main]
async fn main() -> wasmtime::Result<()> {
    let mut args = std::env::args().skip(1).peekable();
    let mut offline = false;
    let mut approved = false;
    let mut credential_env = None;
    while args.peek().is_some_and(|s| s.starts_with("--")) {
        match args.next().unwrap().as_str() {
            "--offline" => offline = true,
            "--approve-permissions" => approved = true,
            "--credential-env" => {
                credential_env = Some(args.next().ok_or_else(|| {
                    wasmtime::Error::msg("missing credential environment variable name")
                })?)
            }
            _ => wasmtime::bail!("unknown runner option"),
        }
    }
    let metadata_path = args.next().ok_or_else(|| wasmtime::Error::msg(
        "usage: brave-search-host [--offline] --approve-permissions [--credential-env NAME] TOOL.toml 'query'"))?;
    let query = args
        .next()
        .ok_or_else(|| wasmtime::Error::msg("missing query"))?;
    wasmtime::ensure!(args.next().is_none(), "quote the query as one argument");
    let metadata: ToolMetadata = metadata::read(Path::new(&metadata_path))?;
    metadata.approve(approved)?;
    let path = metadata.component_path(Path::new(&metadata_path))?;
    let result = run(
        &path,
        &query,
        metadata::key(credential_env.as_deref(), offline)?,
        offline,
        &metadata,
        false,
    )
    .await?;
    println!("{}", serde_json::to_string_pretty(&result)?);
    Ok(())
}

// Deliberately small presentation layer, independent of any provider's record schema.
fn to_json(value: &Val) -> wasmtime::Result<serde_json::Value> {
    use serde_json::Value as Json;
    Ok(match value {
        Val::String(s) => Json::String(s.clone()),
        Val::Bool(b) => Json::Bool(*b),
        Val::List(items) => {
            Json::Array(items.iter().map(to_json).collect::<wasmtime::Result<_>>()?)
        }
        Val::Record(fields) => Json::Object(
            fields
                .iter()
                .map(|(k, v)| Ok((k.clone(), to_json(v)?)))
                .collect::<wasmtime::Result<_>>()?,
        ),
        Val::Result(Ok(Some(value))) => to_json(value)?,
        Val::Result(Ok(None)) | Val::Option(None) => Json::Null,
        Val::Option(Some(value)) => to_json(value)?,
        Val::Result(Err(error)) => wasmtime::bail!(
            "component error: {}",
            match error {
                Some(v) => to_json(v)?.to_string(),
                None => "unspecified".into(),
            }
        ),
        _ => wasmtime::bail!("unsupported output type in this demonstration runner"),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const AUTH_HEADER: &str = "x-subscription-token";
    fn tool() -> ToolMetadata {
        toml::from_str(include_str!("../../tool.toml")).unwrap()
    }
    fn policy() -> HttpPolicy {
        HttpPolicy {
            permission: tool().permissions.http,
            key: HeaderValue::from_static("test-secret"),
            offline: true,
            remaining_requests: 1,
        }
    }
    fn request(method: &str, uri: &str) -> Request<WasiBody> {
        Request::builder()
            .method(method)
            .uri(uri)
            .body(Full::new(Bytes::new()).map_err(Error::from).boxed_unsync())
            .unwrap()
    }

    #[test]
    fn metadata_requires_approval_and_rejects_invalid_permissions() {
        assert!(tool().approve(false).is_err());
        tool().approve(true).unwrap();
        for origin in [
            "http://example.com",
            "https://example.com/path",
            "https://user@example.com",
        ] {
            let mut metadata = tool();
            metadata.permissions.http.origin = origin.into();
            assert!(metadata.approve(true).is_err());
        }
        let mut metadata = tool();
        metadata.permissions.http.header = "host".into();
        assert!(metadata.approve(true).is_err());
        let unknown = include_str!("../../tool.toml").replace(
            "schema_version = 1",
            "schema_version = 1\nallow_everything = true",
        );
        assert!(toml::from_str::<ToolMetadata>(&unknown).is_err());
    }

    #[test]
    fn enforcement_uses_metadata_instead_of_provider_constants() {
        let mut policy = policy();
        policy.permission.origin = "https://other.example:8443".into();
        policy.permission.methods = vec!["POST".into()];
        policy.permission.paths = vec!["/lookup".into()];
        policy.permission.header = "x-other-key".into();
        policy.permission.validate().unwrap();
        let mut denied = request("GET", "https://api.search.brave.com/res/v1/web/search");
        assert!(policy.authorize(&mut denied).is_err());
        let mut allowed = request("POST", "https://other.example:8443/lookup");
        policy.authorize(&mut allowed).unwrap();
        assert_eq!(allowed.headers()["x-other-key"], "test-secret");
        assert_eq!(allowed.headers()[http::header::HOST], "other.example:8443");
    }

    #[test]
    fn rejects_unapproved_destinations_methods_and_paths_before_injecting_key() {
        for (method, uri) in [
            ("GET", "https://example.com/res/v1/web/search"),
            (
                "GET",
                "https://api.search.brave.com.evil.example/res/v1/web/search",
            ),
            ("GET", "http://api.search.brave.com/res/v1/web/search"),
            ("GET", "https://api.search.brave.com:444/res/v1/web/search"),
            ("POST", "https://api.search.brave.com/res/v1/web/search"),
            ("GET", "https://api.search.brave.com/other"),
            ("GET", "https://api.search.brave.com/res/v1/web/../search"),
        ] {
            let mut req = request(method, uri);
            assert!(matches!(
                policy().authorize(&mut req),
                Err(Error::HttpRequestDenied)
            ));
            assert!(!req.headers().contains_key(AUTH_HEADER));
        }
    }

    #[test]
    fn approved_request_gets_host_key_and_only_one_request_is_allowed() {
        let mut policy = policy();
        let mut req = request(
            "GET",
            "https://api.search.brave.com/res/v1/web/search?q=test",
        );
        req.headers_mut()
            .insert(AUTH_HEADER, HeaderValue::from_static("guest-value"));
        policy.authorize(&mut req).unwrap();
        assert_eq!(req.headers()[AUTH_HEADER], "test-secret");
        assert!(matches!(
            policy.authorize(&mut req),
            Err(Error::HttpRequestDenied)
        ));
    }

    #[tokio::test]
    async fn real_component_runs_offline_and_cannot_use_ambient_authority() -> wasmtime::Result<()>
    {
        let path = Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../target/wasm32-wasip2/debug/brave_search_guest.wasm");
        wasmtime::ensure!(path.is_file(), "build the guest first; see README.md");
        let hits = run(&path, "WASI & Rust?", policy().key, true, &tool(), false).await?;
        assert_eq!(hits.as_array().unwrap().len(), 1);
        assert_eq!(hits[0]["title"], "Offline example result");
        assert_eq!(hits[0]["url"], "https://example.com/wasi");
        assert!(
            !hits[0]["description"]
                .as_str()
                .unwrap()
                .contains("test-secret")
        );
        run(&path, "", policy().key, true, &tool(), true).await?;
        Ok(())
    }
}
