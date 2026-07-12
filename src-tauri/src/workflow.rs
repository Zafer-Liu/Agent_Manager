use crate::llm::LlmProvider;
use crate::mcp::McpServer;
use crate::mcp_agent::{chat, McpClient, McpTransport};
use crate::workflow_store::{
    FailureKind, FailureTrace, RunRecord, RunStatus, RunTrigger, StepInstance, WorkflowRunStore,
};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tauri::{Emitter, State};
use futures_util::future::join_all;

// ── Data model ───────────────────────────────────────────────────────────────

// ── Verdict / Submission / NodeStatus (new in v0.3.0, phase 1a) ─────────────
//
// 这些类型引入"结构化裁定"概念，替代旧的 `success: bool + error: Option<String>`
// 二态表达。下游节点应消费 `Submission.artifact` 与 `verdict`，不再解析自然语言
// output 字符串。旧字段保留向后兼容，新字段全部 `#[serde(default)]`，旧前端
// 读取 JSON 时自动忽略。

/// 流程裁定：节点执行后的状态判定，决定工作流下一步走向。
///
/// 三态而非 `Result` 的二态：`Blocked` 表达"等待外部输入或人工处理"，
/// 是多 Agent 协作的必要抽象（与 Fail 的"无法继续"语义不同）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Verdict {
    /// 节点成功完成，下游可继续
    Pass,
    /// 节点失败，按 Edge 的 fail 策略处理（终止 / 回跳 / 跳过）
    Fail {
        reason: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        root_cause: Option<String>,
    },
    /// 节点阻塞，等待外部输入或人工处理
    Blocked {
        reason: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        notify: Option<String>,
    },
}

impl Verdict {
    /// 返回裁定的种类字符串（"pass" / "fail" / "blocked"），用于 Edge 条件匹配。
    pub fn kind(&self) -> &'static str {
        match self {
            Verdict::Pass => "pass",
            Verdict::Fail { .. } => "fail",
            Verdict::Blocked { .. } => "blocked",
        }
    }

    pub fn is_pass(&self) -> bool {
        matches!(self, Verdict::Pass)
    }

    pub fn is_fail(&self) -> bool {
        matches!(self, Verdict::Fail { .. })
    }

    pub fn is_blocked(&self) -> bool {
        matches!(self, Verdict::Blocked { .. })
    }
}

impl Default for Verdict {
    fn default() -> Self {
        Verdict::Pass
    }
}

/// 节点状态机：Step 的当前状态。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum NodeStatus {
    /// 未激活（等待上游 Edge 触发）
    Pending,
    /// 执行中
    Running,
    /// 成功（Verdict::Pass）
    Success,
    /// 失败（Verdict::Fail）
    Failed,
    /// 阻塞（Verdict::Blocked）
    Blocked,
    /// 被跳过（条件分支未命中，或返工时上游保留 Skipped）
    Skipped,
}

impl Default for NodeStatus {
    fn default() -> Self {
        NodeStatus::Pending
    }
}

impl NodeStatus {
    /// 从 Verdict 派生终止态节点状态
    pub fn from_verdict(v: &Verdict) -> Self {
        match v {
            Verdict::Pass => NodeStatus::Success,
            Verdict::Fail { .. } => NodeStatus::Failed,
            Verdict::Blocked { .. } => NodeStatus::Blocked,
        }
    }
}

/// Agent 提交的结构化结果。
///
/// 下游节点应只消费 `artifact` 和 `verdict`，不再解析自然语言 output 字符串。
/// `note` 字段仅供人阅读，系统不消费。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Submission {
    /// 业务产物：真正要交付的内容（JSON 值，由 output_contract 约束）
    pub artifact: Value,
    /// 流程裁定：告诉系统下一步往哪走
    pub verdict: Verdict,
    /// 置信度 0.0-1.0，留在链路里供下游参考
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub confidence: Option<f32>,
    /// 自然语言说明（仅供人看，系统不消费）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub note: Option<String>,
}

impl Submission {
    /// 从旧的纯文本 output 构造一个 Pass Submission（向后兼容辅助）。
    /// artifact 用 Value::String 承载，note 保留原文供人阅读。
    pub fn from_text_pass(text: impl Into<String>) -> Self {
        let s = text.into();
        Submission {
            artifact: Value::String(s.clone()),
            verdict: Verdict::Pass,
            confidence: None,
            note: if s.is_empty() { None } else { Some(s) },
        }
    }

    /// 从错误信息构造一个 Fail Submission（向后兼容辅助）。
    pub fn from_error(reason: impl Into<String>) -> Self {
        let r = reason.into();
        Submission {
            artifact: Value::Null,
            verdict: Verdict::Fail {
                root_cause: if r.is_empty() { None } else { Some(r.clone()) },
                reason: r,
            },
            confidence: None,
            note: None,
        }
    }

    /// 从 Submission 提取文本 output（向后兼容辅助：旧前端仍读 output 字段）。
    pub fn to_text(&self) -> String {
        match &self.artifact {
            Value::String(s) => s.clone(),
            Value::Null => String::new(),
            other => other.to_string(),
        }
    }
}

/// A single step in a workflow pipeline.
///
/// `kind == "tool"` → call an MCP tool (`server` + `tool` + `arguments`).
/// `kind == "llm"`  → run an LLM completion using `prompt`.
/// `kind == "mcp_agent"` → run MCP server as a mini agent with the LLM.
/// `kind == "acceptance"` → 阶段二：到达此节点时 Run 进入 WaitingAcceptance，
///                          等待人通过或驳回（定向 Rework）。
///
/// In both cases the previous node's text output is available to the node:
/// for tool nodes, the placeholder `{{input}}` inside any string argument is
/// replaced with the previous output; for llm nodes, the previous output is
/// appended to the prompt as the user message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowNode {
    pub id: String,
    pub kind: String, // "tool" | "llm" | "mcp_agent" | "acceptance"
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
    /// 阶段二：Acceptance 节点配置。仅 kind == "acceptance" 时使用。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub acceptance: Option<AcceptanceConfig>,
    /// 阶段二：输出契约（JSON Schema 子集，可选）。
    /// 执行后校验 artifact 是否满足，不满足则 Verdict::Fail。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output_contract: Option<Value>,
    /// 阶段四 P1：Fan-out 并行配置。仅 kind == "fan_out" 时使用。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fan_out: Option<FanOutConfig>,
    /// 阶段四 P2：Agent 调度策略。仅 kind == "agent_task" 时使用。
    /// 未配置时使用 Fixed 策略（向后兼容 server 字段）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub dispatch: Option<DispatchConfig>,
}

/// 阶段二：Acceptance 节点配置。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AcceptanceConfig {
    /// 通知谁（用户标识列表，预留阶段四外部通知扩展）
    #[serde(default)]
    pub notify: Vec<String>,
    /// 允许驳回回跳到的 node_id 列表（空 = 允许回跳到任意已执行节点）
    #[serde(default)]
    pub allow_reject_to: Vec<String>,
    /// 超时秒数（None = 不超时）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub timeout_secs: Option<u64>,
    /// 超时动作
    #[serde(default)]
    pub timeout_action: TimeoutAction,
}

impl Default for AcceptanceConfig {
    fn default() -> Self {
        AcceptanceConfig {
            notify: vec![],
            allow_reject_to: vec![],
            timeout_secs: None,
            timeout_action: TimeoutAction::Remind,
        }
    }
}

/// 阶段二：Acceptance 超时动作。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum TimeoutAction {
    /// 超时自动通过
    AutoPass,
    /// 超时自动驳回
    AutoReject,
    /// 超时仅提醒（默认）
    Remind,
}

impl Default for TimeoutAction {
    fn default() -> Self {
        TimeoutAction::Remind
    }
}

// ── Fan-out 配置（阶段四 P1）──────────────────────────────────────────────────

/// 拆分策略
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum SplitStrategy {
    /// 按字段拆分：从 input JSON 的指定字段取数组，每个元素成为一个子任务
    ByField { field: String },
    /// 静态拆分：直接指定子任务列表
    Static { items: Vec<String> },
    /// LLM 拆分：用 LLM 把 input 拆成 N 个子任务
    LlmSplit { count: usize },
}

/// 收敛策略
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ConvergeStrategy {
    /// 全部成功才算成功
    #[default]
    And,
    /// 任一成功即成功
    Or,
}

/// 子任务失败策略
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "lowercase")]
pub enum ChildFailPolicy {
    /// 子任务失败 → 父节点失败
    #[default]
    FailParent,
    /// 子任务失败 → 忽略，继续收敛
    Continue,
    /// 子任务失败 → 取消其他子任务
    CancelSiblings,
}

/// Fan-out 节点配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FanOutConfig {
    pub split: SplitStrategy,
    /// 子任务复用的节点 id（执行时以该节点为模板）
    pub child_node_id: String,
    #[serde(default)]
    pub converge: ConvergeStrategy,
    #[serde(default)]
    pub on_child_fail: ChildFailPolicy,
}

// ── DispatchStrategy 调度策略（阶段四 P2）──────────────────────────────────────

/// 阶段四 P1：fan_out 子任务执行结果（供主循环记录为独立 StepInstance）。
struct FanOutChildResult {
    /// 子任务序号（0-based）
    index: usize,
    /// 子任务输入
    input: String,
    /// 子任务输出
    output: String,
    /// 子任务错误（有则失败）
    error: Option<String>,
    /// 子任务执行耗时（ms）
    started_at: i64,
    finished_at: i64,
}

/// 候选 Agent 定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentCandidate {
    /// Agent 标识（可以是本地 agent_id 或远程 URL 别名）
    pub id: String,
    /// Agent HTTP 端点 URL（如 http://localhost:8502/task）
    pub url: String,
    /// 能力标签列表（用于 CapabilityMatch 策略匹配）
    #[serde(default)]
    pub capabilities: Vec<String>,
    /// 优先级（数字越小优先级越高，默认 100）
    #[serde(default = "default_priority")]
    pub priority: u32,
}

fn default_priority() -> u32 {
    100
}

/// 调度策略
#[derive(Debug, Clone, Serialize, Deserialize, Default, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum DispatchStrategy {
    /// 固定使用 server 字段指定的单一 Agent（默认行为，向后兼容）
    #[default]
    Fixed,
    /// 按优先级依次尝试候选 Agent，前一个失败则轮到下一个
    Failover,
    /// 按能力标签匹配：从候选中选 capabilities 包含所需能力的 Agent
    CapabilityMatch,
    /// 随机负载均衡（从候选中随机选一个）
    Random,
}

/// agent_task 节点的调度配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DispatchConfig {
    #[serde(default)]
    pub strategy: DispatchStrategy,
    /// 候选 Agent 列表（Failover / CapabilityMatch / Random 策略使用）
    #[serde(default)]
    pub candidates: Vec<AgentCandidate>,
    /// CapabilityMatch 策略下需要匹配的能力标签
    #[serde(default)]
    pub required_capabilities: Vec<String>,
    /// 单个候选 dispatch 超时秒数（默认 10s）
    #[serde(default = "default_dispatch_timeout")]
    pub timeout_secs: u64,
}

fn default_dispatch_timeout() -> u64 {
    10
}

// ── Edge 模型（阶段 1c）──────────────────────────────────────────────────────
//
// 显式 Edge 替代旧的"数组下标隐式串联"。支持多入多出、条件分支、并行收敛
// （并行/收敛在阶段四启用，1c 先建立数据结构）。旧 Workflow { nodes: Vec }
// 无 edges 字段，加载时自动派生线性 Edge 链，零破坏性。

/// Edge 条件：上游完成后，按何种条件触发下游。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(tag = "type", rename_all = "lowercase")]
pub enum EdgeCondition {
    /// 无条件：上游 Success 即触发
    Always,
    /// 上游 Verdict 命中指定种类时触发
    OnVerdict { verdict: VerdictKind },
    /// 上游 artifact 字段满足表达式（阶段二扩展）
    #[serde(rename = "on_expression")]
    OnExpression {
        field: String,
        op: String,
        value: Value,
    },
}

/// Verdict 种类（用于 Edge 条件匹配，不含 reason 等 detail）。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum VerdictKind {
    Pass,
    Fail,
    Blocked,
}

impl From<&Verdict> for VerdictKind {
    fn from(v: &Verdict) -> Self {
        match v.kind() {
            "pass" => VerdictKind::Pass,
            "fail" => VerdictKind::Fail,
            "blocked" => VerdictKind::Blocked,
            _ => VerdictKind::Fail,
        }
    }
}

/// 节点间关系：从 `from` 到 `to`，附带触发条件。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkflowEdge {
    pub from: String,
    pub to: String,
    #[serde(default = "default_edge_condition")]
    pub condition: EdgeCondition,
}

fn default_edge_condition() -> EdgeCondition {
    EdgeCondition::OnVerdict {
        verdict: VerdictKind::Pass,
    }
}

impl WorkflowEdge {
    /// 判断给定 Verdict 是否满足此 Edge 的触发条件。
    pub fn matches_verdict(&self, verdict: &Verdict) -> bool {
        match &self.condition {
            EdgeCondition::Always => true,
            EdgeCondition::OnVerdict { verdict: kind } => {
                let actual: VerdictKind = verdict.into();
                kind == &actual
            }
            EdgeCondition::OnExpression { .. } => {
                // 阶二启用，当前默认不匹配
                false
            }
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Workflow {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub description: String,
    #[serde(default)]
    pub nodes: Vec<WorkflowNode>,
    #[serde(default)]
    pub edges: Vec<WorkflowEdge>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub entry_node_id: Option<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub end_node_ids: Vec<String>,
    /// 阶段四：外部系统通过此 key 匹配模板
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_key: Option<String>,
    /// 阶段四：Run 终态时回调此 URL
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callback_url: Option<String>,
    /// 阶段四：允许哪些来源触发（空 = 允许所有）
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_sources: Vec<String>,
    /// 阶段四 P2：定时触发 cron 表达式（如 "0 */5 * * * *" 每 5 分钟）。
    /// 为 None 时不参与定时调度。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub schedule: Option<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

impl Workflow {
    /// 旧模板迁移：若 edges 为空且有 nodes，自动派生线性 Edge 链。
    ///
    /// 线性链：nodes[0] → nodes[1] → ... → nodes[n-1]
    /// 每条 Edge 的条件为 `OnVerdict { verdict: Pass }`（上游成功才推进）。
    /// entry_node_id 取 nodes[0].id。
    /// end_node_ids 取 acceptance 节点 + 无下游 Edge 的节点。
    pub fn migrate_legacy(&mut self) {
        if self.edges.is_empty() && !self.nodes.is_empty() {
            self.edges = self
                .nodes
                .windows(2)
                .map(|w| WorkflowEdge {
                    from: w[0].id.clone(),
                    to: w[1].id.clone(),
                    condition: EdgeCondition::OnVerdict {
                        verdict: VerdictKind::Pass,
                    },
                })
                .collect();
        }
        if self.entry_node_id.is_none() && !self.nodes.is_empty() {
            self.entry_node_id = Some(self.nodes[0].id.clone());
        }
        // 阶段二：迁移 end_node_ids
        if self.end_node_ids.is_empty() && !self.nodes.is_empty() {
            let has_outgoing: std::collections::HashSet<&str> =
                self.edges.iter().map(|e| e.from.as_str()).collect();
            for n in &self.nodes {
                // acceptance 节点必为 end node
                if n.kind == "acceptance" {
                    self.end_node_ids.push(n.id.clone());
                } else if !has_outgoing.contains(n.id.as_str()) {
                    // 无下游 Edge 的节点为 end node
                    self.end_node_ids.push(n.id.clone());
                }
            }
        }
    }

    /// 获取入口节点 id（优先 entry_node_id，回退 nodes[0].id）。
    pub fn entry_node(&self) -> Option<&WorkflowNode> {
        let entry_id = self.entry_node_id.as_deref()?;
        self.nodes.iter().find(|n| n.id == entry_id)
    }

    /// 获取从指定节点出发、匹配给定 Verdict 的所有目标节点 id。
    pub fn next_nodes(&self, from_id: &str, verdict: &Verdict) -> Vec<String> {
        self.edges
            .iter()
            .filter(|e| e.from == from_id && e.matches_verdict(verdict))
            .map(|e| e.to.clone())
            .collect()
    }
}

// ── Config file helpers ──────────────────────────────────────────────────────

fn workflows_path() -> std::path::PathBuf {
    dirs_next::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("agent-manager")
        .join("agent_manager_workflows.json")
}

pub fn read_workflows() -> Vec<Workflow> {
    let path = workflows_path();
    eprintln!("[workflow] reading from: {:?}", path);
    let raw = std::fs::read_to_string(&path).unwrap_or_default();
    eprintln!("[workflow] raw length: {} bytes", raw.len());
    let mut list: Vec<Workflow> = serde_json::from_str::<Vec<Workflow>>(&raw).unwrap_or_else(|e| {
        eprintln!("[workflow] parse error: {}", e);
        vec![]
    });
    eprintln!("[workflow] loaded {} workflows", list.len());
    for wf in &list {
        eprintln!("[workflow]   id={} name={} template_key={:?}", wf.id, wf.name, wf.template_key);
    }
    // 自动迁移旧模板：无 edges 字段时派生线性 Edge 链
    for wf in &mut list {
        wf.migrate_legacy();
    }
    list
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
    // 确保保存的模板有 edges（旧前端可能未传 edges）
    wf.migrate_legacy();
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
    /// 旧字段：纯文本输出。保留向后兼容，新代码应使用 `submission.artifact`。
    pub output: String,
    pub error: Option<String>,
    /// 新字段：结构化提交（artifact + verdict + confidence + note）。
    /// 旧前端读取 JSON 时自动忽略；新前端优先消费此字段。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submission: Option<Submission>,
    /// 新字段：节点状态机状态（pending/running/success/failed/blocked/skipped）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<NodeStatus>,
    /// 新字段：第几次尝试（返工累加，阶段二启用）。0 = 未设置。
    #[serde(default, skip_serializing_if = "is_zero_u32")]
    pub attempt: u32,
    /// 阶段三：失败诊断链路。旧前端会忽略此字段。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_trace: Option<FailureTrace>,
}

fn is_zero_u32(v: &u32) -> bool {
    *v == 0
}

impl WorkflowStepResult {
    /// 构造一个成功步骤（同时填充 output 和 submission，向后兼容）。
    pub fn success(
        node_id: impl Into<String>,
        label: impl Into<String>,
        kind: impl Into<String>,
        output: impl Into<String>,
    ) -> Self {
        let out = output.into();
        let submission = Submission::from_text_pass(&out);
        WorkflowStepResult {
            node_id: node_id.into(),
            label: label.into(),
            kind: kind.into(),
            output: out,
            error: None,
            submission: Some(submission),
            status: Some(NodeStatus::Success),
            attempt: 1,
            failure_trace: None,
        }
    }

    /// 构造一个失败步骤（同时填充 error 和 submission.verdict=Fail）。
    pub fn failure(
        node_id: impl Into<String>,
        label: impl Into<String>,
        kind: impl Into<String>,
        error: impl Into<String>,
    ) -> Self {
        let err = error.into();
        let submission = Submission::from_error(&err);
        WorkflowStepResult {
            node_id: node_id.into(),
            label: label.into(),
            kind: kind.into(),
            output: String::new(),
            error: Some(err),
            submission: Some(submission),
            status: Some(NodeStatus::Failed),
            attempt: 1,
            failure_trace: None,
        }
    }
}

/// 过渡用 Default：让 `run_workflow_core` 旧构造点可用 `..Default::default()`
/// 补全新字段。1d 重构后这些构造点会被 `success()` / `failure()` 取代。
impl Default for WorkflowStepResult {
    fn default() -> Self {
        WorkflowStepResult {
            node_id: String::new(),
            label: String::new(),
            kind: String::new(),
            output: String::new(),
            error: None,
            submission: None,
            status: None,
            attempt: 0,
            failure_trace: None,
        }
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct RunWorkflowRequest {
    pub workflow: Workflow,
    pub provider: Option<LlmProvider>,
    pub mcp_servers: Vec<McpServer>,
    /// Optional initial input fed to the first node.
    #[serde(default)]
    pub input: String,
    /// 阶段二：返工上下文。传入时从 rework.reject_to_node 开始执行而非 entry_node_id。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rework: Option<ReworkRequest>,
    /// 阶段四：本次 Run 的回调 URL（优先于 workflow.callback_url）。
    /// Run 进入终态（Success/Failed/WaitingAcceptance/Closed）时 POST 通知。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub callback_url: Option<String>,
    /// 阶段四 P2：外部指定的触发来源（定时调度用）。
    /// 为 None 时默认 Manual（或 rework 时 Rework）。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub trigger: Option<crate::workflow_store::RunTrigger>,
}

/// 阶段二：返工请求（前端 reject_run 后调用 run_workflow_stream 时传入）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReworkRequest {
    /// 原 run_id
    pub parent_run_id: String,
    /// 回跳目标 node_id
    pub reject_to_node: String,
    /// 驳回原因
    pub reason: String,
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
pub async fn run_workflow(
    store: State<'_, WorkflowRunStore>,
    request: RunWorkflowRequest,
) -> Result<RunWorkflowResult, String> {
    match run_workflow_core(request, None, &store).await {
        Ok(r) => Ok(r),
        Err(e) => Ok(RunWorkflowResult {
            steps: vec![],
            final_output: String::new(),
            success: false,
            error: Some(e),
        }),
    }
}

/// Streaming variant: emits one `workflow-step` event per completed step,
/// then a final `workflow-done` event.  The front-end can update the UI
/// incrementally instead of waiting for the whole pipeline to finish.
#[tauri::command]
pub async fn run_workflow_stream(
    window: tauri::Window,
    store: State<'_, WorkflowRunStore>,
    request: RunWorkflowRequest,
) -> Result<RunWorkflowResult, String> {
    match run_workflow_core(request, Some(window), &store).await {
        Ok(r) => Ok(r),
        Err(e) => Ok(RunWorkflowResult {
            steps: vec![],
            final_output: String::new(),
            success: false,
            error: Some(e),
        }),
    }
}

/// 节点执行结果：返回 output、可选 error、以及是否为"硬失败"（终止流程）。
struct NodeOutcome {
    output: String,
    error: Option<String>,
    /// true = 硬失败（tool/llm 失败，旧行为终止全流程）
    /// false = 软失败（mcp_agent 失败，旧行为把 err_msg 当 output 继续）
    hard_fail: bool,
    /// 阶段二：显式 Verdict（None = 由 hard_fail/error 推断）。
    /// acceptance 节点用此字段返回 Blocked。
    explicit_verdict: Option<Verdict>,
    /// 阶段三：失败分类。只在失败或软失败时填充，用于构造 FailureTrace。
    failure_kind: Option<FailureKind>,
    /// 阶段三：stderr 摘要。3b 会接入 MCP stderr ring buffer。
    stderr_excerpt: Option<String>,
}

impl Default for NodeOutcome {
    fn default() -> Self {
        NodeOutcome {
            output: String::new(),
            error: None,
            hard_fail: false,
            explicit_verdict: None,
            failure_kind: None,
            stderr_excerpt: None,
        }
    }
}

fn node_status_label(status: &NodeStatus) -> &'static str {
    match status {
        NodeStatus::Pending => "pending",
        NodeStatus::Running => "running",
        NodeStatus::Success => "success",
        NodeStatus::Failed => "failed",
        NodeStatus::Blocked => "blocked",
        NodeStatus::Skipped => "skipped",
    }
}

fn build_failure_trace(
    run_id: &str,
    step_id: &str,
    node: &WorkflowNode,
    outcome: &NodeOutcome,
    final_status: &NodeStatus,
) -> Option<FailureTrace> {
    let kind = outcome.failure_kind.clone()?;
    let reason = outcome
        .error
        .clone()
        .or_else(|| {
            outcome.explicit_verdict.as_ref().and_then(|v| match v {
                Verdict::Fail { reason, .. } | Verdict::Blocked { reason, .. } => {
                    Some(reason.clone())
                }
                Verdict::Pass => None,
            })
        })
        .unwrap_or_else(|| "Workflow step failed without detailed reason".to_string());

    Some(FailureTrace {
        run_id: run_id.to_string(),
        step_id: step_id.to_string(),
        node_id: node.id.clone(),
        agent_id: if node.kind == "mcp_agent" && !node.server.is_empty() {
            Some(node.server.clone())
        } else {
            None
        },
        tool: if node.kind == "tool" && !node.tool.is_empty() {
            Some(node.tool.clone())
        } else {
            None
        },
        failure_kind: kind,
        reason,
        stderr_excerpt: outcome.stderr_excerpt.clone(),
        retry_history: vec![],
        final_status: node_status_label(final_status).to_string(),
    })
}

/// 重构后的工作流核心：基于 Edge 的节点调度器（阶段 1d）。
///
/// 关键变化：
/// - `for node in &workflow.nodes` → `while ready.pop_front()` 按 Edge 推进
/// - 每次 run 生成 run_id 并持久化 RunRecord
/// - 节点失败时检查 OnVerdict(Fail) Edge，有则继续，无则终止
/// - 新增 workflow-run-started / workflow-run-finished 事件（带 run_id）
/// - 旧 workflow-step / workflow-done 事件保留兼容
pub async fn run_workflow_core(
    request: RunWorkflowRequest,
    window: Option<tauri::Window>,
    store: &WorkflowRunStore,
) -> Result<RunWorkflowResult, String> {
    let RunWorkflowRequest {
        workflow,
        provider,
        mcp_servers,
        input,
        rework,
        callback_url,
        trigger: external_trigger,
    } = request;

    // 迁移旧模板（确保有 edges 和 entry_node_id）
    let mut wf = workflow;
    wf.migrate_legacy();

    if wf.nodes.is_empty() {
        return Err("Workflow has no nodes".to_string());
    }

    // 阶段二：确定入口节点（rework 时从 reject_to_node 开始）
    let (entry_node, trigger, rework_context) = if let Some(rw) = rework {
        let entry = rw.reject_to_node.clone();
        let ctx = crate::workflow_store::ReworkContext {
            reject_from_step: String::new(),
            reject_to_node: rw.reject_to_node.clone(),
            reason: rw.reason.clone(),
            previous_run_id: rw.parent_run_id.clone(),
            carry_over_artifact: Value::Null,
        };
        (
            entry,
            RunTrigger::Rework {
                parent_run_id: rw.parent_run_id,
            },
            Some(ctx),
        )
    } else {
        let entry = wf.entry_node().map(|n| n.id.clone()).unwrap_or_default();
        let trigger = external_trigger.unwrap_or_else(|| RunTrigger::Manual {
            user: String::new(),
        });
        (entry, trigger, None)
    };

    // ── 生成 run_id 并创建 RunRecord ──────────────────────────────────────────
    let run_id = uuid::Uuid::new_v4().to_string();
    let now = chrono::Utc::now().timestamp_millis();
    let mut run = RunRecord {
        run_id: run_id.clone(),
        template_id: wf.id.clone(),
        template_key: wf.template_key.clone(),
        status: RunStatus::Running,
        steps: vec![],
        created_at: now,
        finished_at: None,
        trigger,
        rework_context,
    };
    let _ = store.create_run(&run);

    // 阶段三 3d：JSONL 事件源 — run_started
    crate::workflow_events::emit_run_started(&run_id, &wf.id, now);

    // 发送 run-started 事件
    if let Some(ref w) = &window {
        let _ = w.emit(
            "workflow-run-started",
            json!({
                "run_id": run_id,
                "template_id": wf.id,
                "created_at": now,
            }),
        );
    }

    // ── Edge 调度器主循环 ──────────────────────────────────────────────────────
    let mut steps: Vec<WorkflowStepResult> = vec![];
    let mut current_input = input;
    let mut ready: VecDeque<String> = VecDeque::new();

    // 入口节点（阶段二：rework 时从 reject_to_node 开始）
    if !entry_node.is_empty() {
        ready.push_back(entry_node.clone());
    } else if let Some(first) = wf.nodes.first() {
        ready.push_back(first.id.clone());
    }

    let mut run_blocked = false;

    while let Some(node_id) = ready.pop_front() {
        let node = match wf.nodes.iter().find(|n| n.id == node_id) {
            Some(n) => n.clone(),
            None => continue,
        };

        let label = if node.label.is_empty() {
            match node.kind.as_str() {
                "tool" => format!("{}__{}", node.server, node.tool),
                _ => "llm".to_string(),
            }
        } else {
            node.label.clone()
        };

        let started_at = chrono::Utc::now().timestamp_millis();

        // 阶段三 3c：执行前先持久化 Running Step（Sweeper 可巡检）
        let pre_step = StepInstance::new(&run_id, &node.id).with_kind(&node.kind);
        let step_id = pre_step.step_id.clone();
        let mut pre_step = pre_step;
        pre_step.mark_running();
        pre_step.started_at = Some(started_at);
        let _ = store.add_running_step(&run_id, &pre_step);

        // 阶段三 3d：JSONL 事件源 — step_started
        crate::workflow_events::emit_step_started(&run_id, &step_id, &node.id, &node.kind);

        // ── 执行节点 ──────────────────────────────────────────────────────────
        let (mut outcome, fan_out_children) = execute_node(&node, &mut current_input, &provider, &mcp_servers, &wf).await;

        let finished_at = chrono::Utc::now().timestamp_millis();

        // 阶段二：output_contract 校验（仅对成功且无显式 verdict 的节点）
        if outcome.explicit_verdict.is_none() && outcome.error.is_none() {
            if let Some(ref contract) = node.output_contract {
                let artifact = Value::String(outcome.output.clone());
                if let Err(violation) = validate_output_contract(&artifact, contract) {
                    let reason = format!("schema violation: {}", violation);
                    outcome = NodeOutcome {
                        output: outcome.output.clone(),
                        error: Some(reason.clone()),
                        hard_fail: true,
                        explicit_verdict: Some(Verdict::Fail {
                            reason,
                            root_cause: Some(violation),
                        }),
                        failure_kind: Some(FailureKind::SchemaViolation),
                        stderr_excerpt: None,
                    };
                }
            }
        }

        // ── 构造 StepResult / StepInstance（保持旧字段 + 新 submission 双写） ──
        let mut step = build_step_result(&node.id, &label, &node.kind, &outcome);
        // 阶段三 3c：复用执行前持久化的 step_id（而非新建）
        let mut step_inst = StepInstance::new(&run_id, &node.id).with_kind(&node.kind);
        step_inst.step_id = step_id.clone(); // 复用执行前的 step_id
        step_inst.started_at = Some(started_at);
        step_inst.finished_at = Some(finished_at);
        if let Some(ref sub) = step.submission {
            step_inst.finish_with_verdict(sub.clone());
        }
        if let Some(trace) = build_failure_trace(
            &run_id,
            &step_inst.step_id,
            &node,
            &outcome,
            &step_inst.status,
        ) {
            step.failure_trace = Some(trace.clone());
            step_inst.failure_trace = Some(trace);
        }

        // 旧 workflow-step 事件（保留兼容，增加 run_id/step_id/failure_trace 字段）
        if let Some(ref w) = &window {
            let _ = w.emit(
                "workflow-step",
                json!({
                    "node_id": step.node_id,
                    "label": step.label,
                    "kind": step.kind,
                    "output": step.output,
                    "error": step.error,
                    "run_id": run_id,
                    "step_id": step_inst.step_id,
                    "submission": step.submission,
                    "status": step.status,
                    "attempt": step.attempt,
                    "failure_trace": step.failure_trace,
                }),
            );
        }

        steps.push(step.clone());
        // 阶段三 3c：更新已持久化的 Running Step 为终态（而非 push 新 step）
        let _ = store.update_step(&run_id, &step_inst);
        // 阶段三 3d：JSONL 事件源 — step_finished
        crate::workflow_events::emit_step_finished(
            &run_id,
            &step_inst.step_id,
            &node.id,
            node_status_label(&step_inst.status),
            finished_at - started_at,
            step_inst.failure_trace.is_some(),
        );
        // 同步内存中的 run.steps：替换已存在的 Running step 或追加
        if let Some(existing) = run
            .steps
            .iter_mut()
            .find(|s| s.step_id == step_inst.step_id)
        {
            *existing = step_inst.clone();
        } else {
            run.steps.push(step_inst);
        }

        // ── fan_out 子任务记录为独立 StepInstance ──────────────────────────────
        for child in &fan_out_children {
            let child_node_id = format!("{}#{}", node.id, child.index);
            let child_label = format!("{} [{}]", label, child.index + 1);
            let child_kind = if let Some(cfg) = &node.fan_out {
                wf.nodes
                    .iter()
                    .find(|n| n.id == cfg.child_node_id)
                    .map(|n| n.kind.as_str())
                    .unwrap_or("fan_out_child")
                    .to_string()
            } else {
                "fan_out_child".to_string()
            };

            let child_outcome = NodeOutcome {
                output: format!("input: {}\noutput: {}", child.input, child.output),
                error: child.error.clone(),
                hard_fail: child.error.is_some(),
                explicit_verdict: child.error.as_ref().map(|_| Verdict::Fail {
                    reason: "child task failed".into(),
                    root_cause: None,
                }),
                failure_kind: child.error.as_ref().map(|_| FailureKind::Unknown),
                stderr_excerpt: None,
            };
            let child_step = build_step_result(&child_node_id, &child_label, &child_kind, &child_outcome);
            let mut child_inst = StepInstance::new(&run_id, &child_node_id).with_kind(&child_kind);
            child_inst.started_at = Some(child.started_at);
            child_inst.finished_at = Some(child.finished_at);
            if let Some(ref sub) = child_step.submission {
                child_inst.finish_with_verdict(sub.clone());
            }

            if let Some(ref w) = &window {
                let _ = w.emit(
                    "workflow-step",
                    json!({
                        "node_id": child_step.node_id,
                        "label": child_step.label,
                        "kind": child_step.kind,
                        "output": child_step.output,
                        "error": child_step.error,
                        "run_id": run_id,
                        "step_id": child_inst.step_id,
                        "submission": child_step.submission,
                        "status": child_step.status,
                        "attempt": child_step.attempt,
                        "is_fan_out_child": true,
                        "parent_node_id": node.id,
                    }),
                );
            }

            steps.push(child_step);
            run.steps.push(child_inst);
        }

        // ── 按 Verdict 决定下一步 ─────────────────────────────────────────────
        let verdict = step
            .submission
            .as_ref()
            .map(|s| &s.verdict)
            .unwrap_or(&Verdict::Pass);

        // 阶段二：acceptance 节点的 Blocked → 进入 WaitingAcceptance
        if verdict.is_blocked() && node.kind == "acceptance" {
            run_blocked = true;
            // 发送 acceptance-requested 事件
            if let Some(ref w) = &window {
                let allow_reject_to = node
                    .acceptance
                    .as_ref()
                    .map(|c| c.allow_reject_to.clone())
                    .unwrap_or_default();
                let _ = w.emit(
                    "workflow-acceptance-requested",
                    json!({
                        "run_id": run_id,
                        "node_id": node.id,
                        "label": label,
                        "allow_reject_to": allow_reject_to,
                        "executed_node_ids": run.steps.iter().map(|s| s.node_id.clone()).collect::<Vec<_>>(),
                    }),
                );
                // 阶段三 3d：JSONL 事件源 — acceptance_requested
                crate::workflow_events::emit_acceptance_requested(&run_id, &step_id, &node.id);
            }
            break;
        }

        let next_nodes = wf.next_nodes(&node_id, verdict);

        if next_nodes.is_empty() {
            // 无匹配 Edge
            if verdict.is_pass() {
                // Pass 且无下游 → 自然结束
            } else if verdict.is_fail() {
                // 硬失败且无 fail 分支 → 终止（保持旧行为）
                break;
            } else if verdict.is_blocked() {
                run_blocked = true;
                break;
            }
        } else {
            for n in next_nodes {
                ready.push_back(n);
            }
        }
    }

    // ── 确定 Run 最终状态 ─────────────────────────────────────────────────────
    let has_fail = steps
        .iter()
        .any(|s| s.error.is_some() && outcome_is_hard_fail(&s));
    if run_blocked {
        // 阶段二：如果有 acceptance 节点在等待，状态为 WaitingAcceptance 而非 Blocked
        let has_acceptance_waiting = steps.iter().any(|s| {
            s.submission
                .as_ref()
                .map(|sub| sub.verdict.is_blocked())
                .unwrap_or(false)
        });
        run.status = if has_acceptance_waiting {
            RunStatus::WaitingAcceptance
        } else {
            RunStatus::Blocked
        };
    } else if has_fail {
        run.status = RunStatus::Failed;
    } else {
        run.status = RunStatus::Success;
    }
    run.finished_at = Some(chrono::Utc::now().timestamp_millis());
    let _ = store.update_run(&run);

    // ── 阶段四：Callback 出站通知 ─────────────────────────────────────────────
    // Run 进入终态时，如果配置了 callback_url（payload 优先于模板），异步 POST 通知外部系统。
    // 失败仅记日志，不阻塞 Run。
    let cb_url = callback_url.or(wf.callback_url.clone());
    if let Some(url) = cb_url {
        let cb_payload = json!({
            "run_id": run_id,
            "template_id": wf.id,
            "template_key": wf.template_key,
            "status": run.status,
            "created_at": run.created_at,
            "finished_at": run.finished_at,
            "steps_summary": run.steps.iter().map(|s| json!({
                "node_id": s.node_id,
                "status": s.status,
            })).collect::<Vec<_>>(),
        });
        tauri::async_runtime::spawn(async move {
            // 指数退避重试：最多 3 次尝试，间隔 2s → 4s → 8s
            let max_retries = 3u32;
            let client = reqwest::Client::new();
            for attempt in 0..max_retries {
                match client
                    .post(&url)
                    .json(&cb_payload)
                    .timeout(std::time::Duration::from_secs(10))
                    .send()
                    .await
                {
                    Ok(resp) => {
                        let status = resp.status();
                        eprintln!(
                            "[callback] POST {} → {} (status {})",
                            url,
                            status,
                            status.as_u16()
                        );
                        // 2xx 视为成功，不再重试
                        if status.is_success() {
                            return;
                        }
                        // 4xx 客户端错误，重试无意义
                        if status.is_client_error() {
                            eprintln!(
                                "[callback] {} 客户端错误 {}，不再重试",
                                url,
                                status.as_u16()
                            );
                            return;
                        }
                    }
                    Err(e) => {
                        eprintln!(
                            "[callback] POST {} 尝试 {}/{} 失败: {}",
                            url,
                            attempt + 1,
                            max_retries,
                            e
                        );
                    }
                }
                // 最后一次不再等待
                if attempt < max_retries - 1 {
                    let backoff = std::time::Duration::from_secs(2u64.pow(attempt + 1));
                    eprintln!(
                        "[callback] {} 等待 {}s 后重试",
                        url,
                        backoff.as_secs()
                    );
                    tokio::time::sleep(backoff).await;
                }
            }
            eprintln!("[callback] POST {} 全部 {} 次重试耗尽", url, max_retries);
        });
    }

    // ── 构造 final output（保留旧的 summariser 逻辑） ─────────────────────────
    let final_output = if steps.len() > 1 && !has_fail {
        if let Some(ref p) = provider {
            let step_summary = steps
                .iter()
                .enumerate()
                .map(|(i, s)| {
                    format!(
                        "Step {} ({}): {}",
                        i + 1,
                        s.label,
                        if s.output.is_empty() {
                            s.error.as_deref().unwrap_or("(no output)")
                        } else {
                            &s.output
                        }
                    )
                })
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
                        .as_str()
                        .unwrap_or("")
                        .to_string();
                    let cleaned = strip_thinking(&raw);
                    if cleaned.is_empty() {
                        current_input
                    } else {
                        cleaned
                    }
                }
                Err(_) => current_input,
            }
        } else {
            current_input
        }
    } else {
        current_input
    };

    let success = !has_fail && !run_blocked;
    let error = if success {
        None
    } else if run_blocked {
        Some("Workflow blocked".to_string())
    } else {
        // 取第一个失败的 error
        steps
            .iter()
            .find(|s| s.error.is_some())
            .and_then(|s| s.error.clone())
            .or_else(|| Some("Workflow failed".to_string()))
    };

    let result = RunWorkflowResult {
        steps,
        final_output,
        success,
        error,
    };

    // ── 发送结束事件 ───────────────────────────────────────────────────────────
    // 阶段三 3d：JSONL 事件源 — run_finished
    let run_duration_ms = run.finished_at.unwrap_or(now) - now;
    crate::workflow_events::emit_run_finished(
        &run_id,
        match &run.status {
            RunStatus::Success => "success",
            RunStatus::Failed => "failed",
            RunStatus::Blocked => "blocked",
            RunStatus::WaitingAcceptance => "waiting_acceptance",
            RunStatus::Closed => "closed",
            RunStatus::Running => "running",
        },
        run_duration_ms,
        run.steps.len(),
    );

    if let Some(ref w) = &window {
        let _ = w.emit(
            "workflow-run-finished",
            json!({
                "run_id": run_id,
                "status": run.status,
                "success": success,
                "finished_at": run.finished_at,
            }),
        );
        // 旧 workflow-done 事件保留兼容
        let _ = w.emit("workflow-done", &result);
    }

    Ok(result)
}

/// 执行单个节点，返回 NodeOutcome。
///
/// - tool/llm 失败 → hard_fail=true（旧行为：终止全流程）
/// - mcp_agent 失败 → hard_fail=false（旧行为：把 err_msg 当 output 继续）
async fn execute_node(
    node: &WorkflowNode,
    current_input: &mut String,
    provider: &Option<LlmProvider>,
    mcp_servers: &[McpServer],
    wf: &Workflow,
) -> (NodeOutcome, Vec<FanOutChildResult>) {
    match node.kind.as_str() {
        "tool" => (execute_tool_node(node, current_input, mcp_servers).await, vec![]),
        "llm" => (execute_llm_node(node, current_input, provider).await, vec![]),
        "mcp_agent" => (execute_mcp_agent_node(node, current_input, provider, mcp_servers).await, vec![]),
        "acceptance" => (execute_acceptance_node(node).await, vec![]),
        "agent_task" => (execute_agent_task_node(node, current_input).await, vec![]),
        "fan_out" => execute_fan_out_node(node, current_input, wf, provider, mcp_servers).await,
        _ => (
            NodeOutcome {
                output: String::new(),
                error: Some(format!("Unknown node kind: {}", node.kind)),
                hard_fail: true,
                explicit_verdict: None,
                failure_kind: Some(FailureKind::Unknown),
                stderr_excerpt: None,
            },
            vec![],
        ),
    }
}

/// 阶段四 P1：执行 fan_out 节点 —— 拆分子任务并行执行，按 converge 策略收敛。
///
/// 流程：
/// 1. 按 split 策略把 current_input 拆成 N 个子任务输入
/// 2. 找到 child_node_id 对应的节点模板
/// 3. 并行执行所有子任务（join_all）
/// 4. 按 converge 策略 + on_child_fail 策略决定父节点 outcome
///
/// 返回 (父节点 outcome, 子任务明细列表) —— 子任务明细供主循环记录为独立 StepInstance。
async fn execute_fan_out_node(
    node: &WorkflowNode,
    current_input: &mut String,
    wf: &Workflow,
    provider: &Option<LlmProvider>,
    mcp_servers: &[McpServer],
) -> (NodeOutcome, Vec<FanOutChildResult>) {
    let config = match node.fan_out.as_ref() {
        Some(c) => c,
        None => {
            return (
                NodeOutcome {
                    output: String::new(),
                    error: Some("fan_out node missing fan_out config".into()),
                    hard_fail: true,
                    explicit_verdict: Some(Verdict::Fail {
                        reason: "missing fan_out config".into(),
                        root_cause: None,
                    }),
                    failure_kind: Some(FailureKind::Unknown),
                    stderr_excerpt: None,
                },
                vec![],
            );
        }
    };

    // 找到子任务模板节点
    let child_node = match wf.nodes.iter().find(|n| n.id == config.child_node_id) {
        Some(n) => n.clone(),
        None => {
            return (
                NodeOutcome {
                    output: String::new(),
                    error: Some(format!(
                        "fan_out child_node_id '{}' not found",
                        config.child_node_id
                    )),
                    hard_fail: true,
                    explicit_verdict: Some(Verdict::Fail {
                        reason: format!("child_node_id '{}' not found", config.child_node_id),
                        root_cause: None,
                    }),
                    failure_kind: Some(FailureKind::Unknown),
                    stderr_excerpt: None,
                },
                vec![],
            );
        }
    };

    // ── 拆分子任务 ─────────────────────────────────────────────────────────
    let child_inputs: Vec<String> = match &config.split {
        SplitStrategy::ByField { field } => {
            // 从 current_input 解析 JSON，取指定字段的数组
            let parsed: Value = serde_json::from_str(current_input).unwrap_or(Value::Null);
            let arr = parsed.get(field).and_then(|v| v.as_array());
            match arr {
                Some(a) => a
                    .iter()
                    .map(|item| serde_json::to_string(item).unwrap_or_else(|_| item.to_string()))
                    .collect(),
                None => {
                    return (
                        NodeOutcome {
                            output: String::new(),
                            error: Some(format!("fan_out split ByField: field '{}' not found or not array", field)),
                            hard_fail: true,
                            explicit_verdict: Some(Verdict::Fail {
                                reason: format!("split field '{}' not array", field),
                                root_cause: None,
                            }),
                            failure_kind: Some(FailureKind::Unknown),
                            stderr_excerpt: None,
                        },
                        vec![],
                    );
                }
            }
        }
        SplitStrategy::Static { items } => items.clone(),
        SplitStrategy::LlmSplit { count: _ } => {
            // P1 简化：LlmSplit 暂不实现 LLM 拆分逻辑，按换行符拆分
            current_input
                .lines()
                .filter(|l| !l.trim().is_empty())
                .map(|l| l.to_string())
                .collect()
        }
    };

    // 子任务数限制（最大 10）
    if child_inputs.len() > 10 {
        return (
            NodeOutcome {
                output: String::new(),
                error: Some(format!("fan_out child count {} exceeds max 10", child_inputs.len())),
                hard_fail: true,
                explicit_verdict: Some(Verdict::Fail {
                    reason: format!("child count {} > 10", child_inputs.len()),
                    root_cause: None,
                }),
                failure_kind: Some(FailureKind::Unknown),
                stderr_excerpt: None,
            },
            vec![],
        );
    }

    if child_inputs.is_empty() {
        return (
            NodeOutcome {
                output: "(no subtasks)".to_string(),
                error: None,
                hard_fail: false,
                explicit_verdict: None,
                failure_kind: None,
                stderr_excerpt: None,
            },
            vec![],
        );
    }

    // ── 并行执行子任务 ────────────────────────────────────────────────────
    eprintln!(
        "[fan-out] node {} dispatching {} children via '{}'",
        node.id,
        child_inputs.len(),
        config.child_node_id
    );

    let child_futures: Vec<_> = child_inputs
        .iter()
        .enumerate()
        .map(|(idx, input)| {
            let child = child_node.clone();
            let inp = input.clone();
            let prov = provider.clone();
            let servers = mcp_servers.to_vec();
            let wf_clone = wf.clone();
            async move {
                let started = chrono::Utc::now().timestamp_millis();
                let mut input = inp;
                let (outcome, _) = execute_node(&child, &mut input, &prov, &servers, &wf_clone).await;
                let finished = chrono::Utc::now().timestamp_millis();
                (idx, input, outcome, started, finished)
            }
        })
        .collect();

    let results = join_all(child_futures).await;

    // ── 构建子任务明细（供主循环记录为独立 StepInstance）────────────────────
    let child_results: Vec<FanOutChildResult> = results
        .iter()
        .map(|(idx, input, outcome, started, finished)| FanOutChildResult {
            index: *idx,
            input: input.clone(),
            output: outcome.output.clone(),
            error: outcome.error.clone(),
            started_at: *started,
            finished_at: *finished,
        })
        .collect();

    // ── 收敛 ──────────────────────────────────────────────────────────────
    let total = results.len();
    let failures: Vec<_> = results
        .iter()
        .filter(|(_, _, o, _, _)| o.error.is_some() || o.explicit_verdict.as_ref().map_or(false, |v| v.is_fail()))
        .collect();
    let success_count = total - failures.len();

    let parent_success = match config.converge {
        ConvergeStrategy::And => failures.is_empty(),
        ConvergeStrategy::Or => success_count > 0,
    };

    // 合并输出：把所有子任务的 output 拼成 JSON 数组
    let merged_output = serde_json::to_string_pretty(
        &results
            .iter()
            .map(|(_, _, o, _, _)| {
                json!({
                    "output": o.output,
                    "error": o.error,
                })
            })
            .collect::<Vec<_>>(),
    )
    .unwrap_or_else(|_| String::new());

    *current_input = merged_output.clone();

    let parent_outcome = if parent_success {
        NodeOutcome {
            output: merged_output,
            error: None,
            hard_fail: false,
            explicit_verdict: None,
            failure_kind: None,
            stderr_excerpt: None,
        }
    } else {
        let reason = format!(
            "fan_out converge failed: {}/{} children failed",
            failures.len(),
            total
        );
        NodeOutcome {
            output: merged_output,
            error: Some(reason.clone()),
            hard_fail: matches!(config.on_child_fail, ChildFailPolicy::FailParent),
            explicit_verdict: Some(Verdict::Fail {
                reason,
                root_cause: None,
            }),
            failure_kind: Some(FailureKind::Unknown),
            stderr_excerpt: None,
        }
    };

    (parent_outcome, child_results)
}

/// 阶段四：执行 agent_task 节点 —— 通过 HTTP 给子 Agent 发任务，等结果回流。
///
/// 流程：
/// 1. 根据 dispatch 策略选出候选 Agent 列表（Fixed=单个，Failover=逐个尝试）
/// 2. 对每个候选：构造 AgentTask → 注册 pending → POST dispatch → 等待 submit 回流
/// 3. 某候选失败时按策略决定是否尝试下一个（Failover/CapabilityMatch）
/// 4. 收到结果后，按 verdict 构造 NodeOutcome
async fn execute_agent_task_node(node: &WorkflowNode, current_input: &mut String) -> NodeOutcome {
    let task_desc = if node.prompt.is_empty() {
        current_input.clone()
    } else {
        node.prompt.clone()
    };

    // ── 解析调度配置，构建候选 Agent 列表 ──────────────────────────────────────
    let (candidates, strategy, timeout_secs) = resolve_candidates(node);

    if candidates.is_empty() {
        return NodeOutcome {
            output: String::new(),
            error: Some("agent_task node has no agent_url and no dispatch candidates".into()),
            hard_fail: true,
            explicit_verdict: None,
            failure_kind: Some(FailureKind::Unknown),
            stderr_excerpt: None,
        };
    }

    let hook_config = crate::agent_http::read_hook_server_config();
    let callback_url = format!("http://localhost:{}/agent/submit", hook_config.port);
    let store = crate::agent_http::get_store();

    let mut last_error = String::new();

    for (idx, candidate) in candidates.iter().enumerate() {
        let task_id = uuid::Uuid::new_v4().to_string();
        let agent_id = candidate.id.clone();
        let agent_url = candidate.url.clone();

        // 注册 pending（工作流挂起等待 submit 回调）
        let run_id = String::new();
        let step_id = String::new();
        let rx = store.register_pending(&task_id, &run_id, &step_id);

        // 构造 AgentTask 并存储
        let agent_task = crate::agent_http::AgentTask {
            task_id: task_id.clone(),
            agent_id: agent_id.clone(),
            agent_url: agent_url.clone(),
            task: task_desc.clone(),
            context: serde_json::json!({"input": current_input.clone()}),
            callback_url: callback_url.clone(),
            created_at: chrono::Utc::now().timestamp_millis(),
            status: "dispatched".to_string(),
            result: None,
            run_id: None,
            step_id: None,
        };
        {
            let mut tasks = store.tasks.lock().unwrap();
            tasks.insert(task_id.clone(), agent_task.clone());
        }

        let payload = serde_json::json!({
            "task_id": agent_task.task_id,
            "agent_id": agent_task.agent_id,
            "task": agent_task.task,
            "context": agent_task.context,
            "callback_url": agent_task.callback_url,
        });

        eprintln!(
            "[agent-task] dispatching {} to {} (candidate {}/{}, strategy={})",
            task_id,
            agent_url,
            idx + 1,
            candidates.len(),
            match strategy {
                DispatchStrategy::Fixed => "fixed",
                DispatchStrategy::Failover => "failover",
                DispatchStrategy::CapabilityMatch => "capability_match",
                DispatchStrategy::Random => "random",
            }
        );

        // POST 到候选 Agent
        let dispatch_result = reqwest::Client::new()
            .post(&agent_url)
            .json(&payload)
            .timeout(std::time::Duration::from_secs(timeout_secs))
            .send()
            .await;

        match dispatch_result {
            Ok(resp) if resp.status().is_success() => {
                eprintln!("[agent-task] agent {} accepted (HTTP {})", agent_id, resp.status());
            }
            Ok(resp) => {
                let status = resp.status();
                last_error = format!("agent {} rejected (HTTP {})", agent_id, status);
                eprintln!("[agent-task] {}", last_error);
                store.pending.lock().unwrap().remove(&task_id);
                // Fixed 策略不重试
                if matches!(strategy, DispatchStrategy::Fixed) {
                    return NodeOutcome {
                        output: String::new(),
                        error: Some(last_error),
                        hard_fail: true,
                        explicit_verdict: None,
                        failure_kind: Some(FailureKind::RpcError),
                        stderr_excerpt: None,
                    };
                }
                continue; // 尝试下一个候选
            }
            Err(e) => {
                last_error = format!("dispatch to {} failed: {}", agent_id, e);
                eprintln!("[agent-task] {}", last_error);
                store.pending.lock().unwrap().remove(&task_id);
                if matches!(strategy, DispatchStrategy::Fixed) {
                    return NodeOutcome {
                        output: String::new(),
                        error: Some(last_error),
                        hard_fail: true,
                        explicit_verdict: None,
                        failure_kind: Some(FailureKind::Network),
                        stderr_excerpt: Some(e.to_string()),
                    };
                }
                continue; // 尝试下一个候选
            }
        }

        // dispatch 成功，挂起等待子 Agent submit（最长 120s）
        match tokio::time::timeout(std::time::Duration::from_secs(120), rx).await {
            Ok(Ok(result)) => {
                eprintln!(
                    "[agent-task] received result task={} verdict={}",
                    task_id, result.verdict
                );
                let output_text =
                    serde_json::to_string_pretty(&result.result).unwrap_or_default();
                *current_input = output_text.clone();

                let verdict = match result.verdict.as_str() {
                    "pass" => crate::workflow::Verdict::Pass,
                    "fail" => crate::workflow::Verdict::Fail {
                        reason: result
                            .note
                            .clone()
                            .unwrap_or_else(|| "agent reported fail".into()),
                        root_cause: None,
                    },
                    "blocked" => crate::workflow::Verdict::Blocked {
                        reason: result.note.clone().unwrap_or_else(|| "agent blocked".into()),
                        notify: None,
                    },
                    _ => crate::workflow::Verdict::Pass,
                };

                return NodeOutcome {
                    output: output_text,
                    error: None,
                    hard_fail: false,
                    explicit_verdict: if matches!(verdict, crate::workflow::Verdict::Pass) {
                        None
                    } else {
                        Some(verdict)
                    },
                    failure_kind: None,
                    stderr_excerpt: None,
                };
            }
            Ok(Err(_)) => {
                last_error = "agent submit channel closed".to_string();
                eprintln!("[agent-task] {} (candidate {})", last_error, agent_id);
                if matches!(strategy, DispatchStrategy::Fixed) {
                    return NodeOutcome {
                        output: String::new(),
                        error: Some(last_error),
                        hard_fail: true,
                        explicit_verdict: None,
                        failure_kind: Some(FailureKind::Unknown),
                        stderr_excerpt: None,
                    };
                }
                continue;
            }
            Err(_) => {
                store.pending.lock().unwrap().remove(&task_id);
                last_error = format!("agent {} timeout (120s)", agent_id);
                eprintln!("[agent-task] {}", last_error);
                if matches!(strategy, DispatchStrategy::Fixed) {
                    return NodeOutcome {
                        output: String::new(),
                        error: Some(last_error),
                        hard_fail: true,
                        explicit_verdict: None,
                        failure_kind: Some(FailureKind::Timeout),
                        stderr_excerpt: None,
                    };
                }
                continue;
            }
        }
    }

    // 所有候选都失败
    NodeOutcome {
        output: String::new(),
        error: Some(format!("all {} candidates failed; last: {}", candidates.len(), last_error)),
        hard_fail: true,
        explicit_verdict: None,
        failure_kind: Some(FailureKind::Network),
        stderr_excerpt: Some(last_error),
    }
}

/// 根据 node.dispatch 配置解析候选 Agent 列表。
/// - Fixed / 无配置：返回 node.server 单个候选
/// - Failover：按 priority 排序返回全部候选
/// - CapabilityMatch：过滤 capabilities 包含 required_capabilities 的候选
/// - Random：随机打乱候选顺序
fn resolve_candidates(node: &WorkflowNode) -> (Vec<AgentCandidate>, DispatchStrategy, u64) {
    let default_timeout = 10u64;

    match &node.dispatch {
        None | Some(DispatchConfig { strategy: DispatchStrategy::Fixed, .. }) => {
            // 向后兼容：无 dispatch 配置或 Fixed 策略，用 server 字段
            if node.server.is_empty() {
                return (vec![], DispatchStrategy::Fixed, default_timeout);
            }
            let candidate = AgentCandidate {
                id: node.label.clone(),
                url: node.server.clone(),
                capabilities: vec![],
                priority: 100,
            };
            let timeout = node
                .dispatch
                .as_ref()
                .map(|d| d.timeout_secs)
                .unwrap_or(default_timeout);
            (vec![candidate], DispatchStrategy::Fixed, timeout)
        }
        Some(cfg) => {
            let timeout = cfg.timeout_secs;
            match cfg.strategy {
                DispatchStrategy::Fixed => {
                    if node.server.is_empty() {
                        (vec![], DispatchStrategy::Fixed, timeout)
                    } else {
                        let candidate = AgentCandidate {
                            id: node.label.clone(),
                            url: node.server.clone(),
                            capabilities: vec![],
                            priority: 100,
                        };
                        (vec![candidate], DispatchStrategy::Fixed, timeout)
                    }
                }
                DispatchStrategy::Failover => {
                    let mut candidates = cfg.candidates.clone();
                    candidates.sort_by_key(|c| c.priority);
                    (candidates, DispatchStrategy::Failover, timeout)
                }
                DispatchStrategy::CapabilityMatch => {
                    let required: std::collections::HashSet<&str> = cfg
                        .required_capabilities
                        .iter()
                        .map(|s| s.as_str())
                        .collect();
                    let matched: Vec<AgentCandidate> = cfg
                        .candidates
                        .iter()
                        .filter(|c| {
                            // 候选 capabilities 包含所有 required
                            let caps: std::collections::HashSet<&str> =
                                c.capabilities.iter().map(|s| s.as_str()).collect();
                            required.is_subset(&caps)
                        })
                        .cloned()
                        .collect();
                    let mut matched = matched;
                    matched.sort_by_key(|c| c.priority);
                    (matched, DispatchStrategy::CapabilityMatch, timeout)
                }
                DispatchStrategy::Random => {
                    // 简单伪随机：用系统时间做 seed 打乱顺序
                    let mut candidates = cfg.candidates.clone();
                    let seed = chrono::Utc::now().timestamp_nanos_opt().unwrap_or(0) as u64;
                    simple_shuffle(&mut candidates, seed);
                    (candidates, DispatchStrategy::Random, timeout)
                }
            }
        }
    }
}

/// 简单的 Fisher-Yates 伪随机 shuffle（不引入 rand crate）。
fn simple_shuffle<T>(slice: &mut [T], mut seed: u64) {
    if seed == 0 {
        seed = 1;
    }
    let n = slice.len();
    for i in (1..n).rev() {
        // 线性同余生成器
        seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        let j = (seed >> 33) as usize % (i + 1);
        slice.swap(i, j);
    }
}

/// 阶段二：执行 Acceptance 节点。
///
/// Acceptance 节点本身不做实际工作，它的"执行"就是返回一个 Blocked Verdict，
/// 让 run_workflow_core 把 Run 状态置为 WaitingAcceptance，并发出
/// workflow-acceptance-requested 事件。真正的验收由 approve_run / reject_run 命令完成。
async fn execute_acceptance_node(node: &WorkflowNode) -> NodeOutcome {
    let cfg = node.acceptance.clone().unwrap_or_default();
    let reason = if cfg.allow_reject_to.is_empty() {
        "Waiting for acceptance".to_string()
    } else {
        format!(
            "Waiting for acceptance (can reject to: {})",
            cfg.allow_reject_to.join(", ")
        )
    };
    let notify = cfg.notify.first().cloned();
    NodeOutcome {
        output: reason.clone(),
        error: None, // Blocked 不是错误
        hard_fail: false,
        explicit_verdict: Some(Verdict::Blocked { reason, notify }),
        failure_kind: None,
        stderr_excerpt: None,
    }
}

async fn execute_tool_node(
    node: &WorkflowNode,
    current_input: &mut String,
    mcp_servers: &[McpServer],
) -> NodeOutcome {
    let server = match mcp_servers.iter().find(|s| s.name == node.server).cloned() {
        Some(s) => s,
        None => {
            return NodeOutcome {
                output: String::new(),
                error: Some(format!(
                    "MCP server '{}' not enabled or not found",
                    node.server
                )),
                hard_fail: true,
                explicit_verdict: None,
                failure_kind: Some(FailureKind::RpcError),
                stderr_excerpt: None,
            };
        }
    };

    let tool_name = node.tool.clone();
    let args = substitute_input(&node.arguments, current_input);
    let args = if args.is_null() { json!({}) } else { args };

    let res = tokio::task::spawn_blocking(move || -> Result<String, (String, Option<String>)> {
        let mut client = McpClient::start(&server).map_err(|e| (e, None))?;
        let result = client
            .call_tool(&tool_name, &args)
            .map_err(|e| {
                let stderr = client.take_stderr_excerpt();
                (e, stderr)
            })?;
        Ok(tool_result_text(&result))
    })
    .await
    .map_err(|e| (e.to_string(), None));

    match res {
        Ok(Ok(out)) => {
            *current_input = out.clone();
            NodeOutcome {
                output: out,
                error: None,
                hard_fail: false,
                explicit_verdict: None,
                failure_kind: None,
                stderr_excerpt: None,
            }
        }
        Ok(Err((e, stderr))) | Err((e, stderr)) => NodeOutcome {
            output: String::new(),
            error: Some(e),
            hard_fail: true,
            explicit_verdict: None,
            failure_kind: Some(FailureKind::RpcError),
            stderr_excerpt: stderr,
        },
    }
}

async fn execute_llm_node(
    node: &WorkflowNode,
    current_input: &mut String,
    provider: &Option<LlmProvider>,
) -> NodeOutcome {
    let Some(ref p) = provider else {
        return NodeOutcome {
            output: String::new(),
            error: Some("No LLM provider configured for this workflow".to_string()),
            hard_fail: true,
            explicit_verdict: None,
            failure_kind: Some(FailureKind::Unknown),
            stderr_excerpt: None,
        };
    };

    let (system_content, user_content) = if current_input.is_empty() {
        (
            "You are a workflow step. Complete the task below and reply with only the result."
                .to_string(),
            node.prompt.clone(),
        )
    } else if node.prompt.is_empty() {
        (
            "You are a workflow step. Process the provided content and reply with only the result."
                .to_string(),
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
            *current_input = out.clone();
            NodeOutcome {
                output: out,
                error: None,
                hard_fail: false,
                explicit_verdict: None,
                failure_kind: None,
                stderr_excerpt: None,
            }
        }
        Err(e) => NodeOutcome {
            output: String::new(),
            error: Some(e),
            hard_fail: true,
            explicit_verdict: None,
            failure_kind: Some(FailureKind::Network),
            stderr_excerpt: None,
        },
    }
}

async fn execute_mcp_agent_node(
    node: &WorkflowNode,
    current_input: &mut String,
    provider: &Option<LlmProvider>,
    mcp_servers: &[McpServer],
) -> NodeOutcome {
    let Some(ref p) = provider else {
        return NodeOutcome {
            output: String::new(),
            error: Some("No LLM provider configured for this workflow".to_string()),
            hard_fail: true,
            explicit_verdict: None,
            failure_kind: Some(FailureKind::Unknown),
            stderr_excerpt: None,
        };
    };

    let server = match mcp_servers.iter().find(|s| s.name == node.server).cloned() {
        Some(s) => s,
        None => {
            return NodeOutcome {
                output: String::new(),
                error: Some(format!(
                    "MCP server '{}' not enabled or not found",
                    node.server
                )),
                hard_fail: true,
                explicit_verdict: None,
                failure_kind: Some(FailureKind::RpcError),
                stderr_excerpt: None,
            };
        }
    };

    let (task, extra_prompt) = if current_input.is_empty() {
        (node.prompt.clone(), String::new())
    } else {
        (current_input.clone(), node.prompt.clone())
    };

    match run_mcp_agent_node(&server, p, &task, &extra_prompt, 10).await {
        Ok(raw) => {
            let out = strip_thinking(&raw);
            *current_input = out.clone();
            NodeOutcome {
                output: out,
                error: None,
                hard_fail: false,
                explicit_verdict: None,
                failure_kind: None,
                stderr_excerpt: None,
            }
        }
        Err((e, stderr)) => {
            // 旧行为：mcp_agent 失败不终止，把 err_msg 当 output 继续
            let err_msg = format!("[MCP agent error: {}]", e);
            *current_input = err_msg.clone();
            NodeOutcome {
                output: err_msg,
                error: Some(e),
                hard_fail: false, // 软失败：继续执行下游
                explicit_verdict: None,
                failure_kind: Some(FailureKind::AgentBlocked),
                stderr_excerpt: stderr,
            }
        }
    }
}

/// 从 NodeOutcome 构造 WorkflowStepResult（双写旧字段 + 新 submission）。
fn build_step_result(
    node_id: &str,
    label: &str,
    kind: &str,
    outcome: &NodeOutcome,
) -> WorkflowStepResult {
    // 阶段二：显式 Verdict 优先（acceptance 节点的 Blocked）
    if let Some(ref verdict) = outcome.explicit_verdict {
        let mut step = WorkflowStepResult::success(node_id, label, kind, &outcome.output);
        // 覆盖 submission 的 verdict
        if let Some(ref mut sub) = step.submission {
            sub.verdict = verdict.clone();
        }
        step.status = Some(NodeStatus::from_verdict(verdict));
        if let Verdict::Blocked { reason, .. } = verdict {
            step.error = None; // Blocked 不是错误
            let _ = reason; // 已在 output 中
        }
        return step;
    }

    if let Some(ref err) = outcome.error {
        if outcome.hard_fail {
            WorkflowStepResult::failure(node_id, label, kind, err)
        } else {
            // 软失败：verdict=Pass（流程继续），但保留 error 字段供前端显示
            let mut step = WorkflowStepResult::success(node_id, label, kind, &outcome.output);
            step.error = Some(err.clone());
            step
        }
    } else {
        WorkflowStepResult::success(node_id, label, kind, &outcome.output)
    }
}

/// 判断 step 是否为硬失败（用于决定 Run 整体状态）。
fn outcome_is_hard_fail(step: &WorkflowStepResult) -> bool {
    step.error.is_some()
        && step
            .submission
            .as_ref()
            .map(|s| s.verdict.is_fail())
            .unwrap_or(true)
}

/// 阶段二：轻量 JSON Schema 子集校验。
///
/// 支持的 schema 关键字：
/// - `type`: "string" | "number" | "integer" | "boolean" | "object" | "array" | "null"
/// - `required`: ["field1", "field2"]（仅对 object 生效）
/// - `properties`: { field: { type: "..." } }（仅校验存在的字段类型，不做深度递归）
///
/// 不支持的：$ref、allOf/anyOf/oneOf、pattern、format、minimum/maximum 等。
/// 用意是覆盖 80% 的常见准出契约场景，避免引入 jsonschema 重型依赖。
fn validate_output_contract(artifact: &Value, schema: &Value) -> Result<(), String> {
    // type 校验
    if let Some(expected_type) = schema["type"].as_str() {
        let actual_type = match artifact {
            Value::Null => "null",
            Value::Bool(_) => "boolean",
            Value::Number(_) => "number",
            Value::String(_) => "string",
            Value::Array(_) => "array",
            Value::Object(_) => "object",
        };
        // integer 是 number 的子集
        let type_ok = if expected_type == "integer" {
            artifact.is_number() && artifact.as_i64().is_some()
        } else if expected_type == "number" {
            actual_type == "number"
        } else {
            actual_type == expected_type
        };
        if !type_ok {
            return Err(format!(
                "expected type '{}', got '{}'",
                expected_type, actual_type
            ));
        }
    }

    // required 校验（仅对 object）
    if let Some(required) = schema["required"].as_array() {
        if let Some(obj) = artifact.as_object() {
            for req in required {
                if let Some(field) = req.as_str() {
                    if !obj.contains_key(field) {
                        return Err(format!("missing required field: '{}'", field));
                    }
                }
            }
        }
    }

    // properties 校验（仅校验存在的字段类型）
    if let Some(props) = schema["properties"].as_object() {
        if let Some(obj) = artifact.as_object() {
            for (field, field_schema) in props {
                if let Some(field_value) = obj.get(field) {
                    // 递归校验单个字段（一层深度）
                    if let Err(e) = validate_output_contract(field_value, field_schema) {
                        return Err(format!("field '{}': {}", field, e));
                    }
                }
            }
        }
    }

    Ok(())
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
) -> Result<String, (String, Option<String>)> {
    let server_clone = server.clone();

    // ── 1. Start MCP client and list tools on a blocking thread ──────────────
    let (client_arc, all_tools) = tokio::task::spawn_blocking(
        move || -> Result<(Arc<Mutex<McpClient>>, Vec<Value>), (String, Option<String>)> {
            let mut c = McpClient::start(&server_clone).map_err(|e| (e, None))?;
            let raw = c.list_tools().map_err(|e| {
                let stderr = c.take_stderr_excerpt();
                (e, stderr)
            })?;
            let tools: Vec<Value> = raw
                .iter()
                .map(|t| {
                    json!({
                        "type": "function",
                        "function": {
                            "name": t["name"].as_str().unwrap_or(""),
                            "description": t["description"].as_str().unwrap_or(""),
                            "parameters": t.get("inputSchema").cloned()
                                .unwrap_or(json!({"type":"object","properties":{}}))
                        }
                    })
                })
                .collect();
            Ok((Arc::new(Mutex::new(c)), tools))
        },
    )
    .await
    .map_err(|e| (e.to_string(), None))??;

    // ── 2. Build system prompt ────────────────────────────────────────────────
    let tool_names: Vec<String> = all_tools
        .iter()
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
        let response = match chat(provider, &messages, &all_tools).await {
            Ok(v) => v,
            Err(e) => {
                // LLM 错误不是 MCP stderr，但可以从 client_arc 尝试提取
                let stderr = client_arc
                    .lock()
                    .ok()
                    .and_then(|c| c.take_stderr_excerpt());
                return Err((e, stderr));
            }
        };
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
                let tc_id = tc["id"].as_str().unwrap_or("").to_string();
                let fn_name = tc["function"]["name"].as_str().unwrap_or("").to_string();
                let fn_args: Value = tc["function"]["arguments"]
                    .as_str()
                    .and_then(|s| serde_json::from_str(s).ok())
                    .unwrap_or(json!({}));

                // Call the MCP tool on a blocking thread.
                let client_arc2 = Arc::clone(&client_arc);
                let fn_name2 = fn_name.clone();
                let fn_args2 = fn_args.clone();
                let tool_result_text = tokio::task::spawn_blocking(move || {
                    let mut client = client_arc2.lock().unwrap();
                    match client.call_tool(&fn_name2, &fn_args2) {
                        Ok(res) => res["content"]
                            .as_array()
                            .map(|arr| {
                                arr.iter()
                                    .filter_map(|c| c["text"].as_str().map(|s| s.to_string()))
                                    .collect::<Vec<_>>()
                                    .join("\n")
                            })
                            .filter(|s| !s.is_empty())
                            .unwrap_or_else(|| serde_json::to_string(&res).unwrap_or_default()),
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
                    resp["choices"][0]["message"]["content"]
                        .as_str()
                        .unwrap_or(""),
                );
            }
            break;
        }
    }

    // Fallback 1: last assistant message that has text content
    if final_answer.is_empty() {
        final_answer = messages
            .iter()
            .rev()
            .find_map(|m| {
                if m["role"] == "assistant" {
                    let c = strip_thinking(m["content"].as_str().unwrap_or(""));
                    if !c.is_empty() {
                        Some(c)
                    } else {
                        None
                    }
                } else {
                    None
                }
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
                resp["choices"][0]["message"]["content"]
                    .as_str()
                    .unwrap_or(""),
            );
        }
    }

    Ok(final_answer)
}

// ── Tests (phase 1a: Verdict / Submission / NodeStatus) ──────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn verdict_kind_strings() {
        assert_eq!(Verdict::Pass.kind(), "pass");
        assert_eq!(
            Verdict::Fail {
                reason: "x".into(),
                root_cause: None
            }
            .kind(),
            "fail"
        );
        assert_eq!(
            Verdict::Blocked {
                reason: "y".into(),
                notify: None
            }
            .kind(),
            "blocked"
        );
    }

    #[test]
    fn verdict_predicates() {
        assert!(Verdict::Pass.is_pass());
        assert!(!Verdict::Pass.is_fail());
        assert!(!Verdict::Pass.is_blocked());

        let f = Verdict::Fail {
            reason: "e".into(),
            root_cause: None,
        };
        assert!(f.is_fail());
        assert!(!f.is_pass());

        let b = Verdict::Blocked {
            reason: "w".into(),
            notify: None,
        };
        assert!(b.is_blocked());
    }

    #[test]
    fn verdict_default_is_pass() {
        assert_eq!(Verdict::default(), Verdict::Pass);
    }

    #[test]
    fn verdict_serializes_lowercase() {
        let v = Verdict::Pass;
        let s = serde_json::to_string(&v).unwrap();
        assert_eq!(s, "\"pass\"");

        let f = Verdict::Fail {
            reason: "boom".into(),
            root_cause: None,
        };
        let s = serde_json::to_string(&f).unwrap();
        assert!(s.contains("\"fail\""));
        assert!(s.contains("\"reason\":\"boom\""));
        assert!(
            !s.contains("root_cause"),
            "None root_cause should be skipped"
        );

        let f2 = Verdict::Fail {
            reason: "boom".into(),
            root_cause: Some("deep".into()),
        };
        let s = serde_json::to_string(&f2).unwrap();
        assert!(s.contains("\"root_cause\":\"deep\""));
    }

    #[test]
    fn verdict_roundtrip_deserialize() {
        let v: Verdict = serde_json::from_str("\"pass\"").unwrap();
        assert_eq!(v, Verdict::Pass);

        let v: Verdict =
            serde_json::from_str(r#"{"fail":{"reason":"x","root_cause":"y"}}"#).unwrap();
        match v {
            Verdict::Fail { reason, root_cause } => {
                assert_eq!(reason, "x");
                assert_eq!(root_cause.as_deref(), Some("y"));
            }
            _ => panic!("expected Fail"),
        }
    }

    #[test]
    fn node_status_from_verdict() {
        assert_eq!(
            NodeStatus::from_verdict(&Verdict::Pass),
            NodeStatus::Success
        );
        assert_eq!(
            NodeStatus::from_verdict(&Verdict::Fail {
                reason: "e".into(),
                root_cause: None
            }),
            NodeStatus::Failed
        );
        assert_eq!(
            NodeStatus::from_verdict(&Verdict::Blocked {
                reason: "w".into(),
                notify: None
            }),
            NodeStatus::Blocked
        );
    }

    #[test]
    fn node_status_default_is_pending() {
        assert_eq!(NodeStatus::default(), NodeStatus::Pending);
    }

    #[test]
    fn node_status_serializes_lowercase() {
        let s = serde_json::to_string(&NodeStatus::Running).unwrap();
        assert_eq!(s, "\"running\"");
        let s = serde_json::to_string(&NodeStatus::Blocked).unwrap();
        assert_eq!(s, "\"blocked\"");
        let s = serde_json::to_string(&NodeStatus::Skipped).unwrap();
        assert_eq!(s, "\"skipped\"");
    }

    #[test]
    fn submission_from_text_pass_builds_correct_verdict() {
        let sub = Submission::from_text_pass("hello world");
        assert!(sub.verdict.is_pass());
        assert_eq!(sub.artifact, Value::String("hello world".into()));
        assert_eq!(sub.note.as_deref(), Some("hello world"));
        assert!(sub.confidence.is_none());
    }

    #[test]
    fn submission_from_empty_text_has_no_note() {
        let sub = Submission::from_text_pass("");
        assert!(sub.verdict.is_pass());
        assert_eq!(sub.artifact, Value::String("".into()));
        assert!(sub.note.is_none(), "empty note should be None");
    }

    #[test]
    fn submission_from_error_builds_fail_verdict() {
        let sub = Submission::from_error("timeout");
        match sub.verdict {
            Verdict::Fail { reason, root_cause } => {
                assert_eq!(reason, "timeout");
                assert_eq!(root_cause.as_deref(), Some("timeout"));
            }
            _ => panic!("expected Fail"),
        }
        assert_eq!(sub.artifact, Value::Null);
        assert!(sub.note.is_none());
    }

    #[test]
    fn submission_to_text_extracts_string_artifact() {
        let sub = Submission::from_text_pass("abc");
        assert_eq!(sub.to_text(), "abc");

        let sub = Submission::from_error("err");
        assert_eq!(sub.to_text(), "", "Null artifact should yield empty text");

        let sub = Submission {
            artifact: json!({"key": "val"}),
            verdict: Verdict::Pass,
            confidence: None,
            note: None,
        };
        assert_eq!(sub.to_text(), r#"{"key":"val"}"#);
    }

    #[test]
    fn submission_roundtrip_serialize() {
        let sub = Submission {
            artifact: json!({"result": 42}),
            verdict: Verdict::Pass,
            confidence: Some(0.8),
            note: Some("looks good".into()),
        };
        let s = serde_json::to_string(&sub).unwrap();
        let back: Submission = serde_json::from_str(&s).unwrap();
        assert_eq!(back.artifact, json!({"result": 42}));
        assert!(back.verdict.is_pass());
        assert_eq!(back.confidence, Some(0.8));
        assert_eq!(back.note.as_deref(), Some("looks good"));
    }

    #[test]
    fn workflow_step_result_success_fills_both_output_and_submission() {
        let step = WorkflowStepResult::success("n1", "Label", "tool", "done output");
        assert_eq!(step.output, "done output");
        assert!(step.error.is_none());
        assert!(step.submission.is_some());
        assert_eq!(step.status, Some(NodeStatus::Success));
        assert_eq!(step.attempt, 1);

        let sub = step.submission.unwrap();
        assert!(sub.verdict.is_pass());
        assert_eq!(sub.to_text(), "done output");
    }

    #[test]
    fn workflow_step_result_failure_fills_error_and_fail_verdict() {
        let step = WorkflowStepResult::failure("n2", "Label", "llm", "boom");
        assert!(step.output.is_empty());
        assert_eq!(step.error.as_deref(), Some("boom"));
        let sub = step.submission.as_ref().unwrap();
        assert!(sub.verdict.is_fail());
        assert_eq!(step.status, Some(NodeStatus::Failed));
    }

    /// 旧前端持久化的 JSON（无 submission/status/attempt 字段）必须能反序列化，
    /// 新字段默认填充 None / 0。这是向后兼容的关键保证。
    #[test]
    fn workflow_step_result_legacy_json_back_compat() {
        let legacy = r#"{
            "node_id": "n1",
            "label": "old step",
            "kind": "tool",
            "output": "legacy output",
            "error": null
        }"#;
        let step: WorkflowStepResult = serde_json::from_str(legacy).unwrap();
        assert_eq!(step.node_id, "n1");
        assert_eq!(step.output, "legacy output");
        assert!(step.error.is_none());
        assert!(
            step.submission.is_none(),
            "legacy JSON should yield None submission"
        );
        assert!(step.status.is_none());
        assert_eq!(step.attempt, 0);
    }

    /// 新前端持久化的 JSON（含 submission/status）必须能反序列化回新结构。
    #[test]
    fn workflow_step_result_new_json_roundtrip() {
        let step = WorkflowStepResult::success("n3", "L", "tool", "out");
        let s = serde_json::to_string(&step).unwrap();
        let back: WorkflowStepResult = serde_json::from_str(&s).unwrap();
        assert_eq!(back.node_id, "n3");
        assert_eq!(back.output, "out");
        assert!(back.submission.is_some());
        assert_eq!(back.status, Some(NodeStatus::Success));
    }

    /// 序列化时 None 字段应被 skip，避免旧前端看到多余字段困惑。
    #[test]
    fn workflow_step_result_skips_none_fields_on_serialize() {
        let legacy_step = WorkflowStepResult {
            node_id: "n".into(),
            label: "l".into(),
            kind: "tool".into(),
            output: "o".into(),
            error: None,
            submission: None,
            status: None,
            attempt: 0,
            failure_trace: None,
        };
        let s = serde_json::to_string(&legacy_step).unwrap();
        assert!(
            !s.contains("submission"),
            "None submission should be skipped"
        );
        assert!(!s.contains("status"), "None status should be skipped");
        assert!(!s.contains("attempt"), "0 attempt should be skipped");
    }

    // ── Edge 模型测试（阶段 1c）─────────────────────────────────────────────

    fn make_node(id: &str, kind: &str) -> WorkflowNode {
        WorkflowNode {
            id: id.into(),
            kind: kind.into(),
            label: String::new(),
            server: String::new(),
            tool: String::new(),
            arguments: Value::Null,
            prompt: String::new(),
            acceptance: None,
            output_contract: None,
            fan_out: None,
            dispatch: None,
        }
    }

    #[test]
    fn build_failure_trace_populates_node_context() {
        let mut node = make_node("node-a", "tool");
        node.server = "filesystem".into();
        node.tool = "read_file".into();
        let outcome = NodeOutcome {
            output: String::new(),
            error: Some("tool failed".into()),
            hard_fail: true,
            explicit_verdict: None,
            failure_kind: Some(FailureKind::RpcError),
            stderr_excerpt: Some("stderr line".into()),
        };

        let trace = build_failure_trace("run-1", "step-1", &node, &outcome, &NodeStatus::Failed)
            .expect("failure trace should be produced");

        assert_eq!(trace.run_id, "run-1");
        assert_eq!(trace.step_id, "step-1");
        assert_eq!(trace.node_id, "node-a");
        assert_eq!(trace.tool.as_deref(), Some("read_file"));
        assert!(matches!(trace.failure_kind, FailureKind::RpcError));
        assert_eq!(trace.reason, "tool failed");
        assert_eq!(trace.stderr_excerpt.as_deref(), Some("stderr line"));
        assert_eq!(trace.final_status, "failed");
    }

    #[test]
    fn build_failure_trace_skips_successful_outcome() {
        let node = make_node("node-a", "llm");
        let outcome = NodeOutcome::default();
        assert!(
            build_failure_trace("run-1", "step-1", &node, &outcome, &NodeStatus::Success).is_none()
        );
    }

    /// 构造一个 acceptance 节点（阶段二）
    fn make_acceptance_node(id: &str, allow_reject_to: Vec<&str>) -> WorkflowNode {
        WorkflowNode {
            id: id.into(),
            kind: "acceptance".into(),
            label: "验收".into(),
            server: String::new(),
            tool: String::new(),
            arguments: Value::Null,
            prompt: String::new(),
            acceptance: Some(AcceptanceConfig {
                notify: vec![],
                allow_reject_to: allow_reject_to.iter().map(|s| s.to_string()).collect(),
                timeout_secs: None,
                timeout_action: TimeoutAction::Remind,
            }),
            output_contract: None,
            fan_out: None,
            dispatch: None,
        }
    }

    #[test]
    fn migrate_legacy_derives_linear_edges() {
        let mut wf = Workflow {
            id: "w1".into(),
            name: "test".into(),
            description: String::new(),
            nodes: vec![
                make_node("a", "tool"),
                make_node("b", "llm"),
                make_node("c", "tool"),
            ],
            edges: vec![],
            entry_node_id: None,
            end_node_ids: vec![],
            template_key: None,
            callback_url: None,
            allowed_sources: vec![],
            schedule: None,
            created_at: String::new(),
            updated_at: String::new(),
        };
        wf.migrate_legacy();

        assert_eq!(wf.edges.len(), 2);
        assert_eq!(wf.edges[0].from, "a");
        assert_eq!(wf.edges[0].to, "b");
        assert_eq!(wf.edges[1].from, "b");
        assert_eq!(wf.edges[1].to, "c");
        assert_eq!(wf.entry_node_id.as_deref(), Some("a"));
    }

    #[test]
    fn migrate_legacy_preserves_existing_edges() {
        let mut wf = Workflow {
            id: "w2".into(),
            name: "test".into(),
            description: String::new(),
            nodes: vec![make_node("x", "tool"), make_node("y", "llm")],
            edges: vec![WorkflowEdge {
                from: "x".into(),
                to: "y".into(),
                condition: EdgeCondition::Always,
            }],
            entry_node_id: Some("x".into()),
            end_node_ids: vec![],
            template_key: None,
            callback_url: None,
            allowed_sources: vec![],
            schedule: None,
            created_at: String::new(),
            updated_at: String::new(),
        };
        wf.migrate_legacy();

        // 已有 edges 不应被覆盖
        assert_eq!(wf.edges.len(), 1);
        assert_eq!(wf.edges[0].condition, EdgeCondition::Always);
    }

    #[test]
    fn migrate_legacy_empty_nodes_noop() {
        let mut wf = Workflow {
            id: "w3".into(),
            name: "empty".into(),
            description: String::new(),
            nodes: vec![],
            edges: vec![],
            entry_node_id: None,
            end_node_ids: vec![],
            template_key: None,
            callback_url: None,
            allowed_sources: vec![],
            schedule: None,
            created_at: String::new(),
            updated_at: String::new(),
        };
        wf.migrate_legacy();
        assert!(wf.edges.is_empty());
        assert!(wf.entry_node_id.is_none());
    }

    #[test]
    fn next_nodes_returns_matching_downstream() {
        let mut wf = Workflow {
            id: "w4".into(),
            name: "branch".into(),
            description: String::new(),
            nodes: vec![
                make_node("a", "tool"),
                make_node("b", "llm"),
                make_node("c", "tool"),
            ],
            edges: vec![
                WorkflowEdge {
                    from: "a".into(),
                    to: "b".into(),
                    condition: EdgeCondition::OnVerdict {
                        verdict: VerdictKind::Pass,
                    },
                },
                WorkflowEdge {
                    from: "a".into(),
                    to: "c".into(),
                    condition: EdgeCondition::OnVerdict {
                        verdict: VerdictKind::Fail,
                    },
                },
            ],
            entry_node_id: Some("a".into()),
            end_node_ids: vec![],
            template_key: None,
            callback_url: None,
            allowed_sources: vec![],
            schedule: None,
            created_at: String::new(),
            updated_at: String::new(),
        };
        let _ = &mut wf;

        // Pass → 只有 b
        let next = wf.next_nodes("a", &Verdict::Pass);
        assert_eq!(next, vec!["b"]);

        // Fail → 只有 c
        let next = wf.next_nodes(
            "a",
            &Verdict::Fail {
                reason: "e".into(),
                root_cause: None,
            },
        );
        assert_eq!(next, vec!["c"]);

        // Blocked → 无匹配
        let next = wf.next_nodes(
            "a",
            &Verdict::Blocked {
                reason: "w".into(),
                notify: None,
            },
        );
        assert!(next.is_empty());
    }

    #[test]
    fn edge_condition_always_matches_any_verdict() {
        let edge = WorkflowEdge {
            from: "a".into(),
            to: "b".into(),
            condition: EdgeCondition::Always,
        };
        assert!(edge.matches_verdict(&Verdict::Pass));
        assert!(edge.matches_verdict(&Verdict::Fail {
            reason: "e".into(),
            root_cause: None,
        }));
        assert!(edge.matches_verdict(&Verdict::Blocked {
            reason: "w".into(),
            notify: None,
        }));
    }

    #[test]
    fn edge_condition_on_verdict_matches_kind_only() {
        let edge = WorkflowEdge {
            from: "a".into(),
            to: "b".into(),
            condition: EdgeCondition::OnVerdict {
                verdict: VerdictKind::Blocked,
            },
        };
        assert!(!edge.matches_verdict(&Verdict::Pass));
        assert!(!edge.matches_verdict(&Verdict::Fail {
            reason: "e".into(),
            root_cause: None,
        }));
        assert!(edge.matches_verdict(&Verdict::Blocked {
            reason: "w".into(),
            notify: None,
        }));
    }

    #[test]
    fn verdict_kind_from_verdict() {
        assert_eq!(VerdictKind::from(&Verdict::Pass), VerdictKind::Pass);
        assert_eq!(
            VerdictKind::from(&Verdict::Fail {
                reason: "e".into(),
                root_cause: None,
            }),
            VerdictKind::Fail
        );
        assert_eq!(
            VerdictKind::from(&Verdict::Blocked {
                reason: "w".into(),
                notify: None,
            }),
            VerdictKind::Blocked
        );
    }

    #[test]
    fn entry_node_returns_first_when_entry_id_set() {
        let wf = Workflow {
            id: "w5".into(),
            name: "test".into(),
            description: String::new(),
            nodes: vec![make_node("a", "tool"), make_node("b", "llm")],
            edges: vec![],
            entry_node_id: Some("b".into()),
            end_node_ids: vec![],
            template_key: None,
            callback_url: None,
            allowed_sources: vec![],
            schedule: None,
            created_at: String::new(),
            updated_at: String::new(),
        };
        let entry = wf.entry_node().unwrap();
        assert_eq!(entry.id, "b");
    }

    /// 旧 JSON（无 edges/entry_node_id 字段）必须能反序列化，
    /// 字段默认为空 vec / None。
    #[test]
    fn workflow_legacy_json_back_compat() {
        let legacy = r#"{
            "id": "w6",
            "name": "legacy",
            "description": "",
            "nodes": [{"id": "n1", "kind": "tool", "label": "", "server": "", "tool": "", "arguments": null, "prompt": ""}],
            "created_at": "2024-01-01",
            "updated_at": "2024-01-01"
        }"#;
        let wf: Workflow = serde_json::from_str(legacy).unwrap();
        assert_eq!(wf.id, "w6");
        assert!(wf.edges.is_empty());
        assert!(wf.entry_node_id.is_none());

        // 迁移后应自动派生
        let mut wf = wf;
        wf.migrate_legacy();
        assert_eq!(wf.edges.len(), 0); // 只有一个节点，无 edge
        assert_eq!(wf.entry_node_id.as_deref(), Some("n1"));
    }

    #[test]
    fn workflow_with_edges_roundtrip() {
        let wf = Workflow {
            id: "w7".into(),
            name: "branch".into(),
            description: String::new(),
            nodes: vec![make_node("a", "tool"), make_node("b", "llm")],
            edges: vec![WorkflowEdge {
                from: "a".into(),
                to: "b".into(),
                condition: EdgeCondition::OnVerdict {
                    verdict: VerdictKind::Pass,
                },
            }],
            entry_node_id: Some("a".into()),
            end_node_ids: vec![],
            template_key: None,
            callback_url: None,
            allowed_sources: vec![],
            schedule: None,
            created_at: "2024-01-01".into(),
            updated_at: "2024-01-01".into(),
        };
        let s = serde_json::to_string(&wf).unwrap();
        let back: Workflow = serde_json::from_str(&s).unwrap();
        assert_eq!(back.edges.len(), 1);
        assert_eq!(back.edges[0].from, "a");
        assert_eq!(back.entry_node_id.as_deref(), Some("a"));
    }

    // ── 阶段二：Acceptance 节点测试 ──────────────────────────────────────────

    #[test]
    fn migrate_legacy_collects_end_node_ids() {
        let mut wf = Workflow {
            id: "w-end".into(),
            name: "test".into(),
            description: String::new(),
            nodes: vec![
                make_node("a", "tool"),
                make_node("b", "llm"),
                make_node("c", "tool"),
            ],
            edges: vec![],
            entry_node_id: None,
            end_node_ids: vec![],
            template_key: None,
            callback_url: None,
            allowed_sources: vec![],
            schedule: None,
            created_at: String::new(),
            updated_at: String::new(),
        };
        wf.migrate_legacy();
        assert!(wf.end_node_ids.contains(&"c".to_string()));
        assert!(!wf.end_node_ids.contains(&"a".to_string()));
    }

    #[test]
    fn migrate_legacy_collects_acceptance_nodes_as_end() {
        let mut wf = Workflow {
            id: "w-acc".into(),
            name: "test".into(),
            description: String::new(),
            nodes: vec![
                make_node("a", "tool"),
                make_node("b", "llm"),
                make_acceptance_node("c", vec!["a", "b"]),
            ],
            edges: vec![],
            entry_node_id: None,
            end_node_ids: vec![],
            template_key: None,
            callback_url: None,
            allowed_sources: vec![],
            schedule: None,
            created_at: String::new(),
            updated_at: String::new(),
        };
        wf.migrate_legacy();
        assert!(wf.end_node_ids.contains(&"c".to_string()));
    }

    #[test]
    fn acceptance_config_default() {
        let cfg = AcceptanceConfig::default();
        assert!(cfg.notify.is_empty());
        assert!(cfg.allow_reject_to.is_empty());
        assert!(cfg.timeout_secs.is_none());
        assert_eq!(cfg.timeout_action, TimeoutAction::Remind);
    }

    #[test]
    fn timeout_action_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&TimeoutAction::AutoPass).unwrap(),
            "\"autopass\""
        );
        assert_eq!(
            serde_json::to_string(&TimeoutAction::AutoReject).unwrap(),
            "\"autoreject\""
        );
        assert_eq!(
            serde_json::to_string(&TimeoutAction::Remind).unwrap(),
            "\"remind\""
        );
    }

    #[test]
    fn acceptance_node_serializes_with_config() {
        let node = make_acceptance_node("acc1", vec!["n1", "n2"]);
        let s = serde_json::to_string(&node).unwrap();
        assert!(s.contains("\"kind\":\"acceptance\""));
        assert!(s.contains("\"allow_reject_to\":[\"n1\",\"n2\"]"));
        let back: WorkflowNode = serde_json::from_str(&s).unwrap();
        assert_eq!(back.kind, "acceptance");
        let cfg = back.acceptance.unwrap();
        assert_eq!(
            cfg.allow_reject_to,
            vec!["n1".to_string(), "n2".to_string()]
        );
    }

    #[test]
    fn workflow_node_legacy_json_back_compat() {
        let legacy = r#"{
            "id": "n1",
            "kind": "tool",
            "label": "old",
            "server": "srv",
            "tool": "t",
            "arguments": null,
            "prompt": ""
        }"#;
        let node: WorkflowNode = serde_json::from_str(legacy).unwrap();
        assert_eq!(node.id, "n1");
        assert!(node.acceptance.is_none());
        assert!(node.output_contract.is_none());
    }

    #[test]
    fn workflow_with_acceptance_end_node_ids_roundtrip() {
        let wf = Workflow {
            id: "w-rt".into(),
            name: "test".into(),
            description: String::new(),
            nodes: vec![make_node("a", "tool"), make_acceptance_node("b", vec!["a"])],
            edges: vec![WorkflowEdge {
                from: "a".into(),
                to: "b".into(),
                condition: EdgeCondition::OnVerdict {
                    verdict: VerdictKind::Pass,
                },
            }],
            entry_node_id: Some("a".into()),
            end_node_ids: vec!["b".into()],
            template_key: None,
            callback_url: None,
            allowed_sources: vec![],
            schedule: None,
            created_at: "2024-01-01".into(),
            updated_at: "2024-01-01".into(),
        };
        let s = serde_json::to_string(&wf).unwrap();
        let back: Workflow = serde_json::from_str(&s).unwrap();
        assert_eq!(back.end_node_ids, vec!["b".to_string()]);
        assert_eq!(back.nodes[1].kind, "acceptance");
        assert!(back.nodes[1].acceptance.is_some());
    }

    // ── 阶段二：output_contract 校验测试 ─────────────────────────────────────

    #[test]
    fn validate_output_contract_type_string_pass() {
        let artifact = Value::String("hello".into());
        let schema = json!({"type": "string"});
        assert!(validate_output_contract(&artifact, &schema).is_ok());
    }

    #[test]
    fn validate_output_contract_type_mismatch_fail() {
        let artifact = Value::Number(serde_json::Number::from(42));
        let schema = json!({"type": "string"});
        assert!(validate_output_contract(&artifact, &schema).is_err());
    }

    #[test]
    fn validate_output_contract_integer_vs_number() {
        let int_artifact = Value::Number(serde_json::Number::from(42));
        let float_artifact = Value::Number(serde_json::Number::from_f64(42.5).unwrap());

        // integer schema 只接受整数
        assert!(validate_output_contract(&int_artifact, &json!({"type": "integer"})).is_ok());
        assert!(validate_output_contract(&float_artifact, &json!({"type": "integer"})).is_err());

        // number schema 接受整数和浮点
        assert!(validate_output_contract(&int_artifact, &json!({"type": "number"})).is_ok());
        assert!(validate_output_contract(&float_artifact, &json!({"type": "number"})).is_ok());
    }

    #[test]
    fn validate_output_contract_required_fields() {
        let artifact = json!({"name": "test", "age": 20});
        let schema = json!({
            "type": "object",
            "required": ["name", "age"]
        });
        assert!(validate_output_contract(&artifact, &schema).is_ok());

        let missing = json!({"name": "test"});
        assert!(validate_output_contract(&missing, &schema).is_err());
    }

    #[test]
    fn validate_output_contract_properties_type_check() {
        let artifact = json!({"name": "test", "count": 5});
        let schema = json!({
            "type": "object",
            "properties": {
                "name": {"type": "string"},
                "count": {"type": "integer"}
            }
        });
        assert!(validate_output_contract(&artifact, &schema).is_ok());

        let bad = json!({"name": 123, "count": 5});
        assert!(validate_output_contract(&bad, &schema).is_err());
    }

    #[test]
    fn validate_output_contract_empty_schema_passes() {
        let artifact = Value::String("anything".into());
        let schema = json!({});
        assert!(validate_output_contract(&artifact, &schema).is_ok());
    }

    #[test]
    fn validate_output_contract_array_type() {
        let artifact = json!([1, 2, 3]);
        assert!(validate_output_contract(&artifact, &json!({"type": "array"})).is_ok());
        assert!(validate_output_contract(&artifact, &json!({"type": "string"})).is_err());
    }

    #[test]
    fn validate_output_contract_null_type() {
        let artifact = Value::Null;
        assert!(validate_output_contract(&artifact, &json!({"type": "null"})).is_ok());
        assert!(validate_output_contract(&artifact, &json!({"type": "string"})).is_err());
    }

    // ── 阶段四 P2：DispatchStrategy 测试 ──────────────────────────────────────

    fn make_candidate(id: &str, url: &str, caps: &[&str], priority: u32) -> AgentCandidate {
        AgentCandidate {
            id: id.into(),
            url: url.into(),
            capabilities: caps.iter().map(|s| s.to_string()).collect(),
            priority,
        }
    }

    fn make_dispatch_node(strategy: DispatchStrategy, candidates: Vec<AgentCandidate>, required: Vec<String>) -> WorkflowNode {
        WorkflowNode {
            id: "d1".into(),
            kind: "agent_task".into(),
            label: "dispatch-test".into(),
            server: "http://default:8501/task".into(),
            tool: String::new(),
            arguments: Value::Null,
            prompt: String::new(),
            acceptance: None,
            output_contract: None,
            fan_out: None,
            dispatch: Some(DispatchConfig {
                strategy,
                candidates,
                required_capabilities: required,
                timeout_secs: 15,
            }),
        }
    }

    #[test]
    fn resolve_candidates_fixed_no_dispatch_uses_server() {
        let node = make_node("n1", "agent_task");
        let (cands, strategy, timeout) = resolve_candidates(&node);
        assert_eq!(strategy, DispatchStrategy::Fixed);
        assert_eq!(timeout, 10);
        assert!(cands.is_empty()); // server 为空
    }

    #[test]
    fn resolve_candidates_fixed_with_server() {
        let mut node = make_node("n1", "agent_task");
        node.server = "http://localhost:8501/task".into();
        node.label = "agent-1".into();
        let (cands, strategy, timeout) = resolve_candidates(&node);
        assert_eq!(strategy, DispatchStrategy::Fixed);
        assert_eq!(timeout, 10);
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].url, "http://localhost:8501/task");
    }

    #[test]
    fn resolve_candidates_failover_sorted_by_priority() {
        let node = make_dispatch_node(
            DispatchStrategy::Failover,
            vec![
                make_candidate("a", "http://a", &[], 200),
                make_candidate("b", "http://b", &[], 50),
                make_candidate("c", "http://c", &[], 100),
            ],
            vec![],
        );
        let (cands, strategy, timeout) = resolve_candidates(&node);
        assert_eq!(strategy, DispatchStrategy::Failover);
        assert_eq!(timeout, 15);
        assert_eq!(cands.len(), 3);
        // priority 升序：50 → 100 → 200
        assert_eq!(cands[0].id, "b");
        assert_eq!(cands[1].id, "c");
        assert_eq!(cands[2].id, "a");
    }

    #[test]
    fn resolve_candidates_capability_match_filters() {
        let node = make_dispatch_node(
            DispatchStrategy::CapabilityMatch,
            vec![
                make_candidate("a", "http://a", &["python"], 100),
                make_candidate("b", "http://b", &["python", "web"], 200),
                make_candidate("c", "http://c", &["rust"], 50),
            ],
            vec!["python".into(), "web".into()],
        );
        let (cands, strategy, _) = resolve_candidates(&node);
        assert_eq!(strategy, DispatchStrategy::CapabilityMatch);
        // 只有 b 同时具备 python + web
        assert_eq!(cands.len(), 1);
        assert_eq!(cands[0].id, "b");
    }

    #[test]
    fn resolve_candidates_capability_match_no_match_returns_empty() {
        let node = make_dispatch_node(
            DispatchStrategy::CapabilityMatch,
            vec![
                make_candidate("a", "http://a", &["python"], 100),
            ],
            vec!["rust".into()],
        );
        let (cands, _, _) = resolve_candidates(&node);
        assert!(cands.is_empty());
    }

    #[test]
    fn resolve_candidates_random_preserves_count() {
        let node = make_dispatch_node(
            DispatchStrategy::Random,
            vec![
                make_candidate("a", "http://a", &[], 100),
                make_candidate("b", "http://b", &[], 100),
                make_candidate("c", "http://c", &[], 100),
            ],
            vec![],
        );
        let (cands, strategy, _) = resolve_candidates(&node);
        assert_eq!(strategy, DispatchStrategy::Random);
        assert_eq!(cands.len(), 3);
        // 验证元素集合不变（只是顺序可能变）
        let ids: std::collections::HashSet<&str> = cands.iter().map(|c| c.id.as_str()).collect();
        assert!(ids.contains("a") && ids.contains("b") && ids.contains("c"));
    }

    #[test]
    fn simple_shuffle_preserves_elements() {
        let mut data = vec![1, 2, 3, 4, 5];
        simple_shuffle(&mut data, 42);
        assert_eq!(data.len(), 5);
        let set: std::collections::HashSet<i32> = data.iter().copied().collect();
        assert_eq!(set.len(), 5);
    }

    #[test]
    fn simple_shuffle_deterministic_with_same_seed() {
        let mut a = vec![10, 20, 30, 40, 50];
        let mut b = vec![10, 20, 30, 40, 50];
        simple_shuffle(&mut a, 12345);
        simple_shuffle(&mut b, 12345);
        assert_eq!(a, b);
    }

    #[test]
    fn simple_shuffle_empty_slice() {
        let mut data: Vec<i32> = vec![];
        simple_shuffle(&mut data, 0); // seed=0 → 内部改为 1
        assert!(data.is_empty());
    }

    #[test]
    fn simple_shuffle_single_element() {
        let mut data = vec![42];
        simple_shuffle(&mut data, 0);
        assert_eq!(data, vec![42]);
    }

    #[test]
    fn dispatch_config_serializes_snake_case() {
        let cfg = DispatchConfig {
            strategy: DispatchStrategy::Failover,
            candidates: vec![make_candidate("a", "http://a", &["python"], 100)],
            required_capabilities: vec!["python".into()],
            timeout_secs: 30,
        };
        let s = serde_json::to_string(&cfg).unwrap();
        assert!(s.contains("\"strategy\":\"failover\""));
        assert!(s.contains("\"required_capabilities\""));
        assert!(s.contains("\"timeout_secs\""));
    }
}
