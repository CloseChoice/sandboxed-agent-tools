use base64::{Engine, engine::general_purpose::STANDARD};
use serde::Deserialize;
use serde_json::{Value, json};
use std::time::{Duration, Instant};

// Canonical ABI lowering generates a function with more than seven arguments.
#[allow(clippy::too_many_arguments)]
mod bindings {
    wit_bindgen::generate!({ world: "browser" });
}
use bindings::{Guest, sat::browser::connection};

struct BrowserTool;
bindings::export!(BrowserTool with_types_in bindings);

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    action: String,
    target: Option<String>,
    value: Option<String>,
    output: Option<String>,
    timeout: Option<u32>,
}

struct Browser {
    session: String,
    events: Vec<Value>,
    dropped: u64,
}

fn exchange(method: &str, params: Value, session: Option<&str>) -> Result<Value, String> {
    let mut request = json!({"method": method, "params": params});
    if let Some(session) = session {
        request["sessionId"] = session.into();
    }
    let reply: Value = serde_json::from_str(&connection::exchange(&request.to_string())?)
        .map_err(|e| e.to_string())?;
    if let Some(error) = reply.get("error") {
        return Err(format!("{method}: {error}"));
    }
    Ok(reply["result"].clone())
}

impl Guest for BrowserTool {
    fn run(
        action: String,
        target: Option<String>,
        value: Option<String>,
        tab: Option<String>,
        output: Option<String>,
        timeout: u32,
    ) -> Result<String, String> {
        if !(1..=30_000).contains(&timeout) {
            return Err("timeout must be 1..30000 ms".into());
        }
        let targets = exchange("Target.getTargets", json!({}), None)?;
        let pages: Vec<Value> = targets["targetInfos"]
            .as_array()
            .ok_or("invalid targets")?
            .iter()
            .filter(|t| t["type"] == "page")
            .cloned()
            .collect();
        if action == "tabs" {
            return Ok(format!("{}\n", json!({"tabs": pages})));
        }
        if action == "new" {
            return Ok(format!(
                "{}\n",
                exchange(
                    "Target.createTarget",
                    json!({"url": value.unwrap_or("about:blank".into())}),
                    None
                )?
            ));
        }
        let tab = match tab {
            Some(id) if pages.iter().any(|t| t["targetId"] == id) => id,
            Some(_) => return Err("tab is not a page in the granted Chrome instance".into()),
            None if pages.len() == 1 => pages[0]["targetId"]
                .as_str()
                .ok_or("invalid target ID")?
                .into(),
            _ => return Err("choose --tab from tabs (expected exactly one page)".into()),
        };
        if action == "close" {
            return Ok(format!(
                "{}\n",
                exchange("Target.closeTarget", json!({"targetId": tab}), None)?
            ));
        }
        let attached = exchange(
            "Target.attachToTarget",
            json!({"targetId": tab, "flatten": true}),
            None,
        )?;
        let mut browser = Browser {
            session: attached["sessionId"]
                .as_str()
                .ok_or("missing session ID")?
                .into(),
            events: Vec::new(),
            dropped: 0,
        };
        browser.command("Page.enable", json!({}))?;
        browser.command("Runtime.enable", json!({}))?;
        browser.command(
            "Network.enable",
            json!({"maxTotalBufferSize": 1_048_576, "maxResourceBufferSize": 262_144}),
        )?;
        let steps: Vec<Step> = if action == "batch" {
            let source = value
                .as_deref()
                .ok_or("batch requires --value containing a JSON array")?;
            if source.len() > 1024 * 1024 {
                return Err("batch exceeds 1 MiB".into());
            }
            serde_json::from_str(source).map_err(|e| format!("invalid batch: {e}"))?
        } else {
            vec![Step {
                action,
                target,
                value,
                output,
                timeout: None,
            }]
        };
        if steps.is_empty() || steps.len() > 100 {
            return Err("batch must contain 1..100 steps".into());
        }
        let deadline = Instant::now() + Duration::from_secs(120);
        let mut results = Vec::new();
        for (index, step) in steps.iter().enumerate() {
            if Instant::now() >= deadline {
                return Err("batch exceeded 120 seconds".into());
            }
            let timeout = step.timeout.unwrap_or(timeout);
            if !(1..=30_000).contains(&timeout) {
                return Err("step timeout must be 1..30000 ms".into());
            }
            let result = browser
                .step(step, timeout)
                .map_err(|error| format!("step {} ({}): {error}", index + 1, step.action))?;
            results.push(json!({"action": step.action, "result": result}));
        }
        Ok(format!("{}\n", json!({"tab": tab, "results": results})))
    }
}

impl Browser {
    fn command(&self, method: &str, params: Value) -> Result<Value, String> {
        exchange(method, params, Some(&self.session))
    }

    fn evaluate(&self, expression: &str, timeout: u32) -> Result<Value, String> {
        let reply = self.command("Runtime.evaluate", json!({"expression": expression, "returnByValue": true, "awaitPromise": true, "timeout": timeout}))?;
        if let Some(exception) = reply.get("exceptionDetails") {
            return Err(format!(
                "JavaScript exception: {}",
                exception["exception"]["description"]
                    .as_str()
                    .unwrap_or("evaluation failed")
            ));
        }
        if reply["result"]["subtype"] == "error" {
            return Err("JavaScript evaluation failed".into());
        }
        Ok(reply["result"]["value"].clone())
    }

    fn selector(target: Option<&str>) -> Result<String, String> {
        let target = target.ok_or("action requires --target CSS selector")?;
        // Each >>> descends through an open shadow root or same-origin iframe.
        Ok(format!(
            r#"(()=>{{let root=document, el; const parts={}.split('>>>'); for(let i=0;i<parts.length;i++){{el=root.querySelector(parts[i].trim());if(!el)return null;if(i<parts.length-1){{root=el.shadowRoot||el.contentDocument;if(!root)return null;}}}}return el;}})()"#,
            json!(target)
        ))
    }

    fn collect(&mut self, milliseconds: u32) -> Result<(), String> {
        let events: Value =
            serde_json::from_str(&connection::poll(milliseconds)?).map_err(|e| e.to_string())?;
        self.dropped += events["dropped"].as_u64().unwrap_or(0);
        for event in events["events"]
            .as_array()
            .ok_or("invalid event response")?
        {
            if event["sessionId"] != self.session {
                continue;
            }
            let method = event["method"].as_str().unwrap_or("");
            if matches!(
                method,
                "Runtime.consoleAPICalled"
                    | "Runtime.exceptionThrown"
                    | "Network.loadingFailed"
                    | "Network.requestWillBeSent"
                    | "Network.responseReceived"
            ) {
                if self.events.len() == 128 {
                    self.events.remove(0);
                    self.dropped += 1;
                }
                // Keep useful summaries, rather than arbitrary response bodies or headers.
                let p = &event["params"];
                let summary = match method {
                    "Runtime.consoleAPICalled" => {
                        json!({"method": method, "level": p["type"], "args": p["args"].as_array().map(|a| a.iter().take(10).map(|v| truncate(v.get("value").unwrap_or(&v["description"]), 2000)).collect::<Vec<_>>())})
                    }
                    "Runtime.exceptionThrown" => {
                        json!({"method": method, "text": truncate(&p["exceptionDetails"], 4000)})
                    }
                    "Network.requestWillBeSent" => {
                        json!({"method": method, "requestId": p["requestId"], "url": truncate(&p["request"]["url"], 2000), "verb": p["request"]["method"]})
                    }
                    "Network.responseReceived" => {
                        json!({"method": method, "requestId": p["requestId"], "url": truncate(&p["response"]["url"], 2000), "status": p["response"]["status"]})
                    }
                    _ => {
                        json!({"method": method, "requestId": p["requestId"], "error": p["errorText"], "blocked": p["blockedReason"]})
                    }
                };
                self.events.push(summary);
            }
        }
        Ok(())
    }

    fn wait(&mut self, selector: &str, text: Option<&str>, timeout: u32) -> Result<Value, String> {
        let expression = format!(
            r#"(()=>{{let el={selector};if(!el)return false;const r=el.getBoundingClientRect(),s=getComputedStyle(el);return r.width>0&&r.height>0&&s.visibility!=='hidden'&&s.display!=='none'&&{};}})()"#,
            text.map(|s| format!("(el.textContent||'').includes({})", json!(s)))
                .unwrap_or("true".into())
        );
        let deadline = Instant::now() + Duration::from_millis(timeout.into());
        loop {
            match self.evaluate(&expression, timeout) {
                Ok(Value::Bool(true)) => return Ok(json!({"matched": true})),
                // Navigation can replace the default execution context while waiting.
                Err(e) if e.contains("context") || e.contains("navigat") => {}
                Err(e) => return Err(e),
                _ => {}
            }
            if Instant::now() >= deadline {
                return Err("condition timed out".into());
            }
            self.collect(50)?;
        }
    }

    fn step(&mut self, step: &Step, timeout: u32) -> Result<Value, String> {
        let value = || {
            step.value
                .as_deref()
                .ok_or("action requires --value".to_owned())
        };
        match step.action.as_str() {
            "navigate" => {
                let result = self.command("Page.navigate", json!({"url": value()?}))?;
                if result.get("errorText").is_some() {
                    return Err(format!("navigation failed: {}", result["errorText"]));
                }
                Ok(result)
            }
            "evaluate" => self.evaluate(value()?, timeout),
            "cdp" => {
                let request: Value = serde_json::from_str(value()?).map_err(|e| e.to_string())?;
                self.command(
                    request["method"].as_str().ok_or("CDP requires method")?,
                    request.get("params").cloned().unwrap_or(json!({})),
                )
            }
            "snapshot" => self.evaluate(include_str!("snapshot.js"), timeout),
            "query" => {
                let selector = Self::selector(step.target.as_deref())?;
                self.evaluate(&format!(r#"(()=>{{let el={selector};if(!el)return null;const r=el.getBoundingClientRect();return {{tag:el.tagName.toLowerCase(),text:(el.innerText||el.textContent||'').slice(0,8000),value:el.value??null,html:el.outerHTML.slice(0,16000),rect:{{x:r.x,y:r.y,width:r.width,height:r.height}}}};}})()"#), timeout)
            }
            "wait-visible" | "wait-text" => self.wait(
                &Self::selector(step.target.as_deref())?,
                if step.action == "wait-text" {
                    Some(value()?)
                } else {
                    None
                },
                timeout,
            ),
            "click" | "fill" | "type" => {
                if step.action != "click" {
                    value()?;
                }
                let selector = Self::selector(step.target.as_deref())?;
                self.wait(&selector, None, timeout)?;
                let point = self.evaluate(
                    &include_str!("point.js").replace("SAT_SELECTOR", &selector),
                    timeout,
                )?;
                self.command("Input.dispatchMouseEvent", json!({"type":"mousePressed","x":point["x"],"y":point["y"],"button":"left","clickCount":1}))?;
                self.command("Input.dispatchMouseEvent", json!({"type":"mouseReleased","x":point["x"],"y":point["y"],"button":"left","clickCount":1}))?;
                if step.action != "click" {
                    if step.action == "fill" {
                        self.evaluate(&format!(r#"(()=>{{const el={selector};if(el.isContentEditable){{const doc=el.ownerDocument,r=doc.createRange();r.selectNodeContents(el);const s=doc.getSelection();s.removeAllRanges();s.addRange(r);}}else if(typeof el.select==='function')el.select();else throw Error('element is not editable');}})()"#), timeout)?;
                    }
                    self.command("Input.insertText", json!({"text": value()?}))?;
                }
                Ok(json!({"done": true}))
            }
            "press" => {
                let key = value()?;
                let code = match key { "Enter" => 13, "Tab" => 9, "Escape" => 27, "Backspace" => 8, "ArrowDown" => 40, "ArrowUp" => 38, "ArrowLeft" => 37, "ArrowRight" => 39, "Delete" => 46, _ => return Err("supported keys: Enter Tab Escape Backspace Delete ArrowUp ArrowDown ArrowLeft ArrowRight".into()) };
                self.command("Input.dispatchKeyEvent", json!({"type":"keyDown","key":key,"code":key,"windowsVirtualKeyCode":code,"text":if key == "Enter" {"\r"} else {""}}))?;
                self.command(
                    "Input.dispatchKeyEvent",
                    json!({"type":"keyUp","key":key,"code":key,"windowsVirtualKeyCode":code}),
                )?;
                Ok(json!({"done": true}))
            }
            "scroll" => {
                let delta: Value = serde_json::from_str(value()?)
                    .map_err(|_| "scroll value must be JSON {\"x\":0,\"y\":500}")?;
                self.command("Input.dispatchMouseEvent", json!({"type":"mouseWheel","x":640,"y":450,"deltaX":delta["x"].as_f64().unwrap_or(0.0),"deltaY":delta["y"].as_f64().ok_or("scroll requires numeric y")?}))?;
                Ok(json!({"done": true}))
            }
            "screenshot" => {
                let path = step
                    .output
                    .as_deref()
                    .ok_or("screenshot requires --output and a write grant")?;
                let mut params = json!({"format":"png", "captureBeyondViewport":false});
                if step.value.as_deref() == Some("full") {
                    let metrics = self.command("Page.getLayoutMetrics", json!({}))?;
                    let size = &metrics["cssContentSize"];
                    let w = size["width"].as_f64().ok_or("missing page width")?;
                    let h = size["height"].as_f64().ok_or("missing page height")?;
                    if w * h > 16_000_000.0 {
                        return Err("full screenshot exceeds 16 million pixels".into());
                    }
                    params = json!({"format":"png","captureBeyondViewport":true,"clip":{"x":0,"y":0,"width":w,"height":h,"scale":1}});
                }
                let result = self.command("Page.captureScreenshot", params)?;
                let bytes = STANDARD
                    .decode(result["data"].as_str().ok_or("missing screenshot")?)
                    .map_err(|_| "invalid screenshot encoding")?;
                std::fs::write(path, &bytes).map_err(|e| {
                    format!("screenshot write failed: {} (requires --write)", e.kind())
                })?;
                Ok(json!({"format":"png","bytes":bytes.len(),"saved":true}))
            }
            "diagnostics" => {
                self.collect(timeout.min(1000))?;
                Ok(
                    json!({"events":std::mem::take(&mut self.events),"dropped":std::mem::take(&mut self.dropped),"scope":"this invocation"}),
                )
            }
            _ => Err(format!("unknown action: {}", step.action)),
        }
    }
}

fn truncate(value: &Value, limit: usize) -> String {
    let text = value
        .as_str()
        .map(str::to_owned)
        .unwrap_or_else(|| value.to_string());
    text.chars().take(limit).collect()
}
