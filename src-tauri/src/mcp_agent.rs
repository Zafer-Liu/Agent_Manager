use crate::llm::LlmProvider;
use crate::mcp::McpServer;
use crate::process_util::no_window;
use reqwest::blocking::Client as HttpClient;
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Arc, Mutex};

// ── stderr ring buffer ──────────────────────────────────────────────────────

/// 轻量诊断缓存：保留最近 2KB 或 80 行 stderr（取更小者）。
/// 只在失败时供 FailureTrace.stderr_excerpt 使用，成功路径不落盘。
const STDERR_MAX_BYTES: usize = 2048;
const STDERR_MAX_LINES: usize = 80;

pub(crate) struct StderrRing {
    lines: Vec<String>,
    total_bytes: usize,
}

impl StderrRing {
    fn new() -> Self {
        StderrRing {
            lines: Vec::with_capacity(STDERR_MAX_LINES),
            total_bytes: 0,
        }
    }

    fn push_line(&mut self, line: &str) {
        if line.is_empty() {
            return;
        }
        let line_bytes = line.len() + 1; // +1 for newline
        self.total_bytes += line_bytes;
        self.lines.push(line.to_string());
        // 按 80 行截断
        if self.lines.len() > STDERR_MAX_LINES {
            let removed = self.lines.remove(0);
            self.total_bytes -= removed.len() + 1;
        }
        // 按 2KB 截断
        while self.total_bytes > STDERR_MAX_BYTES && !self.lines.is_empty() {
            let removed = self.lines.remove(0);
            self.total_bytes -= removed.len() + 1;
        }
    }

    /// 返回截断后的 stderr 摘要（拼接为单个字符串）。
    fn excerpt(&self) -> Option<String> {
        if self.lines.is_empty() {
            return None;
        }
        Some(self.lines.join("\n"))
    }
}

// ── Public types ────────────────────────────────────────────────────────────

// ── MCP stdio client ────────────────────────────────────────────────────────

pub(crate) struct McpClient {
    stdin: ChildStdin,
    reader: BufReader<ChildStdout>,
    stderr_ring: Arc<Mutex<StderrRing>>,
    _child: Child,
    req_id: u64,
}

/// Resolve an MCP server command into a spawnable (executable, leading_args).
///
/// On Windows, a packaged GUI process does not inherit a shell PATH that
/// includes `C:\Program Files\nodejs` or the npm-global dir, so a bare
/// `node` / `npx` / `<some>-mcp` command fails with "program not found".
/// We resolve those to absolute paths the same way agent startup does, so
/// users don't have to type the full `C:\Program Files\nodejs\node` path.
#[cfg(windows)]
fn resolve_mcp_command(command: &str) -> (String, Vec<String>) {
    // Bare `node` → absolute node.exe path.
    let lower = command.to_lowercase();
    if lower == "node" || lower == "node.exe" {
        return (crate::commands::find_node_exe_path(), vec![]);
    }

    // npm-global command (npx, or an installed *-mcp bin) → its .cmd wrapper,
    // launched through cmd.exe so the wrapper script runs.
    if let Some(p) = crate::commands::resolve_npm_global(command) {
        let cmd_path = p.to_string_lossy().to_string();
        return ("cmd.exe".to_string(), vec!["/c".to_string(), cmd_path]);
    }

    // Explicit .cmd / .bat → route through cmd.exe.
    if lower.ends_with(".cmd") || lower.ends_with(".bat") {
        return (
            "cmd.exe".to_string(),
            vec!["/c".to_string(), command.to_string()],
        );
    }

    (command.to_string(), vec![])
}

#[cfg(not(windows))]
fn resolve_mcp_command(command: &str) -> (String, Vec<String>) {
    (command.to_string(), vec![])
}

impl McpClient {
    pub(crate) fn start(server: &McpServer) -> Result<Self, String> {
        let (exe, leading) = resolve_mcp_command(&server.command);
        let mut cmd = Command::new(&exe);
        cmd.args(&leading)
            .args(&server.args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        no_window(&mut cmd);
        for (k, v) in &server.env {
            cmd.env(k, v);
        }
        let mut child = cmd
            .spawn()
            .map_err(|e| format!("Failed to start MCP '{}': {}", server.name, e))?;
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let stderr = child.stderr.take().unwrap();

        // 启动 stderr 读取线程，写入 ring buffer（不阻塞 stdout JSON-RPC 主循环）
        let stderr_ring = Arc::new(Mutex::new(StderrRing::new()));
        let ring_clone = Arc::clone(&stderr_ring);
        std::thread::spawn(move || {
            let reader = BufReader::new(stderr);
            for line in reader.lines() {
                match line {
                    Ok(l) => {
                        if let Ok(mut ring) = ring_clone.lock() {
                            ring.push_line(&l);
                        }
                    }
                    Err(_) => break, // stderr 关闭或读取失败，退出线程
                }
            }
        });

        let mut client = McpClient {
            stdin,
            reader: BufReader::new(stdout),
            stderr_ring,
            _child: child,
            req_id: 0,
        };

        // Initialize handshake
        client.call(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "agent-manager", "version": "0.1.0" }
            }),
        )?;
        client.notify("notifications/initialized", json!({}))?;
        Ok(client)
    }

    fn call(&mut self, method: &str, params: Value) -> Result<Value, String> {
        self.req_id += 1;
        let req = json!({
            "jsonrpc": "2.0",
            "id": self.req_id,
            "method": method,
            "params": params
        });
        // MCP official SDK uses newline-delimited JSON (NOT Content-Length framing)
        let mut msg = serde_json::to_string(&req).unwrap();
        msg.push('\n');
        self.stdin
            .write_all(msg.as_bytes())
            .map_err(|e| e.to_string())?;
        self.stdin.flush().map_err(|e| e.to_string())?;
        self.read_response()
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        let req = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        let mut msg = serde_json::to_string(&req).unwrap();
        msg.push('\n');
        self.stdin
            .write_all(msg.as_bytes())
            .map_err(|e| e.to_string())?;
        self.stdin.flush().map_err(|e| e.to_string())?;
        Ok(())
    }

    fn read_response(&mut self) -> Result<Value, String> {
        // MCP SDK sends one JSON object per line.
        // Skip notification lines (no "id") and loop until we get a response.
        loop {
            let mut line = String::new();
            self.reader
                .read_line(&mut line)
                .map_err(|e| e.to_string())?;
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            let resp: Value = serde_json::from_str(line).map_err(|e| {
                format!(
                    "JSON parse error: {} — raw: {}",
                    e,
                    &line[..line.len().min(200)]
                )
            })?;
            // Skip notifications (no "id") and requests from server
            if resp.get("id").is_none() {
                continue;
            }
            if let Some(err) = resp.get("error") {
                return Err(err["message"]
                    .as_str()
                    .unwrap_or(&err.to_string())
                    .to_string());
            }
            return Ok(resp["result"].clone());
        }
    }

    pub(crate) fn list_tools(&mut self) -> Result<Vec<Value>, String> {
        let result = self.call("tools/list", json!({}))?;
        Ok(result["tools"].as_array().cloned().unwrap_or_default())
    }

    pub(crate) fn call_tool(&mut self, name: &str, arguments: &Value) -> Result<Value, String> {
        let result = self.call(
            "tools/call",
            json!({
                "name": name,
                "arguments": arguments
            }),
        )?;
        Ok(result)
    }

    /// 取出 stderr ring buffer 的截断摘要，供 FailureTrace 使用。
    /// 只在失败路径调用，成功路径不落盘。
    pub(crate) fn take_stderr_excerpt(&self) -> Option<String> {
        let ring = self.stderr_ring.lock().ok()?;
        ring.excerpt()
    }
}

// ── MCP SSE client ──────────────────────────────────────────────────────────

/// MCP client over HTTP/SSE transport.
///
/// Uses `reqwest::blocking` so the sync `list_tools` / `call_tool` interface
/// matches [`McpClient`]. The `reqwest::blocking::Client` internally owns a
/// tokio runtime that panics if dropped inside an async context, so [`Drop`]
/// moves it to a dedicated OS thread for safe cleanup.
pub(crate) struct McpSseClient {
    base_url: String,
    http: Option<HttpClient>,
    session_id: Option<String>,
}

impl McpSseClient {
    pub(crate) fn new(
        url: &str,
        headers: &std::collections::HashMap<String, String>,
    ) -> Result<Self, String> {
        let mut builder = HttpClient::builder().timeout(std::time::Duration::from_secs(30));
        let mut header_map = reqwest::header::HeaderMap::new();
        for (k, v) in headers {
            if let Ok(name) = reqwest::header::HeaderName::from_bytes(k.as_bytes()) {
                if let Ok(val) = reqwest::header::HeaderValue::from_str(v) {
                    header_map.insert(name, val);
                }
            }
        }
        if !header_map.is_empty() {
            builder = builder.default_headers(header_map);
        }
        let http = builder
            .build()
            .map_err(|e| format!("HTTP client error: {}", e))?;

        Ok(Self {
            base_url: url.trim_end_matches('/').to_string(),
            http: Some(http),
            session_id: None,
        })
    }

    fn send_request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let mut body = json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": method,
        });
        if !params.is_null() {
            body["params"] = params;
        }

        let url = format!("{}/messages", self.base_url);
        let http = self.http.as_ref().ok_or("HTTP client already shut down")?;
        let mut req = http.post(&url).json(&body);
        if let Some(ref sid) = self.session_id {
            req = req.header("X-Session-Id", sid);
        }

        let resp = req
            .send()
            .map_err(|e| format!("SSE request failed: {}", e))?;

        // Capture session id from response headers if present.
        if let Some(sid) = resp.headers().get("X-Session-Id") {
            if let Ok(s) = sid.to_str() {
                self.session_id = Some(s.to_string());
            }
        }

        let status = resp.status();
        let text = resp
            .text()
            .map_err(|e| format!("Read response failed: {}", e))?;

        if !status.is_success() {
            return Err(format!("SSE request returned {}: {}", status, text));
        }

        // Notifications (e.g. notifications/initialized) may receive an empty
        // or non-JSON 202 body — treat that as success.
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }

        let json: Value =
            serde_json::from_str(&text).map_err(|e| format!("Parse SSE response failed: {}", e))?;

        if let Some(err) = json.get("error") {
            return Err(format!("RPC error: {}", err));
        }

        Ok(json.get("result").cloned().unwrap_or(Value::Null))
    }

    pub(crate) fn initialize(&mut self) -> Result<(), String> {
        self.send_request(
            "initialize",
            json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": {
                    "name": "agent-manager",
                    "version": "0.2.3"
                }
            }),
        )?;
        // Send initialized notification (best-effort — server may reply 202).
        let _ = self.send_request("notifications/initialized", Value::Null);
        Ok(())
    }

    pub(crate) fn list_tools(&mut self) -> Result<Vec<Value>, String> {
        let result = self.send_request("tools/list", json!({}))?;
        Ok(result
            .get("tools")
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default())
    }
}

impl Drop for McpSseClient {
    fn drop(&mut self) {
        if let Some(http) = self.http.take() {
            // reqwest::blocking::Client owns a tokio runtime that panics if
            // dropped inside an async context. Move it to a separate OS thread
            // so cleanup is always safe. If the thread cannot be spawned the
            // client is simply leaked (avoids the panic).
            let _ = std::thread::Builder::new()
                .name("mcp-sse-cleanup".to_string())
                .spawn(move || drop(http));
        }
    }
}

// ── Streamable HTTP transport (MCP 2025-03) ─────────────────────────────────

/// MCP Streamable HTTP transport：直接 POST JSON-RPC 到单一端点。
/// 与 SSE 不同，不需要长连接——每次请求独立 POST，响应为 JSON。
/// 适用于云/远程 Agent 接入（如部署在云端的 MCP Server）。
pub(crate) struct McpHttpClient {
    endpoint: String,
    http: Option<HttpClient>,
    headers: std::collections::HashMap<String, String>,
    session_id: Option<String>,
    request_id: std::cell::Cell<u64>,
}

impl McpHttpClient {
    pub(crate) fn new(
        url: &str,
        headers: &std::collections::HashMap<String, String>,
    ) -> Result<Self, String> {
        let mut builder = HttpClient::builder().timeout(std::time::Duration::from_secs(60));
        let mut header_map = reqwest::header::HeaderMap::new();
        for (k, v) in headers {
            if let Ok(name) = reqwest::header::HeaderName::from_bytes(k.as_bytes()) {
                if let Ok(val) = reqwest::header::HeaderValue::from_str(v) {
                    header_map.insert(name, val);
                }
            }
        }
        if !header_map.is_empty() {
            builder = builder.default_headers(header_map.clone());
        }
        let http = builder
            .build()
            .map_err(|e| format!("HTTP client error: {}", e))?;

        let mut hdr = std::collections::HashMap::new();
        for (k, v) in headers {
            hdr.insert(k.clone(), v.clone());
        }

        Ok(Self {
            endpoint: url.trim_end_matches('/').to_string(),
            http: Some(http),
            headers: hdr,
            session_id: None,
            request_id: std::cell::Cell::new(1),
        })
    }

    fn next_id(&self) -> u64 {
        let id = self.request_id.get();
        self.request_id.set(id + 1);
        id
    }

    fn send_request(&mut self, method: &str, params: Value) -> Result<Value, String> {
        let id = self.next_id();
        let mut body = json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
        });
        if !params.is_null() {
            body["params"] = params;
        }

        let http = self.http.as_ref().ok_or("HTTP client already shut down")?;
        let mut req = http.post(&self.endpoint).json(&body);

        // 附带自定义 headers
        for (k, v) in &self.headers {
            if let Ok(name) = reqwest::header::HeaderName::from_bytes(k.as_bytes()) {
                if let Ok(val) = reqwest::header::HeaderValue::from_str(v) {
                    req = req.header(name, val);
                }
            }
        }

        // session 管理（Mcp-Session-Id header）
        if let Some(ref sid) = self.session_id {
            req = req.header("Mcp-Session-Id", sid);
        }

        let resp = req
            .send()
            .map_err(|e| format!("HTTP transport request failed: {}", e))?;

        // 捕获 session id
        if let Some(sid) = resp.headers().get("Mcp-Session-Id") {
            if let Ok(s) = sid.to_str() {
                self.session_id = Some(s.to_string());
            }
        }

        let status = resp.status();
        let text = resp
            .text()
            .map_err(|e| format!("Read response failed: {}", e))?;

        if !status.is_success() {
            return Err(format!("HTTP transport returned {}: {}", status, text));
        }

        // 通知类请求可能返回空 body 或 202
        if text.trim().is_empty() {
            return Ok(Value::Null);
        }

        let json: Value = serde_json::from_str(&text)
            .map_err(|e| format!("Parse HTTP transport response failed: {}", e))?;

        if let Some(err) = json.get("error") {
            return Err(format!("RPC error: {}", err));
        }

        Ok(json.get("result").cloned().unwrap_or(Value::Null))
    }

    pub(crate) fn initialize(&mut self) -> Result<(), String> {
        self.send_request(
            "initialize",
            json!({
                "protocolVersion": "2025-03-26",
                "capabilities": {},
                "clientInfo": {
                    "name": "agent-manager",
                    "version": "0.2.3"
                }
            }),
        )?;
        let _ = self.send_request("notifications/initialized", Value::Null);
        Ok(())
    }

    pub(crate) fn list_tools(&mut self) -> Result<Vec<Value>, String> {
        let result = self.send_request("tools/list", json!({}))?;
        Ok(result
            .get("tools")
            .and_then(|t| t.as_array())
            .cloned()
            .unwrap_or_default())
    }
}

impl Drop for McpHttpClient {
    fn drop(&mut self) {
        if let Some(http) = self.http.take() {
            let _ = std::thread::Builder::new()
                .name("mcp-http-cleanup".to_string())
                .spawn(move || drop(http));
        }
    }
}

// ── Unified MCP transport ───────────────────────────────────────────────────

/// Unified MCP client that abstracts over stdio, SSE, and Streamable HTTP transports.
pub(crate) enum McpTransport {
    Stdio(McpClient),
    Sse(McpSseClient),
    Http(McpHttpClient),
}

impl McpTransport {
    /// Create a transport from an [`McpServer`] config, supporting
    /// `stdio` (default), `sse`, and `http` (Streamable HTTP) transports.
    pub(crate) fn from_server_config(server: &McpServer) -> Result<Self, String> {
        match server.transport.as_str() {
            "sse" => {
                if server.url.is_empty() {
                    return Err(format!("SSE server '{}' missing url", server.name));
                }
                let mut client = McpSseClient::new(&server.url, &server.headers)?;
                client.initialize()?;
                Ok(McpTransport::Sse(client))
            }
            "http" => {
                if server.url.is_empty() {
                    return Err(format!("HTTP server '{}' missing url", server.name));
                }
                let mut client = McpHttpClient::new(&server.url, &server.headers)?;
                client.initialize()?;
                Ok(McpTransport::Http(client))
            }
            _ => {
                let client = McpClient::start(server)?;
                Ok(McpTransport::Stdio(client))
            }
        }
    }

    pub(crate) fn list_tools(&mut self) -> Result<Vec<Value>, String> {
        match self {
            McpTransport::Stdio(c) => c.list_tools(),
            McpTransport::Sse(c) => c.list_tools(),
            McpTransport::Http(c) => c.list_tools(),
        }
    }

    /// 取出 stderr 摘要（仅 Stdio transport 有 stderr，SSE/Http 返回 None）。
    #[allow(dead_code)]
    pub(crate) fn take_stderr_excerpt(&self) -> Option<String> {
        match self {
            McpTransport::Stdio(c) => c.take_stderr_excerpt(),
            McpTransport::Sse(_) => None,
            McpTransport::Http(_) => None,
        }
    }
}

// ── LLM chat helper ─────────────────────────────────────────────────────────

pub(crate) async fn chat(
    provider: &LlmProvider,
    messages: &[Value],
    tools: &[Value],
) -> Result<Value, String> {
    let client = reqwest::Client::new();
    let url = format!(
        "{}/chat/completions",
        provider.base_url.trim_end_matches('/')
    );

    let mut body = json!({
        "model": provider.model,
        "messages": messages,
        "max_tokens": provider.max_output_tokens.unwrap_or(4096),
    });

    if !tools.is_empty() {
        body["tools"] = json!(tools);
        body["tool_choice"] = json!("auto");
    }

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", provider.api_key))
        .json(&body)
        .timeout(std::time::Duration::from_secs(60))
        .send()
        .await
        .map_err(|e| format!("LLM request failed: {}", e))?;

    let status = resp.status();
    let text = resp.text().await.unwrap_or_default();
    if !status.is_success() {
        let msg = serde_json::from_str::<Value>(&text)
            .ok()
            .and_then(|v| v["error"]["message"].as_str().map(|s| s.to_string()))
            .unwrap_or(text);
        return Err(format!("LLM error {}: {}", status, msg));
    }

    serde_json::from_str(&text).map_err(|e| e.to_string())
}
