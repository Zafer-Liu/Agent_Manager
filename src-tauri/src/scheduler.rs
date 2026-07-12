//! 定时触发调度器（阶段四 P2）
//!
//! 后台线程每分钟检查所有工作流模板的 schedule（cron 表达式），
//! 匹配则用 RunTrigger::Schedule 触发 run_workflow_core。
//!
//! Cron 格式：简化版 5 字段 `分 时 日 月 周`（如 `*/5 * * * *` 每 5 分钟）。
//! 不依赖外部 cron crate，自行实现轻量解析。

use crate::workflow::{read_workflows, run_workflow_core, RunWorkflowRequest};
use crate::workflow_store::{RunTrigger, WorkflowRunStore};
use chrono::{DateTime, Datelike, Local, Timelike, Utc};
use std::collections::HashMap;
use std::sync::Mutex;
use std::time::Duration;

/// 全局调度器状态：记录每个模板上次触发时间，避免同一分钟重复触发。
static LAST_FIRED: std::sync::LazyLock<Mutex<HashMap<String, i64>>> =
    std::sync::LazyLock::new(|| Mutex::new(HashMap::new()));

/// 启动定时调度器后台线程。
/// 在 Tauri setup 钩子中调用一次。
pub fn start_scheduler() {
    std::thread::spawn(|| {
        eprintln!("[scheduler] 定时调度器已启动，每 60s 检查一次");
        loop {
            // 等待到下一个整分钟边界（对齐 cron 语义）
            let now = Local::now();
            let secs_to_next = 60 - now.second() % 60;
            std::thread::sleep(Duration::from_secs(secs_to_next as u64));

            check_and_fire();
        }
    });
}

/// 检查所有工作流模板的 cron 表达式，匹配则触发。
fn check_and_fire() {
    let now = Utc::now();
    let workflows = read_workflows();

    for wf in workflows {
        let Some(cron_expr) = wf.schedule.clone() else {
            continue;
        };

        let template_id = wf.id.clone();

        // 解析 cron
        let Ok(cron) = parse_cron(&cron_expr) else {
            eprintln!("[scheduler] 模板 {} cron 解析失败: {}", template_id, cron_expr);
            continue;
        };

        // 检查当前时间是否匹配 cron
        if !cron_matches(&cron, &now) {
            continue;
        }

        // 防止同一分钟重复触发
        let now_ms = now.timestamp_millis();
        let should_fire = {
            let last = LAST_FIRED.lock().unwrap();
            if let Some(&last_ms) = last.get(&template_id) {
                // 如果上次触发在 60s 内，跳过
                now_ms - last_ms > 60_000
            } else {
                true
            }
        };

        if !should_fire {
            continue;
        }

        // 记录触发时间
        {
            let mut last = LAST_FIRED.lock().unwrap();
            last.insert(template_id.clone(), now_ms);
        }

        eprintln!(
            "[scheduler] cron 匹配，触发模板 {} ({})",
            template_id, cron_expr
        );

        // 构造请求并异步执行
        let request = RunWorkflowRequest {
            workflow: wf,
            provider: None,
            mcp_servers: vec![],
            input: String::new(),
            rework: None,
            callback_url: None,
            trigger: Some(RunTrigger::Schedule {
                cron: cron_expr.clone(),
            }),
        };

        tauri::async_runtime::spawn(async move {
            let store = WorkflowRunStore::new();
            match run_workflow_core(request, None, &store).await {
                Ok(result) => {
                    eprintln!(
                        "[scheduler] 模板 {} 定时触发完成，success={}",
                        template_id, result.success
                    );
                }
                Err(e) => {
                    eprintln!("[scheduler] 模板 {} 定时触发失败: {}", template_id, e);
                }
            }
        });
    }
}

// ── Cron 解析与匹配 ───────────────────────────────────────────────────────────

/// 简化 cron：5 字段（分 时 日 月 周）
struct CronExpr {
    minutes: Vec<u8>,   // 0-59
    hours: Vec<u8>,     // 0-23
    days: Vec<u8>,      // 1-31
    months: Vec<u8>,    // 1-12
    weekdays: Vec<u8>,  // 0-6 (0=Sunday)
}

/// 解析 cron 表达式。支持: * / N / */N / 数字列表(逗号分隔) / 范围(1-5)
fn parse_cron(expr: &str) -> Result<CronExpr, String> {
    let parts: Vec<&str> = expr.trim().split_whitespace().collect();
    if parts.len() != 5 {
        return Err(format!("expected 5 fields, got {}", parts.len()));
    }

    Ok(CronExpr {
        minutes: parse_field(parts[0], 0, 59)?,
        hours: parse_field(parts[1], 0, 23)?,
        days: parse_field(parts[2], 1, 31)?,
        months: parse_field(parts[3], 1, 12)?,
        weekdays: parse_field(parts[4], 0, 6)?,
    })
}

/// 解析单个 cron 字段。
/// 支持: `*` `5` `*/5` `1,3,5` `1-5` `*/15`
fn parse_field(field: &str, min: u8, max: u8) -> Result<Vec<u8>, String> {
    let mut result = Vec::new();

    for part in field.split(',') {
        if part == "*" {
            result.extend(min..=max);
        } else if let Some(step_str) = part.strip_prefix("*/") {
            let step: u8 = step_str
                .parse()
                .map_err(|_| format!("invalid step: {}", step_str))?;
            if step == 0 {
                return Err("step cannot be 0".into());
            }
            let mut v = min;
            while v <= max {
                result.push(v);
                v = v.saturating_add(step);
            }
        } else if part.contains('-') {
            let range: Vec<&str> = part.split('-').collect();
            if range.len() != 2 {
                return Err(format!("invalid range: {}", part));
            }
            let start: u8 = range[0]
                .parse()
                .map_err(|_| format!("invalid range start: {}", range[0]))?;
            let end: u8 = range[1]
                .parse()
                .map_err(|_| format!("invalid range end: {}", range[1]))?;
            if start > end || start < min || end > max {
                return Err(format!("range out of bounds: {}", part));
            }
            result.extend(start..=end);
        } else {
            let val: u8 = part
                .parse()
                .map_err(|_| format!("invalid value: {}", part))?;
            if val < min || val > max {
                return Err(format!("value {} out of range [{}, {}]", val, min, max));
            }
            result.push(val);
        }
    }

    result.sort();
    result.dedup();
    Ok(result)
}

/// 检查给定时间是否匹配 cron 表达式
fn cron_matches(cron: &CronExpr, dt: &DateTime<Utc>) -> bool {
    // 转换为本地时间进行匹配（cron 通常按本地时间理解）
    let local = dt.with_timezone(&Local);
    cron.minutes.contains(&(local.minute() as u8))
        && cron.hours.contains(&(local.hour() as u8))
        && cron.days.contains(&(local.day() as u8))
        && cron.months.contains(&(local.month() as u8))
        && cron.weekdays.contains(&(local.weekday().num_days_from_sunday() as u8))
}

// ── 测试 ─────────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn parse_simple_cron() {
        let cron = parse_cron("0 */5 * * *").unwrap();
        assert_eq!(cron.minutes, vec![0]);
        assert_eq!(cron.hours, vec![0, 5, 10, 15, 20]);
    }

    #[test]
    fn parse_every_minute() {
        let cron = parse_cron("* * * * *").unwrap();
        assert_eq!(cron.minutes.len(), 60);
    }

    #[test]
    fn parse_comma_list() {
        let cron = parse_cron("0,30 * * * *").unwrap();
        assert_eq!(cron.minutes, vec![0, 30]);
    }

    #[test]
    fn parse_range() {
        let cron = parse_cron("0 9-17 * * 1-5").unwrap();
        assert_eq!(cron.hours, vec![9, 10, 11, 12, 13, 14, 15, 16, 17]);
        assert_eq!(cron.weekdays, vec![1, 2, 3, 4, 5]);
    }

    #[test]
    fn parse_invalid_fields() {
        assert!(parse_cron("60 * * * *").is_err()); // minute out of range
        assert!(parse_cron("* * * *").is_err()); // too few fields
        assert!(parse_cron("* * * * * *").is_err()); // too many fields
        assert!(parse_cron("*/0 * * * *").is_err()); // zero step
    }

    #[test]
    fn cron_match_specific_time() {
        let cron = parse_cron("30 14 * * *").unwrap(); // 每天 14:30
        let dt = Local
            .with_ymd_and_hms(2026, 7, 6, 14, 30, 0)
            .unwrap()
            .with_timezone(&Utc);
        assert!(cron_matches(&cron, &dt));

        let dt2 = Local
            .with_ymd_and_hms(2026, 7, 6, 14, 31, 0)
            .unwrap()
            .with_timezone(&Utc);
        assert!(!cron_matches(&cron, &dt2));
    }
}
