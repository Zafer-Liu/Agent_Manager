use crate::llm::LlmProvider;
use crate::mcp::McpServer;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};

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
        return ("cmd.exe".to_string(), vec!["/c".to_string(), command.to_string()]);
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
            .stderr(Stdio::null());
        for (k, v) in &server.env {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().map_err(|e| format!("Failed to start MCP '{}': {}", server.name, e))?;
        let stdin = child.stdin.take().unwrap();
        let stdout = child.stdout.take().unwrap();
        let mut client = McpClient { stdin, reader: BufReader::new(stdout), _child: child, req_id: 0 };

        // Initialize handshake
        client.call("initialize", json!({
            "protocolVersion": "2024-11-05",
            "capabilities": {},
            "clientInfo": { "name": "agent-manager", "version": "0.1.0" }
        }))?;
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
        self.stdin.write_all(msg.as_bytes()).map_err(|e| e.to_string())?;
        self.stdin.flush().map_err(|e| e.to_string())?;
        self.read_response()
    }

    fn notify(&mut self, method: &str, params: Value) -> Result<(), String> {
        let req = json!({ "jsonrpc": "2.0", "method": method, "params": params });
        let mut msg = serde_json::to_string(&req).unwrap();
        msg.push('\n');
        self.stdin.write_all(msg.as_bytes()).map_err(|e| e.to_string())?;
        self.stdin.flush().map_err(|e| e.to_string())?;
        Ok(())
    }

    fn read_response(&mut self) -> Result<Value, String> {
        // MCP SDK sends one JSON object per line.
        // Skip notification lines (no "id") and loop until we get a response.
        loop {
            let mut line = String::new();
            self.reader.read_line(&mut line).map_err(|e| e.to_string())?;
            let line = line.trim();
            if line.is_empty() { continue; }
            let resp: Value = serde_json::from_str(line)
                .map_err(|e| format!("JSON parse error: {} — raw: {}", e, &line[..line.len().min(200)]))?;
            // Skip notifications (no "id") and requests from server
            if resp.get("id").is_none() { continue; }
            if let Some(err) = resp.get("error") {
                return Err(err["message"].as_str().unwrap_or(&err.to_string()).to_string());
            }
            return Ok(resp["result"].clone());
        }
    }

    pub(crate) fn list_tools(&mut self) -> Result<Vec<Value>, String> {
        let result = self.call("tools/list", json!({}))?;
        Ok(result["tools"].as_array().cloned().unwrap_or_default())
    }

    pub(crate) fn call_tool(&mut self, name: &str, arguments: &Value) -> Result<Value, String> {
        let result = self.call("tools/call", json!({
            "name": name,
            "arguments": arguments
        }))?;
        Ok(result)
    }
}

// ── LLM chat helper ─────────────────────────────────────────────────────────

pub(crate) async fn chat(
    provider: &LlmProvider,
    messages: &[Value],
    tools: &[Value],
) -> Result<Value, String> {
    let client = reqwest::Client::new();
    let url = format!("{}/chat/completions", provider.base_url.trim_end_matches('/'));

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
            steps: vec![AgentStep { kind: StepKind::Error, content: e.clone(), tool: None, tool_input: None }],
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
    let mut clients: Vec<(String, McpClient)> = vec![];
    let mut all_tools: Vec<Value> = vec![];

    for server in &request.mcp_servers {
        match McpClient::start(server) {
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
        all_tools.iter()
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
        let message = &choice["message"];
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
                                let content = result["content"]
                                    .as_array()
                                    .and_then(|arr| arr.first())
                                    .and_then(|c| c["text"].as_str())
                                    .unwrap_or("")
                                    .to_string();
                                content
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
    pub role: String,   // "user" | "assistant"
    pub content: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChatRequest {
    pub history: Vec<ChatMessage>,  // full conversation so far
    pub provider: LlmProvider,
    pub mcp_servers: Vec<McpServer>,
    pub max_iterations: Option<u32>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ChatTurn {
    pub steps: Vec<AgentStep>,   // tool calls + results inline
    pub reply: String,           // final assistant text
    pub success: bool,
    pub error: Option<String>,
}

#[tauri::command]
pub async fn chat_with_mcp(request: ChatRequest) -> ChatTurn {
    match chat_turn_inner(request).await {
        Ok(t) => t,
        Err(e) => ChatTurn {
            steps: vec![AgentStep { kind: StepKind::Error, content: e.clone(), tool: None, tool_input: None }],
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
    let mut clients: Vec<(String, McpClient)> = vec![];
    let mut all_tools: Vec<Value> = vec![];

    for server in &request.mcp_servers {
        if let Ok(mut client) = McpClient::start(server) {
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
    let tool_names: Vec<&str> = all_tools.iter()
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
        messages.push(json!({"role": msg.role, "content": msg.content}));
    }

    let mut reply = String::new();

    // Agentic loop for this turn
    for _ in 0..max_iter {
        let response = chat(&request.provider, &messages, &all_tools).await?;
        let choice = &response["choices"][0];
        let message = &choice["message"];
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
                                result["content"]
                                    .as_array()
                                    .and_then(|arr| arr.first())
                                    .and_then(|c| c["text"].as_str())
                                    .unwrap_or("")
                                    .to_string()
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

    Ok(ChatTurn { steps, reply, success: true, error: None })
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
            steps: vec![AgentStep { kind: StepKind::Error, content: e.clone(), tool: None, tool_input: None }],
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
    let agent_summary: Vec<String> = request.agents.iter().map(|a| {
        let port_str = a.port.map(|p| format!(", port: {}", p)).unwrap_or_default();
        let desc_str = if a.description.is_empty() { String::new() } else { format!(", description: {}", a.description) };
        format!("  - name: \"{}\", id: \"{}\", status: \"{}\"{}{}",
            a.name, a.id, a.status, port_str, desc_str)
    }).collect();

    let system = format!(
        "You are the Manager Agent, an intelligent coordinator for a local AI agent management platform.\n\n\
         Connected agents:\n{}\n\n\
         Use your tools to fulfill user requests. Always call get_agent_status before starting/stopping to confirm current state.\
         Respond in the same language as the user (Chinese if they write in Chinese). Be concise.",
        if agent_summary.is_empty() { "  (none)".to_string() } else { agent_summary.join("\n") }
    );

    let mut messages: Vec<Value> = vec![json!({"role": "system", "content": system})];
    for msg in &request.history {
        messages.push(json!({"role": msg.role, "content": msg.content}));
    }

    let mut reply = String::new();

    for _ in 0..max_iter {
        let response = chat(&request.provider, &messages, tools.as_array().unwrap()).await?;
        let choice = &response["choices"][0];
        let message = &choice["message"];
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
                        let filtered: Vec<&AgentInfo> = request.agents.iter().filter(|a| {
                            filter.is_empty() || a.name.to_lowercase().contains(&filter)
                        }).collect();
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
                    },
                    "get_agent_status" => {
                        let filter = agent_name_arg.clone();
                        let filtered: Vec<&AgentInfo> = request.agents.iter().filter(|a| {
                            filter.is_empty() || a.name.to_lowercase().contains(&filter)
                        }).collect();
                        if filtered.is_empty() {
                            "No agents found.".to_string()
                        } else {
                            let rows: Vec<String> = filtered.iter().map(|a| {
                                let port_info = match a.port {
                                    Some(p) => format!("{} ({})", p, if a.port_open { "open" } else { "closed" }),
                                    None => "none".to_string(),
                                };
                                format!("name={}, id={}, status={}, port={}, desc={}",
                                    a.name, a.id, a.status, port_info, a.description)
                            }).collect();
                            format!("Agents:\n{}", rows.join("\n"))
                        }
                    },
                    "start_agent" => {
                        match matched_agent {
                            Some(a) => format!("__action__:start_agent:{}:{}", a.id, a.name),
                            None => format!("Agent '{}' not found. Available: {}", agent_name_arg,
                                request.agents.iter().map(|a| a.name.as_str()).collect::<Vec<_>>().join(", ")),
                        }
                    },
                    "stop_agent" => {
                        match matched_agent {
                            Some(a) => format!("__action__:stop_agent:{}:{}", a.id, a.name),
                            None => format!("Agent '{}' not found.", agent_name_arg),
                        }
                    },
                    "open_agent_ui" => {
                        match matched_agent {
                            Some(a) => format!("__action__:open_ui:{}:{}", a.id, a.name),
                            None => format!("Agent '{}' not found.", agent_name_arg),
                        }
                    },
                    "open_agent_terminal" => {
                        match matched_agent {
                            Some(a) => format!("__action__:open_terminal:{}:{}", a.id, a.name),
                            None => format!("Agent '{}' not found.", agent_name_arg),
                        }
                    },
                    "navigate_to" => {
                        let page = fn_args["page"].as_str().unwrap_or("agents");
                        format!("__action__:navigate::{}", page)
                    },
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

    Ok(ChatTurn { steps, reply, success: true, error: None })
}
