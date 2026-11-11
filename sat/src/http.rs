//! Every WASI HTTP request passes through this hook. Redirects are not followed
//! by the transport; guest-initiated follow-up requests are checked afresh.

use crate::config::HttpPermissions;
use anyhow::{Result, ensure};
use http::{Request, Response};
use http_body_util::BodyExt;
use std::future::Future;
use wasmtime_wasi_http::{Error, RequestOptions, WasiBody, WasiHttpHooks};

type IoFuture = Box<dyn Future<Output = Result<(), Error>> + Send>;

type ResponseFuture =
    Box<dyn Future<Output = Result<(Response<WasiBody>, IoFuture), Error>> + Send>;
impl HttpPermissions {
    pub fn validate(&self) -> Result<()> {
        ensure!(
            self.allow_http.is_some() ^ self.deny_http.is_some(),
            "set exactly one of allow-http or deny-http"
        );

        for rule in self
            .allow_http
            .iter()
            .chain(self.deny_http.iter())
            .flatten()
        {
            let host = rule.strip_prefix("*.").unwrap_or(rule);
            ensure!(
                !host.is_empty()
                    && !host.contains(['/', ':', '@', '*', '?', '#'])
                    && host.is_ascii()
                    && host.split('.').all(|s| !s.is_empty()
                        && !s.starts_with('-')
                        && !s.ends_with('-')
                        && s.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')),
                "invalid HTTP hostname rule {rule}"
            );
        }

        Ok(())
    }

    pub fn permits(&self, uri: &http::Uri) -> bool {
        if !matches!(uri.scheme_str(), Some("http" | "https"))
            || uri.authority().is_none_or(|a| a.as_str().contains('@'))
        {
            return false;
        }

        let Some(host) = uri.host() else {
            return false;
        };
        let host = host.trim_end_matches('.').to_ascii_lowercase();

        let matches = |rule: &String| {
            let rule = rule.trim_end_matches('.').to_ascii_lowercase();
            if let Some(base) = rule.strip_prefix("*.") {
                host.len() > base.len() + 1 && host.ends_with(&format!(".{base}"))
            } else {
                host == rule
            }
        };

        match (&self.allow_http, &self.deny_http) {
            (Some(list), None) => list.iter().any(matches),
            (None, Some(list)) => !list.iter().any(matches),
            _ => false,
        }
    }
}

impl WasiHttpHooks for HttpPermissions {
    fn send_request(
        &mut self,
        mut request: Request<WasiBody>,
        _: Option<RequestOptions>,
        _: IoFuture,
    ) -> ResponseFuture {
        if !self.permits(request.uri()) {
            return Box::new(async { Err(Error::HttpRequestDenied) });
        }

        let Ok(host) = request.uri().authority().unwrap().as_str().parse() else {
            return Box::new(async { Err(Error::HttpRequestDenied) });
        };
        request.headers_mut().insert(http::header::HOST, host);

        Box::new(async move {
            let (response, io) = wasmtime_wasi_http::default_send_request(request, None).await?;
            Ok((
                response.map(BodyExt::boxed_unsync),
                Box::new(io) as IoFuture,
            ))
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn matching_and_modes() {
        let p = HttpPermissions {
            allow_http: Some(vec!["*.brave.com".into(), "exact.com".into()]),
            deny_http: None,
        };
        p.validate().unwrap();

        for url in [
            "https://api.brave.com/a?x=1",
            "http://foo.api.brave.com:99/",
            "https://exact.com/",
        ] {
            assert!(p.permits(&url.parse().unwrap()));
        }

        for url in [
            "https://brave.com/",
            "https://evilbrave.com/",
            "https://brave.com.evil.com/",
            "https://sub.exact.com/",
            "https://user@exact.com/",
        ] {
            assert!(!p.permits(&url.parse().unwrap()));
        }
        assert!(!HttpPermissions::default().permits(&"https://exact.com/".parse().unwrap()));

        let deny = HttpPermissions {
            allow_http: None,
            deny_http: Some(vec![]),
        };
        assert!(deny.permits(&"http://localhost:123/".parse().unwrap()));
        assert!(
            HttpPermissions {
                allow_http: Some(vec![]),
                deny_http: Some(vec![])
            }
            .validate()
            .is_err()
        );
    }
}
