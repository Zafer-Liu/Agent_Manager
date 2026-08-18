use crate::llm::LlmProvider;
use crate::mcp::McpServer;
use crate::thinking::strip_thinking_blocks;
use reqwest::blocking::Client as HttpClient;
use serde::{Deserialize, Serialize};
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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum StepKind {
    Thought,
    ToolCall,
    ToolResult,
    Answer,
    Error,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentStep {
    pub kind: StepKind,
    pub content: String,
    pub tool: Option<String>,
    pub tool_input: Option<Value>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RunAgentRequest {
    pub task: String,
    pub provider: LlmProvider,
    pub mcp_servers: Vec<McpServer>,
    pub max_iterations: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RunAgentResult {
    pub steps: Vec<AgentStep>,
    pub final_answer: String,
    pub success: bool,
    pub error: Option<String>,
}

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

    pub(crate) fn call_tool(&mut self, name: &str, arguments: &Value) -> Result<Value, String> {
        self.send_request(
            "tools/call",
            json!({
                "name": name,
                "arguments": arguments
            }),
        )
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

    pub(crate) fn call_tool(&mut self, name: &str, arguments: &Value) -> Result<Value, String> {
        self.send_request(
            "tools/call",
            json!({
                "name": name,
                "arguments": arguments
            }),
        )
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

    pub(crate) fn call_tool(&mut self, name: &str, arguments: &Value) -> Result<Value, String> {
        match self {
            McpTransport::Stdio(c) => c.call_tool(name, arguments),
            McpTransport::Sse(c) => c.call_tool(name, arguments),
            McpTransport::Http(c) => c.call_tool(name, arguments),
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

// ── Main command ─────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn run_mcp_agent(request: RunAgentRequest) -> RunAgentResult {
    match run_agent_inner(request).await {
        Ok(result) => result,
        Err(e) => RunAgentResult {
            steps: vec![AgentStep {
                kind: StepKind::Error,
                content: e.clone(),
                tool: None,
                tool_input: None,
            }],
            final_answer: String::new(),
            success: false,
            error: Some(e),
        },
    }
}

async fn run_agent_inner(request: RunAgentRequest) -> Result<RunAgentResult, String> {
    let max_iter = request.max_iterations.unwrap_or(10);
    let mut steps: Vec<AgentStep> = vec![];

    // ── 1. Start MCP servers and collect tools ──────────────────────────────
    let mut clients: Vec<(String, McpTransport)> = vec![];
    let mut all_tools: Vec<Value> = vec![];

    for server in &request.mcp_servers {
        match McpTransport::from_server_config(server) {
            Ok(mut client) => {
                match client.list_tools() {
                    Ok(tools) => {
                        for tool in &tools {
                            // OpenAI function calling format
                            all_tools.push(json!({
                                "type": "function",
                                "function": {
                                    "name": format!("{}__{}", server.name, tool["name"].as_str().unwrap_or("")),
                                    "description": tool["description"].as_str().unwrap_or(""),
                                    "parameters": tool.get("inputSchema").cloned().unwrap_or(json!({"type":"object","properties":{}}))
                                }
                            }));
                        }
                        clients.push((server.name.clone(), client));
                    }
                    Err(e) => {
                        steps.push(AgentStep {
                            kind: StepKind::Error,
                            content: format!("Failed to list tools for '{}': {}", server.name, e),
                            tool: None,
                            tool_input: None,
                        });
                    }
                }
            }
            Err(e) => {
                steps.push(AgentStep {
                    kind: StepKind::Error,
                    content: e,
                    tool: None,
                    tool_input: None,
                });
            }
        }
    }

    // ── 2. Build initial messages ───────────────────────────────────────────
    let tool_list_desc = if all_tools.is_empty() {
        "No tools available.".to_string()
    } else {
        all_tools
            .iter()
            .filter_map(|t| t["function"]["name"].as_str())
            .collect::<Vec<_>>()
            .join(", ")
    };

    let system_prompt = format!(
        "You are a helpful AI agent. Complete the user's task using the available tools.\n\
         Available tools: {}\n\
         When you have fully completed the task, provide a final answer summarizing what was done.",
        tool_list_desc
    );

    let mut messages: Vec<Value> = vec![
        json!({"role": "system", "content": system_prompt}),
        json!({"role": "user", "content": request.task}),
    ];

    // ── 3. Agentic loop ─────────────────────────────────────────────────────
    let mut final_answer = String::new();

    for _iter in 0..max_iter {
        let response = chat(&request.provider, &messages, &all_tools).await?;
        let choice = &response["choices"][0];
        let mut message = choice["message"].clone();
        if let Some(content) = message["content"].as_str() {
            message["content"] = Value::String(strip_thinking_blocks(content));
        }
        let finish_reason = choice["finish_reason"].as_str().unwrap_or("");

        // Add assistant message to history
        messages.push(message.clone());

        // Check for tool calls
        if let Some(tool_calls) = message["tool_calls"].as_array() {
            if tool_calls.is_empty() {
                // No tool calls — treat as final answer
                final_answer = message["content"].as_str().unwrap_or("").to_string();
                steps.push(AgentStep {
                    kind: StepKind::Answer,
                    content: final_answer.clone(),
                    tool: None,
                    tool_input: None,
                });
                break;
            }

            for tc in tool_calls {
                let tc_id = tc["id"].as_str().unwrap_or("").to_string();
                let fn_name = tc["function"]["name"].as_str().unwrap_or("").to_string();
                let fn_args: Value = tc["function"]["arguments"]
                    .as_str()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or(json!({}));

                steps.push(AgentStep {
                    kind: StepKind::ToolCall,
                    content: format!("Calling {}", fn_name),
                    tool: Some(fn_name.clone()),
                    tool_input: Some(fn_args.clone()),
                });

                // Route to correct MCP server (name__tool_name format)
                let tool_result = if let Some((server_name, tool_name)) = fn_name.split_once("__") {
                    if let Some((_, client)) = clients.iter_mut().find(|(n, _)| n == server_name) {
                        match client.call_tool(tool_name, &fn_args) {
                            Ok(result) => {
                                // 完整提取所有 content 项的 text，而非仅取第一个
                                result["content"]
                                    .as_array()
                                    .map(|arr| {
                                        arr.iter()
                                            .filter_map(|c| {
                                                c["text"].as_str().map(|s| s.to_string())
                                            })
                                            .collect::<Vec<_>>()
                                            .join("\n")
                                    })
                                    .filter(|s| !s.is_empty())
                                    .unwrap_or_else(|| {
                                        serde_json::to_string(&result).unwrap_or_default()
                                    })
                            }
                            Err(e) => format!("Tool error: {}", e),
                        }
                    } else {
                        format!("MCP server '{}' not found", server_name)
                    }
                } else {
                    format!("Unknown tool format: {}", fn_name)
                };

                steps.push(AgentStep {
                    kind: StepKind::ToolResult,
                    content: tool_result.clone(),
                    tool: Some(fn_name.clone()),
                    tool_input: None,
                });

                // Add tool result to message history
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": tc_id,
                    "content": tool_result
                }));
            }
        } else {
            // No tool calls — final answer
            final_answer = message["content"].as_str().unwrap_or("").to_string();
            steps.push(AgentStep {
                kind: StepKind::Answer,
                content: final_answer.clone(),
                tool: None,
                tool_input: None,
            });
            break;
        }

        if finish_reason == "stop" {
            final_answer = message["content"].as_str().unwrap_or("").to_string();
            if !final_answer.is_empty() {
                steps.push(AgentStep {
                    kind: StepKind::Answer,
                    content: final_answer.clone(),
                    tool: None,
                    tool_input: None,
                });
            }
            break;
        }
    }

    if final_answer.is_empty() && steps.iter().any(|s| matches!(s.kind, StepKind::ToolResult)) {
        final_answer = "Task completed. See tool results above.".to_string();
    }

    Ok(RunAgentResult {
        steps,
        final_answer,
        success: true,
        error: None,
    })
}

// ── Multi-turn chat command ──────────────────────────────────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct ChatMessage {
    pub role: String, // "user" | "assistant"
    pub content: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChatRequest {
    pub history: Vec<ChatMessage>, // full conversation so far
    pub provider: LlmProvider,
    pub mcp_servers: Vec<McpServer>,
    pub max_iterations: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChatTurn {
    pub steps: Vec<AgentStep>, // tool calls + results inline
    pub reply: String,         // final assistant text
    pub success: bool,
    pub error: Option<String>,
}

#[tauri::command]
pub async fn chat_with_mcp(request: ChatRequest) -> ChatTurn {
    match chat_turn_inner(request).await {
        Ok(t) => t,
        Err(e) => ChatTurn {
            steps: vec![AgentStep {
                kind: StepKind::Error,
                content: e.clone(),
                tool: None,
                tool_input: None,
            }],
            reply: String::new(),
            success: false,
            error: Some(e),
        },
    }
}

async fn chat_turn_inner(request: ChatRequest) -> Result<ChatTurn, String> {
    let max_iter = request.max_iterations.unwrap_or(8);
    let mut steps: Vec<AgentStep> = vec![];

    // Start MCP servers and collect tools
    let mut clients: Vec<(String, McpTransport)> = vec![];
    let mut all_tools: Vec<Value> = vec![];

    for server in &request.mcp_servers {
        if let Ok(mut client) = McpTransport::from_server_config(server) {
            if let Ok(tools) = client.list_tools() {
                for tool in &tools {
                    all_tools.push(json!({
                        "type": "function",
                        "function": {
                            "name": format!("{}__{}", server.name, tool["name"].as_str().unwrap_or("")),
                            "description": tool.get("description").and_then(|v| v.as_str()).unwrap_or(""),
                            "parameters": tool.get("inputSchema").cloned().unwrap_or(json!({"type":"object","properties":{}}))
                        }
                    }));
                }
                clients.push((server.name.clone(), client));
            }
        }
    }

    // Build messages: system + full history
    let tool_names: Vec<&str> = all_tools
        .iter()
        .filter_map(|t| t["function"]["name"].as_str())
        .collect();

    let system = if tool_names.is_empty() {
        "You are a helpful assistant.".to_string()
    } else {
        format!(
            "You are a helpful assistant with access to tools: {}.\n\
             Use tools when needed to answer the user's request accurately.",
            tool_names.join(", ")
        )
    };

    let mut messages: Vec<Value> = vec![json!({"role": "system", "content": system})];
    for msg in &request.history {
        let content = if msg.role == "assistant" {
            strip_thinking_blocks(&msg.content)
        } else {
            msg.content.clone()
        };
        messages.push(json!({"role": msg.role, "content": content}));
    }

    let mut reply = String::new();

    // Agentic loop for this turn
    for _ in 0..max_iter {
        let response = chat(&request.provider, &messages, &all_tools).await?;
        let choice = &response["choices"][0];
        let mut message = choice["message"].clone();
        if let Some(content) = message["content"].as_str() {
            message["content"] = Value::String(strip_thinking_blocks(content));
        }
        let finish_reason = choice["finish_reason"].as_str().unwrap_or("");

        messages.push(message.clone());

        if let Some(tool_calls) = message["tool_calls"].as_array().filter(|tc| !tc.is_empty()) {
            for tc in tool_calls {
                let tc_id = tc["id"].as_str().unwrap_or("").to_string();
                let fn_name = tc["function"]["name"].as_str().unwrap_or("").to_string();
                let fn_args: Value = tc["function"]["arguments"]
                    .as_str()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or(json!({}));

                steps.push(AgentStep {
                    kind: StepKind::ToolCall,
                    content: fn_name.clone(),
                    tool: Some(fn_name.clone()),
                    tool_input: Some(fn_args.clone()),
                });

                let tool_result = if let Some((server_name, tool_name)) = fn_name.split_once("__") {
                    if let Some((_, client)) = clients.iter_mut().find(|(n, _)| n == server_name) {
                        match client.call_tool(tool_name, &fn_args) {
                            Ok(result) => {
                                // 完整提取所有 content 项的 text，而非仅取第一个
                                result["content"]
                                    .as_array()
                                    .map(|arr| {
                                        arr.iter()
                                            .filter_map(|c| {
                                                c["text"].as_str().map(|s| s.to_string())
                                            })
                                            .collect::<Vec<_>>()
                                            .join("\n")
                                    })
                                    .filter(|s| !s.is_empty())
                                    .unwrap_or_else(|| {
                                        serde_json::to_string(&result).unwrap_or_default()
                                    })
                            }
                            Err(e) => format!("Tool error: {}", e),
                        }
                    } else {
                        format!("Server '{}' not connected", server_name)
                    }
                } else {
                    format!("Unknown tool: {}", fn_name)
                };

                steps.push(AgentStep {
                    kind: StepKind::ToolResult,
                    content: tool_result.clone(),
                    tool: Some(fn_name.clone()),
                    tool_input: None,
                });

                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": tc_id,
                    "content": tool_result
                }));
            }
        } else {
            reply = message["content"].as_str().unwrap_or("").to_string();
            break;
        }

        if finish_reason == "stop" {
            reply = message["content"].as_str().unwrap_or("").to_string();
            break;
        }
    }

    Ok(ChatTurn {
        steps,
        reply,
        success: true,
        error: None,
    })
}

// ── Manager Agent chat (with virtual agent-status tool) ──────────────────────

#[derive(Debug, Serialize, Deserialize)]
pub struct AgentInfo {
    pub id: String,
    pub name: String,
    pub status: String,
    pub port: Option<u16>,
    pub description: String,
    pub port_open: bool,
    pub working_dir: String,
    pub command: String,
    pub args: Vec<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ManagerChatRequest {
    pub history: Vec<ChatMessage>,
    pub provider: LlmProvider,
    pub agents: Vec<AgentInfo>,
    pub max_iterations: Option<u32>,
}

#[tauri::command]
pub async fn manager_chat(request: ManagerChatRequest) -> ChatTurn {
    match manager_chat_inner(request).await {
        Ok(t) => t,
        Err(e) => ChatTurn {
            steps: vec![AgentStep {
                kind: StepKind::Error,
                content: e.clone(),
                tool: None,
                tool_input: None,
            }],
            reply: String::new(),
            success: false,
            error: Some(e),
        },
    }
}

async fn manager_chat_inner(request: ManagerChatRequest) -> Result<ChatTurn, String> {
    let max_iter = request.max_iterations.unwrap_or(10);
    let mut steps: Vec<AgentStep> = vec![];

    // All capabilities exposed as real LLM tools
    let tools = json!([
        {
            "type": "function",
            "function": {
                "name": "get_agent_status",
                "description": "Get real-time status of connected agents (running/stopped/error, port open state). Always call this first before acting.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "agent_name": {
                            "type": "string",
                            "description": "Filter by agent name (partial match). Omit to get all agents."
                        }
                    }
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "start_agent",
                "description": "Start a stopped agent by name.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "agent_name": { "type": "string", "description": "Name of the agent to start." }
                    },
                    "required": ["agent_name"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "stop_agent",
                "description": "Stop a running agent by name.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "agent_name": { "type": "string", "description": "Name of the agent to stop." }
                    },
                    "required": ["agent_name"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "open_agent_ui",
                "description": "Open the web UI of an agent (only useful if the agent is running and has a port).",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "agent_name": { "type": "string", "description": "Name of the agent." }
                    },
                    "required": ["agent_name"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "open_agent_terminal",
                "description": "Open a terminal for an agent.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "agent_name": { "type": "string", "description": "Name of the agent." }
                    },
                    "required": ["agent_name"]
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "get_agent_details",
                "description": "Get detailed information about one or all agents: start command, working directory, README content from the project folder. Use this to explain what an agent does.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "agent_name": {
                            "type": "string",
                            "description": "Agent name (partial match). Omit to get details for all agents."
                        }
                    }
                }
            }
        },
        {
            "type": "function",
            "function": {
                "name": "navigate_to",
                "description": "Navigate the app to a specific page.",
                "parameters": {
                    "type": "object",
                    "properties": {
                        "page": {
                            "type": "string",
                            "enum": ["dashboard", "agents", "mcp-agent", "ports"],
                            "description": "The page to navigate to."
                        }
                    },
                    "required": ["page"]
                }
            }
        }
    ]);

    // Build agent snapshot for system prompt
    let agent_summary: Vec<String> = request
        .agents
        .iter()
        .map(|a| {
            let port_str = a.port.map(|p| format!(", port: {}", p)).unwrap_or_default();
            let desc_str = if a.description.is_empty() {
                String::new()
            } else {
                format!(", description: {}", a.description)
            };
            format!(
                "  - name: \"{}\", id: \"{}\", status: \"{}\"{}{}",
                a.name, a.id, a.status, port_str, desc_str
            )
        })
        .collect();

    let system = format!(
        "You are the Manager Agent, an intelligent coordinator for a local AI agent management platform.\n\n\
         Connected agents:\n{}\n\n\
         Use your tools to fulfill user requests. Always call get_agent_status before starting/stopping to confirm current state.\
         Respond in the same language as the user (Chinese if they write in Chinese). Be concise.",
        if agent_summary.is_empty() { "  (none)".to_string() } else { agent_summary.join("\n") }
    );

    let mut messages: Vec<Value> = vec![json!({"role": "system", "content": system})];
    for msg in &request.history {
        let content = if msg.role == "assistant" {
            strip_thinking_blocks(&msg.content)
        } else {
            msg.content.clone()
        };
        messages.push(json!({"role": msg.role, "content": content}));
    }

    let mut reply = String::new();

    for _ in 0..max_iter {
        let response = chat(&request.provider, &messages, tools.as_array().unwrap()).await?;
        let choice = &response["choices"][0];
        let mut message = choice["message"].clone();
        if let Some(content) = message["content"].as_str() {
            message["content"] = Value::String(strip_thinking_blocks(content));
        }
        let finish_reason = choice["finish_reason"].as_str().unwrap_or("");

        messages.push(message.clone());

        if let Some(tool_calls) = message["tool_calls"].as_array().filter(|tc| !tc.is_empty()) {
            for tc in tool_calls {
                let tc_id = tc["id"].as_str().unwrap_or("").to_string();
                let fn_name = tc["function"]["name"].as_str().unwrap_or("").to_string();
                let fn_args: Value = tc["function"]["arguments"]
                    .as_str()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or(json!({}));

                steps.push(AgentStep {
                    kind: StepKind::ToolCall,
                    content: fn_name.clone(),
                    tool: Some(fn_name.clone()),
                    tool_input: Some(fn_args.clone()),
                });

                // Resolve agent name for tools that need it
                let agent_name_arg = fn_args["agent_name"].as_str().unwrap_or("").to_lowercase();
                let matched_agent = request.agents.iter().find(|a| {
                    let n = a.name.to_lowercase();
                    n == agent_name_arg
                        || n.contains(&agent_name_arg)
                        || agent_name_arg.contains(&n)
                        || a.id == fn_args["agent_name"].as_str().unwrap_or("")
                });

                let tool_result = match fn_name.as_str() {
                    "get_agent_details" => {
                        let filter = agent_name_arg.clone();
                        let filtered: Vec<&AgentInfo> = request
                            .agents
                            .iter()
                            .filter(|a| {
                                filter.is_empty() || a.name.to_lowercase().contains(&filter)
                            })
                            .collect();
                        if filtered.is_empty() {
                            format!("No agents found matching '{}'.", filter)
                        } else {
                            let sections: Vec<String> = filtered.iter().map(|a| {
                                let cmd = if a.args.is_empty() {
                                    a.command.clone()
                                } else {
                                    format!("{} {}", a.command, a.args.join(" "))
                                };
                                let port_str = a.port
                                    .map(|p| format!("{} ({})", p, if a.port_open { "open" } else { "closed" }))
                                    .unwrap_or_else(|| "none".to_string());

                                // Try to read README from working dir (multiple filenames)
                                let readme = ["README.md", "readme.md", "README.txt", "README"]
                                    .iter()
                                    .find_map(|fname| {
                                        let p = std::path::Path::new(&a.working_dir).join(fname);
                                        std::fs::read_to_string(&p).ok()
                                    });

                                let readme_section = match readme {
                                    Some(content) => {
                                        // Trim to first 800 chars to keep context reasonable
                                        let trimmed = content.chars().take(800).collect::<String>();
                                        let suffix = if content.len() > 800 { "...(truncated)" } else { "" };
                                        format!("README:\n{}{}", trimmed, suffix)
                                    },
                                    None => "README: (not found)".to_string(),
                                };

                                format!(
                                    "=== {} ===\nstatus: {}\ncommand: {}\nworking_dir: {}\nport: {}\ndescription: {}\n{}",
                                    a.name, a.status, cmd, a.working_dir, port_str, a.description, readme_section
                                )
                            }).collect();
                            sections.join("\n\n")
                        }
                    }
                    "get_agent_status" => {
                        let filter = agent_name_arg.clone();
                        let filtered: Vec<&AgentInfo> = request
                            .agents
                            .iter()
                            .filter(|a| {
                                filter.is_empty() || a.name.to_lowercase().contains(&filter)
                            })
                            .collect();
                        if filtered.is_empty() {
                            "No agents found.".to_string()
                        } else {
                            let rows: Vec<String> = filtered
                                .iter()
                                .map(|a| {
                                    let port_info = match a.port {
                                        Some(p) => format!(
                                            "{} ({})",
                                            p,
                                            if a.port_open { "open" } else { "closed" }
                                        ),
                                        None => "none".to_string(),
                                    };
                                    format!(
                                        "name={}, id={}, status={}, port={}, desc={}",
                                        a.name, a.id, a.status, port_info, a.description
                                    )
                                })
                                .collect();
                            format!("Agents:\n{}", rows.join("\n"))
                        }
                    }
                    "start_agent" => match matched_agent {
                        Some(a) => format!("__action__:start_agent:{}:{}", a.id, a.name),
                        None => format!(
                            "Agent '{}' not found. Available: {}",
                            agent_name_arg,
                            request
                                .agents
                                .iter()
                                .map(|a| a.name.as_str())
                                .collect::<Vec<_>>()
                                .join(", ")
                        ),
                    },
                    "stop_agent" => match matched_agent {
                        Some(a) => format!("__action__:stop_agent:{}:{}", a.id, a.name),
                        None => format!("Agent '{}' not found.", agent_name_arg),
                    },
                    "open_agent_ui" => match matched_agent {
                        Some(a) => format!("__action__:open_ui:{}:{}", a.id, a.name),
                        None => format!("Agent '{}' not found.", agent_name_arg),
                    },
                    "open_agent_terminal" => match matched_agent {
                        Some(a) => format!("__action__:open_terminal:{}:{}", a.id, a.name),
                        None => format!("Agent '{}' not found.", agent_name_arg),
                    },
                    "navigate_to" => {
                        let page = fn_args["page"].as_str().unwrap_or("agents");
                        format!("__action__:navigate::{}", page)
                    }
                    _ => format!("Unknown tool: {}", fn_name),
                };

                steps.push(AgentStep {
                    kind: StepKind::ToolResult,
                    content: tool_result.clone(),
                    tool: Some(fn_name.clone()),
                    tool_input: None,
                });

                // For action tools, tell the LLM the action was dispatched
                let llm_result = if tool_result.starts_with("__action__") {
                    "Action dispatched successfully.".to_string()
                } else {
                    tool_result
                };

                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": tc_id,
                    "content": llm_result
                }));
            }
        } else {
            reply = message["content"].as_str().unwrap_or("").to_string();
            break;
        }

        if finish_reason == "stop" {
            reply = message["content"].as_str().unwrap_or("").to_string();
            break;
        }
    }

    Ok(ChatTurn {
        steps,
        reply,
        success: true,
        error: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stderr_ring_caps_lines() {
        let mut ring = StderrRing::new();
        for i in 0..100 {
            ring.push_line(&format!("line {}", i));
        }
        let excerpt = ring.excerpt().unwrap();
        let lines: Vec<&str> = excerpt.split('\n').collect();
        assert_eq!(lines.len(), 80, "should cap at 80 lines");
        assert_eq!(lines[0], "line 20");
        assert_eq!(lines[79], "line 99");
    }

    #[test]
    fn stderr_ring_caps_bytes() {
        let mut ring = StderrRing::new();
        let big_line = "x".repeat(300);
        for _ in 0..10 {
            ring.push_line(&big_line);
        }
        let excerpt = ring.excerpt().unwrap();
        let lines: Vec<&str> = excerpt.split('\n').collect();
        assert!(lines.len() <= 7, "should respect 2KB byte limit");
    }

    #[test]
    fn stderr_ring_empty_returns_none() {
        let ring = StderrRing::new();
        assert_eq!(ring.excerpt(), None);
    }

    #[test]
    fn stderr_ring_skips_empty_lines() {
        let mut ring = StderrRing::new();
        ring.push_line("");
        ring.push_line("real error");
        ring.push_line("");
        let excerpt = ring.excerpt().unwrap();
        assert_eq!(excerpt, "real error");
    }
}
