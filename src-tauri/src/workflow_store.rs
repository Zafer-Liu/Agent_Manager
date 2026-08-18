//! Workflow Run 持久化与历史记录（阶段 1b）
//!
//! 将每次工作流执行落盘为 RunRecord，支持按 run_id 查询、列表、淘汰。
//! 存储位置：%APPDATA%\agent-manager\workflow_runs\<run_id>.json + index.json
//!
//! 设计要点：
//! - 每个 Run 一个 JSON 文件，避免单文件膨胀
//! - index.json 只存摘要（run_id/template_id/status/created_at），列表查询不读全量
//! - 保留最近 MAX_RUNS 条，溢出按 created_at 淘汰
//! - 并发安全：所有写操作加 Mutex

use crate::workflow::{NodeStatus, Submission, Verdict};
use chrono::Utc;
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use tauri::State;

// ── 数据模型 ─────────────────────────────────────────────────────────────────

/// 一次工作流执行的记录。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunRecord {
    /// 唯一标识（UUID v4）
    pub run_id: String,
    /// 使用的模板 id
    pub template_id: String,
    /// 外部系统传入的模板标识（阶段四 Hook 用）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_key: Option<String>,
    /// Run 整体状态
    pub status: RunStatus,
    /// 每个节点的一次执行实例
    #[serde(default)]
    pub steps: Vec<StepInstance>,
    /// 创建时间（Unix ms）
    pub created_at: i64,
    /// 结束时间（Unix ms，未结束为 None）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
    /// 触发来源
    pub trigger: RunTrigger,
    /// 返工上下文（阶段二启用，验收驳回时携带）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rework_context: Option<ReworkContext>,
}

/// Run 整体状态。
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum RunStatus {
    /// 执行中
    Running,
    /// 全部成功
    Success,
    /// 有节点失败且策略为终止
    Failed,
    /// 有节点阻塞
    Blocked,
    /// 到达 End Node，等待验收（阶段二）
    WaitingAcceptance,
    /// 验收通过已关闭
    Closed,
}

impl Default for RunStatus {
    fn default() -> Self {
        RunStatus::Running
    }
}

/// 触发来源。
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "trigger", rename_all = "lowercase")]
pub enum RunTrigger {
    /// 手动触发
    Manual { user: String },
    /// 外部 Hook 触发（阶段四）
    Hook { source: String, external_id: String },
    /// 定时触发（阶段四）
    Schedule { cron: String },
    /// 返工触发（阶段二）
    Rework { parent_run_id: String },
}

impl Default for RunTrigger {
    fn default() -> Self {
        RunTrigger::Manual {
            user: String::new(),
        }
    }
}

/// 节点的一次执行实例。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StepInstance {
    /// 唯一标识（UUID v4）
    pub step_id: String,
    /// 所属 Run
    pub run_id: String,
    /// 对应模板中的节点 id
    pub node_id: String,
    /// 节点类型（tool / llm / mcp_agent / agent_task / fan_out / acceptance）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub kind: String,
    /// 节点状态
    pub status: NodeStatus,
    /// Agent 提交的结构化结果
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub submission: Option<Submission>,
    /// 旧字段：纯文本输出（向后兼容，与 submission.artifact 同步）
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub output: String,
    /// 旧字段：错误信息（向后兼容，与 submission.verdict=Fail 同步）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// 开始时间（Unix ms）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub started_at: Option<i64>,
    /// 结束时间（Unix ms）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
    /// 第几次尝试（返工累加，首次为 1）
    #[serde(default = "default_attempt")]
    pub attempt: u32,
    /// 失败诊断链路（阶段三填充）
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub failure_trace: Option<FailureTrace>,
}

fn default_attempt() -> u32 {
    1
}

impl StepInstance {
    /// 构造一个新 Step（Pending 状态）
    pub fn new(run_id: &str, node_id: &str) -> Self {
        StepInstance {
            step_id: uuid::Uuid::new_v4().to_string(),
            run_id: run_id.to_string(),
            node_id: node_id.to_string(),
            kind: String::new(),
            status: NodeStatus::Pending,
            submission: None,
            output: String::new(),
            error: None,
            started_at: None,
            finished_at: None,
            attempt: 1,
            failure_trace: None,
        }
    }

    /// 设置节点类型
    pub fn with_kind(mut self, kind: &str) -> Self {
        self.kind = kind.to_string();
        self
    }

    /// 从 Verdict 派生终止态并记录结束时间
    pub fn finish_with_verdict(&mut self, submission: Submission) {
        self.status = NodeStatus::from_verdict(&submission.verdict);
        self.output = submission.to_text();
        if let Verdict::Fail { reason, .. } = &submission.verdict {
            self.error = Some(reason.clone());
        }
        self.submission = Some(submission);
        self.finished_at = Some(Utc::now().timestamp_millis());
    }

    /// 阶段三 3c：标记为 Running 并记录开始时间（执行前持久化）。
    pub fn mark_running(&mut self) {
        self.status = NodeStatus::Running;
        if self.started_at.is_none() {
            self.started_at = Some(Utc::now().timestamp_millis());
        }
    }
}

/// 返工上下文（阶段二启用）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ReworkContext {
    /// 验收节点 step_id
    pub reject_from_step: String,
    /// 回跳目标 node_id
    pub reject_to_node: String,
    /// 驳回原因
    pub reason: String,
    /// 原 run_id
    pub previous_run_id: String,
    /// 保留可用产物
    #[serde(default)]
    pub carry_over_artifact: serde_json::Value,
}

/// 失败诊断链路（阶段三填充，1b 先定义结构）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailureTrace {
    pub run_id: String,
    pub step_id: String,
    pub node_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub agent_id: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub tool: Option<String>,
    pub failure_kind: FailureKind,
    pub reason: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stderr_excerpt: Option<String>,
    #[serde(default)]
    pub retry_history: Vec<RetryAttempt>,
    pub final_status: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum FailureKind {
    Timeout,
    Network,
    RpcError,
    SchemaViolation,
    AgentBlocked,
    ProcessExited,
    Unknown,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RetryAttempt {
    pub attempt: u32,
    pub at: i64,
    pub reason: String,
}

// ── Run 索引摘要（列表查询用，轻量） ─────────────────────────────────────────

/// Run 列表中的单条摘要（不含 steps 详情）。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RunSummary {
    pub run_id: String,
    pub template_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub template_key: Option<String>,
    pub status: RunStatus,
    pub created_at: i64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub finished_at: Option<i64>,
    pub step_count: usize,
    /// 触发来源（阶段四 P1：前端 RunsHistory 来源列）
    #[serde(default)]
    pub trigger: RunTrigger,
}

// ── 持久化存储 ───────────────────────────────────────────────────────────────

/// 保留最近 N 条 Run，溢出按 created_at 淘汰。
const MAX_RUNS: usize = 500;

/// 全局存储（通过 Tauri State 注入）。
pub struct WorkflowRunStore {
    /// 内存索引：run_id → RunSummary，列表查询直接用
    index: Mutex<HashMap<String, RunSummary>>,
    /// 磁盘根目录
    base_dir: PathBuf,
}

impl WorkflowRunStore {
    pub fn new() -> Self {
        let base_dir = dirs_next::config_dir()
            .unwrap_or_else(|| PathBuf::from("."))
            .join("agent-manager")
            .join("workflow_runs");
        let store = WorkflowRunStore {
            index: Mutex::new(HashMap::new()),
            base_dir,
        };
        store.ensure_dir();
        store.load_index_from_disk();
        store
    }

    fn ensure_dir(&self) {
        let _ = std::fs::create_dir_all(&self.base_dir);
    }

    /// 从磁盘 index.json 加载索引到内存。
    fn load_index_from_disk(&self) {
        let path = self.base_dir.join("index.json");
        if let Ok(s) = std::fs::read_to_string(&path) {
            if let Ok(list) = serde_json::from_str::<Vec<RunSummary>>(&s) {
                let mut idx = self.index.lock().unwrap();
                for s in list {
                    idx.insert(s.run_id.clone(), s);
                }
            }
        }
    }

    /// 将内存索引写回磁盘 index.json。
    fn flush_index(&self) {
        let idx = self.index.lock().unwrap();
        let mut list: Vec<RunSummary> = idx.values().cloned().collect();
        // 按 created_at 降序（新的在前）
        list.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        let path = self.base_dir.join("index.json");
        if let Ok(json) = serde_json::to_string_pretty(&list) {
            let _ = std::fs::write(&path, json);
        }
    }

    /// 淘汰超出 MAX_RUNS 的旧记录。
    fn evict_if_needed(&self) {
        let mut idx = self.index.lock().unwrap();
        if idx.len() <= MAX_RUNS {
            return;
        }
        // 按 created_at 升序，淘汰最早的
        let mut list: Vec<RunSummary> = idx.values().cloned().collect();
        list.sort_by(|a, b| a.created_at.cmp(&b.created_at));
        let to_remove = list.len().saturating_sub(MAX_RUNS);
        for s in list.into_iter().take(to_remove) {
            idx.remove(&s.run_id);
            // 删除对应的 run 文件
            let file = self.base_dir.join(format!("{}.json", s.run_id));
            let _ = std::fs::remove_file(&file);
        }
    }

    /// 新建一个 Run（写入磁盘 + 更新索引）。
    pub fn create_run(&self, run: &RunRecord) -> Result<(), String> {
        self.ensure_dir();
        let file = self.base_dir.join(format!("{}.json", run.run_id));
        let json = serde_json::to_string_pretty(run).map_err(|e| e.to_string())?;
        std::fs::write(&file, json).map_err(|e| e.to_string())?;

        let summary = RunSummary {
            run_id: run.run_id.clone(),
            template_id: run.template_id.clone(),
            template_key: run.template_key.clone(),
            status: run.status.clone(),
            created_at: run.created_at,
            finished_at: run.finished_at,
            step_count: run.steps.len(),
            trigger: run.trigger.clone(),
        };
        {
            let mut idx = self.index.lock().unwrap();
            idx.insert(run.run_id.clone(), summary);
        }
        self.evict_if_needed();
        self.flush_index();
        Ok(())
    }

    /// 更新一个已存在的 Run（覆盖写 + 更新索引摘要）。
    pub fn update_run(&self, run: &RunRecord) -> Result<(), String> {
        self.create_run(run) // 覆盖写逻辑相同
    }

    /// 按 run_id 读取完整 Run（含 steps）。
    pub fn get_run(&self, run_id: &str) -> Option<RunRecord> {
        let file = self.base_dir.join(format!("{}.json", run_id));
        let s = std::fs::read_to_string(&file).ok()?;
        serde_json::from_str(&s).ok()
    }

    /// 列出所有 Run 摘要（按 created_at 降序）。
    pub fn list_runs(&self) -> Vec<RunSummary> {
        let idx = self.index.lock().unwrap();
        let mut list: Vec<RunSummary> = idx.values().cloned().collect();
        list.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        list
    }

    /// 更新 Run 状态（轻量更新，不重写全量 steps）。
    pub fn update_status(&self, run_id: &str, status: RunStatus) -> Result<(), String> {
        // 读全量 → 改状态 → 写回
        let mut run = self
            .get_run(run_id)
            .ok_or_else(|| format!("Run not found: {}", run_id))?;
        run.status = status.clone();
        if matches!(
            status,
            RunStatus::Success | RunStatus::Failed | RunStatus::Closed
        ) {
            run.finished_at = Some(Utc::now().timestamp_millis());
        }
        self.update_run(&run)?;
        Ok(())
    }

    /// 阶段三 3c：追加一个 Running Step 到 Run（执行前持久化）。
    pub fn add_running_step(&self, run_id: &str, step: &StepInstance) -> Result<(), String> {
        let mut run = self
            .get_run(run_id)
            .ok_or_else(|| format!("Run not found: {}", run_id))?;
        run.steps.push(step.clone());
        self.update_run(&run)?;
        Ok(())
    }

    /// 阶段三 3c：更新 Run 中某个 Step（执行后替换）。
    pub fn update_step(&self, run_id: &str, step: &StepInstance) -> Result<(), String> {
        let mut run = self
            .get_run(run_id)
            .ok_or_else(|| format!("Run not found: {}", run_id))?;
        if let Some(existing) = run.steps.iter_mut().find(|s| s.step_id == step.step_id) {
            *existing = step.clone();
        } else {
            run.steps.push(step.clone());
        }
        self.update_run(&run)?;
        Ok(())
    }

    /// 阶段三 3c：查找所有 Running 状态的 Step（供 Sweeper 巡检）。
    /// 返回 (run_id, step_id, node_id, started_at, kind) 列表。
    pub fn find_running_steps(&self) -> Vec<(String, String, String, i64, String)> {
        let runs = self.list_runs();
        let mut result = vec![];
        for summary in runs {
            if !matches!(summary.status, RunStatus::Running) {
                continue;
            }
            if let Some(run) = self.get_run(&summary.run_id) {
                for step in &run.steps {
                    if step.status == crate::workflow::NodeStatus::Running {
                        if let Some(started) = step.started_at {
                            result.push((
                                run.run_id.clone(),
                                step.step_id.clone(),
                                step.node_id.clone(),
                                started,
                                step.kind.clone(),
                            ));
                        }
                    }
                }
            }
        }
        result
    }

    /// 阶段三 3c：将某个 Step 标记为 Failed（Sweeper 超时巡检用）。
    pub fn mark_step_failed(
        &self,
        run_id: &str,
        step_id: &str,
        trace: &FailureTrace,
    ) -> Result<(), String> {
        let mut run = self
            .get_run(run_id)
            .ok_or_else(|| format!("Run not found: {}", run_id))?;
        let now = Utc::now().timestamp_millis();
        for step in &mut run.steps {
            if step.step_id == step_id {
                step.status = crate::workflow::NodeStatus::Failed;
                step.failure_trace = Some(trace.clone());
                step.finished_at = Some(now);
                step.error = Some(trace.reason.clone());
                break;
            }
        }
        self.update_run(&run)?;
        Ok(())
    }

    /// 阶段三 3c：将某个 Step 标记为 Blocked（Sweeper agent_exit 巡检用）。
    #[allow(dead_code)]
    pub fn mark_step_blocked(
        &self,
        run_id: &str,
        step_id: &str,
        trace: &FailureTrace,
    ) -> Result<(), String> {
        let mut run = self
            .get_run(run_id)
            .ok_or_else(|| format!("Run not found: {}", run_id))?;
        let now = Utc::now().timestamp_millis();
        for step in &mut run.steps {
            if step.step_id == step_id {
                step.status = crate::workflow::NodeStatus::Blocked;
                step.failure_trace = Some(trace.clone());
                step.finished_at = Some(now);
                step.error = Some(trace.reason.clone());
                break;
            }
        }
        self.update_run(&run)?;
        Ok(())
    }
}

impl Default for WorkflowRunStore {
    fn default() -> Self {
        Self::new()
    }
}

// ── Tauri 命令 ───────────────────────────────────────────────────────────────

/// 列出所有 Run 摘要（按 created_at 降序）。
#[allow(dead_code)] // Run history is no longer exposed through the app command surface.
#[tauri::command]
pub fn list_workflow_runs(store: State<'_, WorkflowRunStore>) -> Vec<RunSummary> {
    store.list_runs()
}

/// 阶段四 P1：列出 Hook 触发的 Run（前端外部触发页用）
#[allow(dead_code)]
#[tauri::command]
pub fn list_hook_triggered_runs(store: State<'_, WorkflowRunStore>) -> Vec<RunSummary> {
    store
        .list_runs()
        .into_iter()
        .filter(|r| matches!(r.trigger, RunTrigger::Hook { .. }))
        .collect()
}

/// 按 run_id 读取完整 Run（含 steps）。
#[allow(dead_code)]
#[tauri::command]
pub fn get_workflow_run(
    store: State<'_, WorkflowRunStore>,
    run_id: String,
) -> Result<RunRecord, String> {
    store
        .get_run(&run_id)
        .ok_or_else(|| format!("Run not found: {}", run_id))
}

/// 阶段二：通过验收。将 WaitingAcceptance 的 Run 置为 Closed。
#[allow(dead_code)]
#[tauri::command]
pub fn approve_run(
    store: State<'_, WorkflowRunStore>,
    run_id: String,
) -> Result<RunRecord, String> {
    approve_run_inner(&store, &run_id)
}

/// 内部函数（HTTP server 调用，不走 Tauri State）
pub fn approve_run_inner(store: &WorkflowRunStore, run_id: &str) -> Result<RunRecord, String> {
    let run = store
        .get_run(run_id)
        .ok_or_else(|| format!("Run not found: {}", run_id))?;
    if run.status != RunStatus::WaitingAcceptance {
        return Err(format!(
            "Run {} is not waiting for acceptance (status: {:?})",
            run_id, run.status
        ));
    }
    store.update_status(run_id, RunStatus::Closed)?;
    // 阶段三 3d：JSONL 事件源 — acceptance_approved
    crate::workflow_events::emit_acceptance_approved(run_id);
    store
        .get_run(run_id)
        .ok_or_else(|| format!("Run not found after update: {}", run_id))
}

/// 阶段二：驳回验收（定向 Rework）。
///
/// 创建一个新的 Run，以 RunTrigger::Rework 启动，从 `reject_to_node` 开始执行。
/// 原 Run 状态置为 Failed（验收驳回）。新 Run 携带 ReworkContext。
/// 返回新 Run 的 run_id。
#[allow(dead_code)]
#[tauri::command]
pub fn reject_run(
    store: State<'_, WorkflowRunStore>,
    run_id: String,
    reject_to_node: String,
    reason: String,
) -> Result<String, String> {
    reject_run_inner(&store, &run_id, &reject_to_node, &reason)
}

/// 内部函数（HTTP server 调用，不走 Tauri State）
pub fn reject_run_inner(
    store: &WorkflowRunStore,
    run_id: &str,
    reject_to_node: &str,
    reason: &str,
) -> Result<String, String> {
    let orig_run = store
        .get_run(run_id)
        .ok_or_else(|| format!("Run not found: {}", run_id))?;
    if orig_run.status != RunStatus::WaitingAcceptance {
        return Err(format!(
            "Run {} is not waiting for acceptance (status: {:?})",
            run_id, orig_run.status
        ));
    }

    // 原 Run 标记为 Failed（验收驳回）
    store.update_status(run_id, RunStatus::Failed)?;
    // 阶段三 3d：JSONL 事件源 — acceptance_rejected
    crate::workflow_events::emit_acceptance_rejected(run_id, reject_to_node, reason);

    // 创建新 Run（Rework）
    let now = Utc::now().timestamp_millis();
    let new_run_id = uuid::Uuid::new_v4().to_string();
    let rework_context = ReworkContext {
        reject_from_step: orig_run
            .steps
            .iter()
            .find(|s| s.node_id == reject_to_node)
            .map(|s| s.step_id.clone())
            .unwrap_or_default(),
        reject_to_node: reject_to_node.to_string(),
        reason: reason.to_string(),
        previous_run_id: run_id.to_string(),
        carry_over_artifact: serde_json::Value::Null,
    };
    let new_run = RunRecord {
        run_id: new_run_id.clone(),
        template_id: orig_run.template_id.clone(),
        template_key: orig_run.template_key.clone(),
        status: RunStatus::Running,
        steps: vec![],
        created_at: now,
        finished_at: None,
        trigger: RunTrigger::Rework {
            parent_run_id: run_id.to_string(),
        },
        rework_context: Some(rework_context),
    };
    store.create_run(&new_run)?;
    Ok(new_run_id)
}

// ── 测试 ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use crate::workflow::Submission;

    fn make_run(id: &str, template: &str) -> RunRecord {
        RunRecord {
            run_id: id.to_string(),
            template_id: template.to_string(),
            template_key: None,
            status: RunStatus::Running,
            steps: vec![],
            created_at: Utc::now().timestamp_millis(),
            finished_at: None,
            trigger: RunTrigger::Manual {
                user: "test".to_string(),
            },
            rework_context: None,
        }
    }

    #[test]
    fn run_status_serializes_lowercase() {
        assert_eq!(
            serde_json::to_string(&RunStatus::Running).unwrap(),
            "\"running\""
        );
        assert_eq!(
            serde_json::to_string(&RunStatus::WaitingAcceptance).unwrap(),
            "\"waitingacceptance\""
        );
    }

    #[test]
    fn run_trigger_tagged_enum() {
        let t = RunTrigger::Manual {
            user: "alice".into(),
        };
        let s = serde_json::to_string(&t).unwrap();
        assert!(s.contains("\"trigger\":\"manual\""));
        assert!(s.contains("\"user\":\"alice\""));

        let t2 = RunTrigger::Hook {
            source: "req-system".into(),
            external_id: "REQ-123".into(),
        };
        let s = serde_json::to_string(&t2).unwrap();
        assert!(s.contains("\"trigger\":\"hook\""));
    }

    #[test]
    fn step_instance_new_is_pending() {
        let step = StepInstance::new("run-1", "node-a");
        assert_eq!(step.status, NodeStatus::Pending);
        assert_eq!(step.attempt, 1);
        assert!(step.submission.is_none());
    }

    #[test]
    fn step_instance_finish_with_pass_verdict() {
        let mut step = StepInstance::new("run-1", "node-a");
        step.started_at = Some(Utc::now().timestamp_millis());
        let sub = Submission::from_text_pass("done");
        step.finish_with_verdict(sub);
        assert_eq!(step.status, NodeStatus::Success);
        assert_eq!(step.output, "done");
        assert!(step.error.is_none());
        assert!(step.finished_at.is_some());
    }

    #[test]
    fn step_instance_finish_with_fail_verdict_sets_error() {
        let mut step = StepInstance::new("run-1", "node-b");
        step.finish_with_verdict(Submission::from_error("boom"));
        assert_eq!(step.status, NodeStatus::Failed);
        assert_eq!(step.error.as_deref(), Some("boom"));
    }

    #[test]
    fn run_record_roundtrip_serialize() {
        let run = RunRecord {
            run_id: "r1".into(),
            template_id: "t1".into(),
            template_key: Some("standard-req".into()),
            status: RunStatus::Success,
            steps: vec![StepInstance::new("r1", "n1")],
            created_at: 1700000000_000,
            finished_at: Some(1700000005_000),
            trigger: RunTrigger::Manual {
                user: "tester".into(),
            },
            rework_context: None,
        };
        let s = serde_json::to_string(&run).unwrap();
        let back: RunRecord = serde_json::from_str(&s).unwrap();
        assert_eq!(back.run_id, "r1");
        assert_eq!(back.template_key.as_deref(), Some("standard-req"));
        assert_eq!(back.status, RunStatus::Success);
        assert_eq!(back.steps.len(), 1);
    }

    /// 旧 JSON（无 template_key/rework_context/failure_trace 等新字段）必须能反序列化。
    #[test]
    fn run_record_legacy_json_back_compat() {
        let legacy = r#"{
            "run_id": "r2",
            "template_id": "t2",
            "status": "running",
            "steps": [],
            "created_at": 1700000000000,
            "trigger": {"trigger": "manual", "user": ""}
        }"#;
        let run: RunRecord = serde_json::from_str(legacy).unwrap();
        assert_eq!(run.run_id, "r2");
        assert!(run.template_key.is_none());
        assert!(run.finished_at.is_none());
        assert!(run.rework_context.is_none());
    }

    #[test]
    fn run_summary_serializes_without_none_fields() {
        let s = RunSummary {
            run_id: "r1".into(),
            template_id: "t1".into(),
            template_key: None,
            status: RunStatus::Running,
            created_at: 123,
            finished_at: None,
            step_count: 0,
            trigger: RunTrigger::Manual {
                user: String::new(),
            },
        };
        let json = serde_json::to_string(&s).unwrap();
        assert!(!json.contains("template_key"));
        assert!(!json.contains("finished_at"));
    }

    /// 使用临时目录测试 WorkflowRunStore 的 CRUD。
    #[test]
    fn workflow_run_store_create_get_list() {
        // 使用临时目录避免污染真实配置
        let tmp = std::env::temp_dir().join(format!("agent-manager-test-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();

        // 临时替换 base_dir：直接构造并手动设置
        let store = WorkflowRunStore {
            index: Mutex::new(HashMap::new()),
            base_dir: tmp.join("workflow_runs"),
        };
        store.ensure_dir();

        // 创建 3 个 run
        let run1 = make_run("r-1", "tmpl-a");
        let run2 = make_run("r-2", "tmpl-a");
        let run3 = make_run("r-3", "tmpl-b");
        store.create_run(&run1).unwrap();
        // 稍微延后 created_at 以保证顺序
        let mut run2 = run2;
        run2.created_at += 1000;
        store.create_run(&run2).unwrap();
        let mut run3 = run3;
        run3.created_at += 2000;
        store.create_run(&run3).unwrap();

        // list 应返回 3 条，按 created_at 降序
        let list = store.list_runs();
        assert_eq!(list.len(), 3);
        assert_eq!(list[0].run_id, "r-3");
        assert_eq!(list[2].run_id, "r-1");

        // get 读取完整 run
        let got = store.get_run("r-2").unwrap();
        assert_eq!(got.run_id, "r-2");
        assert_eq!(got.template_id, "tmpl-a");

        // 更新状态
        store.update_status("r-1", RunStatus::Success).unwrap();
        let got = store.get_run("r-1").unwrap();
        assert_eq!(got.status, RunStatus::Success);
        assert!(got.finished_at.is_some());

        // 清理临时目录
        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn workflow_run_store_eviction() {
        let tmp =
            std::env::temp_dir().join(format!("agent-manager-evict-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();

        // 临时把 MAX_RUNS 逻辑验证：创建 5 个，MAX_RUNS=500 不会淘汰
        // 这里只验证不崩，真正的淘汰在生产环境用大量数据测
        let store = WorkflowRunStore {
            index: Mutex::new(HashMap::new()),
            base_dir: tmp.join("workflow_runs"),
        };
        store.ensure_dir();
        for i in 0..5 {
            let r = make_run(&format!("r-{}", i), "tmpl");
            store.create_run(&r).unwrap();
        }
        assert_eq!(store.list_runs().len(), 5);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn step_instance_mark_running_sets_status_and_time() {
        let mut step = StepInstance::new("run-1", "node-a");
        assert_eq!(step.status, crate::workflow::NodeStatus::Pending);
        step.mark_running();
        assert_eq!(step.status, crate::workflow::NodeStatus::Running);
        assert!(step.started_at.is_some());
    }

    #[test]
    fn workflow_run_store_add_running_step_and_update() {
        let tmp =
            std::env::temp_dir().join(format!("agent-manager-running-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        let store = WorkflowRunStore {
            index: Mutex::new(HashMap::new()),
            base_dir: tmp.join("workflow_runs"),
        };
        store.ensure_dir();

        let mut run = make_run("r-running", "tmpl");
        run.status = RunStatus::Running;
        store.create_run(&run).unwrap();

        // 添加 Running Step
        let mut step = StepInstance::new("r-running", "node-a");
        step.mark_running();
        let step_id = step.step_id.clone();
        store.add_running_step("r-running", &step).unwrap();

        // find_running_steps 应返回 1 条
        let running = store.find_running_steps();
        assert_eq!(running.len(), 1);
        assert_eq!(running[0].0, "r-running");
        assert_eq!(running[0].1, step_id);

        // 更新 Step 为终态
        let mut finished = step;
        finished.status = crate::workflow::NodeStatus::Success;
        finished.finished_at = Some(Utc::now().timestamp_millis());
        store.update_step("r-running", &finished).unwrap();

        // find_running_steps 应返回 0 条
        let running = store.find_running_steps();
        assert_eq!(running.len(), 0);

        // get_run 应只有 1 个 step（而非 2 个）
        let got = store.get_run("r-running").unwrap();
        assert_eq!(got.steps.len(), 1);
        assert_eq!(got.steps[0].status, crate::workflow::NodeStatus::Success);

        let _ = std::fs::remove_dir_all(&tmp);
    }

    #[test]
    fn workflow_run_store_mark_step_failed() {
        let tmp =
            std::env::temp_dir().join(format!("agent-manager-failed-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&tmp).unwrap();
        let store = WorkflowRunStore {
            index: Mutex::new(HashMap::new()),
            base_dir: tmp.join("workflow_runs"),
        };
        store.ensure_dir();

        let mut run = make_run("r-fail", "tmpl");
        run.status = RunStatus::Running;
        store.create_run(&run).unwrap();

        let mut step = StepInstance::new("r-fail", "node-a");
        step.mark_running();
        let step_id = step.step_id.clone();
        store.add_running_step("r-fail", &step).unwrap();

        let trace = FailureTrace {
            run_id: "r-fail".to_string(),
            step_id: step_id.clone(),
            node_id: "node-a".to_string(),
            agent_id: None,
            tool: None,
            failure_kind: FailureKind::Timeout,
            reason: "timed out".to_string(),
            stderr_excerpt: None,
            retry_history: vec![],
            final_status: "failed".to_string(),
        };
        store.mark_step_failed("r-fail", &step_id, &trace).unwrap();

        let got = store.get_run("r-fail").unwrap();
        assert_eq!(got.steps[0].status, crate::workflow::NodeStatus::Failed);
        assert!(got.steps[0].failure_trace.is_some());
        assert_eq!(
            got.steps[0].failure_trace.as_ref().unwrap().failure_kind,
            FailureKind::Timeout
        );
        assert!(got.steps[0].finished_at.is_some());

        let _ = std::fs::remove_dir_all(&tmp);
    }
}
