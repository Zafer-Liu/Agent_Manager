use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::collections::HashMap;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpServer {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
    pub env: HashMap<String, String>,
}

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
    servers
        .iter()
        .map(|(name, val)| McpServer {
            name: name.clone(),
            command: val["command"].as_str().unwrap_or("").to_string(),
            args: val["args"]
                .as_array()
                .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_string())).collect())
                .unwrap_or_default(),
            env: val["env"]
                .as_object()
                .map(|o| {
                    o.iter()
                        .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                        .collect()
                })
                .unwrap_or_default(),
        })
        .collect()
}

#[tauri::command]
pub fn save_mcp_server(server: McpServer) -> Result<(), String> {
    let mut config = read_config();
    let servers = config
        .as_object_mut()
        .ok_or("Invalid config")?
        .entry("mcpServers")
        .or_insert_with(|| json!({}))
        .as_object_mut()
        .ok_or("Invalid mcpServers")?;

    let mut entry = json!({
        "command": server.command,
        "args": server.args,
    });
    if !server.env.is_empty() {
        entry["env"] = json!(server.env);
    }
    servers.insert(server.name, entry);
    write_config(&config)
}

#[tauri::command]
pub fn delete_mcp_server(name: String) -> Result<(), String> {
    let mut config = read_config();
    if let Some(servers) = config
        .as_object_mut()
        .and_then(|o| o.get_mut("mcpServers"))
        .and_then(|v| v.as_object_mut())
    {
        servers.remove(&name);
    }
    write_config(&config)
}
