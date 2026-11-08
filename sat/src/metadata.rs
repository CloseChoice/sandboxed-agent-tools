use anyhow::{Context, Result, bail, ensure};
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;
use wasm_encoder::Encode;
use wasmparser::{Encoding, Parser, Payload};

pub const MAX_FUEL: u64 = 1_000_000_000;

#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Tool {
    pub name: String,
    pub version: String,
    pub description: String,
    pub entrypoint: String,
    #[serde(default)]
    pub arguments: Vec<Argument>,
    #[serde(default)]
    pub options: BTreeMap<String, OptionSpec>,
    #[serde(default)]
    pub examples: Vec<Example>,
    #[serde(default)]
    pub exactly_one_of: Vec<String>,
    /// Requested Wasmtime fuel. The host enforces a hard maximum.
    #[serde(default)]
    pub fuel: Option<u64>,
    /// Optional string parameter filled only when the caller passes --stdin.
    #[serde(default)]
    pub stdin: Option<String>,
    /// Present a returned string without adding a newline (e.g. jq -j).
    #[serde(default)]
    pub verbatim_output: bool,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Argument {
    pub name: String,
    pub help: String,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct OptionSpec {
    pub short: Option<char>,
    pub help: String,
    pub default: Option<toml::Value>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Example {
    pub command: String,
}

#[derive(Debug, Clone, Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Policy {
    // None means this package adds no restriction; [] denies the category.
    pub http: Option<Vec<HttpRule>>,
    pub files: Option<Vec<FsRule>>,
    pub folders: Option<Vec<FsRule>>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct HttpRule {
    pub methods: Vec<String>,
    pub origin: String,
    pub paths: Vec<String>,
}
#[derive(Debug, Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FsRule {
    pub name: String,
    pub access: Vec<String>,
}

pub struct Package {
    pub tool: Tool,
    pub policy: Policy,
    pub tool_text: String,
    pub policy_text: Option<String>,
}

pub fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
}

pub fn sections(bytes: &[u8]) -> Result<BTreeMap<String, String>> {
    let mut sections = BTreeMap::new();
    let mut depth = 0;
    for payload in Parser::new(0).parse_all(bytes) {
        match payload? {
            Payload::Version { encoding, .. } if depth == 0 => {
                ensure!(
                    encoding == Encoding::Component,
                    "expected a WebAssembly component"
                );
            }
            Payload::ModuleSection { .. } | Payload::ComponentSection { .. } => depth += 1,
            Payload::End(_) if depth > 0 => depth -= 1,
            Payload::CustomSection(s) if depth == 0 => {
                if s.name().starts_with("sat.tool.") || s.name().starts_with("sat.policy.") {
                    ensure!(
                        matches!(s.name(), "sat.tool.v1" | "sat.policy.v1"),
                        "unsupported SAT metadata version"
                    );
                    let text = std::str::from_utf8(s.data())?.to_owned();
                    ensure!(
                        sections.insert(s.name().into(), text).is_none(),
                        "duplicate {} section",
                        s.name()
                    );
                }
            }
            _ => {}
        }
    }
    Ok(sections)
}

impl Package {
    pub fn read(bytes: &[u8]) -> Result<Self> {
        let mut sections = sections(bytes)?;
        let tool_text = sections
            .remove("sat.tool.v1")
            .context("missing sat.tool.v1 section; use sat pack")?;
        let policy_text = sections.remove("sat.policy.v1");
        let tool: Tool = toml::from_str(&tool_text).context("invalid tool metadata")?;
        ensure!(
            tool.fuel.is_none_or(|fuel| (1_000_000..=MAX_FUEL).contains(&fuel)),
            "tool fuel must be between 1,000,000 and 1,000,000,000"
        );
        ensure!(
            valid_name(&tool.name),
            "tool name must contain only letters, digits, hyphens and underscores"
        );
        ensure!(
            !["pack", "install", "list", "search", "inspect", "help"].contains(&tool.name.as_str()),
            "reserved tool name"
        );
        let policy: Policy = policy_text
            .as_deref()
            .map(toml::from_str)
            .transpose()
            .context("invalid package policy")?
            .unwrap_or_default();
        for rule in policy.http.iter().flatten() {
            let uri: http::Uri = rule.origin.parse()?;
            ensure!(
                matches!(uri.scheme_str(), Some("http" | "https"))
                    && uri.host().is_some()
                    && !uri.authority().unwrap().as_str().contains('@')
                    && uri.path() == "/"
                    && uri.query().is_none(),
                "HTTP origin must be a plain http(s) origin"
            );
            for method in &rule.methods {
                let _: http::Method = method.parse()?;
            }
            ensure!(
                rule.paths
                    .iter()
                    .all(|p| p.starts_with('/') && !p.contains(['?', '#', '*'])),
                "HTTP paths must be exact paths without query strings or wildcards"
            );
        }
        for rules in [&policy.files, &policy.folders].into_iter().flatten() {
            let mut names = std::collections::HashSet::new();
            for rule in rules {
                ensure!(names.insert(&rule.name), "duplicate filesystem rule");
                ensure!(
                    rule.access.iter().all(|s| s == "read" || s == "write"),
                    "filesystem access must be read or write"
                );
            }
        }
        Ok(Self {
            tool,
            policy,
            tool_text,
            policy_text,
        })
    }
}

pub fn embed(mut bytes: Vec<u8>, tool: &str, policy: Option<&str>) -> Result<Vec<u8>> {
    ensure!(
        sections(&bytes)?.is_empty(),
        "component already contains SAT metadata; package the original build instead"
    );
    for (name, text) in [("sat.tool.v1", Some(tool)), ("sat.policy.v1", policy)] {
        if let Some(text) = text {
            bytes.push(0); // custom section id
            wasm_encoder::CustomSection {
                name: name.into(),
                data: text.as_bytes().into(),
            }
            .encode(&mut bytes);
        }
    }
    Package::read(&bytes)?;
    Ok(bytes)
}

pub fn wit(bytes: &[u8]) -> Result<String> {
    match wit_component::decode(bytes)? {
        wit_component::DecodedWasm::Component(resolve, world) => {
            let pkg = resolve.worlds[world]
                .package
                .context("component has no WIT package")?;
            let nested: Vec<_> = resolve
                .packages
                .iter()
                .map(|(id, _)| id)
                .filter(|id| *id != pkg)
                .collect();
            let mut printer = wit_component::WitPrinter::default();
            printer.print(&resolve, pkg, &nested)?;
            Ok(printer.output.to_string())
        }
        _ => bail!("expected component, not WIT package"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const TOOL: &str = "name='sample'\nversion='1'\ndescription='demo'\nentrypoint='run'\n";

    #[test]
    fn missing_policy_is_unrestricted_but_invalid_or_duplicate_is_rejected() {
        let bytes = embed(wasm_encoder::Component::new().finish(), TOOL, None).unwrap();
        assert!(Package::read(&bytes).unwrap().policy.http.is_none());
        assert!(embed(bytes.clone(), TOOL, None).is_err());
        assert!(
            embed(
                wasm_encoder::Component::new().finish(),
                TOOL,
                Some("unexpected=true")
            )
            .is_err()
        );
        let mut duplicate = bytes;
        duplicate.push(0);
        wasm_encoder::CustomSection {
            name: "sat.tool.v1".into(),
            data: TOOL.as_bytes().into(),
        }
        .encode(&mut duplicate);
        assert!(Package::read(&duplicate).is_err());
    }
}
