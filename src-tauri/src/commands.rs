use crate::agent::*;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::State;
use uuid::Uuid;

/// 安全获取 Mutex 锁，即使 poison 也恢复数据，避免 panic
fn lock_safe<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|e| e.into_inner())
}

fn get_data_dir() -> std::path::PathBuf {
    dirs_next::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("agent-manager")
}

fn load_configs() -> HashMap<String, AgentConfig> {
    let path = get_data_dir().join("agents.json");
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_configs(configs: &HashMap<String, AgentConfig>) {
    let dir = get_data_dir();
    let _ = std::fs::create_dir_all(&dir);
    let path = dir.join("agents.json");
    if let Ok(json) = serde_json::to_string_pretty(configs) {
        let _ = std::fs::write(path, json);
    }
}

fn is_port_open(port: u16) -> bool {
    TcpStream::connect_timeout(
        &format!("127.0.0.1:{}", port).parse().unwrap(),
        Duration::from_millis(200),
    )
    .is_ok()
}

fn push_log(logs: &Arc<Mutex<HashMap<String, Vec<LogEntry>>>>, id: &str, level: LogLevel, message: String) {
    let mut logs = lock_safe(logs);
    let entries = logs.entry(id.to_string()).or_default();
    // 最多保留 2000 条
    if entries.len() >= 2000 {
        entries.drain(0..200);
    }
    entries.push(LogEntry {
        timestamp: Utc::now().to_rfc3339(),
        level,
        message,
    });
}

/// 终止进程树：Windows 用 taskkill /T /F 递归杀掉子进程，避免孤儿进程
#[cfg(windows)]
fn kill_process_tree(child: &mut std::process::Child) {
    let pid = child.id();
    let _ = std::process::Command::new("taskkill")
        .args(["/PID", &pid.to_string(), "/T", "/F"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status();
    // 兜底：确保直接子进程也被 kill
    let _ = child.kill();
}

#[cfg(not(windows))]
fn kill_process_tree(child: &mut std::process::Child) {
    // TODO: Unix 下可用 pgid 杀进程组，这里先用基础 kill
    let _ = child.kill();
}

#[tauri::command]
pub fn list_agents(store: State<AgentStore>) -> Vec<AgentState> {
    let configs = load_configs();
    let mut agents = lock_safe(&store.agents);

    // 检查进程是否还活着，同步更新状态
    let mut processes = lock_safe(&store.processes);
    let mut exited: Vec<(String, Option<i32>)> = Vec::new();
    for (id, child) in processes.iter_mut() {
        if let Ok(Some(status)) = child.try_wait() {
            let code = status.code();
            exited.push((id.clone(), code));
            if let Some(state) = agents.get_mut(id) {
                state.status = AgentStatus::Stopped;
                state.pid = None;
                state.last_exit_code = code;
            }
        }
    }

    // 修复 retain 逻辑：只保留仍在运行的进程句柄
    processes.retain(|_id, child| child.try_wait().ok().flatten().is_none());

    drop(processes);
    drop(agents);

    // 对已退出的 agent 触发自动重启检查（在独立线程中，不阻塞响应）
    for (id, exit_code) in exited {
        try_schedule_auto_restart(
            &id,
            exit_code,
            &configs,
            &store,
        );
    }

    // 重新加锁构建返回值
    let agents = lock_safe(&store.agents);
    configs
        .values()
        .map(|config| {
            let state = agents.get(&config.id);
            let port_open = config.port.map(is_port_open).unwrap_or(false);
            let mut result = AgentState {
                status: state.map(|s| s.status.clone()).unwrap_or(AgentStatus::Stopped),
                pid: state.and_then(|s| s.pid),
                started_at: state.and_then(|s| s.started_at.clone()),
                port_open,
                config: config.clone(),
                restart_count: state.map(|s| s.restart_count).unwrap_or(0),
                last_exit_code: state.and_then(|s| s.last_exit_code),
            };
            // 如果端口已开但状态还是 Stopped，视为 Running（兼容外部启动）
            if port_open && result.status == AgentStatus::Stopped && result.pid.is_none() {
                result.status = AgentStatus::Running;
            }
            result
        })
        .collect()
}

/// 检查是否应该自动重启，如果是则 spawn 一个带退避的重启线程
fn try_schedule_auto_restart(
    id: &str,
    exit_code: Option<i32>,
    configs: &HashMap<String, AgentConfig>,
    store: &State<AgentStore>,
) {
    let config = match configs.get(id) {
        Some(c) => c.clone(),
        None => return,
    };

    // 必须开启 auto_restart
    if !config.auto_restart {
        return;
    }

    // 检查是否被 stop_agent 显式阻止
    let restarting = lock_safe(&store.restarting);
    if let Some(&val) = restarting.get(id) {
        if val == u32::MAX {
            return; // 被显式停止，不再重启
        }
    }
    drop(restarting);

    // 检查重启次数
    let agents = lock_safe(&store.agents);
    let current_count = agents.get(id).map(|s| s.restart_count).unwrap_or(0);
    drop(agents);

    if current_count >= restart_policy::MAX_RESTARTS {
        push_log(&store.logs, id, LogLevel::Warn,
            format!("Auto-restart skipped: reached max restarts ({})", restart_policy::MAX_RESTARTS));
        return;
    }

    // 记录重启意图
    {
        let mut restarting = lock_safe(&store.restarting);
        restarting.insert(id.to_string(), current_count + 1);
    }

    push_log(&store.logs, id, LogLevel::Warn,
        format!("Process exited (code={:?}), scheduling auto-restart #{} in {}ms...",
                exit_code, current_count + 1, restart_policy::RESTART_BACKOFF_MS));

    // spawn 重启线程
    let id_owned = id.to_string();
    let agents_arc = Arc::clone(&store.agents);
    let processes_arc = Arc::clone(&store.processes);
    let logs_arc = Arc::clone(&store.logs);
    let restarting_arc = Arc::clone(&store.restarting);

    std::thread::spawn(move || {
        // 退避等待
        std::thread::sleep(Duration::from_millis(restart_policy::RESTART_BACKOFF_MS));

        // 再次检查是否被 stop
        {
            let restarting = lock_safe(&restarting_arc);
            if let Some(&val) = restarting.get(&id_owned) {
                if val == u32::MAX {
                    return; // 期间被用户停止
                }
            }
        }

        // 执行重启
        match spawn_agent_with_arcs(&id_owned, &config, &agents_arc, &processes_arc, &logs_arc) {
            Ok(pid) => {
                push_log(&logs_arc, &id_owned, LogLevel::Info,
                    format!("Auto-restarted successfully (PID {})", pid));
                // 更新重启计数
                {
                    let mut agents = lock_safe(&agents_arc);
                    if let Some(state) = agents.get_mut(&id_owned) {
                        state.restart_count += 1;
                    }
                }
                // 清除重启标记
                {
                    let mut restarting = lock_safe(&restarting_arc);
                    restarting.remove(&id_owned);
                }
            }
            Err(e) => {
                push_log(&logs_arc, &id_owned, LogLevel::Error,
                    format!("Auto-restart failed: {}", e));
                {
                    let mut agents = lock_safe(&agents_arc);
                    if let Some(state) = agents.get_mut(&id_owned) {
                        state.status = AgentStatus::Error;
                    }
                }
                let mut restarting = lock_safe(&restarting_arc);
                restarting.remove(&id_owned);
            }
        }
    });
}

#[tauri::command]
pub async fn start_agent(id: String, store: State<'_, AgentStore>) -> Result<(), String> {
    let configs = load_configs();
    let config = configs.get(&id).ok_or("Agent not found")?.clone();

    // 已在运行则直接返回
    {
        let agents = lock_safe(&store.agents);
        if let Some(state) = agents.get(&id) {
            if state.status == AgentStatus::Running && state.pid.is_some() {
                return Ok(());
            }
        }
    }

    // 清除可能的重启阻止标记
    {
        let mut restarting = lock_safe(&store.restarting);
        restarting.remove(&id);
    }

    // 标记为 Starting
    {
        let mut agents = lock_safe(&store.agents);
        agents.insert(
            id.clone(),
            AgentState {
                config: config.clone(),
                status: AgentStatus::Starting,
                pid: None,
                started_at: Some(Utc::now().to_rfc3339()),
                port_open: false,
                restart_count: 0,
                last_exit_code: None,
            },
        );
    }
    push_log(&store.logs, &id, LogLevel::Info, format!("Starting agent '{}'...", config.name));

    // 记录本次启动前的日志基准索引，用于后续只收集本次启动产生的错误
    let log_start_idx = {
        let logs = lock_safe(&store.logs);
        logs.get(&id).map(|v| v.len()).unwrap_or(0)
    };

    // 执行实际的 spawn
    let pid = spawn_agent_with_arcs(
        &id,
        &config,
        &store.agents,
        &store.processes,
        &store.logs,
    )?;

    // 存活检查：等待一段时间后确认进程仍存活
    tokio::time::sleep(Duration::from_millis(restart_policy::HEALTH_CHECK_MS)).await;
    let (alive, exit_code) = {
        let mut processes = lock_safe(&store.processes);
        match processes.get_mut(&id) {
            Some(child) => match child.try_wait() {
                Ok(None) => (true, None),
                Ok(Some(status)) => (false, status.code()),
                Err(_) => (false, None),
            },
            None => (false, None),
        }
    };

    if !alive {
        // 进程已退出 —— 再等一段时间让 stderr 后台线程把残留输出写入日志
        tokio::time::sleep(Duration::from_millis(500)).await;

        // 只收集本次启动后（log_start_idx 之后）产生的错误，避免累加上次的错误信息
        let error_context = collect_errors_since(&store.logs, &id, log_start_idx);
        let err_msg = if error_context.is_empty() {
            format!("Agent exited immediately (code={:?}). Check command/path/env.", exit_code)
        } else {
            format!("Agent exited immediately (code={:?}):\n{}", exit_code, error_context)
        };

        // 更新状态为 Error
        {
            let mut agents = lock_safe(&store.agents);
            if let Some(state) = agents.get_mut(&id) {
                state.status = AgentStatus::Error;
                state.pid = None;
                state.last_exit_code = exit_code;
            }
        }
        // 清理句柄（关闭管道，让后台读线程收到 EOF 退出）
        {
            let mut processes = lock_safe(&store.processes);
            processes.remove(&id);
        }
        push_log(&store.logs, &id, LogLevel::Error, err_msg.clone());
        return Err(err_msg);
    }

    let _ = pid;
    Ok(())
}

/// 核心的 spawn 逻辑，接收 Arc 引用，可供 start_agent 命令和自动重启线程复用。
/// 返回新进程的 PID。
fn spawn_agent_with_arcs(
    id: &str,
    config: &AgentConfig,
    agents: &Arc<Mutex<HashMap<String, AgentState>>>,
    processes: &Arc<Mutex<HashMap<String, Child>>>,
    logs: &Arc<Mutex<HashMap<String, Vec<LogEntry>>>>,
) -> Result<u32, String> {
    // On Windows, resolve npm-global .cmd wrappers and route through cmd.exe
    let resolved_command = resolve_npm_global(&config.command)
        .map(|p| p.to_string_lossy().to_string())
        .unwrap_or_else(|| config.command.clone());

    // On Windows: 对于非 npm 的普通命令名（如 python/node），通过 where.exe 解析出完整路径，
    // 避免 CreateProcessW 搜索到 Windows Store stub（WindowsApps/python.exe）导致 exit 1/49。
    #[cfg(windows)]
    let resolved_command = {
        let cmd_path = std::path::Path::new(&resolved_command);
        // 只在非绝对路径、非 .cmd/.bat/npm wrapper 时做 where.exe 解析
        if !cmd_path.is_absolute()
            && !resolved_command.to_lowercase().ends_with(".cmd")
            && !resolved_command.to_lowercase().ends_with(".bat")
            && resolve_npm_global(&config.command).is_none()
        {
            if let Ok(output) = std::process::Command::new("where.exe")
                .arg(&config.command)
                .output()
            {
                if output.status.success() {
                    let where_result = String::from_utf8_lossy(&output.stdout);
                    // 取第一个结果，但跳过 Windows Store stub
                    let real_exe = where_result.lines()
                        .map(str::trim)
                        .filter(|line| !line.is_empty())
                        .find(|line| {
                            let lower = line.to_lowercase();
                            // 排除 WindowsApps 下的 Store stub 和 App Installer
                            !lower.contains("\\windowsapps\\") && !lower.contains("appinstaller")
                        })
                        .map(|s| s.to_string());

                    if let Some(exe_path) = real_exe {
                        push_log(logs, id, LogLevel::Debug,
                            format!("Resolved '{}' -> '{}'", config.command, exe_path));
                        exe_path
                    } else {
                        push_log(logs, id, LogLevel::Warn,
                            format!("where.exe found only Store stub for '{}', using raw name (may fail)", config.command));
                        resolved_command
                    }
                } else {
                    resolved_command
                }
            } else {
                resolved_command
            }
        } else {
            resolved_command
        }
    };

    #[cfg(windows)]
    let (exe, leading): (&str, Vec<String>) = {
        let lower = resolved_command.to_lowercase();
        if lower.ends_with(".cmd") || lower.ends_with(".bat") {
            ("cmd.exe", vec!["/c".to_string(), resolved_command.clone()])
        } else {
            (resolved_command.as_str(), vec![])
        }
    };
    #[cfg(not(windows))]
    let (exe, leading): (&str, Vec<String>) = (resolved_command.as_str(), vec![]);

    let mut cmd = Command::new(exe);
    cmd.args(&leading).args(&config.args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if !config.working_dir.is_empty() {
        cmd.current_dir(&config.working_dir);
    }
    for (k, v) in &config.env {
        cmd.env(k, v);
    }

    let mut child = cmd.spawn().map_err(|e| {
        // spawn 失败 —— 同步更新状态
        let mut agents_guard = lock_safe(agents);
        if let Some(state) = agents_guard.get_mut(id) {
            state.status = AgentStatus::Error;
        }
        push_log(logs, id, LogLevel::Error, format!("Failed to spawn: {}", e));
        format!("Failed to start: {}", e)
    })?;
    let pid = child.id();

    // 取出 stdout/stderr 管道
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    // 存储进程
    {
        let mut processes_guard = lock_safe(processes);
        processes_guard.insert(id.to_string(), child);
    }

    // 更新状态为 Running（保留已有的 restart_count）
    {
        let mut agents_guard = lock_safe(agents);
        let prev_restart_count = agents_guard.get(id).map(|s| s.restart_count).unwrap_or(0);
        agents_guard.insert(
            id.to_string(),
            AgentState {
                config: config.clone(),
                status: AgentStatus::Running,
                pid: Some(pid),
                started_at: Some(Utc::now().to_rfc3339()),
                port_open: false,
                restart_count: prev_restart_count,
                last_exit_code: None,
            },
        );
    }

    // 记录启动日志
    push_log(logs, id, LogLevel::Info, format!("Agent started (PID {})", pid));

    // 后台线程读取 stdout
    let logs_arc = Arc::clone(logs);
    let id_stdout = id.to_string();
    if let Some(out) = stdout {
        std::thread::spawn(move || {
            let reader = BufReader::new(out);
            for line in reader.lines().flatten() {
                push_log(&logs_arc, &id_stdout, LogLevel::Info, line);
            }
        });
    }

    // 后台线程读取 stderr
    let logs_arc2 = Arc::clone(logs);
    let id_stderr = id.to_string();
    if let Some(err) = stderr {
        std::thread::spawn(move || {
            let reader = BufReader::new(err);
            for line in reader.lines().flatten() {
                push_log(&logs_arc2, &id_stderr, LogLevel::Error, line);
            }
        });
    }

    Ok(pid)
}

/// 收集指定索引之后的错误日志（避免累加上次启动失败的错误信息）
fn collect_errors_since(logs: &Arc<Mutex<HashMap<String, Vec<LogEntry>>>>, id: &str, since: usize) -> String {
    let logs = lock_safe(logs);
    if let Some(entries) = logs.get(id) {
        let errors: Vec<String> = entries
            .iter()
            .skip(since)  // 只看本次启动后的日志
            .filter(|e| matches!(e.level, LogLevel::Error))
            .map(|e| e.message.clone())
            .collect();
        errors.join("\n")
    } else {
        String::new()
    }
}

#[tauri::command]
pub fn stop_agent(id: String, store: State<AgentStore>) -> Result<(), String> {
    // 标记为不再自动重启（哨兵值）
    {
        let mut restarting = lock_safe(&store.restarting);
        restarting.insert(id.clone(), u32::MAX);
    }

    // 使用进程树终止
    {
        let mut processes = lock_safe(&store.processes);
        if let Some(child) = processes.get_mut(&id) {
            kill_process_tree(child);
        }
        processes.remove(&id);
    }

    let mut agents = lock_safe(&store.agents);
    if let Some(state) = agents.get_mut(&id) {
        state.status = AgentStatus::Stopped;
        state.pid = None;
    }
    drop(agents);

    // 清除重启标记
    {
        let mut restarting = lock_safe(&store.restarting);
        restarting.remove(&id);
    }

    push_log(&store.logs, &id, LogLevel::Info, "Agent stopped".to_string());

    Ok(())
}

#[tauri::command]
pub fn get_agent_logs(id: String, store: State<AgentStore>) -> Vec<LogEntry> {
    let logs = lock_safe(&store.logs);
    logs.get(&id).cloned().unwrap_or_default()
}

#[tauri::command]
pub fn save_agent_config(config: Value) -> Result<String, String> {
    let mut configs = load_configs();

    let id = config
        .get("id")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| Uuid::new_v4().to_string());

    let now = Utc::now().to_rfc3339();
    let agent = AgentConfig {
        id: id.clone(),
        name: config["name"].as_str().unwrap_or("").to_string(),
        description: config["description"].as_str().unwrap_or("").to_string(),
        command: config["command"].as_str().unwrap_or("").to_string(),
        args: config["args"]
            .as_array()
            .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
            .unwrap_or_default(),
        working_dir: config["working_dir"].as_str().unwrap_or("").to_string(),
        env: config["env"]
            .as_object()
            .map(|o| {
                o.iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default(),
        port: config["port"].as_u64().map(|p| p as u16),
        ui_token: config["ui_token"].as_str().filter(|s| !s.is_empty()).map(|s| s.to_string()),
        auto_restart: config["auto_restart"].as_bool().unwrap_or(false),
        created_at: configs
            .get(&id)
            .map(|c| c.created_at.clone())
            .unwrap_or(now.clone()),
        updated_at: now,
    };

    configs.insert(id.clone(), agent);
    save_configs(&configs);
    Ok(id)
}

#[tauri::command]
pub fn delete_agent(id: String, store: State<AgentStore>) -> Result<(), String> {
    // 标记不再重启
    {
        let mut restarting = lock_safe(&store.restarting);
        restarting.insert(id.clone(), u32::MAX);
    }

    // 先停止进程（进程树终止）
    {
        let mut processes = lock_safe(&store.processes);
        if let Some(mut child) = processes.remove(&id) {
            kill_process_tree(&mut child);
        }
    }

    let mut configs = load_configs();
    configs.remove(&id);
    save_configs(&configs);

    let mut agents = lock_safe(&store.agents);
    agents.remove(&id);

    // 清理重启标记
    {
        let mut restarting = lock_safe(&store.restarting);
        restarting.remove(&id);
    }

    Ok(())
}

#[tauri::command]
pub fn get_port_status(port: u16) -> bool {
    is_port_open(port)
}

#[derive(Debug, Serialize, Deserialize)]
pub struct ProjectScanResult {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub port: Option<u16>,
    pub description: String,
    pub project_type: String,
}

#[tauri::command]
pub fn scan_project_dir(dir: String) -> Result<ProjectScanResult, String> {
    let path = Path::new(&dir);
    if !path.exists() || !path.is_dir() {
        return Err("Directory not found".to_string());
    }

    let name = path
        .file_name()
        .and_then(|n| n.to_str())
        .unwrap_or("agent")
        .to_string();

    // 检测文件存在的辅助闭包
    let has = |file: &str| path.join(file).exists();

    // Python 项目
    if has("pyproject.toml") || has("setup.py") || has("requirements.txt") {
        let (cmd, args, port) = detect_python_entry(path);
        return Ok(ProjectScanResult {
            name,
            command: cmd,
            args,
            port,
            description: "Python project".to_string(),
            project_type: "python".to_string(),
        });
    }

    // Node.js 项目
    if has("package.json") {
        let (cmd, args, port) = detect_node_entry(path);
        return Ok(ProjectScanResult {
            name,
            command: cmd,
            args,
            port,
            description: "Node.js project".to_string(),
            project_type: "node".to_string(),
        });
    }

    // Rust 项目
    if has("Cargo.toml") {
        return Ok(ProjectScanResult {
            name,
            command: "cargo".to_string(),
            args: vec!["run".to_string()],
            port: None,
            description: "Rust project".to_string(),
            project_type: "rust".to_string(),
        });
    }

    // Go 项目
    if has("go.mod") {
        return Ok(ProjectScanResult {
            name,
            command: "go".to_string(),
            args: vec!["run".to_string(), ".".to_string()],
            port: None,
            description: "Go project".to_string(),
            project_type: "go".to_string(),
        });
    }

    // npm 全局命令目录（如 C:\Users\xxx\.claude）
    // 目录名本身就是命令名，且能在 npm 全局路径里找到对应 .cmd
    if let Some(dir_name) = path.file_name().and_then(|n| n.to_str()) {
        // 去掉前缀点，如 ".claude" → "claude"
        let cmd_name = dir_name.trim_start_matches('.');
        if !cmd_name.is_empty() {
            if let Some(resolved) = resolve_npm_global(cmd_name) {
                let _ = resolved; // path confirmed it exists; store only the bare name
                return Ok(ProjectScanResult {
                    name: cmd_name.to_string(),
                    command: cmd_name.to_string(),
                    args: vec![],
                    port: None,
                    description: format!("npm global command: {}", cmd_name),
                    project_type: "npm-global".to_string(),
                });
            }
        }
    }

    // 可执行文件扫描
    if let Some(exe) = find_executable(path) {
        return Ok(ProjectScanResult {
            name,
            command: exe,
            args: vec![],
            port: None,
            description: "Executable".to_string(),
            project_type: "binary".to_string(),
        });
    }

    // 兜底：返回空模板让用户手动填
    Ok(ProjectScanResult {
        name,
        command: String::new(),
        args: vec![],
        port: None,
        description: String::new(),
        project_type: "unknown".to_string(),
    })
}

fn detect_python_entry(path: &Path) -> (String, Vec<String>, Option<u16>) {
    // 入口文件候选，按优先级排列，同时搜索根目录和 src/ 子目录
    let root_candidates = ["main.py", "app.py", "server.py", "run.py", "agent.py",
                           "start.py", "manage.py", "wsgi.py", "asgi.py", "__main__.py"];
    let entry: Option<String> = root_candidates.iter()
        .find(|f| path.join(f).exists())
        .map(|f| f.to_string())
        .or_else(|| {
            // 搜索 src/ 子目录
            root_candidates.iter()
                .find(|f| path.join("src").join(f).exists())
                .map(|f| format!("src/{}", f))
        });

    let has = |f: &str| path.join(f).exists();

    // 读文件内容辅助（支持多个文件）
    let file_contains = |file: &str, keyword: &str| -> bool {
        std::fs::read_to_string(path.join(file))
            .map(|c| c.to_lowercase().contains(keyword))
            .unwrap_or(false)
    };
    let any_contains = |keyword: &str| -> bool {
        ["requirements.txt", "pyproject.toml", "setup.py", "setup.cfg", "Pipfile"]
            .iter()
            .any(|f| file_contains(f, keyword))
    };

    // 检测 uv / poetry / pipenv
    let use_uv = has(".python-version") || has("uv.lock");
    let python_cmd = if use_uv { "uv" } else { "python" };

    // 从 pyproject.toml 读 scripts 入口（[project.scripts] 或 [tool.poetry.scripts]）
    let script_entry = std::fs::read_to_string(path.join("pyproject.toml")).ok()
        .and_then(|content| {
            // 简单正则：找第一个 "xxx = \"module:func\"" 行
            for line in content.lines() {
                if line.contains(" = \"") && line.contains(':') && !line.starts_with('[') {
                    if let Some(val) = line.splitn(2, '=').nth(1) {
                        let val = val.trim().trim_matches('"');
                        if val.contains(':') {
                            return Some(val.to_string()); // e.g. "myapp.main:app"
                        }
                    }
                }
            }
            None
        });

    let port = detect_python_port(path);

    // uv 项目
    if use_uv {
        // 优先 pyproject scripts 入口
        if let Some(ref s) = script_entry {
            return ("uv".to_string(), vec!["run".to_string(), s.clone()], port);
        }
        if let Some(ref e) = entry {
            return ("uv".to_string(), vec!["run".to_string(), e.clone()], port);
        }
        return ("uv".to_string(), vec!["run".to_string()], port);
    }

    // FastAPI / uvicorn
    if any_contains("fastapi") || any_contains("uvicorn") {
        let module = entry.as_deref()
            .map(|e| e.trim_end_matches(".py").replace('/', ".").replace('\\', "."))
            .unwrap_or_else(|| "main".to_string());
        // 尝试检测 app 变量名：app / application / create_app
        let app_var = entry.as_deref()
            .and_then(|e| std::fs::read_to_string(path.join(e)).ok())
            .and_then(|content| {
                for var in &["application", "create_app", "app"] {
                    if content.contains(&format!("{} =", var))
                        || content.contains(&format!("{}=", var))
                        || content.contains(&format!("def {}(", var))
                    {
                        return Some(var.to_string());
                    }
                }
                None
            })
            .unwrap_or_else(|| "app".to_string());
        let uvicorn_port = port.unwrap_or(8000);
        return (
            "uvicorn".to_string(),
            vec![
                format!("{}:{}", module, app_var),
                "--reload".to_string(),
                "--port".to_string(),
                uvicorn_port.to_string(),
            ],
            Some(uvicorn_port),
        );
    }

    // Flask
    if any_contains("flask") {
        let e = entry.as_deref().unwrap_or("app.py").to_string();
        let flask_port = port.unwrap_or(5000);
        return ("python".to_string(), vec![e], Some(flask_port));
    }

    // Django
    if any_contains("django") || has("manage.py") {
        let django_port = port.unwrap_or(8000);
        return (
            "python".to_string(),
            vec!["manage.py".to_string(), "runserver".to_string(),
                 format!("0.0.0.0:{}", django_port)],
            Some(django_port),
        );
    }

    // Streamlit
    if any_contains("streamlit") {
        let e = entry.as_deref().unwrap_or("app.py").to_string();
        let st_port = port.unwrap_or(8501);
        return (
            "streamlit".to_string(),
            vec!["run".to_string(), e,
                 "--server.port".to_string(), st_port.to_string()],
            Some(st_port),
        );
    }

    // 通用：有入口文件就直接 python 运行
    if let Some(e) = entry {
        (python_cmd.to_string(), vec![e], port)
    } else {
        (python_cmd.to_string(), vec![], port)
    }
}

fn detect_python_port(path: &Path) -> Option<u16> {
    // 1. .env 文件：PORT= / APP_PORT= / AGENT_PORT= 等
    for env_file in &[".env", ".env.local", ".env.development"] {
        if let Ok(content) = std::fs::read_to_string(path.join(env_file)) {
            for line in content.lines() {
                let line = line.trim();
                if line.starts_with('#') { continue; }
                // 匹配任意以 PORT= 结尾的键（PORT= / APP_PORT= / AGENT_PORT= / SERVER_PORT= 等）
                if let Some(eq) = line.find('=') {
                    let key = line[..eq].trim().to_uppercase();
                    if key == "PORT" || key.ends_with("_PORT") {
                        let val = line[eq + 1..].trim().trim_matches('"').trim_matches('\'');
                        if let Ok(p) = val.parse::<u16>() {
                            if p > 1000 { return Some(p); }
                        }
                    }
                }
            }
        }
    }

    // 2. 入口 py 文件中的端口声明
    let py_candidates = ["main.py", "app.py", "server.py", "run.py", "agent.py", "start.py"];
    for f in &py_candidates {
        if let Ok(content) = std::fs::read_to_string(path.join(f)) {
            for line in content.lines() {
                let trimmed = line.trim();
                // 跳过注释行
                if trimmed.starts_with('#') { continue; }

                // ── 模式 A：os.environ.get("AGENT_PORT", 5001) ──────────────
                // 匹配 environ.get(..., XXXX) 或 environ.get(..., "XXXX") 中的默认值
                // 仅当键名以 PORT 结尾（PORT / AGENT_PORT / APP_PORT 等）
                if trimmed.contains("environ") && trimmed.contains("get(") {
                    if let Some(p) = extract_environ_get_default(trimmed) {
                        return Some(p);
                    }
                }

                // ── 模式 B：port=XXXX（直接数字，词边界保护）──────────────
                // 覆盖：app.run(port=5001)、uvicorn.run(..., port=5001)
                let bytes = trimmed.as_bytes();
                let mut search = trimmed;
                while let Some(idx) = search.find("port=") {
                    let abs = trimmed.len() - search.len() + idx;
                    let prev_ok = abs == 0 || {
                        let prev = bytes[abs - 1] as char;
                        !prev.is_ascii_alphabetic() && prev != '_'
                    };
                    if prev_ok {
                        let rest = search[idx + 5..].trim_start();
                        let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                        if let Ok(p) = num.parse::<u16>() {
                            if p > 1000 { return Some(p); }
                        }
                    }
                    search = &search[idx + 5..];
                }

                // ── 模式 C：--port XXXX ──────────────────────────────────────
                if let Some(idx) = trimmed.find("--port ") {
                    let rest = trimmed[idx + 7..].trim_start();
                    let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                    if let Ok(p) = num.parse::<u16>() {
                        if p > 1000 { return Some(p); }
                    }
                }

                // ── 模式 D：PORT = 5001 / PORT=5001（脚本顶部大写赋值）────
                let up = trimmed.to_uppercase();
                for prefix in &["PORT = ", "PORT="] {
                    if let Some(rest) = up.strip_prefix(prefix) {
                        let num: String = rest.trim_start().chars().take_while(|c| c.is_ascii_digit()).collect();
                        if let Ok(p) = num.parse::<u16>() {
                            if p > 1000 { return Some(p); }
                        }
                    }
                }
            }
        }
    }

    None
}

/// 从 `os.environ.get("AGENT_PORT", 5001)` 这类表达式中提取默认端口。
/// 仅当键名以 PORT 结尾时才提取（避免误匹配无关的 get 调用）。
fn extract_environ_get_default(line: &str) -> Option<u16> {
    // 找到所有 .get( 的位置，逐一分析
    let mut search = line;
    while let Some(get_idx) = search.find(".get(") {
        let inner_start = get_idx + 5;
        let inner = &search[inner_start..];

        // 提取括号内的完整内容（找配对的 )）
        let mut depth = 1usize;
        let mut end = 0;
        for (i, c) in inner.char_indices() {
            match c {
                '(' => depth += 1,
                ')' => {
                    depth -= 1;
                    if depth == 0 { end = i; break; }
                }
                _ => {}
            }
        }
        if end == 0 {
            search = &search[inner_start..];
            continue;
        }

        let args_str = &inner[..end]; // 括号内容，如 `"AGENT_PORT", 5001`
        let parts: Vec<&str> = args_str.splitn(2, ',').collect();
        if parts.len() == 2 {
            // 第一个参数：键名（去掉引号和空白）
            let key = parts[0].trim().trim_matches('"').trim_matches('\'').to_uppercase();
            // 键名必须以 PORT 结尾
            if key == "PORT" || key.ends_with("PORT") {
                // 第二个参数：默认值
                let default_str = parts[1].trim().trim_matches('"').trim_matches('\'');
                if let Ok(p) = default_str.parse::<u16>() {
                    if p > 1000 { return Some(p); }
                }
            }
        }

        search = &search[inner_start..];
    }
    None
}

fn detect_node_entry(path: &Path) -> (String, Vec<String>, Option<u16>) {
    let pkg = std::fs::read_to_string(path.join("package.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok());

    let pm = if path.join("pnpm-lock.yaml").exists() { "pnpm" }
        else if path.join("yarn.lock").exists() { "yarn" }
        else { "npm" };

    if let Some(ref p) = pkg {
        let scripts = p.get("scripts");
        let deps = p.get("dependencies");
        let dev_deps = p.get("devDependencies");

        let has_dep = |name: &str| -> bool {
            deps.and_then(|d| d.get(name)).is_some()
                || dev_deps.and_then(|d| d.get(name)).is_some()
        };

        let script_val = |key: &str| -> Option<&str> {
            scripts.and_then(|s| s.get(key)).and_then(|v| v.as_str())
        };

        // 1. Vite 项目 (dev server) — 优先用 dev 脚本
        if has_dep("vite") || has_dep("@vitejs/plugin-react") {
            let script = if script_val("dev").is_some() { "dev" } else { "start" };
            let port = script_val(script)
                .and_then(|s| extract_port_from_script(s))
                .unwrap_or(5173);
            return (pm.to_string(), vec!["run".to_string(), script.to_string()], Some(port));
        }

        // 2. Next.js
        if has_dep("next") {
            let script = if script_val("dev").is_some() { "dev" } else { "start" };
            let port = script_val(script)
                .and_then(|s| extract_port_from_script(s))
                .unwrap_or(3000);
            return (pm.to_string(), vec!["run".to_string(), script.to_string()], Some(port));
        }

        // 3. Express / Fastify / Hono 等服务端框架 — 优先 start，次选 dev
        let is_server = has_dep("express") || has_dep("fastify") || has_dep("hono")
            || has_dep("koa") || has_dep("@hono/node-server");
        if is_server {
            let (script, port) = if let Some(s) = script_val("start") {
                ("start", extract_port_from_script(s).unwrap_or(3000))
            } else if let Some(s) = script_val("dev") {
                ("dev", extract_port_from_script(s).unwrap_or(3000))
            } else {
                ("start", 3000)
            };
            return (pm.to_string(), vec!["run".to_string(), script.to_string()], Some(port));
        }

        // 4. 有 dev 脚本就用 dev
        if let Some(dev_script) = script_val("dev") {
            let port = extract_port_from_script(dev_script);
            return (pm.to_string(), vec!["run".to_string(), "dev".to_string()], port);
        }

        // 5. 有 start 脚本就用 start
        if let Some(start_script) = script_val("start") {
            let port = extract_port_from_script(start_script);
            return (pm.to_string(), vec!["run".to_string(), "start".to_string()], port);
        }
    }

    // 6. 没有可用脚本，找入口文件
    let candidates = ["index.js", "server.js", "app.js", "main.js", "index.ts", "server.ts"];
    let entry = candidates.iter().find(|f| path.join(f).exists()).copied();
    let has_ts = path.join("tsconfig.json").exists();
    let port = detect_node_port(path, pkg.as_ref());

    if let Some(e) = entry {
        if has_ts && e.ends_with(".ts") {
            return ("npx".to_string(), vec!["tsx".to_string(), e.to_string()], port);
        }
        return ("node".to_string(), vec![e.to_string()], port);
    }

    // 兜底：至少给出包管理器，让用户补全
    (pm.to_string(), vec!["run".to_string()], port)
}

fn detect_node_port(path: &Path, pkg: Option<&Value>) -> Option<u16> {
    // 从 .env 读
    if let Ok(content) = std::fs::read_to_string(path.join(".env")) {
        for line in content.lines() {
            if let Some(val) = line.strip_prefix("PORT=") {
                if let Ok(p) = val.trim().parse::<u16>() {
                    return Some(p);
                }
            }
        }
    }
    // 从 package.json scripts 推断
    if let Some(p) = pkg {
        if let Some(start) = p.pointer("/scripts/start").and_then(|v| v.as_str()) {
            return extract_port_from_script(start);
        }
    }
    None
}

fn extract_port_from_script(script: &str) -> Option<u16> {
    // 匹配 --port 3000 / -p 3000 / PORT=3000
    let re_patterns = ["--port ", "-p ", "PORT="];
    for pat in re_patterns {
        if let Some(idx) = script.find(pat) {
            let rest = &script[idx + pat.len()..];
            let num: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
            if let Ok(p) = num.parse::<u16>() {
                return Some(p);
            }
        }
    }
    None
}

fn find_executable(path: &Path) -> Option<String> {
    let exts = if cfg!(windows) { vec!["exe", "bat", "cmd"] } else { vec!["sh", ""] };
    if let Ok(entries) = std::fs::read_dir(path) {
        for entry in entries.flatten() {
            let p = entry.path();
            if p.is_file() {
                if let Some(ext) = p.extension().and_then(|e| e.to_str()) {
                    if exts.contains(&ext) {
                        return Some(p.to_string_lossy().to_string());
                    }
                }
            }
        }
    }
    None
}

/// Resolve a shell command wrapper to its full .cmd/.bat path on Windows.
#[cfg(windows)]
pub fn resolve_npm_global(cmd: &str) -> Option<std::path::PathBuf> {
    let command_path = std::path::Path::new(cmd);
    if command_path.is_file() && is_windows_command_wrapper(command_path) {
        return Some(command_path.to_path_buf());
    }

    // `std::process::Command` does not apply PATHEXT like an interactive
    // Windows shell does. `where npm` commonly returns both an extensionless
    // shim and npm.cmd, so explicitly select a runnable command wrapper.
    if let Ok(output) = std::process::Command::new("where.exe").arg(cmd).output() {
        if output.status.success() {
            if let Some(path) =
                select_windows_command_wrapper(&String::from_utf8_lossy(&output.stdout))
            {
                return Some(path);
            }
        }
    }

    let command_name = command_path
        .file_stem()
        .and_then(|name| name.to_str())
        .unwrap_or(cmd);

    if let Some(appdata) = std::env::var_os("APPDATA") {
        let candidate = std::path::Path::new(&appdata)
            .join("npm")
            .join(format!("{}.cmd", command_name));
        if candidate.exists() {
            return Some(candidate);
        }
    }
    if let Some(profile) = std::env::var_os("USERPROFILE") {
        let candidate = std::path::Path::new(&profile)
            .join("AppData")
            .join("Roaming")
            .join("npm")
            .join(format!("{}.cmd", command_name));
        if candidate.exists() {
            return Some(candidate);
        }
    }

    // npm/npx installed with Node itself live beside node.exe rather than in
    // the per-user npm global bin directory.
    let node_dir = std::path::PathBuf::from(find_node_exe_path());
    if let Some(parent) = node_dir.parent() {
        for extension in ["cmd", "bat"] {
            let candidate = parent.join(format!("{}.{}", command_name, extension));
            if candidate.exists() {
                return Some(candidate);
            }
        }
    }

    None
}

#[cfg(windows)]
fn is_windows_command_wrapper(path: &std::path::Path) -> bool {
    path.extension()
        .and_then(|extension| extension.to_str())
        .map(|extension| {
            extension.eq_ignore_ascii_case("cmd") || extension.eq_ignore_ascii_case("bat")
        })
        .unwrap_or(false)
}

#[cfg(windows)]
fn select_windows_command_wrapper(output: &str) -> Option<std::path::PathBuf> {
    output
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .map(std::path::PathBuf::from)
        .find(|path| is_windows_command_wrapper(path))
}

/// For PTY use: resolve an npm-global command to (node_exe, js_entry).
/// Reads the package.json "bin" field under node_modules to find the real JS entry.
/// Returns None if not an npm-global command.
#[cfg(windows)]
pub fn resolve_npm_global_to_node(cmd: &str) -> Option<(String, String)> {
    let cmd_path = resolve_npm_global(cmd)?;
    let npm_dir = cmd_path.parent()?;
    let node_modules = npm_dir.join("node_modules");

    // Walk node_modules (including scoped @scope/pkg dirs) looking for
    // a package whose "bin" has an entry matching cmd.
    let js_entry = find_bin_entry(&node_modules, cmd)?;

    // Find node.exe: try where.exe first, then common paths
    let node_exe = find_node_exe_path();
    Some((node_exe, js_entry))
}

#[cfg(windows)]
fn find_bin_entry(node_modules: &std::path::Path, cmd: &str) -> Option<String> {
    let Ok(top_entries) = std::fs::read_dir(node_modules) else { return None };

    let mut dirs_to_check: Vec<std::path::PathBuf> = Vec::new();

    for entry in top_entries.flatten() {
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if name.starts_with('@') {
            // Scoped: descend one more level
            if let Ok(inner) = std::fs::read_dir(&path) {
                for inner_entry in inner.flatten() {
                    dirs_to_check.push(inner_entry.path());
                }
            }
        } else {
            dirs_to_check.push(path);
        }
    }

    for pkg_dir in dirs_to_check {
        let pkg_json = pkg_dir.join("package.json");
        if !pkg_json.exists() { continue; }
        let Ok(text) = std::fs::read_to_string(&pkg_json) else { continue };
        let Ok(json) = serde_json::from_str::<serde_json::Value>(&text) else { continue };

        let bin = json.get("bin")?;
        let js_rel = if let Some(s) = bin.as_str() {
            s.to_string()
        } else if let Some(obj) = bin.as_object() {
            obj.get(cmd).and_then(|v| v.as_str())?.to_string()
        } else {
            continue;
        };

        let js_rel_clean = js_rel.trim_start_matches("./").replace('/', std::path::MAIN_SEPARATOR_STR);
        let full = pkg_dir.join(&js_rel_clean);
        if full.exists() {
            return Some(full.to_string_lossy().to_string());
        }
    }
    None
}

#[cfg(windows)]
pub fn find_node_exe_path() -> String {
    // 1. where.exe node
    if let Ok(out) = std::process::Command::new("where.exe").arg("node").output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout);
            if let Some(first) = s.lines().next() {
                let p = std::path::Path::new(first.trim());
                if p.exists() { return first.trim().to_string(); }
            }
        }
    }
    // 2. Common install paths
    for p in &[
        r"C:\Program Files\nodejs\node.exe",
        r"C:\Program Files (x86)\nodejs\node.exe",
    ] {
        if std::path::Path::new(p).exists() { return p.to_string(); }
    }
    // 3. nvm4w: NVM_SYMLINK env var points to current node dir
    if let Ok(symlink) = std::env::var("NVM_SYMLINK") {
        let p = std::path::Path::new(&symlink).join("node.exe");
        if p.exists() { return p.to_string_lossy().to_string(); }
    }
    "node".to_string()
}

#[cfg(not(windows))]
pub fn resolve_npm_global(_cmd: &str) -> Option<std::path::PathBuf> {
    None
}

#[cfg(not(windows))]
pub fn resolve_npm_global_to_node(_cmd: &str) -> Option<(String, String)> {
    None
}

#[cfg(all(test, windows))]
mod tests {
    use super::select_windows_command_wrapper;

    #[test]
    fn selects_cmd_wrapper_from_where_output() {
        let node_dir = std::path::Path::new(r"C:\Program Files\nodejs");
        let npm_cmd = node_dir.join("npm.cmd");
        let output = format!(
            "{}\r\n{}\r\n",
            node_dir.join("npm").display(),
            npm_cmd.display()
        );

        assert_eq!(select_windows_command_wrapper(&output), Some(npm_cmd));
    }
}
