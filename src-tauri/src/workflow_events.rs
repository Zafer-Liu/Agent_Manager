//! 阶段三 3d：workflow_events.jsonl 事件源
//!
//! Append-only JSONL 写入器，将工作流执行过程中的 8 类事件落盘到
//! `workflow_events.jsonl`，供事后审计、失败回溯和 Metrics 聚合使用。
//!
//! 事件类型：
//! 1. `run_started`     — 工作流启动
//! 2. `step_started`    — 节点开始执行（Running 持久化）
//! 3. `step_finished`   — 节点执行完成（Success/Failed/Blocked）
//! 4. `run_finished`    — 工作流结束（含最终 status）
//! 5. `acceptance_requested` — 验收节点触发
//! 6. `acceptance_approved`  — 验收通过
//! 7. `acceptance_rejected`  — 验收驳回（含 rework 信息）
//! 8. `sweeper_recovered`    — Sweeper 巡检恢复（timeout/agent_exit）
//!
//! 文件位置：`<data_dir>/agent-manager/workflow_events.jsonl`
//! 格式：每行一个 JSON 对象，含 `ts`（Unix ms）、`type`、`run_id`、及事件特有字段。

use serde::Serialize;
use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs::OpenOptions;
use std::io::Write;
use std::sync::Mutex;

/// 单例写入器（进程级，所有工作流共享一个文件）。
static EVENT_WRITER: Mutex<Option<EventWriter>> = Mutex::new(None);

/// 事件写入器：持有文件路径，append-only 写入。
struct EventWriter {
    path: std::path::PathBuf,
}

impl EventWriter {
    fn new() -> Self {
        let dir = dirs_next::data_dir()
            .unwrap_or_else(|| std::path::PathBuf::from("."))
            .join("agent-manager");
        let _ = std::fs::create_dir_all(&dir);
        let path = dir.join("workflow_events.jsonl");
        EventWriter { path }
    }

    fn append(&self, event: &Value) {
        let mut line = serde_json::to_string(event).unwrap_or_else(|_| "{}".to_string());
        line.push('\n');
        if let Ok(mut f) = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
        {
            let _ = f.write_all(line.as_bytes());
        }
    }
}

/// 初始化事件写入器（在 lib.rs setup 中调用）。
pub fn init_event_writer() {
    let mut guard = EVENT_WRITER.lock().unwrap();
    if guard.is_none() {
        *guard = Some(EventWriter::new());
    }
}

/// 内部：写入一条事件。
fn write_event(event_type: &str, run_id: &str, payload: Value) {
    let ts = chrono::Utc::now().timestamp_millis();
    let event = json!({
        "ts": ts,
        "type": event_type,
        "run_id": run_id,
        "payload": payload,
    });
    let guard = EVENT_WRITER.lock().unwrap();
    if let Some(ref writer) = *guard {
        writer.append(&event);
    }
}

// ── 公共 API ─────────────────────────────────────────────────────────────────

/// 工作流启动事件。
pub fn emit_run_started(run_id: &str, template_id: &str, created_at: i64) {
    write_event(
        "run_started",
        run_id,
        json!({ "template_id": template_id, "created_at": created_at }),
    );
}

/// 节点开始执行事件。
pub fn emit_step_started(run_id: &str, step_id: &str, node_id: &str, kind: &str) {
    write_event(
        "step_started",
        run_id,
        json!({ "step_id": step_id, "node_id": node_id, "kind": kind }),
    );
}

/// 节点执行完成事件。
pub fn emit_step_finished(
    run_id: &str,
    step_id: &str,
    node_id: &str,
    status: &str,
    duration_ms: i64,
    has_trace: bool,
) {
    write_event(
        "step_finished",
        run_id,
        json!({
            "step_id": step_id,
            "node_id": node_id,
            "status": status,
            "duration_ms": duration_ms,
            "has_trace": has_trace,
        }),
    );
}

/// 工作流结束事件。
pub fn emit_run_finished(run_id: &str, status: &str, duration_ms: i64, step_count: usize) {
    write_event(
        "run_finished",
        run_id,
        json!({ "status": status, "duration_ms": duration_ms, "step_count": step_count }),
    );
}

/// 验收请求事件。
pub fn emit_acceptance_requested(run_id: &str, step_id: &str, node_id: &str) {
    write_event(
        "acceptance_requested",
        run_id,
        json!({ "step_id": step_id, "node_id": node_id }),
    );
}

/// 验收通过事件。
pub fn emit_acceptance_approved(run_id: &str) {
    write_event("acceptance_approved", run_id, json!({}));
}

/// 验收驳回事件。
pub fn emit_acceptance_rejected(run_id: &str, reject_to_node: &str, reason: &str) {
    write_event(
        "acceptance_rejected",
        run_id,
        json!({ "reject_to_node": reject_to_node, "reason": reason }),
    );
}

/// Sweeper 巡检恢复事件。
pub fn emit_sweeper_recovered(run_id: &str, step_id: &str, reason: &str, failure_kind: &str) {
    write_event(
        "sweeper_recovered",
        run_id,
        json!({
            "step_id": step_id,
            "reason": reason,
            "failure_kind": failure_kind,
        }),
    );
}

// ── 事件读取（供 Metrics 聚合和前端审计使用） ───────────────────────────────

/// 读取所有事件（按时间顺序）。
pub fn read_all_events() -> Vec<Value> {
    let dir = dirs_next::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("agent-manager");
    let path = dir.join("workflow_events.jsonl");
    let content = match std::fs::read_to_string(&path) {
        Ok(c) => c,
        Err(_) => return vec![],
    };
    content
        .lines()
        .filter(|l| !l.is_empty())
        .filter_map(|l| serde_json::from_str(l).ok())
        .collect()
}

/// Tauri 命令：读取工作流事件列表（供前端审计面板使用）。
#[tauri::command]
pub fn list_workflow_events() -> Vec<Value> {
    read_all_events()
}

// ── Metrics 聚合（阶段三 3f） ───────────────────────────────────────────────

/// 四卡片指标数据。
#[derive(Debug, Clone, Serialize)]
pub struct WorkflowMetrics {
    /// 完成率（success / total_runs * 100）
    pub success_rate: f64,
    /// 总运行数
    pub total_runs: usize,
    /// 成功运行数
    pub success_runs: usize,
    /// 平均耗时（ms，仅成功 run）
    pub avg_duration_ms: f64,
    /// Top 失败节点（按出现次数降序，最多 5 个）
    pub top_failed_nodes: Vec<TopFailedNode>,
    /// 返工率（acceptance_rejected / acceptance_requested * 100）
    pub rework_rate: f64,
    /// 验收请求总数
    pub acceptance_total: usize,
    /// 验收驳回数
    pub acceptance_rejected: usize,
}

#[derive(Debug, Clone, Serialize)]
pub struct TopFailedNode {
    pub node_id: String,
    pub fail_count: usize,
    pub failure_kind: String,
}

/// Tauri 命令：聚合 Metrics 四卡片数据。
#[tauri::command]
pub fn get_workflow_metrics() -> WorkflowMetrics {
    let events = read_all_events();

    let mut total_runs = 0usize;
    let mut success_runs = 0usize;
    let mut durations: Vec<i64> = vec![];
    let mut acceptance_total = 0usize;
    let mut acceptance_rejected = 0usize;
    let mut fail_counts: HashMap<String, (usize, String)> = HashMap::new();

    for event in &events {
        let event_type = event["type"].as_str().unwrap_or("");
        match event_type {
            "run_finished" => {
                total_runs += 1;
                let status = event["payload"]["status"].as_str().unwrap_or("");
                if status == "success" {
                    success_runs += 1;
                }
                if let Some(d) = event["payload"]["duration_ms"].as_i64() {
                    if status == "success" {
                        durations.push(d);
                    }
                }
            }
            "step_finished" => {
                let status = event["payload"]["status"].as_str().unwrap_or("");
                if status == "failed" {
                    let node_id = event["payload"]["node_id"].as_str().unwrap_or("").to_string();
                    let entry = fail_counts.entry(node_id.clone()).or_insert((0, "unknown".to_string()));
                    entry.0 += 1;
                }
            }
            "sweeper_recovered" => {
                // sweeper_recovered 也计入失败节点
                let step_id = event["payload"]["step_id"].as_str().unwrap_or("").to_string();
                let failure_kind = event["payload"]["failure_kind"].as_str().unwrap_or("unknown").to_string();
                let entry = fail_counts.entry(step_id).or_insert((0, failure_kind.clone()));
                entry.0 += 1;
                entry.1 = failure_kind;
            }
            "acceptance_requested" => {
                acceptance_total += 1;
            }
            "acceptance_rejected" => {
                acceptance_rejected += 1;
            }
            _ => {}
        }
    }

    let success_rate = if total_runs > 0 {
        success_runs as f64 / total_runs as f64 * 100.0
    } else {
        0.0
    };

    let avg_duration_ms = if durations.is_empty() {
        0.0
    } else {
        durations.iter().sum::<i64>() as f64 / durations.len() as f64
    };

    let rework_rate = if acceptance_total > 0 {
        acceptance_rejected as f64 / acceptance_total as f64 * 100.0
    } else {
        0.0
    };

    let mut top_failed: Vec<TopFailedNode> = fail_counts
        .into_iter()
        .map(|(node_id, (count, kind))| TopFailedNode {
            node_id,
            fail_count: count,
            failure_kind: kind,
        })
        .collect();
    top_failed.sort_by(|a, b| b.fail_count.cmp(&a.fail_count));
    top_failed.truncate(5);

    WorkflowMetrics {
        success_rate,
        total_runs,
        success_runs,
        avg_duration_ms,
        top_failed_nodes: top_failed,
        rework_rate,
        acceptance_total,
        acceptance_rejected,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn emit_and_read_events() {
        // 初始化到临时目录
        let tmp = std::env::temp_dir().join(format!(
            "agent-manager-events-{}",
            uuid::Uuid::new_v4()
        ));
        std::fs::create_dir_all(&tmp).unwrap();

        // 手动覆盖 EVENT_WRITER 的路径
        {
            let mut guard = EVENT_WRITER.lock().unwrap();
            *guard = Some(EventWriter {
                path: tmp.join("workflow_events.jsonl"),
            });
        }

        emit_run_started("r-1", "tmpl-a", 1000);
        emit_step_started("r-1", "s-1", "node-a", "tool");
        emit_step_finished("r-1", "s-1", "node-a", "success", 500, false);
        emit_run_finished("r-1", "success", 500, 1);
        emit_acceptance_requested("r-1", "s-1", "node-a");
        emit_acceptance_approved("r-1");
        emit_acceptance_rejected("r-1", "node-b", "quality issue");
        emit_sweeper_recovered("r-1", "s-1", "timeout", "timeout");

        // 读取并验证（注意：read_all_events 用的是真实路径，这里直接读临时文件）
        let path = tmp.join("workflow_events.jsonl");
        let content = std::fs::read_to_string(&path).unwrap();
        let lines: Vec<&str> = content.lines().filter(|l| !l.is_empty()).collect();
        assert_eq!(lines.len(), 8, "should have 8 events");

        // 验证第一个事件
        let first: Value = serde_json::from_str(lines[0]).unwrap();
        assert_eq!(first["type"], "run_started");
        assert_eq!(first["run_id"], "r-1");
        assert_eq!(first["payload"]["template_id"], "tmpl-a");

        // 验证最后一个事件
        let last: Value = serde_json::from_str(lines[7]).unwrap();
        assert_eq!(last["type"], "sweeper_recovered");
        assert_eq!(last["payload"]["failure_kind"], "timeout");

        // 清理
        let _ = std::fs::remove_dir_all(&tmp);
        // 重置 writer
        {
            let mut guard = EVENT_WRITER.lock().unwrap();
            *guard = None;
        }
    }
}
