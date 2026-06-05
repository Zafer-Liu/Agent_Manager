use serde::{Deserialize, Serialize};
use std::process::Command;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PortInfo {
    pub port: u16,
    pub pid: u32,
    pub process_name: String,
    pub protocol: String,
    pub state: String,
}

#[tauri::command]
pub fn list_listening_ports() -> Vec<PortInfo> {
    #[cfg(windows)]
    {
        list_ports_windows()
    }
    #[cfg(unix)]
    {
        list_ports_unix()
    }
}

#[cfg(windows)]
fn list_ports_windows() -> Vec<PortInfo> {
    // netstat -ano 拿到端口+PID，再用 tasklist 查进程名
    let netstat = Command::new("netstat")
        .args(["-ano"])
        .output()
        .ok();

    let mut ports: Vec<PortInfo> = vec![];

    if let Some(out) = netstat {
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            // Proto  Local  Foreign  State  PID
            if parts.len() < 5 { continue; }
            let proto = parts[0];
            if proto != "TCP" && proto != "UDP" { continue; }

            let state = if proto == "TCP" { parts[3] } else { "—" };
            let pid_str = parts[parts.len() - 1];

            if state != "LISTENING" && state != "—" { continue; }

            let local = parts[1];
            let port = local.rsplit(':').next()
                .and_then(|p| p.parse::<u16>().ok());
            let pid = pid_str.parse::<u32>().ok();

            if let (Some(port), Some(pid)) = (port, pid) {
                if pid == 0 { continue; }
                ports.push(PortInfo {
                    port,
                    pid,
                    process_name: String::new(),
                    protocol: proto.to_string(),
                    state: state.to_string(),
                });
            }
        }
    }

    // 批量查进程名
    let tasklist = Command::new("tasklist")
        .args(["/fo", "csv", "/nh"])
        .output()
        .ok();

    let mut pid_name: std::collections::HashMap<u32, String> = std::collections::HashMap::new();
    if let Some(out) = tasklist {
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines() {
            // "chrome.exe","1234","Console","1","50,000 K"
            let parts: Vec<&str> = line.splitn(3, ',').collect();
            if parts.len() < 2 { continue; }
            let name = parts[0].trim_matches('"').to_string();
            let pid = parts[1].trim_matches('"').parse::<u32>().ok();
            if let Some(p) = pid {
                pid_name.insert(p, name);
            }
        }
    }

    for info in &mut ports {
        info.process_name = pid_name
            .get(&info.pid)
            .cloned()
            .unwrap_or_else(|| format!("PID {}", info.pid));
    }

    // 去重（同端口多条记录取第一条）
    let mut seen = std::collections::HashSet::new();
    ports.retain(|p| seen.insert(p.port));
    ports.sort_by_key(|p| p.port);
    ports
}

#[cfg(unix)]
fn list_ports_unix() -> Vec<PortInfo> {
    let out = Command::new("ss")
        .args(["-tlnp"])
        .output()
        .ok();

    let mut ports = vec![];
    if let Some(out) = out {
        let text = String::from_utf8_lossy(&out.stdout);
        for line in text.lines().skip(1) {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 5 { continue; }
            let local = parts[3];
            let port = local.rsplit(':').next().and_then(|p| p.parse::<u16>().ok());
            if let Some(port) = port {
                // 从 users:(("node",pid=1234,...)) 提取
                let process_info = parts.get(6).unwrap_or(&"");
                let (name, pid) = parse_ss_process(process_info);
                ports.push(PortInfo {
                    port,
                    pid,
                    process_name: name,
                    protocol: "TCP".to_string(),
                    state: "LISTENING".to_string(),
                });
            }
        }
    }
    ports.sort_by_key(|p| p.port);
    ports
}

#[cfg(unix)]
fn parse_ss_process(s: &str) -> (String, u32) {
    // users:(("node",pid=1234,fd=5))
    let name = s.split('"').nth(1).unwrap_or("").to_string();
    let pid = s.split("pid=").nth(1)
        .and_then(|p| p.split(',').next())
        .and_then(|p| p.parse::<u32>().ok())
        .unwrap_or(0);
    (name, pid)
}

#[tauri::command]
pub fn kill_port(port: u16) -> Result<String, String> {
    #[cfg(windows)]
    {
        // 找到 PID
        let out = Command::new("netstat")
            .args(["-ano"])
            .output()
            .map_err(|e| e.to_string())?;
        let text = String::from_utf8_lossy(&out.stdout);
        let mut pid: Option<u32> = None;
        for line in text.lines() {
            let parts: Vec<&str> = line.split_whitespace().collect();
            if parts.len() < 5 { continue; }
            if parts[0] != "TCP" { continue; }
            if parts[3] != "LISTENING" { continue; }
            let local_port = parts[1].rsplit(':').next().and_then(|p| p.parse::<u16>().ok());
            if local_port == Some(port) {
                pid = parts[4].parse::<u32>().ok();
                break;
            }
        }
        let pid = pid.ok_or_else(|| format!("No process listening on port {}", port))?;
        let result = Command::new("taskkill")
            .args(["/PID", &pid.to_string(), "/F"])
            .output()
            .map_err(|e| e.to_string())?;
        if result.status.success() {
            Ok(format!("Killed PID {} on port {}", pid, port))
        } else {
            Err(String::from_utf8_lossy(&result.stderr).to_string())
        }
    }
    #[cfg(unix)]
    {
        let out = Command::new("lsof")
            .args(["-ti", &format!(":{}", port)])
            .output()
            .map_err(|e| e.to_string())?;
        let pid_str = String::from_utf8_lossy(&out.stdout).trim().to_string();
        if pid_str.is_empty() {
            return Err(format!("No process on port {}", port));
        }
        Command::new("kill")
            .args(["-9", &pid_str])
            .output()
            .map_err(|e| e.to_string())?;
        Ok(format!("Killed PID {} on port {}", pid_str, port))
    }
}
