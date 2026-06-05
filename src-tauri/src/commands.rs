use crate::agent::*;
use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::io::{BufRead, BufReader};
use std::net::TcpStream;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tauri::State;
use uuid::Uuid;

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
    let mut logs = logs.lock().unwrap();
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

#[tauri::command]
pub fn list_agents(store: State<AgentStore>) -> Vec<AgentState> {
    let configs = load_configs();
    let mut agents = store.agents.lock().unwrap();

    // 检查进程是否还活着
    let mut processes = store.processes.lock().unwrap();
    for (id, child) in processes.iter_mut() {
        if let Ok(Some(_)) = child.try_wait() {
            if let Some(state) = agents.get_mut(id) {
                state.status = AgentStatus::Stopped;
                state.pid = None;
            }
        }
    }
    // 清理已停止的进程
    processes.retain(|id, child| child.try_wait().ok().flatten().is_none() || {
        agents.get(id).map(|s| s.status == AgentStatus::Running).unwrap_or(false)
    });

    configs
        .values()
        .map(|config| {
            let state = agents.get(&config.id);
            let port_open = config.port.map(is_port_open).unwrap_or(false);
            AgentState {
                status: state.map(|s| s.status.clone()).unwrap_or(AgentStatus::Stopped),
                pid: state.and_then(|s| s.pid),
                started_at: state.and_then(|s| s.started_at.clone()),
                port_open,
                config: config.clone(),
            }
        })
        .collect()
}

#[tauri::command]
pub fn start_agent(id: String, store: State<AgentStore>) -> Result<(), String> {
    let configs = load_configs();
    let config = configs.get(&id).ok_or("Agent not found")?.clone();

    let mut cmd = Command::new(&config.command);
    cmd.args(&config.args)
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    if !config.working_dir.is_empty() {
        cmd.current_dir(&config.working_dir);
    }
    for (k, v) in &config.env {
        cmd.env(k, v);
    }

    let mut child = cmd.spawn().map_err(|e| format!("Failed to start: {}", e))?;
    let pid = child.id();

    // 取出 stdout/stderr 管道
    let stdout = child.stdout.take();
    let stderr = child.stderr.take();

    // 存储进程
    {
        let mut processes = store.processes.lock().unwrap();
        processes.insert(id.clone(), child);
    }

    // 更新状态
    {
        let mut agents = store.agents.lock().unwrap();
        agents.insert(
            id.clone(),
            AgentState {
                config: config.clone(),
                status: AgentStatus::Running,
                pid: Some(pid),
                started_at: Some(Utc::now().to_rfc3339()),
                port_open: false,
            },
        );
    }

    // 记录启动日志
    {
        let mut logs = store.logs.lock().unwrap();
        logs.entry(id.clone()).or_default().push(LogEntry {
            timestamp: Utc::now().to_rfc3339(),
            level: LogLevel::Info,
            message: format!("Agent started (PID {})", pid),
        });
    }

    // 后台线程读取 stdout
    let logs_arc = Arc::clone(&store.logs);
    let id_stdout = id.clone();
    if let Some(out) = stdout {
        std::thread::spawn(move || {
            let reader = BufReader::new(out);
            for line in reader.lines().flatten() {
                push_log(&logs_arc, &id_stdout, LogLevel::Info, line);
            }
        });
    }

    // 后台线程读取 stderr
    let logs_arc2 = Arc::clone(&store.logs);
    let id_stderr = id.clone();
    if let Some(err) = stderr {
        std::thread::spawn(move || {
            let reader = BufReader::new(err);
            for line in reader.lines().flatten() {
                push_log(&logs_arc2, &id_stderr, LogLevel::Error, line);
            }
        });
    }

    Ok(())
}

#[tauri::command]
pub fn stop_agent(id: String, store: State<AgentStore>) -> Result<(), String> {
    {
        let mut processes = store.processes.lock().unwrap();
        if let Some(child) = processes.get_mut(&id) {
            let _ = child.kill();
        }
        processes.remove(&id);
    }

    let mut agents = store.agents.lock().unwrap();
    if let Some(state) = agents.get_mut(&id) {
        state.status = AgentStatus::Stopped;
        state.pid = None;
    }

    let mut logs = store.logs.lock().unwrap();
    logs.entry(id).or_default().push(LogEntry {
        timestamp: Utc::now().to_rfc3339(),
        level: LogLevel::Info,
        message: "Agent stopped".to_string(),
    });

    Ok(())
}

#[tauri::command]
pub fn get_agent_logs(id: String, store: State<AgentStore>) -> Vec<LogEntry> {
    let logs = store.logs.lock().unwrap();
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
    // 先停止进程
    {
        let mut processes = store.processes.lock().unwrap();
        if let Some(mut child) = processes.remove(&id) {
            let _ = child.kill();
        }
    }

    let mut configs = load_configs();
    configs.remove(&id);
    save_configs(&configs);

    let mut agents = store.agents.lock().unwrap();
    agents.remove(&id);

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
    // 查找入口文件优先级
    let candidates = ["main.py", "app.py", "server.py", "run.py", "agent.py", "__main__.py"];
    let entry = candidates.iter().find(|f| path.join(f).exists()).copied();

    // 检测常见框架及端口
    let has = |f: &str| path.join(f).exists();
    let content_has = |file: &str, keyword: &str| -> bool {
        path.join(file)
            .exists()
            .then(|| std::fs::read_to_string(path.join(file)).unwrap_or_default())
            .map(|c| c.contains(keyword))
            .unwrap_or(false)
    };

    // 检测 uv
    let python_cmd = if has(".python-version") || has("uv.lock") {
        "uv"
    } else {
        "python"
    };

    if python_cmd == "uv" {
        if let Some(e) = entry {
            return ("uv".to_string(), vec!["run".to_string(), e.to_string()], detect_python_port(path));
        }
        return ("uv".to_string(), vec!["run".to_string()], detect_python_port(path));
    }

    // FastAPI / uvicorn
    if content_has("requirements.txt", "fastapi") || content_has("pyproject.toml", "fastapi") {
        let module = entry.map(|e| e.trim_end_matches(".py").to_string()).unwrap_or("main".to_string());
        return (
            "uvicorn".to_string(),
            vec![format!("{}:app", module), "--reload".to_string()],
            Some(8000),
        );
    }

    // Flask
    if content_has("requirements.txt", "flask") || content_has("pyproject.toml", "flask") {
        let e = entry.unwrap_or("app.py");
        return ("python".to_string(), vec![e.to_string()], Some(5000));
    }

    // 通用
    if let Some(e) = entry {
        (python_cmd.to_string(), vec![e.to_string()], detect_python_port(path))
    } else {
        (python_cmd.to_string(), vec![], None)
    }
}

fn detect_python_port(path: &Path) -> Option<u16> {
    // 从 .env 文件读 PORT
    if let Ok(content) = std::fs::read_to_string(path.join(".env")) {
        for line in content.lines() {
            if let Some(val) = line.strip_prefix("PORT=") {
                if let Ok(p) = val.trim().parse::<u16>() {
                    return Some(p);
                }
            }
        }
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
