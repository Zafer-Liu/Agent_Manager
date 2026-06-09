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
    let bytes: Vec<u8> = input.bytes().filter(|&b| b != b'\n' && b != b'\r' && b != b' ').collect();
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

#[tauri::command]
pub async fn github_fetch_repo_info(url: String) -> Result<GithubRepoInfo, String> {
    let (owner, repo) = parse_github_url(&url)
        .ok_or_else(|| format!("无法解析 GitHub URL: {}", url))?;

    let api_url = format!("https://api.github.com/repos/{}/{}", owner, repo);

    let mut client_builder = reqwest::Client::builder()
        .user_agent("agent-manager/0.2.1")
        .timeout(std::time::Duration::from_secs(15));

    if let Some(proxy_url) = detect_system_proxy() {
        if let Ok(proxy) = reqwest::Proxy::all(&proxy_url) {
            client_builder = client_builder.proxy(proxy);
        }
    }

    let client = client_builder.build().map_err(|e| e.to_string())?;

    let resp = client
        .get(&api_url)
        .send()
        .await
        .map_err(|e| format!("请求失败: {}", e))?;

    if !resp.status().is_success() {
        let status = resp.status();
        return Err(format!("GitHub API 返回 {}: {}", status, api_url));
    }

    let json: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;

    let name = json["name"].as_str().unwrap_or(&repo).to_string();
    let full_name = json["full_name"].as_str().unwrap_or("").to_string();
    let description = json["description"].as_str().unwrap_or("").to_string();
    let stars = json["stargazers_count"].as_u64().unwrap_or(0) as u32;
    let language = json["language"].as_str().unwrap_or("Unknown").to_string();
    let default_branch = json["default_branch"].as_str().unwrap_or("main").to_string();
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
        let home = dirs_next::home_dir()
            .ok_or_else(|| "无法获取用户主目录".to_string())?;
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
