/// proxy.rs — Caddy 反向代理管理
/// 负责：用户账号管理、Caddyfile 生成、Caddy 进程控制
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::{Command, Stdio};

// ── 数据结构 ────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyUser {
    pub username: String,
    /// bcrypt 哈希，由 caddy hash-password 生成
    pub password_hash: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProxyRule {
    pub id: String,
    /// 对外域名或 IP，例如 "agent.example.com" 或 ":8443"
    pub domain: String,
    /// 转发到本机的端口
    pub target_port: u16,
    /// agent 名称（仅展示用）
    pub agent_name: String,
    /// 启用 HTTPS（需要域名）
    pub https: bool,
    /// 允许访问的用户列表（空 = 所有已配置用户）
    pub allowed_users: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct ProxyConfig {
    pub users: Vec<ProxyUser>,
    pub rules: Vec<ProxyRule>,
    /// Caddy 可执行文件路径（留空则自动查找）
    pub caddy_path: Option<String>,
    /// Caddy 监听的管理 API 端口（默认 2019）
    pub admin_port: Option<u16>,
}

// ── 持久化路径 ──────────────────────────────────────────────

fn config_dir() -> PathBuf {
    dirs_next::data_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("agent-manager")
}

fn proxy_config_path() -> PathBuf {
    config_dir().join("proxy.json")
}

fn caddyfile_path() -> PathBuf {
    config_dir().join("Caddyfile")
}

fn load_config() -> ProxyConfig {
    let path = proxy_config_path();
    if let Ok(data) = std::fs::read_to_string(&path) {
        serde_json::from_str(&data).unwrap_or_default()
    } else {
        ProxyConfig::default()
    }
}

fn save_config(cfg: &ProxyConfig) -> Result<(), String> {
    let dir = config_dir();
    std::fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let json = serde_json::to_string_pretty(cfg).map_err(|e| e.to_string())?;
    std::fs::write(proxy_config_path(), json).map_err(|e| e.to_string())
}

// ── Caddy 查找 ──────────────────────────────────────────────

fn find_caddy(cfg: &ProxyConfig) -> Option<String> {
    // 1. 用户配置路径
    if let Some(p) = &cfg.caddy_path {
        if std::path::Path::new(p).exists() {
            return Some(p.clone());
        }
    }
    // 2. PATH 里查找
    #[cfg(windows)]
    let candidates = ["caddy.exe", "caddy"];
    #[cfg(not(windows))]
    let candidates = ["caddy"];

    for c in &candidates {
        if let Ok(out) = Command::new("where").arg(c).output() {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout);
                if let Some(line) = s.lines().next() {
                    return Some(line.trim().to_string());
                }
            }
        }
        // Unix / Mac: which
        if let Ok(out) = Command::new("which").arg(c).output() {
            if out.status.success() {
                let s = String::from_utf8_lossy(&out.stdout);
                if let Some(line) = s.lines().next() {
                    return Some(line.trim().to_string());
                }
            }
        }
    }
    // 3. 常见安装路径
    #[cfg(windows)]
    let known = [
        r"C:\ProgramData\chocolatey\bin\caddy.exe",
        r"C:\tools\caddy\caddy.exe",
    ];
    #[cfg(not(windows))]
    let known = ["/usr/bin/caddy", "/usr/local/bin/caddy", "/opt/homebrew/bin/caddy"];

    for p in &known {
        if std::path::Path::new(p).exists() {
            return Some(p.to_string());
        }
    }
    None
}

// ── Caddyfile 生成 ──────────────────────────────────────────

fn build_caddyfile(cfg: &ProxyConfig) -> String {
    if cfg.rules.is_empty() {
        return String::new();
    }

    // 建立 username -> hash 映射
    let user_map: HashMap<&str, &str> = cfg
        .users
        .iter()
        .map(|u| (u.username.as_str(), u.password_hash.as_str()))
        .collect();

    let mut out = String::new();

    // 全局选项
    let admin_port = cfg.admin_port.unwrap_or(2019);
    out.push_str(&format!(
        "{{\n    admin localhost:{admin_port}\n}}\n\n"
    ));

    for rule in &cfg.rules {
        // 确定参与认证的用户
        let auth_users: Vec<(&str, &str)> = if rule.allowed_users.is_empty() {
            // 所有用户
            cfg.users
                .iter()
                .map(|u| (u.username.as_str(), u.password_hash.as_str()))
                .collect()
        } else {
            rule.allowed_users
                .iter()
                .filter_map(|name| user_map.get(name.as_str()).map(|h| (name.as_str(), *h)))
                .collect()
        };

        // 域名行
        let domain_line = if rule.https {
            rule.domain.clone()
        } else {
            // 不启用 HTTPS：监听 http:// 或裸端口
            if rule.domain.starts_with(':') || rule.domain.parse::<u16>().is_ok() {
                format!(":{}", rule.domain.trim_start_matches(':'))
            } else {
                format!("http://{}", rule.domain)
            }
        };

        out.push_str(&format!("{domain_line} {{\n"));

        // basicauth 块（有用户才加）
        if !auth_users.is_empty() {
            out.push_str("    basicauth /* {\n");
            for (user, hash) in &auth_users {
                out.push_str(&format!("        {user} {hash}\n"));
            }
            out.push_str("    }\n");
        }

        out.push_str(&format!(
            "    reverse_proxy localhost:{}\n",
            rule.target_port
        ));
        out.push_str("}\n\n");
    }

    out
}

// ── Tauri 命令 ──────────────────────────────────────────────

/// 获取当前代理配置
#[tauri::command]
pub fn proxy_get_config() -> ProxyConfig {
    load_config()
}

/// 保存整体配置（用户 + 规则）
#[tauri::command]
pub fn proxy_save_config(config: ProxyConfig) -> Result<(), String> {
    save_config(&config)
}

/// 检测 Caddy 是否安装，返回可执行文件路径
#[tauri::command]
pub fn proxy_check_caddy() -> Option<String> {
    let cfg = load_config();
    find_caddy(&cfg)
}

/// 用 Caddy 为给定明文密码生成 bcrypt 哈希
#[tauri::command]
pub fn proxy_hash_password(caddy_path: String, plaintext: String) -> Result<String, String> {
    let out = Command::new(&caddy_path)
        .args(["hash-password", "--plaintext", &plaintext])
        .output()
        .map_err(|e| format!("无法运行 caddy: {e}"))?;

    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr);
        return Err(format!("caddy hash-password 失败: {err}"));
    }

    let hash = String::from_utf8_lossy(&out.stdout).trim().to_string();
    Ok(hash)
}

/// 生成并写入 Caddyfile，然后启动/重载 Caddy
#[tauri::command]
pub fn proxy_apply(config: ProxyConfig) -> Result<String, String> {
    // 1. 保存配置
    save_config(&config)?;

    // 2. 生成 Caddyfile
    let caddyfile = build_caddyfile(&config);
    let cf_path = caddyfile_path();
    std::fs::create_dir_all(cf_path.parent().unwrap()).map_err(|e| e.to_string())?;
    std::fs::write(&cf_path, &caddyfile).map_err(|e| e.to_string())?;

    if caddyfile.is_empty() {
        return Ok("Caddyfile 已清空（无规则）".to_string());
    }

    // 3. 找 Caddy
    let caddy = find_caddy(&config).ok_or_else(|| {
        "未找到 Caddy，请先安装：https://caddyserver.com/docs/install".to_string()
    })?;

    // 4. 先尝试 reload（如果 Caddy 已运行）
    let admin_port = config.admin_port.unwrap_or(2019);
    let reload = Command::new(&caddy)
        .args([
            "reload",
            "--config",
            cf_path.to_str().unwrap(),
            "--adapter",
            "caddyfile",
            "--address",
            &format!("localhost:{admin_port}"),
        ])
        .output();

    match reload {
        Ok(out) if out.status.success() => {
            return Ok("Caddy 配置已热重载".to_string());
        }
        _ => {
            // Caddy 未运行，启动它
        }
    }

    // 5. 启动 Caddy（后台运行）
    Command::new(&caddy)
        .args(["run", "--config", cf_path.to_str().unwrap(), "--adapter", "caddyfile"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("无法启动 Caddy: {e}"))?;

    Ok("Caddy 已启动".to_string())
}

/// 停止 Caddy（通过管理 API 发送 /stop）
#[tauri::command]
pub fn proxy_stop() -> Result<String, String> {
    let cfg = load_config();
    let caddy = find_caddy(&cfg).ok_or("未找到 Caddy")?;
    let admin_port = cfg.admin_port.unwrap_or(2019);

    let out = Command::new(&caddy)
        .args(["stop", "--address", &format!("localhost:{admin_port}")])
        .output()
        .map_err(|e| format!("无法运行 caddy stop: {e}"))?;

    if out.status.success() {
        Ok("Caddy 已停止".to_string())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        Err(format!("caddy stop 失败: {err}"))
    }
}

/// 检查 Caddy 是否正在运行（ping 管理 API）
#[tauri::command]
pub fn proxy_status() -> bool {
    let cfg = load_config();
    let admin_port = cfg.admin_port.unwrap_or(2019);
    // 尝试连接管理端口
    std::net::TcpStream::connect(format!("127.0.0.1:{admin_port}")).is_ok()
}

/// 获取当前生成的 Caddyfile 内容（调试用）
#[tauri::command]
pub fn proxy_get_caddyfile() -> String {
    std::fs::read_to_string(caddyfile_path()).unwrap_or_default()
}

/// 预览即将生成的 Caddyfile（不写入）
#[tauri::command]
pub fn proxy_preview_caddyfile(config: ProxyConfig) -> String {
    build_caddyfile(&config)
}

// ═══════════════════════════════════════════════════════════
// Cloudflare Tunnel — 临时公网分享
// ═══════════════════════════════════════════════════════════

use std::sync::{Arc, Mutex};
use std::io::{BufRead, BufReader};

/// 全局隧道进程表：agent_id → (child_process, tunnel_url)
type TunnelMap = Arc<Mutex<HashMap<String, (std::process::Child, String)>>>;

/// Tauri managed state
pub struct TunnelStore {
    pub tunnels: TunnelMap,
}

impl TunnelStore {
    pub fn new() -> Self {
        Self {
            tunnels: Arc::new(Mutex::new(HashMap::new())),
        }
    }
}

/// 查找 cloudflared 可执行文件
fn find_cloudflared() -> Option<String> {
    // 常见安装路径
    #[cfg(windows)]
    let known: &[&str] = &[
        r"C:\Program Files\cloudflared\cloudflared.exe",
        r"C:\ProgramData\chocolatey\bin\cloudflared.exe",
        r"C:\Users\Public\scoop\apps\cloudflared\current\cloudflared.exe",
    ];
    #[cfg(not(windows))]
    let known: &[&str] = &[
        "/usr/bin/cloudflared",
        "/usr/local/bin/cloudflared",
        "/opt/homebrew/bin/cloudflared",
    ];

    for p in known {
        if std::path::Path::new(p).exists() {
            return Some(p.to_string());
        }
    }

    // 用户 scoop 目录
    #[cfg(windows)]
    if let Ok(home) = std::env::var("USERPROFILE") {
        let p = format!(r"{home}\scoop\apps\cloudflared\current\cloudflared.exe");
        if std::path::Path::new(&p).exists() {
            return Some(p);
        }
        // winget 安装位置
        let p2 = format!(r"{home}\AppData\Local\Microsoft\WinGet\Links\cloudflared.exe");
        if std::path::Path::new(&p2).exists() {
            return Some(p2);
        }
    }

    // PATH 查找
    #[cfg(windows)]
    let cmd = "where";
    #[cfg(not(windows))]
    let cmd = "which";

    if let Ok(out) = Command::new(cmd).arg("cloudflared").output() {
        if out.status.success() {
            let s = String::from_utf8_lossy(&out.stdout);
            if let Some(line) = s.lines().next() {
                return Some(line.trim().to_string());
            }
        }
    }

    None
}

/// 检测 cloudflared 是否已安装
#[tauri::command]
pub fn tunnel_check_cloudflared() -> Option<String> {
    find_cloudflared()
}

/// 启动隧道，返回公网 URL
/// 通过读取 cloudflared 的 stderr 解析 trycloudflare.com URL
#[tauri::command]
pub fn tunnel_start(
    agent_id: String,
    port: u16,
    store: tauri::State<'_, TunnelStore>,
) -> Result<String, String> {
    // 如果已有隧道，先停掉
    {
        let mut map = store.tunnels.lock().unwrap();
        if let Some((mut child, _)) = map.remove(&agent_id) {
            let _ = child.kill();
        }
    }

    let cloudflared = find_cloudflared()
        .ok_or_else(|| "未找到 cloudflared，请先安装：https://github.com/cloudflare/cloudflared/releases/latest".to_string())?;

    // 启动子进程，捕获 stderr（cloudflared 把日志输出到 stderr）
    let mut cmd = Command::new(&cloudflared);
    cmd.args(["tunnel", "--url", &format!("http://localhost:{port}"), "--no-autoupdate"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());

    // 注意：故意【不】注入系统代理。
    // cloudflared 能直连 Cloudflare 边缘节点，注入 HTTP 代理反而会让所有
    // 隧道数据绕代理出口（如新加坡），实测同一资源直连 0.2s vs 走代理 7.3s（慢 33 倍），
    // 导致页面静态资源加载超时、"打不开"。
    // cloudflared 唯一依赖的是系统 DNS 能解析 *.argotunnel.com / api.trycloudflare.com，
    // 这属于 DNS 问题，应由用户修复 DNS（换公共 DNS 或热点），而非靠代理绕行。
    // 显式清掉可能从父进程继承的代理环境变量，确保直连。
    cmd.env_remove("HTTPS_PROXY")
        .env_remove("HTTP_PROXY")
        .env_remove("https_proxy")
        .env_remove("http_proxy")
        .env_remove("ALL_PROXY")
        .env_remove("all_proxy");

    let mut child = cmd.spawn()
        .map_err(|e| format!("无法启动 cloudflared: {e}"))?;

    // 从 stderr 读取行，等待出现 trycloudflare.com URL
    // 使用独立线程 + channel 实现超时，避免网络不通时无限阻塞
    let stderr = child.stderr.take().ok_or("无法读取 cloudflared 输出")?;

    // channel 传 (url_or_empty, 收集到的日志行)
    let (tx, rx) = std::sync::mpsc::channel::<(String, Vec<String>)>();
    std::thread::spawn(move || {
        let reader = BufReader::new(stderr);
        let mut log_lines: Vec<String> = Vec::new();
        for line in reader.lines() {
            let line = match line {
                Ok(l) => l,
                Err(_) => break,
            };
            // 只保留最近 20 行日志，避免太长
            if log_lines.len() >= 20 {
                log_lines.remove(0);
            }
            log_lines.push(line.clone());
            if let Some(url) = extract_tunnel_url(&line) {
                let _ = tx.send((url, log_lines));
                return;
            }
        }
        // 进程退出或读取结束，发送空字符串 + 收集到的日志
        let _ = tx.send((String::new(), log_lines));
    });

    // 最多等待 30 秒
    let tunnel_url = match rx.recv_timeout(std::time::Duration::from_secs(30)) {
        Ok((url, _)) if !url.is_empty() => url,
        Ok((_, logs)) => {
            let _ = child.kill();
            let log_text = logs.join("\n");

            // 分析日志，给出精准原因
            let hint = if log_text.contains("no such host")
                || log_text.contains("lookup")
                || log_text.contains("i/o timeout")
            {
                concat!(
                    "\n⚠️  DNS 解析失败：cloudflared 无法解析 Cloudflare 的服务器域名。\n\n",
                    "这是【系统 DNS】问题，不是代理问题。常见原因是路由器/局域网 DNS 失效。\n\n",
                    "解决方法（任选其一）：\n",
                    "  1. 把本机 DNS 改为公共 DNS：阿里 223.5.5.5 或 Google 8.8.8.8\n",
                    "     （设置 → 网络 → 适配器 → IPv4 属性 → 使用下面的 DNS）\n",
                    "  2. 临时切换到手机热点（热点自带可用的 DNS）\n",
                    "  3. 重启路由器，恢复其 DNS 服务\n",
                )
            } else if log_text.contains("certificate") || log_text.contains("tls") || log_text.contains("x509") {
                "\n⚠️  TLS 证书验证失败，可能是代理软件 MITM 证书未受信任。\n"
            } else if log_text.contains("connection refused") || log_text.contains("connect: connection refused") {
                "\n⚠️  连接被拒绝，请检查网络防火墙设置。\n"
            } else if log_text.is_empty() {
                "\n⚠️  cloudflared 无任何输出即退出，可能版本过旧或可执行文件损坏。\n"
            } else {
                ""
            };

            return Err(format!(
                "cloudflared 已退出但未生成隧道 URL。{hint}\ncloudflared 输出：\n{log_text}"
            ));
        }
        Err(_) => {
            let _ = child.kill();
            return Err(concat!(
                "等待隧道 URL 超时（30秒）。\n\n",
                "⚠️  最可能是【系统 DNS】无法解析 Cloudflare 服务器域名。\n\n",
                "解决方法（任选其一）：\n",
                "  1. 把本机 DNS 改为公共 DNS：阿里 223.5.5.5 或 Google 8.8.8.8\n",
                "  2. 临时切换到手机热点\n",
                "  3. 重启路由器，恢复其 DNS 服务\n",
                "  4. 确认本地 Agent 已启动且端口正确"
            ).to_string());
        }
    };

    // 把进程句柄存起来，供后续 stop 使用
    store.tunnels.lock().unwrap().insert(agent_id, (child, tunnel_url.clone()));

    Ok(tunnel_url)
}

/// 停止指定 agent 的隧道
#[tauri::command]
pub fn tunnel_stop(
    agent_id: String,
    store: tauri::State<'_, TunnelStore>,
) -> Result<(), String> {
    let mut map = store.tunnels.lock().unwrap();
    if let Some((mut child, _)) = map.remove(&agent_id) {
        child.kill().map_err(|e| format!("无法停止隧道: {e}"))?;
    }
    Ok(())
}

/// 停止所有隧道（退出时调用）
#[tauri::command]
pub fn tunnel_stop_all(store: tauri::State<'_, TunnelStore>) -> Result<(), String> {
    let mut map = store.tunnels.lock().unwrap();
    for (_, (mut child, _)) in map.drain() {
        let _ = child.kill();
    }
    Ok(())
}

/// 查询所有活跃隧道：返回 { agent_id: url }
#[tauri::command]
pub fn tunnel_list(store: tauri::State<'_, TunnelStore>) -> HashMap<String, String> {
    let map = store.tunnels.lock().unwrap();
    map.iter()
        .map(|(id, (_, url))| (id.clone(), url.clone()))
        .collect()
}

/// 检查单个隧道进程是否还活着
#[tauri::command]
pub fn tunnel_alive(agent_id: String, store: tauri::State<'_, TunnelStore>) -> bool {
    let mut map = store.tunnels.lock().unwrap();
    if let Some((child, _)) = map.get_mut(&agent_id) {
        // try_wait: None = still running
        matches!(child.try_wait(), Ok(None))
    } else {
        false
    }
}

/// 从 cloudflared 日志行中提取隧道 URL。
///
/// 隧道 URL 的唯一特征是：
///   https://<随机子域>.trycloudflare.com
/// 不带任何路径（结尾直接是 .com 或 .com/）。
///
/// 明确排除：
///   - www.cloudflare.com/...  （法律条款、官网链接）
///   - developers.cloudflare.com/...
///   - blog.cloudflare.com/...
fn extract_tunnel_url(line: &str) -> Option<String> {
    // 在行中查找所有 https:// 的起始位置，逐一检查
    let mut search_from = 0;
    while let Some(rel_idx) = line[search_from..].find("https://") {
        let idx = search_from + rel_idx;
        let rest = &line[idx..];

        // 取出 URL（以空白、引号、竖线、逗号、右括号结束）
        let end = rest
            .find(|c: char| c.is_whitespace() || c == '"' || c == '\'' || c == '|' || c == ',' || c == ')')
            .unwrap_or(rest.len());
        let url = rest[..end].trim_end_matches('/');

        // 必须以 .trycloudflare.com 结尾（不含子路径）
        // 形如：https://abc-def-123.trycloudflare.com
        if url.ends_with(".trycloudflare.com") {
            return Some(url.to_string());
        }

        search_from = idx + 8; // 跳过这个 "https://"，继续找下一个
    }
    None
}
