//! A separately granted, exact CDP connection. No guest-selected destinations.
//!
//! This grants control of the entire Chrome instance behind the endpoint, including
//! its file/network authority. It is not a one-tab or browser-egress sandbox.
use anyhow::{Context, Result, bail, ensure};
use futures_util::{SinkExt, StreamExt};
use serde_json::{Value, json};
use std::{
    collections::VecDeque,
    io::Write,
    path::PathBuf,
    process::{Child, Command, Stdio},
    time::Duration,
};
use tokio_tungstenite::{
    MaybeTlsStream, WebSocketStream,
    tungstenite::{Message, protocol::WebSocketConfig},
};
use wasmtime::{StoreContextMut, component::Linker};

pub const INTERFACE: &str = "sat:browser/connection@0.1.0";
const MAX_MESSAGE: usize = 16 * 1024 * 1024;
const MAX_EVENTS: usize = 256;
const MAX_EVENT_BYTES: usize = 1024 * 1024;
type Socket = WebSocketStream<MaybeTlsStream<tokio::net::TcpStream>>;

pub fn validate_endpoint(endpoint: &str) -> Result<()> {
    let url = url::Url::parse(endpoint).context("invalid CDP endpoint")?;
    let id = url.path().strip_prefix("/devtools/browser/").unwrap_or("");
    ensure!(
        url.scheme() == "ws"
            && url.host_str() == Some("127.0.0.1")
            && url.port().is_some_and(|p| p != 0)
            && url.username().is_empty()
            && url.password().is_none()
            && url.query().is_none()
            && url.fragment().is_none()
            && !id.is_empty()
            && id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
            && url.as_str() == endpoint,
        "CDP grant must be an exact ws://127.0.0.1:PORT/devtools/browser/ID endpoint"
    );
    Ok(())
}

pub struct Connection {
    endpoint: Option<String>,
    socket: Option<Socket>,
    next_id: u64,
    events: VecDeque<String>,
    event_bytes: usize,
    dropped: u64,
}

impl Connection {
    pub fn new(endpoint: Option<String>) -> Result<Self> {
        if let Some(endpoint) = &endpoint {
            validate_endpoint(endpoint)?;
        }
        Ok(Self {
            endpoint,
            socket: None,
            next_id: 0,
            events: VecDeque::new(),
            event_bytes: 0,
            dropped: 0,
        })
    }

    async fn connect(&mut self) -> Result<()> {
        let endpoint = self
            .endpoint
            .as_deref()
            .context("browser access denied: use --allow-cdp with a dedicated Chrome endpoint")?;
        if self.socket.is_none() {
            let config = WebSocketConfig::default()
                .max_message_size(Some(MAX_MESSAGE))
                .max_frame_size(Some(MAX_MESSAGE));
            let (socket, _) =
                tokio_tungstenite::connect_async_with_config(endpoint, Some(config), true)
                    .await
                    .map_err(|_| anyhow::anyhow!("CDP connection failed"))?;
            self.socket = Some(socket);
        }
        Ok(())
    }

    fn buffer(&mut self, event: String) {
        if event.len() > MAX_EVENT_BYTES {
            self.dropped += 1;
            return;
        }
        while self.events.len() >= MAX_EVENTS || self.event_bytes + event.len() > MAX_EVENT_BYTES {
            if let Some(old) = self.events.pop_front() {
                self.event_bytes -= old.len();
                self.dropped += 1;
            }
        }
        self.event_bytes += event.len();
        self.events.push_back(event);
    }

    async fn receive(&mut self) -> Result<Value> {
        loop {
            let socket = self.socket.as_mut().context("CDP disconnected")?;
            match socket
                .next()
                .await
                .context("CDP disconnected")?
                .context("CDP transport failed")?
            {
                Message::Text(text) => {
                    return serde_json::from_str(&text).context("invalid CDP response");
                }
                Message::Ping(bytes) => socket.send(Message::Pong(bytes)).await?,
                Message::Close(_) => bail!("CDP disconnected"),
                _ => {}
            }
        }
    }

    async fn exchange_inner(&mut self, message: &str) -> Result<String> {
        ensure!(message.len() <= 1024 * 1024, "CDP command exceeds 1 MiB");
        let mut command: Value =
            serde_json::from_str(message).context("invalid CDP command JSON")?;
        let object = command
            .as_object_mut()
            .context("CDP command must be an object")?;
        ensure!(
            object.get("method").is_some_and(Value::is_string)
                && object
                    .keys()
                    .all(|k| ["method", "params", "sessionId"].contains(&k.as_str())),
            "invalid CDP command fields"
        );
        self.connect().await?;
        self.next_id += 1;
        object.insert("id".into(), self.next_id.into());
        self.socket
            .as_mut()
            .unwrap()
            .send(Message::Text(command.to_string().into()))
            .await?;
        loop {
            let reply = self.receive().await?;
            if reply["id"].as_u64() == Some(self.next_id) {
                return Ok(reply.to_string());
            }
            if reply.get("method").is_some() {
                self.buffer(reply.to_string());
            }
        }
    }

    pub async fn exchange(&mut self, message: &str) -> Result<String, String> {
        let outcome =
            tokio::time::timeout(Duration::from_secs(15), self.exchange_inner(message)).await;
        match outcome {
            Ok(Ok(reply)) => Ok(reply),
            Ok(Err(error)) => {
                self.socket = None;
                Err(error.to_string())
            }
            Err(_) => {
                self.socket = None;
                Err("CDP command timed out; connection discarded".into())
            }
        }
    }

    pub async fn poll(&mut self, milliseconds: u32) -> Result<String, String> {
        if milliseconds > 1000 {
            return Err("event poll exceeds 1000 ms".into());
        }
        let operation = async {
            self.connect().await?;
            let deadline = tokio::time::Instant::now() + Duration::from_millis(milliseconds.into());
            loop {
                match tokio::time::timeout_at(deadline, self.receive()).await {
                    Ok(Ok(event)) => {
                        if event.get("method").is_some() {
                            self.buffer(event.to_string());
                        }
                    }
                    Ok(Err(e)) => {
                        self.socket = None;
                        return Err(e);
                    }
                    Err(_) => break,
                }
            }
            let events: Vec<Value> = self
                .events
                .drain(..)
                .map(|s| serde_json::from_str(&s).unwrap())
                .collect();
            self.event_bytes = 0;
            let dropped = std::mem::take(&mut self.dropped);
            Ok(json!({"events": events, "dropped": dropped}).to_string())
        };
        // Includes the handshake, which is not bounded by the event deadline.
        match tokio::time::timeout(Duration::from_secs(5), operation).await {
            Ok(result) => result.map_err(|e| e.to_string()),
            Err(_) => {
                self.socket = None;
                Err("CDP connection timed out".into())
            }
        }
    }
}

pub fn link(linker: &mut Linker<crate::runtime::RuntimeState>) -> Result<()> {
    let mut interface = linker.instance(INTERFACE)?;
    interface.func_wrap_async(
        "exchange",
        |mut store: StoreContextMut<'_, crate::runtime::RuntimeState>, (message,): (String,)| {
            Box::new(async move { Ok((store.data_mut().browser.exchange(&message).await,)) })
        },
    )?;
    interface.func_wrap_async(
        "poll",
        |mut store: StoreContextMut<'_, crate::runtime::RuntimeState>, (milliseconds,): (u32,)| {
            Box::new(async move { Ok((store.data_mut().browser.poll(milliseconds).await,)) })
        },
    )?;
    Ok(())
}

struct Chrome(Child);
impl Drop for Chrome {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

/// Retain the session until stdin EOF (or the optional duration), or Ctrl-C.
pub async fn serve(executable: Option<PathBuf>, headed: bool, duration: Option<u64>) -> Result<()> {
    let profile = tempfile::Builder::new().prefix("sat-chrome-").tempdir()?;
    let candidates = executable.map(|p| vec![p]).unwrap_or_else(|| {
        [
            "google-chrome",
            "chromium",
            "chromium-browser",
            "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
            "C:\\Program Files\\Google\\Chrome\\Application\\chrome.exe",
            "C:\\Program Files (x86)\\Google\\Chrome\\Application\\chrome.exe",
        ]
        .iter()
        .map(PathBuf::from)
        .collect()
    });
    let mut child = None;
    for candidate in candidates {
        let mut command = Command::new(candidate);
        command
            .arg(format!("--user-data-dir={}", profile.path().display()))
            .args([
                "--remote-debugging-port=0",
                "--remote-debugging-address=127.0.0.1",
                "--no-first-run",
                "--no-default-browser-check",
                "--disable-background-networking",
                "--window-size=1280,900",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        if !headed {
            command.arg("--headless=new");
        }
        command.arg("about:blank");
        match command.spawn() {
            Ok(process) => {
                child = Some(Chrome(process));
                break;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => return Err(error).context("could not launch Chrome"),
        }
    }
    let mut child = child.context("Chrome not found; supply --chrome PATH")?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let endpoint = loop {
        ensure!(
            child.0.try_wait()?.is_none(),
            "Chrome exited before debugging became available (its OS sandbox must be usable)"
        );
        if let Ok(contents) = std::fs::read_to_string(profile.path().join("DevToolsActivePort")) {
            let mut lines = contents.lines();
            if let (Some(port), Some(route)) = (lines.next(), lines.next()) {
                let endpoint = format!("ws://127.0.0.1:{port}{route}");
                validate_endpoint(&endpoint)?;
                break endpoint;
            }
        }
        ensure!(
            tokio::time::Instant::now() < deadline,
            "Chrome startup timed out"
        );
        tokio::time::sleep(Duration::from_millis(50)).await;
    };
    let lifetime = duration
        .map(|s| format!("{s} seconds or Ctrl-C"))
        .unwrap_or_else(|| "until stdin EOF or Ctrl-C".into());
    println!(
        "{}",
        json!({"endpoint": endpoint, "scope": "dedicated-chrome-instance", "lifetime": lifetime})
    );
    std::io::stdout().flush()?;
    let (sender, mut receiver) = tokio::sync::oneshot::channel();
    // A plain detached thread avoids a blocking Tokio stdin task holding up shutdown.
    if duration.is_none() {
        std::thread::spawn(move || {
            use std::io::Read;
            let mut bytes = [0; 1024];
            while std::io::stdin().read(&mut bytes).is_ok_and(|n| n != 0) {}
            let _ = sender.send(());
        });
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(duration.unwrap_or(3600));
    loop {
        tokio::select! {
            _ = &mut receiver, if duration.is_none() => break,
            _ = tokio::time::sleep_until(deadline), if duration.is_some() => break,
            signal = tokio::signal::ctrl_c() => { signal?; break; },
            _ = tokio::time::sleep(Duration::from_millis(100)) => {
                if child.0.try_wait()?.is_some() { bail!("Chrome session exited"); }
            }
        }
    }
    // Ask Chrome to release its children/profile before the kill-and-wait fallback.
    let mut connection = Connection::new(Some(endpoint))?;
    let _ = tokio::time::timeout(
        Duration::from_secs(2),
        connection.exchange(r#"{"method":"Browser.close"}"#),
    )
    .await;
    for _ in 0..20 {
        if child.0.try_wait()?.is_some() {
            break;
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    drop(child);
    drop(profile);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn incompatible_browser_import_is_rejected_before_execution() {
        let bytes = wat::parse_str(r#"(component
            (import "sat:browser/connection@0.1.0" (instance (export "exchange" (func (param "wrong" u32)))))
            (core module $m (func (export "run") (result i32) i32.const 0))
            (core instance $i (instantiate $m))
            (func (export "run") (result s32) (canon lift (core func $i "run")))
        )"#).unwrap();
        let metadata = crate::metadata::ToolMetadata::parse(
            "name='browser-test'\nversion='0.1.0'\ndescription='probe'\nentrypoint='run'",
        )
        .unwrap();
        assert!(crate::runtime::LoadedTool::load(&bytes, metadata).is_err());
    }
    #[test]
    fn only_exact_numeric_loopback_browser_endpoints() {
        validate_endpoint("ws://127.0.0.1:9222/devtools/browser/abc-123").unwrap();
        for endpoint in [
            "ws://localhost:9222/devtools/browser/abc",
            "ws://127.0.0.2:9222/devtools/browser/abc",
            "ws://example.com:9222/devtools/browser/abc",
            "wss://127.0.0.1:9222/devtools/browser/abc",
            "ws://127.0.0.1:9222/json",
            "ws://127.0.0.1:9222/devtools/page/abc",
            "ws://127.0.0.1:9222/devtools/browser/abc?x=1",
            "ws://user@127.0.0.1:9222/devtools/browser/abc",
            "ws://127.0.0.1:0/devtools/browser/abc",
            "ws://127.0.0.1:9222/devtools/browser/abc/",
            "ws://127.0.0.1:9222/devtools/browser/%61bc",
            "ws://127.0.0.1/devtools/browser/abc",
        ] {
            assert!(validate_endpoint(endpoint).is_err(), "{endpoint}");
        }
    }
    #[tokio::test]
    async fn no_grant_denies_both_imports() {
        let mut connection = Connection::new(None).unwrap();
        assert!(
            connection
                .exchange(r#"{"method":"Target.getTargets"}"#)
                .await
                .unwrap_err()
                .contains("denied")
        );
        assert!(connection.poll(1).await.unwrap_err().contains("denied"));
    }
    #[test]
    fn events_are_bounded_and_report_loss() {
        let mut connection = Connection::new(None).unwrap();
        for _ in 0..300 {
            connection.buffer("{}".into());
        }
        assert_eq!(connection.events.len(), MAX_EVENTS);
        assert_eq!(connection.dropped, 44);
        connection.buffer("x".repeat(MAX_EVENT_BYTES + 1));
        assert_eq!(connection.dropped, 45);
        assert!(connection.event_bytes <= MAX_EVENT_BYTES);
    }

    #[tokio::test]
    #[allow(clippy::result_large_err)] // The upstream handshake callback fixes this error type.
    async fn exact_route_interleaved_events_and_host_owned_ids() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!(
            "ws://127.0.0.1:{}/devtools/browser/test",
            listener.local_addr().unwrap().port()
        );
        let server = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.unwrap();
            let mut socket = tokio_tungstenite::accept_hdr_async(
                stream,
                |request: &tokio_tungstenite::tungstenite::handshake::server::Request, response| {
                    assert_eq!(request.uri().path(), "/devtools/browser/test");
                    Ok(response)
                },
            )
            .await
            .unwrap();
            for id in 1..=2 {
                let command: Value =
                    serde_json::from_str(socket.next().await.unwrap().unwrap().to_text().unwrap())
                        .unwrap();
                assert_eq!(command["id"], id);
                socket
                    .send(Message::Text(
                        json!({"method":"Runtime.consoleAPICalled","params":{"type":"log"}})
                            .to_string()
                            .into(),
                    ))
                    .await
                    .unwrap();
                socket
                    .send(Message::Text(
                        json!({"id": id,"result":{"ok":true}}).to_string().into(),
                    ))
                    .await
                    .unwrap();
            }
            // Retain the transport while the client drains buffered events.
            tokio::time::sleep(Duration::from_millis(200)).await;
        });
        let mut connection = Connection::new(Some(endpoint)).unwrap();
        for _ in 0..2 {
            let result: Value = serde_json::from_str(
                &connection
                    .exchange(r#"{"method":"Runtime.enable","params":{}}"#)
                    .await
                    .unwrap(),
            )
            .unwrap();
            assert_eq!(result["result"]["ok"], true);
        }
        let events: Value = serde_json::from_str(&connection.poll(10).await.unwrap()).unwrap();
        assert_eq!(events["events"].as_array().unwrap().len(), 2);
        assert_eq!(events["dropped"], 0);
        assert!(connection.poll(1001).await.is_err());
        server.await.unwrap();
    }

    #[tokio::test]
    async fn commands_cannot_override_endpoint_or_host_request_id() {
        let mut connection =
            Connection::new(Some("ws://127.0.0.1:9/devtools/browser/test".into())).unwrap();
        for command in [
            r#"{"method":"Runtime.enable","endpoint":"ws://127.0.0.1:80/"}"#,
            r#"{"method":"Runtime.enable","id":100}"#,
            "[]",
            "null",
            "{}",
            "not json",
        ] {
            let error = connection.exchange(command).await.unwrap_err();
            assert!(
                !error.contains("connection failed"),
                "command must be rejected before connecting: {error}"
            );
        }
    }

    #[tokio::test]
    async fn websocket_handshake_redirect_is_not_followed() {
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let endpoint = format!(
            "ws://127.0.0.1:{}/devtools/browser/test",
            listener.local_addr().unwrap().port()
        );
        let destination = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let redirect = format!(
            "HTTP/1.1 302 Found\r\nLocation: ws://127.0.0.1:{}/devtools/browser/other\r\nContent-Length: 0\r\n\r\n",
            destination.local_addr().unwrap().port()
        );
        let server = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            let mut bytes = [0; 4096];
            let received = stream.read(&mut bytes).await.unwrap();
            assert!(received > 0);
            stream.write_all(redirect.as_bytes()).await.unwrap();
        });
        let mut connection = Connection::new(Some(endpoint)).unwrap();
        assert!(
            connection
                .exchange(r#"{"method":"Runtime.enable"}"#)
                .await
                .is_err()
        );
        assert!(
            tokio::time::timeout(Duration::from_millis(100), destination.accept())
                .await
                .is_err()
        );
        server.await.unwrap();
    }
}
