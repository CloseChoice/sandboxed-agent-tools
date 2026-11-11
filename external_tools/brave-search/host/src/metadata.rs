use http::{HeaderName, HeaderValue, Method, Uri};
use serde::Deserialize;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HttpPermission {
    pub origin: String,
    pub methods: Vec<String>,
    pub paths: Vec<String>,
    pub credential: String,
    pub header: String,
}

#[derive(Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Entrypoint {
    pub interface: String,
    pub function: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolMetadata {
    pub schema_version: u32,
    pub id: String,
    pub component: String,
    pub entrypoint: Entrypoint,
    pub permissions: Permissions,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Permissions {
    pub http: HttpPermission,
}

impl HttpPermission {
    pub fn validate(&self) -> wasmtime::Result<()> {
        let uri: Uri = self.origin.parse()?;
        wasmtime::ensure!(
            uri.scheme_str() == Some("https")
                && uri.host().is_some()
                && uri.authority().is_some_and(|a| !a.as_str().contains('@'))
                && uri.path() == "/"
                && uri.query().is_none()
                && !self.origin.contains(['?', '#']),
            "expected an HTTPS origin without a path or userinfo"
        );
        let header: HeaderName = self.header.parse()?;
        wasmtime::ensure!(
            !wasmtime_wasi_http::DEFAULT_FORBIDDEN_HEADERS.contains(&header)
                && header != http::header::CONTENT_LENGTH,
            "invalid credential header"
        );
        wasmtime::ensure!(
            !self.methods.is_empty() && !self.paths.is_empty() && !self.credential.is_empty(),
            "empty HTTP permission"
        );
        for method in &self.methods {
            let _: Method = method.parse()?;
        }
        for path in &self.paths {
            wasmtime::ensure!(
                path.starts_with('/')
                    && !path.starts_with("//")
                    && !path.contains(['?', '#', '%', '\\'])
                    && !path.split('/').any(|part| part == "." || part == ".."),
                "HTTP permissions require exact, unencoded absolute paths"
            );
        }
        Ok(())
    }
}

impl ToolMetadata {
    pub fn approve(&self, approved: bool) -> wasmtime::Result<()> {
        wasmtime::ensure!(
            approved,
            "review tool.toml and pass --approve-permissions to grant its requested permissions"
        );
        wasmtime::ensure!(self.schema_version == 1, "unsupported metadata schema");
        self.permissions.http.validate()?;
        wasmtime::ensure!(
            !self.id.is_empty()
                && !self.entrypoint.interface.is_empty()
                && !self.entrypoint.function.is_empty(),
            "empty tool identity or entrypoint"
        );
        Ok(())
    }

    pub fn component_path(&self, metadata_path: &Path) -> wasmtime::Result<PathBuf> {
        let root = metadata_path.canonicalize()?.parent().unwrap().to_owned();
        let path = root.join(&self.component).canonicalize()?;
        wasmtime::ensure!(
            path.starts_with(&root) && path.is_file(),
            "component must be a file inside the tool package"
        );
        Ok(path)
    }
}

pub fn key(credential_env: Option<&str>, offline: bool) -> wasmtime::Result<HeaderValue> {
    let secret = if offline {
        "offline-test-key".into()
    } else {
        std::env::var(
            credential_env
                .ok_or_else(|| wasmtime::Error::msg("live mode requires --credential-env NAME"))?,
        )
        .map_err(|_| {
            wasmtime::Error::msg(
                "set the credential environment variable selected by --credential-env",
            )
        })?
    };
    wasmtime::ensure!(!secret.trim().is_empty(), "empty credential");
    let mut key = HeaderValue::from_str(&secret)
        .map_err(|_| wasmtime::Error::msg("credential is not a valid HTTP header value"))?;
    key.set_sensitive(true);
    Ok(key)
}

pub fn read<T: serde::de::DeserializeOwned>(path: &Path) -> wasmtime::Result<T> {
    Ok(toml::from_str(&std::fs::read_to_string(path)?)?)
}
