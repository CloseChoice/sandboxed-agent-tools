mod bindings {
    use super::BraveSearch;
    wit_bindgen::generate!({ path: "../wit", world: "brave-search", generate_all });
    export!(BraveSearch with_types_in self);
}

use bindings::exports::example::brave_search::search::{Guest, Hit};
use bindings::wasi;

struct BraveSearch;

impl Guest for BraveSearch {
    fn search(query: String) -> Result<Vec<Hit>, String> {
        search(&query)
    }
}

impl bindings::Guest for BraveSearch {
    fn run(query: String) -> Result<Vec<bindings::SearchHit>, String> {
        search(&query).map(|hits| {
            hits.into_iter()
                .map(|hit| bindings::SearchHit {
                    title: hit.title,
                    url: hit.url,
                    description: hit.description,
                })
                .collect()
        })
    }

    fn sandbox_probe() -> Result<(), String> {
        probe()
    }
}

use wasi::http::{
    outgoing_handler,
    types::{Fields, IncomingBody, Method, OutgoingRequest, Scheme},
};
use wasi::io::streams::StreamError;

fn search(query: &str) -> Result<Vec<Hit>, String> {
    let query = url::form_urlencoded::Serializer::new(String::new())
        .append_pair("q", &query)
        .append_pair("count", "5")
        .finish();
    let request = request(
        "api.search.brave.com",
        &format!("/res/v1/web/search?{query}"),
    )?;
    // No authentication header: the host adds it after checking the destination.
    let pending = outgoing_handler::handle(request, None).map_err(|e| format!("request: {e:?}"))?;
    pending.subscribe().block();
    let response = pending
        .get()
        .ok_or("response is not ready")?
        .map_err(|_| "response already consumed")?
        .map_err(|e| format!("request failed: {e:?}"))?;
    if response.status() != 200 {
        // Don't echo provider error bodies, which could contain sensitive data.
        return Err(format!("provider returned HTTP {}", response.status()));
    }
    let body = response.consume().map_err(|_| "missing response body")?;
    let stream = body.stream().map_err(|_| "missing response stream")?;
    let mut bytes = Vec::new();
    loop {
        match stream.blocking_read(16 * 1024) {
            Ok(chunk) => {
                if bytes.len() + chunk.len() > 2 * 1024 * 1024 {
                    return Err("response exceeds 2 MiB".into());
                }
                bytes.extend(chunk);
            }
            Err(StreamError::Closed) => break,
            Err(_) => return Err("response body read failed".into()),
        }
    }
    drop(stream);
    let trailers = IncomingBody::finish(body);
    trailers.subscribe().block();
    trailers
        .get()
        .ok_or("trailers are not ready")?
        .map_err(|_| "trailers already consumed")?
        .map_err(|_| "response body incomplete")?;
    let json: serde_json::Value =
        serde_json::from_slice(&bytes).map_err(|_| "invalid provider JSON")?;
    json["web"]["results"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|hit| {
            Ok(Hit {
                title: hit["title"].as_str().ok_or("missing result title")?.into(),
                url: hit["url"].as_str().ok_or("missing result URL")?.into(),
                description: hit["description"].as_str().unwrap_or_default().into(),
            })
        })
        .collect()
}

fn request(authority: &str, path: &str) -> Result<OutgoingRequest, String> {
    let headers = Fields::new();
    headers
        .set("accept", &[b"application/json".to_vec()])
        .map_err(|_| "invalid header")?;
    let request = OutgoingRequest::new(headers);
    request
        .set_method(&Method::Get)
        .map_err(|_| "invalid method")?;
    request
        .set_scheme(Some(&Scheme::Https))
        .map_err(|_| "invalid scheme")?;
    request
        .set_authority(Some(authority))
        .map_err(|_| "invalid authority")?;
    request
        .set_path_with_query(Some(path))
        .map_err(|_| "invalid path")?;
    Ok(request)
}

// Executed by the host's offline integration test inside the actual component.
fn probe() -> Result<(), String> {
    if std::env::vars_os().next().is_some() {
        return Err("guest unexpectedly received environment variables".into());
    }
    if std::fs::read("/etc/passwd").is_ok() || std::fs::write("/escape.txt", "no").is_ok() {
        return Err("guest unexpectedly received filesystem access".into());
    }
    if std::net::TcpStream::connect("127.0.0.1:9").is_ok() {
        return Err("guest unexpectedly connected a socket".into());
    }
    let req = request("example.com", "/")?;
    match outgoing_handler::handle(req, None) {
        Err(wasi::http::types::ErrorCode::HttpRequestDenied) => {}
        Ok(pending) => {
            pending.subscribe().block();
            if !matches!(
                pending.get(),
                Some(Ok(Err(wasi::http::types::ErrorCode::HttpRequestDenied)))
            ) {
                return Err("unrelated HTTP destination was not denied by policy".into());
            }
        }
        _ => return Err("unexpected HTTP rejection".into()),
    }
    Ok(())
}
