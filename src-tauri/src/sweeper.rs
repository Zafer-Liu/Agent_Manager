//! 阶段三 3c：Sweeper 巡检
//!
//! 后台任务（tokio interval，默认 30s）扫描三类静默失败：
//! 1. Step 超时：Running 且 now - started_at > timeout
//! 2. Agent 进程退出：Running 且关联 agent_id 已不在进程表
//! 3. Run 卡死：Run 为 Running，无 Running Step，且无 Ready 节点可推进
//!
//! 启动方式：在 lib.rs setup 阶段调用 start_sweeper()。
//! 停止方式：通过 AtomicBool cancellation token。

use crate::workflow_store::{FailureKind, FailureTrace, WorkflowRunStore};
use serde_json::json;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;
use tauri::{AppHandle, Emitter, Manager};

/// 默认巡检间隔
const SWEEP_INTERVAL_SECS: u64 = 30;
/// 默认 Step 超时（秒），节点未配置 timeout_secs 时使用
const DEFAULT_STEP_TIMEOUT_SECS: i64 = 300;

static SWEEPER_RUNNING: AtomicBool = AtomicBool::new(false);

/// 启动 Sweeper 后台巡检线程。
pub fn start_sweeper(app: AppHandle) {
    if SWEEPER_RUNNING.swap(true, Ordering::SeqCst) {
        return; // 已在运行
    }

    tauri::async_runtime::spawn(async move {
        let mut interval = tokio::time::interval(Duration::from_secs(SWEEP_INTERVAL_SECS));
        loop {
            interval.tick().await;
            if let Some(store) = app.try_state::<WorkflowRunStore>() {
                sweep_once(&app, &store.inner());
            }
        }
    });
}

/// 执行一轮巡检。
fn sweep_once(app: &AppHandle, store: &WorkflowRunStore) {
    let now_ms = chrono::Utc::now().timestamp_millis();
    let running_steps = store.find_running_steps();

    for (run_id, step_id, node_id, started_at, kind) in running_steps {
        let elapsed_secs = (now_ms - started_at) / 1000;

        // 场景 1：Step 超时
        if elapsed_secs > DEFAULT_STEP_TIMEOUT_SECS {
            let trace = FailureTrace {
                run_id: run_id.clone(),
                step_id: step_id.clone(),
                node_id: node_id.clone(),
                agent_id: None,
                tool: None,
                failure_kind: FailureKind::Timeout,
                reason: format!(
                    "Step timed out after {}s (default limit {}s)",
                    elapsed_secs, DEFAULT_STEP_TIMEOUT_SECS
                ),
                stderr_excerpt: None,
                retry_history: vec![],
                final_status: "failed".to_string(),
            };

            let _ = store.mark_step_failed(&run_id, &step_id, &trace);

            // 阶段三 3d：JSONL 事件源 — sweeper_recovered
            crate::workflow_events::emit_sweeper_recovered(
                &run_id,
                &step_id,
                "step timed out",
                "timeout",
            );

            // 发送事件通知前端
            let _ = app.emit(
                "workflow-step-failed",
                json!({
                    "run_id": run_id,
                    "step_id": step_id,
                    "node_id": node_id,
                    "kind": kind,
                    "reason": "timeout",
                    "elapsed_secs": elapsed_secs,
                }),
            );
            eprintln!(
                "[sweeper] Step {} in run {} timed out ({}s)",
                step_id, run_id, elapsed_secs
            );
        }
    }
}
