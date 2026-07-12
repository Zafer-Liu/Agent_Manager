use serde::{Deserialize, Serialize};

#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct GithubRepoInfo {
    pub name: String,
    pub full_name: String,
    pub description: String,
    pub stars: u32,
    pub language: String,
    pub default_branch: String,
    pub clone_url: String,
    pub readme_excerpt: String,
}

fn base64_decode(input: &str) -> Vec<u8> {
    let table: [u8; 128] = {
        let mut t = [255u8; 128];
        for (i, &c) in b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/"
            .iter()
            .enumerate()
        {
            t[c as usize] = i as u8;
        }
        t
    };
    let bytes: Vec<u8> = input
        .bytes()
        .filter(|&b| b != b'\n' && b != b'\r' && b != b' ')
        .collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    let mut i = 0;
    while i + 3 < bytes.len() {
        let b = &bytes[i..i + 4];
        let vals: Vec<u8> = b
            .iter()
            .map(|&c| if c == b'=' { 0 } else { table[c as usize] })
            .collect();
        out.push((vals[0] << 2) | (vals[1] >> 4));
        if b[2] != b'=' {
            out.push((vals[1] << 4) | (vals[2] >> 2));
        }
        if b[3] != b'=' {
            out.push((vals[2] << 6) | vals[3]);
        }
        i += 4;
    }
    out
}

/// Detect system proxy: env vars first, then Windows registry.
pub fn detect_system_proxy() -> Option<String> {
    // 1. Environment variables
    for var in &["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy"] {
        if let Ok(val) = std::env::var(var) {
            if !val.is_empty() {
                return Some(val);
            }
        }
    }
    // 2. Windows registry
    #[cfg(target_os = "windows")]
    {
        if let Some(proxy) = read_wininet_proxy() {
            return Some(proxy);
        }
    }
    None
}

#[cfg(target_os = "windows")]
fn read_wininet_proxy() -> Option<String> {
    let output = std::process::Command::new("reg")
        .args([
            "query",
            r"HKCU\Software\Microsoft\Windows\CurrentVersion\Internet Settings",
            "/v",
            "ProxyServer",
        ])
        .output()
        .ok()?;
    let text = String::from_utf8_lossy(&output.stdout);
    for line in text.lines() {
        if line.trim_start().starts_with("ProxyServer") {
            // Format: "    ProxyServer    REG_SZ    127.0.0.1:7890"
            let parts: Vec<&str> = line.splitn(4, "REG_SZ").collect();
            if let Some(val) = parts.get(1) {
                let proxy = val.trim().to_string();
                if !proxy.is_empty() {
                    // Ensure http:// prefix
                    if proxy.starts_with("http") {
                        return Some(proxy);
                    } else {
                        return Some(format!("http://{}", proxy));
                    }
                }
            }
        }
    }
    None
}

/// Parse GitHub URL in multiple formats into (owner, repo).
fn parse_github_url(url: &str) -> Option<(String, String)> {
    let url = url.trim().trim_end_matches('/');
    // Strip common prefixes
    let path = url
        .trim_start_matches("https://")
        .trim_start_matches("http://")
        .trim_start_matches("github.com/");
    let parts: Vec<&str> = path.splitn(3, '/').collect();
    if parts.len() >= 2 {
        let owner = parts[0].to_string();
        let repo = parts[1].trim_end_matches(".git").to_string();
        if !owner.is_empty() && !repo.is_empty() {
            return Some((owner, repo));
        }
    }
    None
}

/// Token 持久化文件路径：与其它配置同目录（%APPDATA%\agent-manager\github.json）。
fn token_config_path() -> std::path::PathBuf {
    dirs_next::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("agent-manager")
        .join("github.json")
}

#[derive(Serialize, Deserialize, Default)]
struct GithubConfig {
    token: String,
}

/// 读取可选的 GitHub Token（提高 API 速率限制：未认证 60 次/小时 → 认证 5000 次/小时）。
/// 优先级：用户在 UI 中保存的 Token（配置文件） > GITHUB_TOKEN > GH_TOKEN 环境变量。
fn github_token() -> Option<String> {
    // 1. UI 保存的配置文件
    if let Ok(data) = std::fs::read_to_string(token_config_path()) {
        if let Ok(cfg) = serde_json::from_str::<GithubConfig>(&data) {
            let t = cfg.token.trim().to_string();
            if !t.is_empty() {
                return Some(t);
            }
        }
    }
    // 2. 环境变量
    for key in ["GITHUB_TOKEN", "GH_TOKEN"] {
        if let Ok(v) = std::env::var(key) {
            let v = v.trim().to_string();
            if !v.is_empty() {
                return Some(v);
            }
        }
    }
    None
}

/// 保存 GitHub Token 到配置文件（空字符串表示清除）。
#[tauri::command]
pub fn github_save_token(token: String) -> Result<(), String> {
    let path = token_config_path();
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let cfg = GithubConfig {
        token: token.trim().to_string(),
    };
    let json = serde_json::to_string_pretty(&cfg).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())
}

/// 查询当前是否已配置 Token（不返回明文，只返回是否存在 + 来源）。
/// 返回 (是否已配置, 是否来自环境变量)。
#[tauri::command]
pub fn github_token_status() -> (bool, bool) {
    // 配置文件里的 token
    let from_file = std::fs::read_to_string(token_config_path())
        .ok()
        .and_then(|d| serde_json::from_str::<GithubConfig>(&d).ok())
        .map(|c| !c.token.trim().is_empty())
        .unwrap_or(false);
    if from_file {
        return (true, false);
    }
    // 环境变量
    let from_env = ["GITHUB_TOKEN", "GH_TOKEN"].iter().any(|k| {
        std::env::var(k)
            .map(|v| !v.trim().is_empty())
            .unwrap_or(false)
    });
    (from_env, from_env)
}

/// 构建带正确 header、代理、可选 Token 的 GitHub API 客户端。
fn build_github_client() -> Result<reqwest::Client, String> {
    use reqwest::header::{HeaderMap, HeaderValue, ACCEPT, AUTHORIZATION};

    let mut headers = HeaderMap::new();
    headers.insert(
        ACCEPT,
        HeaderValue::from_static("application/vnd.github+json"),
    );
    headers.insert(
        "X-GitHub-Api-Version",
        HeaderValue::from_static("2022-11-28"),
    );

    if let Some(token) = github_token() {
        if let Ok(mut val) = HeaderValue::from_str(&format!("Bearer {token}")) {
            val.set_sensitive(true);
            headers.insert(AUTHORIZATION, val);
        }
    }

    let mut client_builder = reqwest::Client::builder()
        .user_agent("agent-manager/0.2.3")
        .default_headers(headers)
        .timeout(std::time::Duration::from_secs(15));

    if let Some(proxy_url) = detect_system_proxy() {
        if let Ok(proxy) = reqwest::Proxy::all(&proxy_url) {
            client_builder = client_builder.proxy(proxy);
        }
    }

    client_builder.build().map_err(|e| e.to_string())
}

/// 根据失败的响应生成可操作的中文错误信息。
async fn github_error_message(resp: reqwest::Response, owner: &str, repo: &str) -> String {
    let status = resp.status();

    // 读取速率限制头（在消费 body 之前）
    let remaining = resp
        .headers()
        .get("x-ratelimit-remaining")
        .and_then(|v| v.to_str().ok())
        .map(|s| s.to_string());
    let reset = resp
        .headers()
        .get("x-ratelimit-reset")
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<i64>().ok());

    // 尝试读取 GitHub 返回的错误消息体
    let body = resp.text().await.unwrap_or_default();
    let api_msg = serde_json::from_str::<serde_json::Value>(&body)
        .ok()
        .and_then(|j| j["message"].as_str().map(|s| s.to_string()))
        .unwrap_or_default();

    match status.as_u16() {
        403 | 429 => {
            // 速率限制
            if remaining.as_deref() == Some("0") {
                let when = reset
                    .map(|ts| {
                        let now = std::time::SystemTime::now()
                            .duration_since(std::time::UNIX_EPOCH)
                            .map(|d| d.as_secs() as i64)
                            .unwrap_or(0);
                        let mins = ((ts - now).max(0) + 59) / 60;
                        format!("约 {mins} 分钟后恢复")
                    })
                    .unwrap_or_else(|| "稍后".to_string());
                format!(
                    "GitHub API 速率受限（未登录每小时仅 60 次，{when}）。\n\n\
                     解决方法：\n\
                     • 展开上方「GitHub Token（可选）」填入 Token（额度提升到 5000 次/小时）\n\
                     • 或等待额度恢复后重试\n\
                     • 也可直接复制仓库地址手动 git clone"
                )
            } else {
                format!(
                    "GitHub 拒绝访问（403）。\n可能是代理出口 IP 被限流，或仓库需要授权。\n\
                     GitHub 返回：{}",
                    if api_msg.is_empty() {
                        "（无）"
                    } else {
                        &api_msg
                    }
                )
            }
        }
        404 => format!(
            "未找到仓库 {owner}/{repo}（404）。\n请检查仓库地址是否正确，或该仓库是否为私有。"
        ),
        401 => "GitHub 认证失败（401）。\n请检查 GITHUB_TOKEN 是否有效。".to_string(),
        _ => format!(
            "GitHub API 返回 {}{}。",
            status,
            if api_msg.is_empty() {
                String::new()
            } else {
                format!("：{api_msg}")
            }
        ),
    }
}

#[tauri::command]
pub async fn github_fetch_repo_info(url: String) -> Result<GithubRepoInfo, String> {
    let (owner, repo) =
        parse_github_url(&url).ok_or_else(|| format!("无法解析 GitHub URL: {}", url))?;

    let api_url = format!("https://api.github.com/repos/{}/{}", owner, repo);

    let client = build_github_client()?;

    let resp = client
        .get(&api_url)
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;

    if !resp.status().is_success() {
        return Err(github_error_message(resp, &owner, &repo).await);
    }

    let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;

    let name = json["name"].as_str().unwrap_or(&repo).to_string();
    let full_name = json["full_name"].as_str().unwrap_or("").to_string();
    let description = json["description"].as_str().unwrap_or("").to_string();
    let stars = json["stargazers_count"].as_u64().unwrap_or(0) as u32;
    let language = json["language"].as_str().unwrap_or("Unknown").to_string();
    let default_branch = json["default_branch"]
        .as_str()
        .unwrap_or("main")
        .to_string();
    let clone_url = json["clone_url"].as_str().unwrap_or("").to_string();

    // Fetch README
    let readme_excerpt = fetch_readme_excerpt(&client, &owner, &repo, &default_branch)
        .await
        .unwrap_or_default();

    Ok(GithubRepoInfo {
        name,
        full_name,
        description,
        stars,
        language,
        default_branch,
        clone_url,
        readme_excerpt,
    })
}

async fn fetch_readme_excerpt(
    client: &reqwest::Client,
    owner: &str,
    repo: &str,
    _branch: &str,
) -> Option<String> {
    let url = format!("https://api.github.com/repos/{}/{}/readme", owner, repo);
    let resp = client.get(&url).send().await.ok()?;
    if !resp.status().is_success() {
        return None;
    }
    let json: serde_json::Value = resp.json().await.ok()?;
    let encoded = json["content"].as_str()?;
    let bytes = base64_decode(encoded);
    let text = String::from_utf8_lossy(&bytes).to_string();
    // Return first 500 chars
    let excerpt: String = text.chars().take(500).collect();
    Some(excerpt)
}

#[tauri::command]
pub async fn github_clone_repo(
    clone_url: String,
    repo_name: String,
    target_dir: String,
) -> Result<String, String> {
    // Determine destination directory
    let dest = if target_dir.trim().is_empty() {
        let home = dirs_next::home_dir().ok_or_else(|| "无法获取用户主目录".to_string())?;
        home.join("agent-repos").join(&repo_name)
    } else {
        std::path::PathBuf::from(target_dir.trim()).join(&repo_name)
    };

    if dest.exists() {
        return Err(format!("目标目录已存在: {}", dest.display()));
    }

    let dest_str = dest.to_string_lossy().to_string();

    let mut cmd = std::process::Command::new("git");
    cmd.args(["clone", "--depth", "1", &clone_url, &dest_str]);

    // Inject proxy into git subprocess
    if let Some(proxy_url) = detect_system_proxy() {
        cmd.env("HTTP_PROXY", &proxy_url);
        cmd.env("HTTPS_PROXY", &proxy_url);
        cmd.env("http_proxy", &proxy_url);
        cmd.env("https_proxy", &proxy_url);
    }

    let output = cmd
        .output()
        .map_err(|e| format!("无法启动 git: {}。请确认 git 已安装并在 PATH 中", e))?;

    if output.status.success() {
        Ok(dest_str)
    } else {
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();
        Err(format!("git clone failed: {}", stderr))
    }
}

#[tauri::command]
pub fn github_check_git() -> bool {
    std::process::Command::new("git")
        .arg("--version")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

#[tauri::command]
pub fn github_get_proxy() -> String {
    detect_system_proxy().unwrap_or_default()
}
