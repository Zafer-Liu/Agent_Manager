use crate::llm::LlmProvider;
use crate::mcp::McpServer;
use crate::mcp_agent::{chat, McpClient, McpTransport};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::sync::{Arc, Mutex};
use tauri::Emitter;

// ── Data model ───────────────────────────────────────────────────────────────

/// A single step in a workflow pipeline.
///
/// `kind == "tool"` → call an MCP tool (`server` + `tool` + `arguments`).
/// `kind == "llm"`  → run an LLM completion using `prompt`.
///
/// In both cases the previous node's text output is available to the node:
/// for tool nodes, the placeholder `{{input}}` inside any string argument is
/// replaced with the previous output; for llm nodes, the previous output is
/// appended to the prompt as the user message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowNode {
    pub id: String,
    pub kind: String, // "tool" | "llm"
    #[serde(default)]
    pub label: String,
    // tool node fields
    #[serde(default)]
    pub server: String,
    #[serde(default)]
    pub tool: String,
    #[serde(default)]
    pub arguments: Value, // object template; strings may contain {{input}}
    // llm node fields
    #[serde(default)]
    pub prompt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub nodes: Vec<WorkflowNode>, // ordered = execution order
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

// ── Config file helpers ──────────────────────────────────────────────────────

fn workflows_path() -> std::path::PathBuf {
    dirs_next::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("agent-manager")
        .join("agent_manager_workflows.json")
}

fn read_workflows() -> Vec<Workflow> {
    std::fs::read_to_string(workflows_path())
        .ok()
        .and_then(|s| serde_json::from_str::<Vec<Workflow>>(&s).ok())
        .unwrap_or_default()
}

fn write_workflows(list: &[Workflow]) -> Result<(), String> {
    let path = workflows_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(list).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())
}

// ── CRUD commands ────────────────────────────────────────────────────────────

#[tauri::command]
pub fn list_workflows() -> Vec<Workflow> {
    read_workflows()
}

#[tauri::command]
pub fn save_workflow(workflow: Workflow) -> Result<(), String> {
    let mut list = read_workflows();
    let now = chrono::Utc::now().to_rfc3339();
    let mut wf = workflow;
    if wf.id.trim().is_empty() {
        return Err("Workflow id is required".to_string());
    }
    if wf.name.trim().is_empty() {
        return Err("Workflow name is required".to_string());
    }
    if wf.created_at.is_empty() {
        wf.created_at = now.clone();
    }
    wf.updated_at = now;

    if let Some(existing) = list.iter_mut().find(|w| w.id == wf.id) {
        // preserve original created_at on update
        wf.created_at = existing.created_at.clone();
        *existing = wf;
    } else {
        list.push(wf);
    }
    write_workflows(&list)
}

#[tauri::command]
pub fn delete_workflow(id: String) -> Result<(), String> {
    let mut list = read_workflows();
    list.retain(|w| w.id != id);
    write_workflows(&list)
}

// ── Tool discovery ───────────────────────────────────────────────────────────

#[derive(Debug, Serialize)]
pub struct McpToolInfo {
    pub server: String,
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// Start the given MCP servers and return the union of their tools.
/// Used by the workflow builder to render the palette of draggable tools.
#[tauri::command]
pub async fn list_mcp_tools(servers: Vec<McpServer>) -> Result<Vec<McpToolInfo>, String> {
    // McpClient uses blocking stdio; run on a blocking thread.
    tokio::task::spawn_blocking(move || {
        let mut out: Vec<McpToolInfo> = vec![];
        for server in &servers {
            // Support both stdio and SSE transports via the unified transport layer.
            let mut client = match McpTransport::from_server_config(server) {
                Ok(c) => c,
                Err(_) => continue,
            };
            if let Ok(tools) = client.list_tools() {
                for tool in tools {
                    out.push(McpToolInfo {
                        server: server.name.clone(),
                        name: tool["name"].as_str().unwrap_or("").to_string(),
                        description: tool["description"].as_str().unwrap_or("").to_string(),
                        input_schema: tool
                            .get("inputSchema")
                            .cloned()
                            .unwrap_or(json!({ "type": "object", "properties": {} })),
                    });
                }
            }
        }
        Ok(out)
    })
    .await
    .map_err(|e| e.to_string())?
}

// ── Execution ────────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowStepResult {
    pub node_id: String,
    pub label: String,
    pub kind: String,
    pub output: String,
    pub error: Option<String>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RunWorkflowRequest {
    pub workflow: Workflow,
    pub provider: Option<LlmProvider>,
    pub mcp_servers: Vec<McpServer>,
    /// Optional initial input fed to the first node.
    #[serde(default)]
    pub input: String,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RunWorkflowResult {
    pub steps: Vec<WorkflowStepResult>,
    pub final_output: String,
    pub success: bool,
    pub error: Option<String>,
}

/// Recursively replace the `{{input}}` placeholder in every string value.
fn substitute_input(value: &Value, input: &str) -> Value {
    match value {
        Value::String(s) => Value::String(s.replace("{{input}}", input)),
        Value::Array(arr) => Value::Array(arr.iter().map(|v| substitute_input(v, input)).collect()),
        Value::Object(obj) => Value::Object(
            obj.iter()
                .map(|(k, v)| (k.clone(), substitute_input(v, input)))
                .collect(),
        ),
        other => other.clone(),
    }
}

/// Remove `<think>...</think>` blocks (including nested content) that
/// reasoning models like DeepSeek-R1 prepend to their responses.
/// Also trims leading/trailing whitespace from the result.
fn strip_thinking(s: &str) -> String {
    let mut result = String::with_capacity(s.len());
    let mut rest = s;
    loop {
        if let Some(start) = rest.find("<think>") {
            result.push_str(&rest[..start]);
            if let Some(end) = rest[start..].find("</think>") {
                rest = &rest[start + end + "</think>".len()..];
            } else {
                // Unclosed tag — drop the rest entirely
                break;
            }
        } else {
            result.push_str(rest);
            break;
        }
    }
    result.trim().to_string()
}

fn tool_result_text(result: &Value) -> String {
    if let Some(arr) = result["content"].as_array() {
        let parts: Vec<String> = arr
            .iter()
            .filter_map(|c| c["text"].as_str().map(|s| s.to_string()))
            .collect();
        if !parts.is_empty() {
            return parts.join("\n");
        }
    }
    // Fallback: stringify whole result
    serde_json::to_string(result).unwrap_or_default()
}

#[tauri::command]
pub async fn run_workflow(request: RunWorkflowRequest) -> RunWorkflowResult {
    match run_workflow_inner(request).await {
        Ok(r) => r,
        Err(e) => RunWorkflowResult {
            steps: vec![],
            final_output: String::new(),
            success: false,
            error: Some(e),
        },
    }
}

/// Streaming variant: emits one `workflow-step` event per completed step,
/// then a final `workflow-done` event.  The front-end can update the UI
/// incrementally instead of waiting for the whole pipeline to finish.
#[tauri::command]
pub async fn run_workflow_stream(
    window: tauri::Window,
    request: RunWorkflowRequest,
) -> RunWorkflowResult {
    match run_workflow_stream_inner(window, request).await {
        Ok(r) => r,
        Err(e) => RunWorkflowResult {
            steps: vec![],
            final_output: String::new(),
            success: false,
            error: Some(e),
        },
    }
}

async fn run_workflow_inner(request: RunWorkflowRequest) -> Result<RunWorkflowResult, String> {
    run_workflow_core(request, None).await
}

async fn run_workflow_stream_inner(
    window: tauri::Window,
    request: RunWorkflowRequest,
) -> Result<RunWorkflowResult, String> {
    run_workflow_core(request, Some(window)).await
}

async fn run_workflow_core(
    request: RunWorkflowRequest,
    window: Option<tauri::Window>,
) -> Result<RunWorkflowResult, String> {
    // Helper: emit a step event if streaming.
    let emit_step = |step: &WorkflowStepResult| {
        if let Some(ref w) = window {
            let _ = w.emit("workflow-step", step);
        }
    };
    let RunWorkflowRequest {
        workflow,
        provider,
        mcp_servers,
        input,
    } = request;

    if workflow.nodes.is_empty() {
        return Err("Workflow has no nodes".to_string());
    }

    let mut steps: Vec<WorkflowStepResult> = vec![];
    let mut current_input = input;

    for node in &workflow.nodes {
        let label = if node.label.is_empty() {
            match node.kind.as_str() {
                "tool" => format!("{}__{}", node.server, node.tool),
                _ => "llm".to_string(),
            }
        } else {
            node.label.clone()
        };

        match node.kind.as_str() {
            "tool" => {
                // Find the matching server config.
                let server = mcp_servers.iter().find(|s| s.name == node.server).cloned();
                let Some(server) = server else {
                    let err = format!("MCP server '{}' not enabled or not found", node.server);
                    steps.push(WorkflowStepResult {
                        node_id: node.id.clone(),
                        label,
                        kind: node.kind.clone(),
                        output: String::new(),
                        error: Some(err.clone()),
                    });
                    return finish(steps, err);
                };

                let tool_name = node.tool.clone();
                let args = substitute_input(&node.arguments, &current_input);
                let args = if args.is_null() { json!({}) } else { args };

                // Blocking stdio call on a worker thread.
                let res = tokio::task::spawn_blocking(move || -> Result<String, String> {
                    let mut client = McpClient::start(&server)?;
                    let result = client.call_tool(&tool_name, &args)?;
                    Ok(tool_result_text(&result))
                })
                .await
                .map_err(|e| e.to_string())?;

                match res {
                    Ok(out) => {
                        current_input = out.clone();
                        let step = WorkflowStepResult {
                            node_id: node.id.clone(),
                            label,
                            kind: node.kind.clone(),
                            output: out,
                            error: None,
                        };
                        emit_step(&step);
                        steps.push(step);
                    }
                    Err(e) => {
                        let step = WorkflowStepResult {
                            node_id: node.id.clone(),
                            label,
                            kind: node.kind.clone(),
                            output: String::new(),
                            error: Some(e.clone()),
                        };
                        emit_step(&step);
                        steps.push(step);
                        return finish(steps, e);
                    }
                }
            }
            "llm" => {
                let Some(ref p) = provider else {
                    let err = "No LLM provider configured for this workflow".to_string();
                    steps.push(WorkflowStepResult {
                        node_id: node.id.clone(),
                        label,
                        kind: node.kind.clone(),
                        output: String::new(),
                        error: Some(err.clone()),
                    });
                    return finish(steps, err);
                };

                // Build messages so the LLM clearly understands:
                //   - system: general workflow-step role
                //   - user:   instruction (node.prompt) + the content to process (current_input)
                // When prompt is empty, the input IS the full task.
                // When input is empty, the prompt is the full task (first node, no prior step).
                let (system_content, user_content) = if current_input.is_empty() {
                    (
                        "You are a workflow step. Complete the task below and reply with only the result.".to_string(),
                        node.prompt.clone(),
                    )
                } else if node.prompt.is_empty() {
                    (
                        "You are a workflow step. Process the provided content and reply with only the result.".to_string(),
                        current_input.clone(),
                    )
                } else {
                    (
                        format!("You are a workflow step. Instruction: {}", node.prompt),
                        format!("Content to process:\n\n{}", current_input),
                    )
                };

                let messages = vec![
                    json!({ "role": "system", "content": system_content }),
                    json!({ "role": "user",   "content": user_content }),
                ];

                match chat(p, &messages, &[]).await {
                    Ok(resp) => {
                        let raw = resp["choices"][0]["message"]["content"]
                            .as_str()
                            .unwrap_or("")
                            .to_string();
                        let out = strip_thinking(&raw);
                        current_input = out.clone();
                        let step = WorkflowStepResult {
                            node_id: node.id.clone(),
                            label,
                            kind: node.kind.clone(),
                            output: out,
                            error: None,
                        };
                        emit_step(&step);
                        steps.push(step);
                    }
                    Err(e) => {
                        let step = WorkflowStepResult {
                            node_id: node.id.clone(),
                            label,
                            kind: node.kind.clone(),
                            output: String::new(),
                            error: Some(e.clone()),
                        };
                        emit_step(&step);
                        steps.push(step);
                        return finish(steps, e);
                    }
                }
            }
            "mcp_agent" => {
                // Run the named MCP server as a mini agent with the LLM.
                let Some(ref p) = provider else {
                    let err = "No LLM provider configured for this workflow".to_string();
                    steps.push(WorkflowStepResult {
                        node_id: node.id.clone(),
                        label,
                        kind: node.kind.clone(),
                        output: String::new(),
                        error: Some(err.clone()),
                    });
                    return finish(steps, err);
                };

                let server = mcp_servers.iter().find(|s| s.name == node.server).cloned();
                let Some(server) = server else {
                    let err = format!("MCP server '{}' not enabled or not found", node.server);
                    steps.push(WorkflowStepResult {
                        node_id: node.id.clone(),
                        label,
                        kind: node.kind.clone(),
                        output: String::new(),
                        error: Some(err.clone()),
                    });
                    return finish(steps, err);
                };

                // `node.prompt` = extra system instructions for the agent (may be empty).
                // `current_input` = the task the agent must work on (the previous step's output).
                // If there's no prior output yet, fall back to node.prompt as the task.
                let (task, extra_prompt) = if current_input.is_empty() {
                    (node.prompt.clone(), String::new())
                } else {
                    (current_input.clone(), node.prompt.clone())
                };

                match run_mcp_agent_node(&server, p, &task, &extra_prompt, 10).await {
                    Ok(raw) => {
                        let out = strip_thinking(&raw);
                        current_input = out.clone();
                        let step = WorkflowStepResult {
                            node_id: node.id.clone(),
                            label,
                            kind: node.kind.clone(),
                            output: out,
                            error: None,
                        };
                        emit_step(&step);
                        steps.push(step);
                    }
                    Err(e) => {
                        let err_msg = format!("[MCP agent error: {}]", e);
                        current_input = err_msg.clone();
                        let step = WorkflowStepResult {
                            node_id: node.id.clone(),
                            label,
                            kind: node.kind.clone(),
                            output: err_msg,
                            error: Some(e),
                        };
                        emit_step(&step);
                        steps.push(step);
                    }
                }
            }
            other => {
                let err = format!("Unknown node kind: {}", other);
                steps.push(WorkflowStepResult {
                    node_id: node.id.clone(),
                    label,
                    kind: node.kind.clone(),
                    output: String::new(),
                    error: Some(err.clone()),
                });
                return finish(steps, err);
            }
        }
    }

    // ── Build final output ────────────────────────────────────────────────────
    // If there is more than one step and a provider is available, ask the LLM
    // to produce a coherent final summary from all step outputs.  Otherwise
    // the last step's clean output is used as-is.
    let final_output = if steps.len() > 1 {
        if let Some(ref p) = provider {
            let step_summary = steps.iter().enumerate()
                .map(|(i, s)| format!("Step {} ({}): {}", i + 1, s.label,
                    if s.output.is_empty() { s.error.as_deref().unwrap_or("(no output)") }
                    else { &s.output }))
                .collect::<Vec<_>>()
                .join("\n\n");

            let messages = vec![
                json!({
                    "role": "system",
                    "content": "You are a workflow summariser. Given the outputs of each step in a pipeline, write a single coherent final answer that integrates all the information. Be concise and informative."
                }),
                json!({
                    "role": "user",
                    "content": format!("Pipeline step outputs:\n\n{}", step_summary)
                }),
            ];

            match chat(p, &messages, &[]).await {
                Ok(resp) => {
                    let raw = resp["choices"][0]["message"]["content"]
                        .as_str().unwrap_or("").to_string();
                    let cleaned = strip_thinking(&raw);
                    if cleaned.is_empty() { current_input } else { cleaned }
                }
                Err(_) => current_input,
            }
        } else {
            current_input
        }
    } else {
        current_input
    };

    let result = Ok(RunWorkflowResult {
        steps,
        final_output,
        success: true,
        error: None,
    });

    if let Ok(ref r) = result {
        if let Some(ref w) = window {
            let _ = w.emit("workflow-done", r);
        }
    }

    result
}

fn finish(steps: Vec<WorkflowStepResult>, err: String) -> Result<RunWorkflowResult, String> {
    Ok(RunWorkflowResult {
        steps,
        final_output: String::new(),
        success: false,
        error: Some(err),
    })
}

// ── MCP-agent node helper ─────────────────────────────────────────────────────

/// Run a single MCP server as an agent node.
///
/// MCP stdio I/O is blocking, so it runs on a `spawn_blocking` worker.
/// LLM HTTP calls are async and run directly on the caller's tokio runtime —
/// this avoids the nested-runtime / TLS-init problem that occurs when trying
/// to drive `reqwest` from inside a `spawn_blocking` closure.
///
/// `task`         – the user-facing task (previous step's output).
/// `extra_prompt` – optional extra instruction added to the system message.
/// `max_iter`     – maximum LLM ↔ tool loop iterations.
async fn run_mcp_agent_node(
    server: &McpServer,
    provider: &LlmProvider,
    task: &str,
    extra_prompt: &str,
    max_iter: u32,
) -> Result<String, String> {
    let server_clone = server.clone();

    // ── 1. Start MCP client and list tools on a blocking thread ──────────────
    let (client_arc, all_tools) = tokio::task::spawn_blocking(move || -> Result<(Arc<Mutex<McpClient>>, Vec<Value>), String> {
        let mut c = McpClient::start(&server_clone)?;
        let raw = c.list_tools()?;
        let tools: Vec<Value> = raw.iter().map(|t| json!({
            "type": "function",
            "function": {
                "name": t["name"].as_str().unwrap_or(""),
                "description": t["description"].as_str().unwrap_or(""),
                "parameters": t.get("inputSchema").cloned()
                    .unwrap_or(json!({"type":"object","properties":{}}))
            }
        })).collect();
        Ok((Arc::new(Mutex::new(c)), tools))
    })
    .await
    .map_err(|e| e.to_string())??;

    // ── 2. Build system prompt ────────────────────────────────────────────────
    let tool_names: Vec<String> = all_tools.iter()
        .filter_map(|t| t["function"]["name"].as_str().map(|s| s.to_string()))
        .collect();

    let system = if extra_prompt.is_empty() {
        format!(
            "You are a helpful AI agent. Complete the user's task using the available tools.\n\
             Available tools: {}\n\
             When finished, provide a concise final answer.",
            tool_names.join(", ")
        )
    } else {
        format!(
            "{}\nAvailable tools: {}\nWhen finished, provide a concise final answer.",
            extra_prompt,
            tool_names.join(", ")
        )
    };

    let mut messages: Vec<Value> = vec![
        json!({"role": "system", "content": system}),
        json!({"role": "user",   "content": task}),
    ];

    // ── 3. Async agent loop ───────────────────────────────────────────────────
    // HTTP calls (.await) stay on the tokio runtime; only MCP stdio calls go
    // back to spawn_blocking.
    let mut final_answer = String::new();
    let mut last_tool_results: Vec<String> = vec![];

    for _ in 0..max_iter {
        let response = chat(provider, &messages, &all_tools).await?;
        let choice = &response["choices"][0];
        let message = &choice["message"];
        let finish_reason = choice["finish_reason"].as_str().unwrap_or("");

        messages.push(message.clone());

        if let Some(tool_calls) = message["tool_calls"].as_array() {
            if tool_calls.is_empty() {
                final_answer = strip_thinking(message["content"].as_str().unwrap_or(""));
                break;
            }

            last_tool_results.clear();
            for tc in tool_calls {
                let tc_id    = tc["id"].as_str().unwrap_or("").to_string();
                let fn_name  = tc["function"]["name"].as_str().unwrap_or("").to_string();
                let fn_args: Value = tc["function"]["arguments"]
                    .as_str()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or(json!({}));

                // Call the MCP tool on a blocking thread.
                let client_arc2 = Arc::clone(&client_arc);
                let fn_name2   = fn_name.clone();
                let fn_args2   = fn_args.clone();
                let tool_result_text = tokio::task::spawn_blocking(move || {
                    let mut client = client_arc2.lock().unwrap();
                    match client.call_tool(&fn_name2, &fn_args2) {
                        Ok(res) => {
                            res["content"].as_array()
                                .map(|arr| arr.iter()
                                    .filter_map(|c| c["text"].as_str().map(|s| s.to_string()))
                                    .collect::<Vec<_>>()
                                    .join("\n"))
                                .filter(|s| !s.is_empty())
                                .unwrap_or_else(|| serde_json::to_string(&res).unwrap_or_default())
                        }
                        Err(e) => format!("Tool error: {}", e),
                    }
                })
                .await
                .unwrap_or_else(|e| format!("Spawn error: {}", e));

                last_tool_results.push(format!("[{}]\n{}", fn_name, tool_result_text));
                messages.push(json!({
                    "role": "tool",
                    "tool_call_id": tc_id,
                    "content": tool_result_text,
                }));
            }
        } else {
            // No tool_calls key — plain text answer
            final_answer = strip_thinking(message["content"].as_str().unwrap_or(""));
            break;
        }

        if finish_reason == "stop" {
            // LLM finished tool calls; ask for a plain-text summary
            if let Ok(resp) = chat(provider, &messages, &[]).await {
                final_answer = strip_thinking(
                    resp["choices"][0]["message"]["content"].as_str().unwrap_or("")
                );
            }
            break;
        }
    }

    // Fallback 1: last assistant message that has text content
    if final_answer.is_empty() {
        final_answer = messages.iter().rev()
            .find_map(|m| {
                if m["role"] == "assistant" {
                    let c = strip_thinking(m["content"].as_str().unwrap_or(""));
                    if !c.is_empty() { Some(c) } else { None }
                } else { None }
            })
            .unwrap_or_default();
    }

    // Fallback 2: concatenate last round of tool results so the next node
    // still receives meaningful data even if the LLM never produced a final answer
    if final_answer.is_empty() && !last_tool_results.is_empty() {
        final_answer = last_tool_results.join("\n\n");
    }

    // Fallback 3: if still empty (LLM produced only thinking, no action/answer),
    // send one more request explicitly asking for a text-only response
    if final_answer.is_empty() {
        messages.push(json!({
            "role": "user",
            "content": "Please provide your final answer as plain text."
        }));
        if let Ok(resp) = chat(provider, &messages, &[]).await {
            final_answer = strip_thinking(
                resp["choices"][0]["message"]["content"].as_str().unwrap_or("")
            );
        }
    }

    Ok(final_answer)
}
