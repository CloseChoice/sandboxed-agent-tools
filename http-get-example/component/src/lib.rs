wit_bindgen::generate!({ path: "../wit", world: "http-get", generate_all });

use wasi::http::{outgoing_handler, types::{Fields, IncomingBody, Method, OutgoingRequest, Scheme}};
use wasi::io::streams::StreamError;

struct HttpGet;

impl Guest for HttpGet {
    fn run(method: String, url: String) -> Result<String, String> {
        if method != "GET" {
            return Err("this example supports only GET".into());
        }
        let url = url::Url::parse(&url).map_err(|e| e.to_string())?;
        let scheme = match url.scheme() {
            "http" => Scheme::Http,
            "https" => Scheme::Https,
            _ => return Err("expected an http or https URL".into()),
        };
        if url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() {
            return Err("expected a host without URL credentials".into());
        }
        let authority = &url[url::Position::BeforeHost..url::Position::AfterPort];
        let path = &url[url::Position::BeforePath..url::Position::AfterQuery];
        let request = OutgoingRequest::new(Fields::new());
        request.set_method(&Method::Get).map_err(|_| "invalid method")?;
        request.set_scheme(Some(&scheme)).map_err(|_| "invalid scheme")?;
        request.set_authority(Some(authority)).map_err(|_| "invalid authority")?;
        request.set_path_with_query(Some(path)).map_err(|_| "invalid path")?;

        let pending = outgoing_handler::handle(request, None).map_err(|e| format!("{e:?}"))?;
        pending.subscribe().block();
        let response = pending.get().ok_or("response not ready")?
            .map_err(|_| "response already consumed")?
            .map_err(|e| format!("{e:?}"))?;
        if !(200..300).contains(&response.status()) {
            return Err(format!("HTTP {}", response.status()));
        }
        let body = response.consume().map_err(|_| "missing body")?;
        let stream = body.stream().map_err(|_| "missing stream")?;
        let mut bytes = Vec::new();
        loop {
            match stream.blocking_read(8192) {
                Ok(chunk) => {
                    if bytes.len() + chunk.len() > 65536 {
                        return Err("response exceeds 64 KiB".into());
                    }
                    bytes.extend(chunk);
                }
                Err(StreamError::Closed) => break,
                Err(_) => return Err("response read failed".into()),
            }
        }
        drop(stream);
        let trailers = IncomingBody::finish(body);
        trailers.subscribe().block();
        trailers.get().ok_or("body not finished")?
            .map_err(|_| "body already consumed")?
            .map_err(|e| format!("{e:?}"))?;
        String::from_utf8(bytes).map_err(|e| e.to_string())
    }
}

export!(HttpGet);
