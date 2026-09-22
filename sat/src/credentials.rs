//! Trusted host configuration, separate from untrusted component metadata.
use crate::metadata::{HttpRule, Policy};
use anyhow::{Context, Result, ensure};
use http::{HeaderName, HeaderValue, Method, Uri};
use serde::Deserialize;
use std::collections::BTreeMap;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Spec {
    env: String,
    header: String,
    origin: String,
    methods: Vec<String>,
    paths: Vec<String>,
}

pub(crate) struct Credential {
    pub header: HeaderName,
    pub value: HeaderValue,
    pub scope: Policy,
}

pub(crate) fn load(tool: &str) -> Result<Option<Credential>> {
    let path = crate::home()?.join("credentials.toml");
    let text = match std::fs::read_to_string(path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let mut specs: BTreeMap<String, Spec> =
        toml::from_str(&text).context("invalid host credentials.toml")?;
    specs
        .remove(tool)
        .map(|spec| {
            let origin: Uri = spec.origin.parse().context("invalid credential origin")?;
            ensure!(
                origin.scheme_str() == Some("https")
                    && origin.host().is_some()
                    && !origin.authority().unwrap().as_str().contains('@')
                    && origin.path() == "/"
                    && origin.query().is_none(),
                "credential origin must be a plain HTTPS origin"
            );
            for method in &spec.methods {
                let _: Method = method.parse()?;
            }
            ensure!(
                spec.paths
                    .iter()
                    .all(|p| p.starts_with('/') && !p.contains(['?', '#', '*'])),
                "credential paths must be exact paths"
            );
            let header: HeaderName = spec.header.parse().context("invalid credential header")?;
            ensure!(
                !wasmtime_wasi_http::DEFAULT_FORBIDDEN_HEADERS.contains(&header),
                "invalid credential header"
            );
            let secret = std::env::var(&spec.env)
                .map_err(|_| anyhow::anyhow!("set {} for {tool}", spec.env))?;
            ensure!(
                !secret.is_empty(),
                "credential environment variable is empty"
            );
            let mut value = HeaderValue::from_str(&secret)
                .map_err(|_| anyhow::anyhow!("credential is not a valid HTTP header value"))?;
            value.set_sensitive(true);
            Ok(Credential {
                header,
                value,
                scope: Policy {
                    http: Some(vec![HttpRule {
                        methods: spec.methods,
                        origin: spec.origin,
                        paths: spec.paths,
                    }]),
                    ..Default::default()
                },
            })
        })
        .transpose()
}
