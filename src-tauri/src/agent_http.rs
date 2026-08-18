//! Agent HTTP 双向通信 + 工作流引擎对接（阶段四）
//!
//! HTTP server 在 9420 端口暴露：
//! - POST /agent/dispatch   → 给子 Agent 发任务（支持两种模式：直接转发 / 触发工作流）
//! - POST /agent/submit     → 接收子 Agent 提交的结果（更新 StepInstance，推进工作流）
//! - POST /hook             → 外部 Hook 触发工作流（按 template_key 找模板）
//! - GET  /agent/tasks      → 查看待处理任务
//! - GET  /agent/results    → 查看已接收结果
//! - GET  /runs             → 列出工作流 Run 摘要
//! - GET  /runs/:id         → 获取 Run 详情
//! - POST /runs/:id/approve → 通过验收
//! - POST /runs/:id/reject  → 驳回返工
//! - GET  /health           → 健康检查

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::sync::{Arc, Mutex};

// ── Hook Server 配置（持久化到 hook_server.json）──────────────────────────────

/// Hook Server 配置（端口 + 鉴权 + 开关）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookServerConfig {
    /// 监听端口（默认 9420）
    #[serde(default = "default_port")]
    pub port: u16,
    /// 鉴权 token（None 或空 = 不鉴权）；请求需带 `Authorization: Bearer <token>`
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub auth_token: Option<String>,
    /// 是否启用 Hook Server
    #[serde(default = "default_enabled")]
    pub enabled: bool,
    /// 最大并发 Run 数（预留）
    #[serde(default = "default_max_concurrent")]
    pub max_concurrent_runs: u32,
}

fn default_port() -> u16 {
    9420
}
fn default_enabled() -> bool {
    true
}
fn default_max_concurrent() -> u32 {
    5
}

impl Default for HookServerConfig {
    fn default() -> Self {
        HookServerConfig {
            port: 9420,
            auth_token: None,
            enabled: true,
            max_concurrent_runs: 5,
        }
    }
}

fn hook_server_config_path() -> std::path::PathBuf {
    dirs_next::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("agent-manager")
        .join("hook_server.json")
}

/// 从磁盘读取 Hook Server 配置（文件不存在时返回默认值）
pub fn read_hook_server_config() -> HookServerConfig {
    let path = hook_server_config_path();
    let raw = std::fs::read_to_string(&path).unwrap_or_default();
    serde_json::from_str(&raw).unwrap_or_default()
}

/// 将 Hook Server 配置持久化到磁盘
pub fn write_hook_server_config(config: &HookServerConfig) -> Result<(), String> {
    let path = hook_server_config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())?;
    Ok(())
}

// ── 数据模型 ─────────────────────────────────────────────────────────────────

/// 给子 Agent 的任务
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentTask {
    pub task_id: String,
    pub agent_id: String,
    /// 子 Agent 的 HTTP 端点 URL（例如 http://localhost:8501/task）
    pub agent_url: String,
    /// 任务描述
    pub task: String,
    /// 上下文（上游输出等）
    #[serde(default)]
    pub context: Value,
    /// 回调 URL（子 Agent 完成后 POST 到这里）
    pub callback_url: String,
    /// 创建时间
    pub created_at: i64,
    /// 状态：dispatched / submitted / timeout / dispatch_failed
    pub status: String,
    /// 子 Agent 提交的结果
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result: Option<Value>,
    /// 工作流关联：如果此任务由工作流 agent_task 节点触发，记录 run_id + step_id
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub run_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub step_id: Option<String>,
}

/// 子 Agent 提交的结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentResult {
    pub task_id: String,
    pub agent_id: String,
    /// 结果内容
    pub result: Value,
    /// 裁定：pass / fail / blocked
    #[serde(default = "default_verdict")]
    pub verdict: String,
    /// 说明
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
    /// 提交时间
    pub submitted_at: i64,
}

fn default_verdict() -> String {
    "pass".to_string()
}

// ── 内存存储（spike 用，不落盘）──────────────────────────────────────────────
//
// 用全局 OnceLock<Arc<AgentHttpStore>> 让 HTTP server 和 Tauri 命令共享同一份 store，
// 不走 Tauri State（避免 setup 钩子里的生命周期问题）。

use std::sync::OnceLock;

static HTTP_STORE: OnceLock<Arc<AgentHttpStore>> = OnceLock::new();

/// 当前运行的 HTTP server 句柄（用于 restart 时关闭旧 server）
static SERVER_HANDLE: OnceLock<Arc<Mutex<Option<tauri::async_runtime::JoinHandle<()>>>>> =
    OnceLock::new();

/// 获取 server handle（restart 用）
pub fn get_server_handle() -> Arc<Mutex<Option<tauri::async_runtime::JoinHandle<()>>>> {
    Arc::clone(SERVER_HANDLE.get_or_init(|| Arc::new(Mutex::new(None))))
}

/// 关闭当前运行的 HTTP server（restart 时调用）
async fn shutdown_current_server() {
    let handle = get_server_handle();
    let opt = {
        let mut guard = handle.lock().unwrap();
        guard.take()
    };
    if let Some(h) = opt {
        h.abort();
        eprintln!("[agent-http] previous server aborted");
    }
}

pub struct AgentHttpStore {
    /// task_id → AgentTask
    pub tasks: Mutex<HashMap<String, AgentTask>>,
    /// task_id → AgentResult
    pub results: Mutex<HashMap<String, AgentResult>>,
    /// 工作流等待中的任务：task_id → (run_id, step_id, oneshot_sender)
    /// agent_task 节点 dispatch 后挂起等待 submit 回调，submit 时通过 sender 唤醒
    pub pending: Mutex<HashMap<String, PendingTask>>,
}

/// 工作流中等待子 Agent 结果的挂起任务
pub struct PendingTask {
    #[allow(dead_code)]
    pub run_id: String,
    #[allow(dead_code)]
    pub step_id: String,
    pub tx: tokio::sync::oneshot::Sender<AgentResult>,
}

/// 获取全局 store（HTTP server 和 Tauri 命令共用）
pub fn get_store() -> Arc<AgentHttpStore> {
    Arc::clone(HTTP_STORE.get().expect("AgentHttpStore not initialized"))
}

/// 初始化全局 store（在 Tauri setup 钩子里调用一次）
pub fn init_store() -> Arc<AgentHttpStore> {
    HTTP_STORE
        .get_or_init(|| Arc::new(AgentHttpStore::new()))
        .clone()
}

impl AgentHttpStore {
    pub fn new() -> Self {
        AgentHttpStore {
            tasks: Mutex::new(HashMap::new()),
            results: Mutex::new(HashMap::new()),
            pending: Mutex::new(HashMap::new()),
        }
    }

    /// 注册一个工作流等待中的任务（agent_task 节点 dispatch 后调用）
    pub fn register_pending(
        &self,
        task_id: &str,
        run_id: &str,
        step_id: &str,
    ) -> tokio::sync::oneshot::Receiver<AgentResult> {
        let (tx, rx) = tokio::sync::oneshot::channel();
        let mut pending = self.pending.lock().unwrap();
        pending.insert(
            task_id.to_string(),
            PendingTask {
                run_id: run_id.to_string(),
                step_id: step_id.to_string(),
                tx,
            },
        );
        rx
    }

    /// 子 Agent submit 时调用，唤醒等待的工作流
    pub fn resolve_pending(&self, task_id: &str, result: AgentResult) -> bool {
        let mut pending = self.pending.lock().unwrap();
        if let Some(p) = pending.remove(task_id) {
            let _ = p.tx.send(result);
            true
        } else {
            false
        }
    }
}

impl Default for AgentHttpStore {
    fn default() -> Self {
        Self::new()
    }
}

// ── HTTP Server（极简手写，不引入框架）──────────────────────────────────────

/// 启动 HTTP server。在 Tauri setup 里 tokio::spawn 调用。
pub async fn start_agent_http_server(
    port: u16,
    auth_token: Option<String>,
    store: Arc<AgentHttpStore>,
) {
    let addr = format!("127.0.0.1:{}", port);
    // A development restart or a previous app instance can briefly keep the
    // port occupied.  Hooks must recover once that process exits instead of
    // silently remaining offline for the lifetime of this desktop instance.
    let listener = loop {
        match tokio::net::TcpListener::bind(&addr).await {
            Ok(listener) => {
                eprintln!(
                    "[agent-http] listening on http://{} (auth={})",
                    addr,
                    auth_token.is_some()
                );
                break listener;
            }
            Err(error) => {
                eprintln!("[agent-http] failed to bind {addr}: {error}; retrying in 5 seconds");
                tokio::time::sleep(std::time::Duration::from_secs(5)).await;
            }
        }
    };

    loop {
        match listener.accept().await {
            Ok((stream, _)) => {
                let store = Arc::clone(&store);
                let token = auth_token.clone();
                tokio::spawn(handle_connection(stream, store, token));
            }
            Err(e) => {
                eprintln!("[agent-http] accept error: {}", e);
            }
        }
    }
}

async fn handle_connection(
    mut stream: tokio::net::TcpStream,
    store: Arc<AgentHttpStore>,
    auth_token: Option<String>,
) {
    use tokio::io::AsyncWriteExt;

    // Read the complete HTTP body.  Hook payloads often include tool output and
    // exceed a single TCP frame; the old 8 KiB one-shot read silently truncated
    // such events and turned them into ignored JSON.
    const MAX_REQUEST_BYTES: usize = 1024 * 1024;
    let raw = match read_http_request(&mut stream, MAX_REQUEST_BYTES).await {
        Ok(raw) => raw,
        Err(message) => {
            let status = if message.contains("too large") {
                "413 Payload Too Large"
            } else {
                "400 Bad Request"
            };
            let response = json!({"error": message}).to_string();
            let http = format!(
                "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                response.len(), response
            );
            let _ = stream.write_all(http.as_bytes()).await;
            return;
        }
    };

    // 解析 HTTP 请求行
    let first_line = raw.lines().next().unwrap_or("");
    let parts: Vec<&str> = first_line.split_whitespace().collect();
    if parts.len() < 2 {
        let _ = stream.write_all(b"HTTP/1.1 400 Bad Request\r\n\r\n").await;
        return;
    }
    let method = parts[0];
    let path = parts[1];

    // 提取 body（\r\n\r\n 之后）
    let body = if let Some(idx) = raw.find("\r\n\r\n") {
        &raw[idx + 4..]
    } else {
        ""
    };

    // 提取 Authorization header（阶段四：auth_token 校验）
    let auth_header = extract_header(&raw, "authorization");

    // 鉴权校验
    if let Some(ref expected_token) = auth_token {
        if !expected_token.is_empty() {
            let provided = auth_header
                .as_deref()
                .and_then(|h| h.strip_prefix("Bearer "))
                .unwrap_or("");
            if provided != expected_token {
                let resp = json!({"error": "unauthorized", "hint": "provide Authorization: Bearer <token>"}).to_string();
                let http = format!(
                    "HTTP/1.1 401 Unauthorized\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",
                    resp.len(),
                    resp
                );
                let _ = stream.write_all(http.as_bytes()).await;
                return;
            }
        }
    }

    // 路由
    let (status, response) = route(method, path, body, &store).await;

    let http_response = format!(
        "HTTP/1.1 {}\r\nContent-Type: application/json\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, POST, OPTIONS\r\nAccess-Control-Allow-Headers: Content-Type, Authorization\r\nContent-Length: {}\r\n\r\n{}",
        status,
        response.len(),
        response
    );

    let _ = stream.write_all(http_response.as_bytes()).await;
    let _ = stream.flush().await;
}

/// Read one HTTP/1.1 request without assuming that headers and body arrive in
/// one `read`.  The hook endpoint only needs JSON bodies, so a bounded
/// Content-Length parser is sufficient and keeps this lightweight server safe.
async fn read_http_request(
    stream: &mut tokio::net::TcpStream,
    max_request_bytes: usize,
) -> Result<String, String> {
    use tokio::io::AsyncReadExt;

    let mut bytes = Vec::with_capacity(4096);
    let mut chunk = [0u8; 4096];
    let header_end;
    loop {
        let count = stream.read(&mut chunk).await.map_err(|e| e.to_string())?;
        if count == 0 {
            return Err("connection closed before HTTP headers".into());
        }
        bytes.extend_from_slice(&chunk[..count]);
        if bytes.len() > max_request_bytes {
            return Err("request too large".into());
        }
        if let Some(pos) = bytes.windows(4).position(|window| window == b"\r\n\r\n") {
            header_end = pos + 4;
            break;
        }
    }

    let headers =
        std::str::from_utf8(&bytes[..header_end]).map_err(|_| "HTTP headers are not UTF-8")?;
    let content_length = headers
        .lines()
        .find_map(|line| {
            line.split_once(':')
                .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
        })
        .map(|(_, value)| {
            value
                .trim()
                .parse::<usize>()
                .map_err(|_| "invalid Content-Length".to_string())
        })
        .transpose()?
        .unwrap_or(0);
    if header_end.saturating_add(content_length) > max_request_bytes {
        return Err("request too large".into());
    }

    let required = header_end + content_length;
    while bytes.len() < required {
        let count = stream.read(&mut chunk).await.map_err(|e| e.to_string())?;
        if count == 0 {
            return Err("connection closed before HTTP body".into());
        }
        bytes.extend_from_slice(&chunk[..count]);
        if bytes.len() > max_request_bytes {
            return Err("request too large".into());
        }
    }
    String::from_utf8(bytes[..required].to_vec()).map_err(|_| "HTTP request is not UTF-8".into())
}

/// 从原始 HTTP 请求文本中提取指定 header 的值（大小写不敏感）
fn extract_header(raw: &str, name: &str) -> Option<String> {
    let name_lower = name.to_lowercase();
    for line in raw.lines() {
        if line.is_empty() || !line.contains(':') {
            continue;
        }
        // 跳过请求行
        if line.starts_with("GET ") || line.starts_with("POST ") || line.starts_with("OPTIONS ") {
            continue;
        }
        let mut parts = line.splitn(2, ':');
        let key = parts.next()?.trim().to_lowercase();
        let value = parts.next()?.trim().to_string();
        if key == name_lower {
            return Some(value);
        }
    }
    None
}

async fn route(
    method: &str,
    path: &str,
    body: &str,
    store: &Arc<AgentHttpStore>,
) -> (&'static str, String) {
    // CORS preflight
    if method == "OPTIONS" {
        return ("204 No Content", String::new());
    }

    match (method, path) {
        ("GET", "/health") => (
            "200 OK",
            json!({"status": "ok", "service": "agent-manager-http"}).to_string(),
        ),

        ("GET", "/agent/tasks") => {
            let tasks = store.tasks.lock().unwrap();
            let list: Vec<&AgentTask> = tasks.values().collect();
            ("200 OK", serde_json::to_string(&list).unwrap_or_default())
        }

        ("GET", "/agent/results") => {
            let results = store.results.lock().unwrap();
            let list: Vec<&AgentResult> = results.values().collect();
            ("200 OK", serde_json::to_string(&list).unwrap_or_default())
        }

        ("POST", p) if p == "/agent/dispatch" || p.starts_with("/agent/dispatch") => {
            let task: AgentTask = match serde_json::from_str(body) {
                Ok(t) => t,
                Err(e) => {
                    return (
                        "400 Bad Request",
                        json!({"error": format!("invalid body: {}", e)}).to_string(),
                    );
                }
            };

            // 存储任务
            {
                let mut tasks = store.tasks.lock().unwrap();
                tasks.insert(task.task_id.clone(), task.clone());
            }

            // 异步发任务给子 Agent
            let store2 = Arc::clone(store);
            let task_clone = task.clone();
            tokio::spawn(async move {
                dispatch_to_agent(store2, task_clone).await;
            });

            (
                "202 Accepted",
                json!({
                    "task_id": task.task_id,
                    "status": "dispatched",
                    "agent_url": task.agent_url,
                })
                .to_string(),
            )
        }

        ("POST", p) if p == "/agent/submit" || p.starts_with("/agent/submit") => {
            let result: AgentResult = match serde_json::from_str(body) {
                Ok(r) => r,
                Err(e) => {
                    return (
                        "400 Bad Request",
                        json!({"error": format!("invalid body: {}", e)}).to_string(),
                    );
                }
            };

            // 存储结果
            {
                let mut results = store.results.lock().unwrap();
                results.insert(result.task_id.clone(), result.clone());
            }

            // 更新任务状态
            {
                let mut tasks = store.tasks.lock().unwrap();
                if let Some(task) = tasks.get_mut(&result.task_id) {
                    task.status = "submitted".to_string();
                    task.result = Some(result.result.clone());
                }
            }

            // 唤醒等待中的工作流（如果此任务由 agent_task 节点触发）
            let resolved = store.resolve_pending(&result.task_id, result.clone());

            eprintln!(
                "[agent-http] submit task={} verdict={} resolved_pending={}",
                result.task_id, result.verdict, resolved
            );

            (
                "200 OK",
                json!({
                    "task_id": result.task_id,
                    "status": "received",
                    "verdict": result.verdict,
                    "workflow_resolved": resolved,
                })
                .to_string(),
            )
        }

        // ── 阶段四：外部 Hook 触发工作流 ──────────────────────────────────────
        ("POST", p) if p == "/memory/hook" || p.starts_with("/memory/hook") => {
            // Agent hook 回调：自动沉淀记忆与 skill（异步处理，不阻塞）
            let source = p
                .split_once('?')
                .and_then(|(_, query)| {
                    query
                        .split('&')
                        .find_map(|part| part.strip_prefix("source="))
                })
                .filter(|source| matches!(*source, "claude" | "qoder" | "codex" | "workbuddy"))
                .unwrap_or("unknown");
            if let Some(ingest) = crate::memory_ingest::ingest_store() {
                if let Some(backend) = crate::memory_backend::shared_backend() {
                    return match ingest.handle_hook(backend, body, source) {
                        Ok(resp) => ("200 OK", resp.to_string()),
                        Err(error) => ("400 Bad Request", json!({"error": error}).to_string()),
                    };
                }
            }
            (
                "200 OK",
                json!({"status": "ok", "note": "ingest not ready"}).to_string(),
            )
        }

        // Normalized telemetry endpoint for adapters that do not implement the
        // Claude/Qoder Hook payload.  Any supported Agent adapter can submit
        // a final `session_usage` record here without being coerced into a
        // conversation-memory event.  Native transcript import remains an
        // optional enrichment for the four locally integrated Agents.
        ("POST", p) if p.starts_with("/telemetry/events/") => {
            let source = p
                .trim_start_matches("/telemetry/events/")
                .split('?')
                .next()
                .unwrap_or("");
            if !matches!(
                source,
                "codex"
                    | "workbuddy"
                    | "claude"
                    | "qoder"
                    | "gemini"
                    | "opencode"
                    | "openclaw"
                    | "pi"
                    | "grokbuild"
            ) {
                return (
                    "400 Bad Request",
                    json!({"error": "unsupported telemetry source"}).to_string(),
                );
            }
            let payload: Value = match serde_json::from_str(body) {
                Ok(payload) => payload,
                Err(error) => {
                    return (
                        "400 Bad Request",
                        json!({"error": format!("invalid JSON: {error}")}).to_string(),
                    )
                }
            };
            match crate::telemetry_store::shared_store() {
                Some(store) => match store.record_hook(source, &payload).and_then(|_| store.record_final_session_usage(source, &payload)) {
                    Ok(true) => ("202 Accepted", json!({"status": "confirmed_session_usage", "source": source}).to_string()),
                    Ok(false) => ("202 Accepted", json!({"status": "recorded_unverified", "source": source, "note": "event retained but not included in token totals; send event=session_usage for a final session total"}).to_string()),
                    Err(error) => ("500 Internal Server Error", json!({"error": error}).to_string()),
                },
                None => ("503 Service Unavailable", json!({"error": "telemetry store not ready"}).to_string()),
            }
        }

        ("POST", p) if p == "/hook" || p.starts_with("/hook") => {
            let payload: Value = match serde_json::from_str(body) {
                Ok(v) => v,
                Err(e) => {
                    return (
                        "400 Bad Request",
                        json!({"error": format!("invalid body: {}", e)}).to_string(),
                    );
                }
            };

            let template_key = payload["template_key"].as_str().unwrap_or("");
            if template_key.is_empty() {
                return (
                    "400 Bad Request",
                    json!({"error": "template_key is required"}).to_string(),
                );
            }

            // 按 template_key 查找工作流模板
            let workflows = crate::workflow::read_workflows();
            let wf = workflows
                .into_iter()
                .find(|w| w.template_key.as_deref() == Some(template_key));

            let Some(wf) = wf else {
                return (
                    "404 Not Found",
                    json!({"error": format!("template not found: {}", template_key)}).to_string(),
                );
            };

            // 构造 RunWorkflowRequest
            let title = payload["title"].as_str().unwrap_or("").to_string();
            let description = payload["description"].as_str().unwrap_or("").to_string();
            let input = if !description.is_empty() {
                description
            } else {
                title
            };

            // 阶段四：payload 可携带 callback_url，优先于模板配置
            let callback_url = payload["callback_url"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(|s| s.to_string());

            let request = crate::workflow::RunWorkflowRequest {
                workflow: wf,
                provider: None, // 用默认 LLM 配置
                mcp_servers: vec![],
                input,
                rework: None,
                callback_url,
                trigger: Some(crate::workflow_store::RunTrigger::Hook {
                    source: payload["source"].as_str().unwrap_or("external").to_string(),
                    external_id: payload["external_id"].as_str().unwrap_or("").to_string(),
                }),
            };

            // 异步启动工作流
            let run_store = crate::workflow_store::WorkflowRunStore::new();
            tauri::async_runtime::spawn(async move {
                let _ = crate::workflow::run_workflow_core(request, None, &run_store).await;
            });

            (
                "202 Accepted",
                json!({
                    "status": "workflow_started",
                    "template_key": template_key,
                })
                .to_string(),
            )
        }

        // ── 阶段四：Run 管理 API ──────────────────────────────────────────────
        ("GET", p) if p == "/runs" || p.starts_with("/runs?") => {
            let run_store = crate::workflow_store::WorkflowRunStore::new();
            let list = run_store.list_runs();
            ("200 OK", serde_json::to_string(&list).unwrap_or_default())
        }

        ("GET", p) if p.starts_with("/runs/") => {
            let run_id = p.trim_start_matches("/runs/");
            let run_store = crate::workflow_store::WorkflowRunStore::new();
            match run_store.get_run(run_id) {
                Some(run) => ("200 OK", serde_json::to_string(&run).unwrap_or_default()),
                None => (
                    "404 Not Found",
                    json!({"error": "run not found"}).to_string(),
                ),
            }
        }

        ("POST", p) if p.contains("/approve") && p.starts_with("/runs/") => {
            let run_id = p
                .trim_start_matches("/runs/")
                .trim_end_matches("/approve")
                .to_string();
            let run_store = crate::workflow_store::WorkflowRunStore::new();
            match crate::workflow_store::approve_run_inner(&run_store, &run_id) {
                Ok(run) => ("200 OK", serde_json::to_string(&run).unwrap_or_default()),
                Err(e) => ("400 Bad Request", json!({"error": e}).to_string()),
            }
        }

        ("POST", p) if p.contains("/reject") && p.starts_with("/runs/") => {
            let run_id = p
                .trim_start_matches("/runs/")
                .trim_end_matches("/reject")
                .to_string();
            let payload: Value = serde_json::from_str(body).unwrap_or(json!({}));
            let reject_to = payload["reject_to_node"].as_str().unwrap_or("").to_string();
            let reason = payload["reason"].as_str().unwrap_or("").to_string();

            let run_store = crate::workflow_store::WorkflowRunStore::new();
            match crate::workflow_store::reject_run_inner(&run_store, &run_id, &reject_to, &reason)
            {
                Ok(new_run_id) => (
                    "200 OK",
                    json!({"status": "rejected", "new_run_id": new_run_id}).to_string(),
                ),
                Err(e) => ("400 Bad Request", json!({"error": e}).to_string()),
            }
        }

        _ => (
            "404 Not Found",
            json!({"error": "not found", "path": path}).to_string(),
        ),
    }
}

/// 给子 Agent 发任务（HTTP POST 到子 Agent 的端点）
async fn dispatch_to_agent(store: Arc<AgentHttpStore>, task: AgentTask) {
    eprintln!(
        "[agent-http] dispatching task {} to agent {} at {}",
        task.task_id, task.agent_id, task.agent_url
    );

    let payload = json!({
        "task_id": task.task_id,
        "agent_id": task.agent_id,
        "task": task.task,
        "context": task.context,
        "callback_url": task.callback_url,
    });

    let result = reqwest::Client::new()
        .post(&task.agent_url)
        .json(&payload)
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await;

    match result {
        Ok(resp) => {
            eprintln!(
                "[agent-http] agent {} accepted task {} (HTTP {})",
                task.agent_id,
                task.task_id,
                resp.status()
            );
        }
        Err(e) => {
            eprintln!(
                "[agent-http] failed to dispatch to agent {}: {}",
                task.agent_id, e
            );
            // 更新任务状态为失败
            let mut tasks = store.tasks.lock().unwrap();
            if let Some(t) = tasks.get_mut(&task.task_id) {
                t.status = "dispatch_failed".to_string();
            }
        }
    }
}

// ── Tauri 命令（前端可调用，用于测试）────────────────────────────────────────
//
// 注意：不用 State<'_, AgentHttpStore>，改用全局 get_store()。
// 因为 setup 钩子里 Tauri runtime 还没完全起来，manage() 注册的 State
// 和 HTTP server 的 Arc<AgentHttpStore> 是两份独立数据，无法共享。
// 全局 OnceLock 保证两者访问同一份 store。

/// 手动给子 Agent 发任务（测试用）
#[tauri::command]
pub async fn dispatch_agent_task(
    agent_id: String,
    agent_url: String,
    task: String,
    context: Option<Value>,
) -> Result<String, String> {
    let store = get_store();
    let task_id = uuid::Uuid::new_v4().to_string();
    let config = read_hook_server_config();
    let callback_url = format!("http://localhost:{}/agent/submit", config.port);

    let agent_task = AgentTask {
        task_id: task_id.clone(),
        agent_id,
        agent_url,
        task,
        context: context.unwrap_or(json!({})),
        callback_url,
        created_at: chrono::Utc::now().timestamp_millis(),
        status: "dispatched".to_string(),
        result: None,
        run_id: None,
        step_id: None,
    };

    // 存储任务
    {
        let mut tasks = store.tasks.lock().unwrap();
        tasks.insert(task_id.clone(), agent_task.clone());
    }

    // 同步发任务给子 Agent
    let payload = json!({
        "task_id": agent_task.task_id,
        "agent_id": agent_task.agent_id,
        "task": agent_task.task,
        "context": agent_task.context,
        "callback_url": agent_task.callback_url,
    });

    eprintln!(
        "[agent-http] dispatching task {} to {}",
        agent_task.task_id, agent_task.agent_url
    );

    match reqwest::Client::new()
        .post(&agent_task.agent_url)
        .json(&payload)
        .timeout(std::time::Duration::from_secs(120))
        .send()
        .await
    {
        Ok(resp) => {
            eprintln!("[agent-http] agent accepted task (HTTP {})", resp.status());
            Ok(task_id)
        }
        Err(e) => {
            let mut tasks = store.tasks.lock().unwrap();
            if let Some(t) = tasks.get_mut(&task_id) {
                t.status = "dispatch_failed".to_string();
            }
            Err(format!("dispatch failed: {}", e))
        }
    }
}

/// 查看所有任务（测试用）
#[tauri::command]
pub fn list_agent_tasks() -> Vec<AgentTask> {
    let store = get_store();
    let tasks = store.tasks.lock().unwrap();
    tasks.values().cloned().collect()
}

/// 查看所有结果（测试用）
#[tauri::command]
pub fn list_agent_results() -> Vec<AgentResult> {
    let store = get_store();
    let results = store.results.lock().unwrap();
    results.values().cloned().collect()
}

// ── 阶段四 4h：Hook Server 配置管理（Tauri 命令）──────────────────────────────

/// 读取当前 Hook Server 配置
#[tauri::command]
pub fn get_hook_server_config() -> HookServerConfig {
    read_hook_server_config()
}

/// 保存 Hook Server 配置（持久化到 hook_server.json）
#[tauri::command]
pub fn set_hook_server_config(config: HookServerConfig) -> Result<HookServerConfig, String> {
    write_hook_server_config(&config)?;
    Ok(config)
}

/// 重启 Hook Server（先关闭旧 server，再用新配置启动）
#[tauri::command]
pub async fn restart_hook_server() -> Result<String, String> {
    let config = read_hook_server_config();
    if !config.enabled {
        shutdown_current_server().await;
        return Ok("hook server disabled (config.enabled=false)".to_string());
    }

    // 关闭旧 server
    shutdown_current_server().await;

    // 启动新 server
    let store = get_store();
    let port = config.port;
    let token = config.auth_token.clone();
    let handle = tauri::async_runtime::spawn(async move {
        start_agent_http_server(port, token, store).await;
    });

    // 记录新 handle
    let h = get_server_handle();
    {
        let mut guard = h.lock().unwrap();
        *guard = Some(handle);
    }

    Ok(format!("hook server restarted on port {}", config.port))
}
