use crate::llm::LlmProvider;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;

// ── Data model ───────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServer {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    // SSE transport fields
    #[serde(default)]
    pub transport: String,   // "stdio" | "sse"
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub headers: HashMap<String, String>,
    #[serde(default)]
    pub description: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpScanResult {
    pub transport: String,
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
    pub url: String,
    pub headers: HashMap<String, String>,
    pub description: String,
    pub warnings: Vec<String>,
    pub confidence: u8,
}

// ── Config file helpers ──────────────────────────────────────────────────────

fn config_path() -> std::path::PathBuf {
    dirs_next::config_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("Claude")
        .join("claude_desktop_config.json")
}

fn read_config() -> Value {
    std::fs::read_to_string(config_path())
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}))
}

fn write_config(config: &Value) -> Result<(), String> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let json = serde_json::to_string_pretty(config).map_err(|e| e.to_string())?;
    std::fs::write(&path, json).map_err(|e| e.to_string())
}

// ── CRUD commands ────────────────────────────────────────────────────────────

#[tauri::command]
pub fn get_mcp_config_path() -> String {
    config_path().to_string_lossy().to_string()
}

#[tauri::command]
pub fn list_mcp_servers() -> Vec<McpServer> {
    let config = read_config();
    let servers = match config.get("mcpServers").and_then(|v| v.as_object()) {
        Some(s) => s,
        None => return vec![],
    };
    servers.iter().map(|(name, val)| {
        let transport = val["transport"].as_str().unwrap_or("stdio").to_string();
        McpServer {
            name: name.clone(),
            transport: transport.clone(),
            command: val["command"].as_str().unwrap_or("").to_string(),
            args: val["args"].as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                .unwrap_or_default(),
            env: val["env"].as_object()
                .map(|o| o.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect())
                .unwrap_or_default(),
            url: val["url"].as_str().unwrap_or("").to_string(),
            headers: val["headers"].as_object()
                .map(|o| o.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect())
                .unwrap_or_default(),
            description: val["description"].as_str().unwrap_or("").to_string(),
        }
    }).collect()
}

#[tauri::command]
pub fn save_mcp_server(server: McpServer) -> Result<(), String> {
    let mut config = read_config();
    let servers = config.as_object_mut().ok_or("Invalid config")?
        .entry("mcpServers").or_insert_with(|| json!({}))
        .as_object_mut().ok_or("Invalid mcpServers")?;

    let transport = if server.transport.is_empty() { "stdio" } else { &server.transport };
    let mut entry = json!({ "transport": transport, "description": server.description });

    if transport == "sse" {
        entry["url"] = json!(server.url);
        if !server.headers.is_empty() { entry["headers"] = json!(server.headers); }
    } else {
        entry["command"] = json!(server.command);
        entry["args"] = json!(server.args);
        if !server.env.is_empty() { entry["env"] = json!(server.env); }
    }

    servers.insert(server.name, entry);
    write_config(&config)
}

#[tauri::command]
pub fn delete_mcp_server(name: String) -> Result<(), String> {
    let mut config = read_config();
    if let Some(servers) = config.as_object_mut()
        .and_then(|o| o.get_mut("mcpServers"))
        .and_then(|v| v.as_object_mut())
    {
        servers.remove(&name);
    }
    write_config(&config)
}

// ── Local directory scan ─────────────────────────────────────────────────────

#[tauri::command]
pub async fn scan_mcp_local(dir: String, provider: Option<LlmProvider>) -> Result<McpScanResult, String> {
    let base = Path::new(&dir);
    if !base.exists() || !base.is_dir() {
        return Err("Directory not found".to_string());
    }

    let mut warnings: Vec<String> = vec![];

    // Find package.json
    let pkg_path = find_package_json(base);
    if pkg_path.is_none() {
        return Err("No package.json found — not an npm MCP package".to_string());
    }
    let pkg_path = pkg_path.unwrap();
    let pkg_dir = pkg_path.parent().unwrap();

    let pkg_text = std::fs::read_to_string(&pkg_path).map_err(|e| e.to_string())?;
    let pkg: Value = serde_json::from_str(&pkg_text).map_err(|e| e.to_string())?;

    if !is_mcp_package(&pkg) {
        return Err("Not an MCP package (@modelcontextprotocol/sdk not found in dependencies)".to_string());
    }

    let (entry, confidence) = infer_node_entry(pkg_dir, &pkg);
    if entry.is_empty() {
        warnings.push("Entry file not found — dist/ may not be built yet. Run 'npm run build' first.".to_string());
    }

    // Read README
    let readme = ["README.md", "readme.md", "README.txt", "README"]
        .iter()
        .find_map(|name| std::fs::read_to_string(pkg_dir.join(name)).ok())
        .unwrap_or_default();

    // Derive name and label
    let pkg_name = pkg["name"].as_str().unwrap_or(base.file_name().and_then(|n| n.to_str()).unwrap_or("mcp"));
    let bare = pkg_name.split('/').last().unwrap_or(pkg_name);
    let label = bare.replace("-mcp", "").replace("mcp-", "").replace('-', " ");
    let label = title_case(&label);

    // LLM enrichment
    let (description, env, llm_warnings) = if let Some(p) = provider {
        match llm_enrich(&p, &pkg, &readme).await {
            Ok((d, e, w)) => (d, e, w),
            Err(e) => {
                warnings.push(format!("LLM enrichment failed (skipped): {}", e));
                (pkg["description"].as_str().unwrap_or("").to_string(), HashMap::new(), vec![])
            }
        }
    } else {
        (pkg["description"].as_str().unwrap_or("").to_string(), HashMap::new(), vec![])
    };
    warnings.extend(llm_warnings);

    if confidence < 70 {
        warnings.push(format!("Entry point confidence low ({}%) — verify the path is correct.", confidence));
    }

    let args = if entry.is_empty() { vec![] } else { vec![entry.clone()] };

    Ok(McpScanResult {
        transport: "stdio".to_string(),
        name: label.to_lowercase().replace(' ', "_"),
        command: if entry.is_empty() { String::new() } else { "node".to_string() },
        args,
        env,
        url: String::new(),
        headers: HashMap::new(),
        description,
        warnings,
        confidence,
    })
}

fn find_package_json(base: &Path) -> Option<std::path::PathBuf> {
    // Direct package.json
    let direct = base.join("package.json");
    if direct.exists() {
        if let Ok(text) = std::fs::read_to_string(&direct) {
            if let Ok(pkg) = serde_json::from_str::<Value>(&text) {
                if is_mcp_package(&pkg) { return Some(direct); }
            }
        }
    }
    // Look inside node_modules
    let nm = base.join("node_modules");
    if nm.is_dir() {
        if let Ok(entries) = std::fs::read_dir(&nm) {
            for entry in entries.flatten() {
                let path = entry.path();
                let pkg_json = path.join("package.json");
                if pkg_json.exists() {
                    if let Ok(text) = std::fs::read_to_string(&pkg_json) {
                        if let Ok(pkg) = serde_json::from_str::<Value>(&text) {
                            if is_mcp_package(&pkg) { return Some(pkg_json); }
                        }
                    }
                }
            }
        }
    }
    None
}

fn is_mcp_package(pkg: &Value) -> bool {
    let markers = ["@modelcontextprotocol/sdk", "mcp", "fastmcp"];
    let deps = pkg.get("dependencies").and_then(|v| v.as_object());
    let dev_deps = pkg.get("devDependencies").and_then(|v| v.as_object());
    for m in &markers {
        if deps.map(|d| d.contains_key(*m)).unwrap_or(false) { return true; }
        if dev_deps.map(|d| d.contains_key(*m)).unwrap_or(false) { return true; }
    }
    let name = pkg["name"].as_str().unwrap_or("").to_lowercase();
    let desc = pkg["description"].as_str().unwrap_or("").to_lowercase();
    name.contains("mcp") || desc.contains("mcp")
}

fn infer_node_entry(pkg_dir: &Path, pkg: &Value) -> (String, u8) {
    // Priority 1: bin field
    if let Some(bin) = pkg.get("bin") {
        if let Some(s) = bin.as_str() {
            let p = pkg_dir.join(s);
            if p.exists() { return (p.to_string_lossy().to_string(), 95); }
        } else if let Some(obj) = bin.as_object() {
            for (_, v) in obj {
                if let Some(s) = v.as_str() {
                    let p = pkg_dir.join(s);
                    if p.exists() { return (p.to_string_lossy().to_string(), 90); }
                }
            }
        }
    }
    // Priority 2: main field
    if let Some(main) = pkg["main"].as_str().filter(|s| !s.is_empty()) {
        let p = pkg_dir.join(main);
        if p.exists() { return (p.to_string_lossy().to_string(), 75); }
    }
    // Priority 3: common fallbacks
    for fallback in &["dist/index.js", "index.js", "build/index.js", "out/index.js"] {
        let p = pkg_dir.join(fallback);
        if p.exists() { return (p.to_string_lossy().to_string(), 50); }
    }
    (String::new(), 0)
}

async fn llm_enrich(provider: &LlmProvider, pkg: &Value, readme: &str) -> Result<(String, HashMap<String, String>, Vec<String>), String> {
    let system = r#"You are helping configure an MCP (Model Context Protocol) server.
Given a package summary and README excerpt, extract:
1. A one-line description of what this MCP server does (in English, ≤80 chars)
2. Environment variables the server needs (name → placeholder value)
Reply ONLY with JSON: {"description": "...", "env": {"VAR": "YOUR_VALUE_HERE"}}"#;

    let summary = json!({
        "name": pkg["name"],
        "description": pkg["description"],
        "scripts": pkg["scripts"],
    });
    let user_content = format!(
        "package.json:\n{}\n\nREADME (first 500 chars):\n{}",
        serde_json::to_string_pretty(&summary).unwrap_or_default(),
        &readme[..readme.len().min(500)]
    );

    let body = json!({
        "model": provider.model,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user_content}
        ],
        "max_tokens": 256,
        "temperature": 0
    });

    let client = reqwest::Client::new();
    let url = format!("{}/chat/completions", provider.base_url.trim_end_matches('/'));
    let resp = client.post(&url)
        .header("Authorization", format!("Bearer {}", provider.api_key))
        .json(&body)
        .timeout(std::time::Duration::from_secs(20))
        .send().await.map_err(|e| e.to_string())?;

    let text = resp.text().await.unwrap_or_default();
    let val: Value = serde_json::from_str(&text).map_err(|e| e.to_string())?;
    let content = val["choices"][0]["message"]["content"].as_str().unwrap_or("");
    // Strip markdown fences
    let clean = content.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
    let result: Value = serde_json::from_str(clean).unwrap_or(json!({}));

    let desc = result["description"].as_str().unwrap_or("").to_string();
    let env: HashMap<String, String> = result["env"].as_object()
        .map(|o| o.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect())
        .unwrap_or_default();

    Ok((desc, env, vec![]))
}

// ── Smart text parse ─────────────────────────────────────────────────────────

#[tauri::command]
pub async fn parse_mcp_text(text: String, provider: LlmProvider) -> Result<McpScanResult, String> {
    if text.trim().is_empty() { return Err("Text is empty".to_string()); }

    // Quick heuristic: if it looks like a plain URL, treat as SSE
    let trimmed = text.trim();
    if trimmed.starts_with("http://") || trimmed.starts_with("https://") {
        if !trimmed.contains('\n') && !trimmed.contains(' ') {
            return Ok(McpScanResult {
                transport: "sse".to_string(),
                name: "remote_mcp".to_string(),
                command: String::new(),
                args: vec![],
                env: HashMap::new(),
                url: trimmed.to_string(),
                headers: HashMap::new(),
                description: "Remote MCP server".to_string(),
                warnings: vec![],
                confidence: 80,
            });
        }
    }

    let system = r#"You are a configuration parser for MCP (Model Context Protocol) servers.
Parse the given text (npm command, JSON snippet, README excerpt, etc.) and return ONLY JSON:
{
  "transport": "stdio" or "sse",
  "name": "<server_id snake_case>",
  "description": "<one-line description>",
  "command": "<npx|uvx|node|python|python3|uv|deno — stdio only>",
  "args": ["arg1", "arg2"],
  "env": {"KEY": "PLACEHOLDER_VALUE"},
  "url": "<SSE URL or empty>",
  "headers": {}
}
Rules:
- For unknown env values use placeholders like YOUR_API_KEY_HERE
- Never include shell metacharacters in args: ; & | ` $ < > ( )
- Return valid JSON only, no markdown"#;

    let body = json!({
        "model": provider.model,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": format!("Parse:\n\n{}", &text[..text.len().min(4000)])}
        ],
        "max_tokens": 512,
        "temperature": 0
    });

    let client = reqwest::Client::new();
    let url = format!("{}/chat/completions", provider.base_url.trim_end_matches('/'));
    let resp = client.post(&url)
        .header("Authorization", format!("Bearer {}", provider.api_key))
        .json(&body)
        .timeout(std::time::Duration::from_secs(30))
        .send().await.map_err(|e| e.to_string())?;

    let resp_text = resp.text().await.unwrap_or_default();
    let val: Value = serde_json::from_str(&resp_text).map_err(|e| e.to_string())?;
    let content = val["choices"][0]["message"]["content"].as_str().unwrap_or("");
    let clean = content.trim().trim_start_matches("```json").trim_start_matches("```").trim_end_matches("```").trim();
    let result: Value = serde_json::from_str(clean)
        .map_err(|_| "LLM returned invalid JSON — try again or fill manually".to_string())?;

    let transport = result["transport"].as_str().unwrap_or("stdio").to_string();
    Ok(McpScanResult {
        transport: transport.clone(),
        name: result["name"].as_str().unwrap_or("mcp_server").to_string(),
        command: if transport == "sse" { String::new() } else { result["command"].as_str().unwrap_or("").to_string() },
        args: result["args"].as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect()).unwrap_or_default(),
        env: result["env"].as_object().map(|o| o.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect()).unwrap_or_default(),
        url: result["url"].as_str().unwrap_or("").to_string(),
        headers: result["headers"].as_object().map(|o| o.iter().filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string()))).collect()).unwrap_or_default(),
        description: result["description"].as_str().unwrap_or("").to_string(),
        warnings: vec![],
        confidence: 85,
    })
}

fn title_case(s: &str) -> String {
    s.split_whitespace()
        .map(|w| {
            let mut c = w.chars();
            match c.next() {
                None => String::new(),
                Some(f) => f.to_uppercase().to_string() + c.as_str(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}
