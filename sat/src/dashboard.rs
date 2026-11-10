//! Read-only loopback HTTP server for the embedded Svelte dashboard.
use crate::history::{self, ExecutionQuery};
use anyhow::{Context, Result, ensure};
use std::{io::Cursor, path::Path};
use tiny_http::{Header, Method, Request, Response, Server, StatusCode};

include!(concat!(env!("OUT_DIR"), "/dashboard_assets.rs"));

type Reply = Response<Cursor<Vec<u8>>>;
fn reply(status: u16, content_type: &str, body: impl Into<Vec<u8>>) -> Reply {
    let mut response = Response::from_data(body).with_status_code(StatusCode(status));
    for (name, value) in [
        ("Content-Type", content_type),
        ("Cache-Control", "no-store"),
        ("X-Content-Type-Options", "nosniff"),
        ("Referrer-Policy", "no-referrer"),
        ("X-Frame-Options", "DENY"),
        (
            "Content-Security-Policy",
            "default-src 'self'; script-src 'self'; style-src 'self' 'unsafe-inline'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
        ),
    ] {
        response.add_header(Header::from_bytes(name, value).unwrap());
    }
    response
}
fn header<'a>(request: &'a Request, name: &str) -> Option<&'a str> {
    request
        .headers()
        .iter()
        .find(|h| h.field.as_str().as_str().eq_ignore_ascii_case(name))
        .map(|h| h.value.as_str())
}

fn route(home: &Path, request: &Request, port: u16) -> Result<Reply> {
    let hosts = [format!("127.0.0.1:{port}"), format!("localhost:{port}")];
    if !header(request, "Host").is_some_and(|host| {
        hosts
            .iter()
            .any(|allowed| allowed.eq_ignore_ascii_case(host))
    }) {
        return Ok(reply(403, "text/plain", "Unrecognized host"));
    }
    if header(request, "Origin")
        .is_some_and(|origin| !hosts.iter().any(|host| origin == format!("http://{host}")))
        || header(request, "Sec-Fetch-Site") == Some("cross-site")
    {
        return Ok(reply(
            403,
            "text/plain",
            "Cross-origin requests are not allowed",
        ));
    }
    if !matches!(request.method(), Method::Get | Method::Head) {
        return Ok(reply(405, "text/plain", "This dashboard is read-only"));
    }
    let (path, query) = request.url().split_once('?').unwrap_or((request.url(), ""));
    if path == "/api/overview" {
        return Ok(reply(
            200,
            "application/json",
            serde_json::to_vec(&history::overview(home)?)?,
        ));
    }
    if path == "/api/executions" {
        let mut filter = ExecutionQuery {
            limit: 25,
            ..Default::default()
        };
        for (key, value) in url::form_urlencoded::parse(query.as_bytes()) {
            match key.as_ref() {
                "tool" if !value.is_empty() => filter.tool = Some(value.into_owned()),
                "status" if !value.is_empty() => {
                    if !matches!(value.as_ref(), "success" | "failed" | "running") {
                        return Ok(reply(400, "text/plain", "Invalid status"));
                    }
                    filter.status = Some(value.into_owned());
                }
                "q" => filter.search = value.into_owned(),
                "limit" | "offset" => {
                    let Ok(number) = value.parse::<u32>() else {
                        return Ok(reply(400, "text/plain", "Invalid pagination"));
                    };
                    if key == "limit" {
                        filter.limit = number.clamp(1, 100);
                    } else {
                        filter.offset = number;
                    }
                }
                _ => {}
            }
        }
        return Ok(reply(
            200,
            "application/json",
            serde_json::to_vec(&history::executions(home, &filter)?)?,
        ));
    }
    let path = if path == "/" { "/index.html" } else { path };
    if let Some((_, bytes)) = ASSETS.iter().find(|(name, _)| *name == path) {
        let mime = match path.rsplit('.').next() {
            Some("html") => "text/html; charset=utf-8",
            Some("js") => "text/javascript; charset=utf-8",
            Some("css") => "text/css; charset=utf-8",
            Some("svg") => "image/svg+xml",
            Some("woff2") => "font/woff2",
            _ => "application/octet-stream",
        };
        return Ok(reply(200, mime, bytes.to_vec()));
    }
    Ok(reply(404, "text/plain", "Not found"))
}

pub fn serve(home: &Path, port: u16, open_browser: bool) -> Result<()> {
    ensure!(
        ASSETS.iter().any(|(name, _)| *name == "/index.html"),
        "dashboard assets missing; run npm run build in sat/dashboard"
    );
    let server = Server::http((std::net::Ipv4Addr::LOCALHOST, port))
        .map_err(|e| anyhow::anyhow!("cannot bind localhost dashboard: {e}"))?;
    let port = server
        .server_addr()
        .to_ip()
        .context("missing listener address")?
        .port();
    let address = format!("http://127.0.0.1:{port}");
    println!("SAT dashboard: {address}");
    println!("Press Ctrl+C to stop. Recording is managed with sat dashboard enable/disable.");
    if open_browser && let Err(error) = webbrowser::open(&address) {
        eprintln!("Could not open browser: {error}. Open {address} manually.");
    }
    for request in server.incoming_requests() {
        let response = match route(home, &request, port) {
            Ok(response) => response,
            Err(error) => {
                eprintln!("Dashboard request failed: {error:#}");
                reply(
                    500,
                    "application/json",
                    br#"{"error":"Could not read execution history"}"#.to_vec(),
                )
            }
        };
        let _ = request.respond(response);
    }
    Ok(())
}
