//! 跨 Agent 的 MCP 服务器中央注册表（「MCP 库」）。
//!
//! 与 Skill 库不同，MCP 条目没有文件负载——配置本身就是数据，因此整个
//! 目录存放在 SQLite `app_setting`（键 `mcp_catalog`）。装备 = 把条目写入
//! 各 Agent 的 MCP 配置（文件型 JSON，或 codex/claude CLI 的用户级注册表），
//! 卸载 = 移除对应键。状态检查只读配置文件（毫秒级），不 spawn CLI；
//! 写 CLI 型 Agent 时走子进程（与 memory_mcp 的安装流程一致）。

use serde::{Deserialize, Serialize};
use serde_json::{json, Map, Value};
use std::collections::{BTreeMap, HashMap, HashSet};
use std::path::PathBuf;

use crate::memory_mcp::{
    agent_label, cli_detail, desktop_config_path, run_agent_cli, servers_map, servers_map_mut,
};

const MCP_CATALOG_SETTING_KEY: &str = "mcp_catalog";
/// 共享记忆 MCP 的服务器名：导入时跳过，避免把开发期运行副本收编入库。
const MEMORY_SERVER_NAME: &str = "agent-manager-memory";

// ── 数据模型 ──────────────────────────────────────────────────────────────────

/// MCP 库的一个条目。name 即各 Agent 配置里的服务器 id。
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct McpCatalogEntry {
    pub name: String,
    #[serde(default)]
    pub description: String,
    /// "stdio" | "sse" | "http"；空串按 stdio 处理。
    #[serde(default)]
    pub transport: String,
    /// stdio 启动命令（如 npx / node / uvx）。
    #[serde(default)]
    pub command: String,
    #[serde(default)]
    pub args: Vec<String>,
    #[serde(default)]
    pub env: BTreeMap<String, String>,
    /// sse / http 远程地址。
    #[serde(default)]
    pub url: String,
    #[serde(default)]
    pub headers: BTreeMap<String, String>,
    /// 已装备的 Agent（MemoryMcpTarget id）。codex_cli/codex_desktop 共用
    /// 同一 codex 用户级注册表，装备时二者等价，这里保留用户勾选的原样。
    #[serde(default)]
    pub assigned_agents: Vec<String>,
    #[serde(default)]
    pub created_at: String,
    #[serde(default)]
    pub updated_at: String,
}

/// 一个库条目在某个已装备 Agent 上的状态。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct McpAgentStatus {
    pub name: String,
    pub agent: String,
    /// "installed" | "missing" | "differs"
    pub state: String,
}

/// 「从 Agent 导入」发现的候选：来自某个 Agent 的现有配置。
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct McpImportCandidate {
    pub agent: String,
    pub entry: McpCatalogEntry,
    pub already_in_catalog: bool,
}

// ── 目标与 transport 能力 ─────────────────────────────────────────────────────

fn is_mcp_target(agent: &str) -> bool {
    matches!(
        agent,
        "codex_cli"
            | "claude_cli"
            | "codex_desktop"
            | "claude_desktop"
            | "qoder"
            | "workbuddy"
            | "minimax"
            | "kimi"
            | "zcode"
    )
}

/// stdio 所有目标可用；sse/http 仅 Claude 系支持（其余 Agent 的 mcp.json
/// 远程 MCP 格式未文档化，v1 不冒险写入；zcode 同理，仅 stdio）。
fn supports_transport(agent: &str, transport: &str) -> bool {
    if transport == "stdio" {
        is_mcp_target(agent)
    } else {
        matches!(agent, "claude_cli" | "claude_desktop")
    }
}

fn normalize_transport(transport: &str) -> &str {
    if transport.is_empty() {
        "stdio"
    } else {
        transport
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_')
}

// ── 目录存取（SQLite app_setting） ───────────────────────────────────────────

fn read_catalog() -> Vec<McpCatalogEntry> {
    crate::telemetry_store::shared_store()
        .and_then(|store| store.app_setting_get::<Vec<McpCatalogEntry>>(MCP_CATALOG_SETTING_KEY))
        .unwrap_or_default()
}

fn write_catalog(entries: &[McpCatalogEntry]) -> Result<(), String> {
    let Some(store) = crate::telemetry_store::shared_store() else {
        return Err("本地数据库不可用".into());
    };
    let owned = entries.to_vec();
    store.app_setting_set(MCP_CATALOG_SETTING_KEY, &owned)
}

fn find_entry(catalog: &[McpCatalogEntry], name: &str) -> Result<McpCatalogEntry, String> {
    catalog
        .iter()
        .find(|entry| entry.name == name)
        .cloned()
        .ok_or_else(|| format!("MCP 库中不存在 {name}"))
}

/// 把 agent 加入/移出条目的 assigned_agents（去重排序）后写回目录。
fn save_assignment(name: &str, agent: &str, equipped: bool) -> Result<McpCatalogEntry, String> {
    let mut catalog = read_catalog();
    let entry = find_entry(&catalog, name)?;
    let mut assigned = entry.assigned_agents.clone();
    if equipped {
        if !assigned.iter().any(|a| a == agent) {
            assigned.push(agent.to_string());
            assigned.sort();
            assigned.dedup();
        }
    } else {
        assigned.retain(|a| a != agent);
    }
    let updated = McpCatalogEntry {
        assigned_agents: assigned,
        updated_at: chrono::Utc::now().to_rfc3339(),
        ..entry
    };
    if let Some(slot) = catalog.iter_mut().find(|e| e.name == name) {
        *slot = updated.clone();
    }
    write_catalog(&catalog)?;
    Ok(updated)
}

// ── 各 Agent 配置的读写 ──────────────────────────────────────────────────────

/// 文件型 JSON 配置的位置。claude_desktop/qoder/... 复用 memory_mcp 的解析
/// （含跨设备目录覆盖）；claude_cli 的 user scope 存在 `~/.claude.json`。
fn json_config_path(agent: &str) -> Option<PathBuf> {
    match agent {
        "claude_desktop" | "qoder" | "workbuddy" | "minimax" | "kimi" | "zcode" => {
            desktop_config_path(agent)
        }
        "claude_cli" => dirs_next::home_dir().map(|home| home.join(".claude.json")),
        _ => None,
    }
}

fn codex_config_path() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("CODEX_HOME") {
        if !home.trim().is_empty() {
            return Some(PathBuf::from(home).join("config.toml"));
        }
    }
    dirs_next::home_dir().map(|home| home.join(".codex").join("config.toml"))
}

/// 读某 Agent 当前的全部 MCP 服务器定义（name → 配置 Value）。
/// 只读文件：codex 解析 config.toml，其余读 JSON 的服务器映射节点
/// （zcode 嵌套在 mcp.servers，其余为根级 mcpServers）。
fn read_agent_mcp_servers(agent: &str) -> Map<String, Value> {
    if agent.starts_with("codex") {
        let Some(path) = codex_config_path() else {
            return Map::new();
        };
        let Ok(text) = std::fs::read_to_string(path) else {
            return Map::new();
        };
        let Ok(table) = text.parse::<toml::Table>() else {
            return Map::new();
        };
        table
            .get("mcp_servers")
            .cloned()
            .and_then(|servers| serde_json::to_value(servers).ok())
            .and_then(|value| value.as_object().cloned())
            .unwrap_or_default()
    } else {
        let Some(path) = json_config_path(agent) else {
            return Map::new();
        };
        std::fs::read_to_string(path)
            .ok()
            .and_then(|text| serde_json::from_str::<Value>(&text).ok())
            .and_then(|config| servers_map(&config, agent).cloned())
            .unwrap_or_default()
    }
}

/// 库条目 → 某 Agent JSON 配置里的 mcpServers 条目形状。
/// minimax 需要 type/enabled；claude 系远程用 {type,url,headers}。
fn agent_entry_value(agent: &str, entry: &McpCatalogEntry) -> Value {
    let transport = normalize_transport(&entry.transport);
    let mut value = json!({});
    if transport == "stdio" {
        value["command"] = json!(entry.command);
        value["args"] = json!(entry.args);
        if !entry.env.is_empty() {
            value["env"] = json!(entry.env);
        }
        if agent == "minimax" {
            value["type"] = json!("stdio");
            value["enabled"] = json!(true);
        }
    } else {
        value["type"] = json!(transport);
        value["url"] = json!(entry.url);
        if !entry.headers.is_empty() {
            value["headers"] = json!(entry.headers);
        }
        if agent == "minimax" {
            value["enabled"] = json!(true);
        }
    }
    if !entry.description.is_empty() && agent != "zcode" {
        // zcode 的服务器 schema 未文档化扩展键，保持最小 command/args 形状。
        value["description"] = json!(entry.description);
    }
    value
}

/// 从某个 Agent 配置里的服务器定义反推库条目（导入用）。
/// type 为 sse/http、或带 url 的按远程处理，其余按 stdio。
fn entry_from_config(name: &str, agent: &str, value: &Value) -> Option<McpCatalogEntry> {
    let transport = value
        .get("type")
        .and_then(Value::as_str)
        .filter(|t| *t == "sse" || *t == "http")
        .map(str::to_string)
        .or_else(|| {
            value
                .get("url")
                .and_then(Value::as_str)
                .filter(|url| !url.is_empty())
                .map(|_| "http".to_string())
        })
        .unwrap_or_else(|| "stdio".to_string());
    let string_map = |key: &str| -> BTreeMap<String, String> {
        value
            .get(key)
            .and_then(Value::as_object)
            .map(|object| {
                object
                    .iter()
                    .filter_map(|(k, v)| v.as_str().map(|s| (k.clone(), s.to_string())))
                    .collect()
            })
            .unwrap_or_default()
    };
    let command = value
        .get("command")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    let url = value
        .get("url")
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_string();
    if transport == "stdio" && command.is_empty() {
        return None;
    }
    if transport != "stdio" && url.is_empty() {
        return None;
    }
    Some(McpCatalogEntry {
        name: name.to_string(),
        description: value
            .get("description")
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_string(),
        transport,
        command,
        args: value
            .get("args")
            .and_then(Value::as_array)
            .map(|items| {
                items
                    .iter()
                    .filter_map(Value::as_str)
                    .map(str::to_string)
                    .collect()
            })
            .unwrap_or_default(),
        env: string_map("env"),
        url,
        headers: string_map("headers"),
        // 该服务器已在此 Agent 上，导入即视为已装备。
        assigned_agents: vec![agent.to_string()],
        created_at: String::new(),
        updated_at: String::new(),
    })
}

/// 忽略 description/enabled 等噪音，只保留语义字段并补默认值的形状。
fn normalized_shape(value: &Value) -> Value {
    let transport = value
        .get("type")
        .and_then(Value::as_str)
        .filter(|t| *t == "sse" || *t == "http")
        .unwrap_or("stdio");
    let mut shape = json!({ "type": transport });
    if transport == "stdio" {
        shape["command"] = value.get("command").cloned().unwrap_or(json!(""));
        shape["args"] = value.get("args").cloned().unwrap_or(json!([]));
        shape["env"] = value.get("env").cloned().unwrap_or(json!({}));
    } else {
        shape["url"] = value.get("url").cloned().unwrap_or(json!(""));
        shape["headers"] = value.get("headers").cloned().unwrap_or(json!({}));
    }
    shape
}

/// 语义相等：对象按键比较（与键序无关），数组与标量按值比较。
fn value_equal(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Object(x), Value::Object(y)) => {
            x.len() == y.len()
                && x.iter()
                    .all(|(k, v)| y.get(k).is_some_and(|w| value_equal(v, w)))
        }
        _ => a == b,
    }
}

fn entry_matches_config(agent: &str, entry: &McpCatalogEntry, configured: &Value) -> bool {
    let expected = agent_entry_value(agent, entry);
    value_equal(&normalized_shape(&expected), &normalized_shape(configured))
}

// ── 装备 / 卸载的写入路径 ────────────────────────────────────────────────────

fn write_json_entry(agent: &str, name: &str, entry_value: &Value) -> Result<(), String> {
    let path = json_config_path(agent).ok_or("无法定位 MCP 配置文件")?;
    let mut config: Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| json!({}));
    let servers = servers_map_mut(&mut config, agent)?;
    servers.insert(name.to_string(), entry_value.clone());
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&config).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("写入 {} 失败：{error}", path.display()))
}

fn remove_json_entry(agent: &str, name: &str) -> Result<(), String> {
    let path = json_config_path(agent).ok_or("无法定位 MCP 配置文件")?;
    let mut config: Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| json!({}));
    if let Some(servers) = servers_map_mut(&mut config, agent).ok() {
        servers.remove(name);
    }
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&config).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("写入 {} 失败：{error}", path.display()))
}

/// claude/codex CLI 型装备：先移除旧配置再 add（覆盖式，与
/// memory_mcp_install 一致）。env 走 `-e K=V`，远程走 `--transport`。
fn cli_equip(agent: &str, entry: &McpCatalogEntry) -> Result<(), String> {
    let cli = if agent.starts_with("codex") {
        "codex"
    } else {
        "claude"
    };
    let transport = normalize_transport(&entry.transport);
    let _ = run_agent_cli(cli, &["mcp".into(), "remove".into(), entry.name.clone()]);
    let mut args = vec!["mcp".into(), "add".into()];
    if agent == "claude_cli" {
        args.push("--scope".into());
        args.push("user".into());
        if transport != "stdio" {
            args.push("--transport".into());
            args.push(transport.to_string());
        }
    } else if transport != "stdio" {
        return Err("Codex 暂不支持远程 MCP 装备".into());
    }
    for (key, value) in &entry.env {
        args.push("-e".into());
        args.push(format!("{key}={value}"));
    }
    args.push(entry.name.clone());
    if transport == "stdio" {
        args.push("--".into());
        args.push(entry.command.clone());
        args.extend(entry.args.iter().cloned());
    } else {
        args.push(entry.url.clone());
        for (key, value) in &entry.headers {
            args.push("--header".into());
            args.push(format!("{key}: {value}"));
        }
    }
    let output = run_agent_cli(cli, &args)?;
    if !output.status.success() {
        return Err(format!(
            "配置 {} MCP 失败：{}",
            agent_label(agent),
            cli_detail(&output)
        ));
    }
    Ok(())
}

fn apply_equip(agent: &str, entry: &McpCatalogEntry) -> Result<(), String> {
    if agent == "claude_cli" || agent.starts_with("codex") {
        cli_equip(agent, entry)
    } else {
        write_json_entry(agent, &entry.name, &agent_entry_value(agent, entry))
    }
}

fn apply_unequip(agent: &str, name: &str) -> Result<(), String> {
    if agent == "claude_cli" || agent.starts_with("codex") {
        let cli = if agent.starts_with("codex") {
            "codex"
        } else {
            "claude"
        };
        let output = run_agent_cli(cli, &["mcp".into(), "remove".into(), name.to_string()])?;
        if !output.status.success() {
            return Err(format!(
                "移除 {} MCP 失败：{}",
                agent_label(agent),
                cli_detail(&output)
            ));
        }
        Ok(())
    } else {
        remove_json_entry(agent, name)
    }
}

// ── Tauri 命令 ────────────────────────────────────────────────────────────────

#[tauri::command]
pub async fn mcp_catalog_list() -> Vec<McpCatalogEntry> {
    read_catalog()
}

#[tauri::command]
pub async fn mcp_catalog_upsert(entry: McpCatalogEntry) -> Result<McpCatalogEntry, String> {
    let name = entry.name.trim().to_string();
    if !valid_name(&name) {
        return Err("名称只能包含小写字母、数字、- 和 _".into());
    }
    let transport = normalize_transport(&entry.transport).to_string();
    if transport == "stdio" && entry.command.trim().is_empty() {
        return Err("stdio 服务器必须提供启动命令".into());
    }
    if transport != "stdio" && entry.url.trim().is_empty() {
        return Err("远程服务器必须提供 URL".into());
    }
    let mut catalog = read_catalog();
    let now = chrono::Utc::now().to_rfc3339();
    let created_at = catalog
        .iter()
        .find(|e| e.name == name)
        .map(|e| e.created_at.clone())
        .filter(|t| !t.is_empty())
        .unwrap_or_else(|| now.clone());
    let mut assigned: Vec<String> = entry
        .assigned_agents
        .into_iter()
        .filter(|agent| is_mcp_target(agent) && supports_transport(agent, &transport))
        .collect();
    assigned.sort();
    assigned.dedup();
    let next = McpCatalogEntry {
        name: name.clone(),
        description: entry.description.trim().to_string(),
        transport,
        command: entry.command.trim().to_string(),
        args: entry.args,
        env: entry.env,
        url: entry.url.trim().to_string(),
        headers: entry.headers,
        assigned_agents: assigned,
        created_at,
        updated_at: now,
    };
    match catalog.iter_mut().find(|e| e.name == name) {
        Some(slot) => *slot = next.clone(),
        None => catalog.push(next.clone()),
    }
    write_catalog(&catalog)?;
    Ok(next)
}

/// 删除库条目。已写入各 Agent 的配置不动（与 Skill 删除语义一致），
/// 需要先在装备对话框逐个卸载。
#[tauri::command]
pub async fn mcp_catalog_delete(name: String) -> Result<(), String> {
    let mut catalog = read_catalog();
    let before = catalog.len();
    catalog.retain(|entry| entry.name != name);
    if catalog.len() == before {
        return Err(format!("MCP 库中不存在 {name}"));
    }
    write_catalog(&catalog)
}

/// 装备：写入目标 Agent 配置，并把 agent 记入 assigned_agents。
#[tauri::command]
pub async fn mcp_equip(name: String, agent: String) -> Result<McpCatalogEntry, String> {
    if !is_mcp_target(&agent) {
        return Err(format!("暂不支持 {agent} 的 MCP 配置"));
    }
    let catalog = read_catalog();
    let entry = find_entry(&catalog, &name)?;
    if !supports_transport(&agent, normalize_transport(&entry.transport)) {
        return Err(format!("{} 不支持该 transport", agent_label(&agent)));
    }
    apply_equip(&agent, &entry)?;
    save_assignment(&name, &agent, true)
}

/// 卸载：从目标 Agent 配置移除，并从 assigned_agents 去掉。
#[tauri::command]
pub async fn mcp_unequip(name: String, agent: String) -> Result<McpCatalogEntry, String> {
    if !is_mcp_target(&agent) {
        return Err(format!("暂不支持 {agent} 的 MCP 配置"));
    }
    apply_unequip(&agent, &name)?;
    save_assignment(&name, &agent, false)
}

/// 全部已装备条目 × Agent 的状态总览。只读配置文件，不 spawn CLI。
#[tauri::command]
pub async fn mcp_status_all() -> Vec<McpAgentStatus> {
    let catalog = read_catalog();
    let mut cache: HashMap<String, Map<String, Value>> = HashMap::new();
    let mut result = Vec::new();
    for entry in &catalog {
        for agent in &entry.assigned_agents {
            if !is_mcp_target(agent) {
                continue;
            }
            let servers = cache
                .entry(agent.clone())
                .or_insert_with(|| read_agent_mcp_servers(agent));
            let state = match servers.get(&entry.name) {
                None => "missing",
                Some(configured) => {
                    if entry_matches_config(agent, entry, configured) {
                        "installed"
                    } else {
                        "differs"
                    }
                }
            };
            result.push(McpAgentStatus {
                name: entry.name.clone(),
                agent: agent.clone(),
                state: state.into(),
            });
        }
    }
    result
}

/// 扫描 7 个配置源，收集可导入的现有 MCP 服务器（跳过共享记忆 MCP，
/// 按 name 去重）。
#[tauri::command]
pub async fn mcp_import_from_agents() -> Vec<McpImportCandidate> {
    let catalog_names: HashSet<String> = read_catalog()
        .iter()
        .map(|entry| entry.name.clone())
        .collect();
    let mut seen: HashSet<String> = HashSet::new();
    let mut result = Vec::new();
    // codex_cli 与 codex_desktop 共用同一份 config.toml，扫一次即可。
    for agent in [
        "claude_cli",
        "claude_desktop",
        "codex_cli",
        "qoder",
        "workbuddy",
        "minimax",
        "kimi",
        "zcode",
    ] {
        for (name, value) in read_agent_mcp_servers(agent) {
            if name == MEMORY_SERVER_NAME || !seen.insert(name.clone()) {
                continue;
            }
            if let Some(entry) = entry_from_config(&name, agent, &value) {
                let already = catalog_names.contains(&name);
                result.push(McpImportCandidate {
                    agent: agent.to_string(),
                    entry,
                    already_in_catalog: already,
                });
            }
        }
    }
    result
}

/// 一键同步：把 assigned 给该 Agent 的所有条目按库内配置重写（漂移修复）。
#[tauri::command]
pub async fn mcp_sync_agent(agent: String) -> Result<(), String> {
    if !is_mcp_target(&agent) {
        return Err(format!("暂不支持 {agent} 的 MCP 配置"));
    }
    let catalog = read_catalog();
    let assigned: Vec<&McpCatalogEntry> = catalog
        .iter()
        .filter(|entry| entry.assigned_agents.iter().any(|a| a == &agent))
        .collect();
    if assigned.is_empty() {
        return Err("没有装备到该 Agent 的条目".into());
    }
    let mut errors = Vec::new();
    for entry in &assigned {
        if !supports_transport(&agent, normalize_transport(&entry.transport)) {
            errors.push(format!(
                "{}：{} 不支持该 transport",
                entry.name,
                agent_label(&agent)
            ));
            continue;
        }
        if let Err(error) = apply_equip(&agent, entry) {
            errors.push(format!("{}：{error}", entry.name));
        }
    }
    if !errors.is_empty() {
        return Err(errors.join("；"));
    }
    Ok(())
}

// ── 单元测试 ─────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_must_be_lowercase_ids() {
        assert!(valid_name("playwright"));
        assert!(valid_name("my-mcp_2"));
        assert!(!valid_name(""));
        assert!(!valid_name("My MCP"));
        assert!(!valid_name("服务器"));
    }

    #[test]
    fn minimax_stdio_entry_has_type_and_enabled() {
        let entry = McpCatalogEntry {
            name: "demo".into(),
            transport: "stdio".into(),
            command: "npx".into(),
            args: vec!["-y".into(), "@playwright/mcp".into()],
            ..Default::default()
        };
        let value = agent_entry_value("minimax", &entry);
        assert_eq!(value["type"], "stdio");
        assert_eq!(value["enabled"], true);
        assert_eq!(value["command"], "npx");
        // 非 minimax 的文件型 Agent 不带 type/enabled
        let plain = agent_entry_value("qoder", &entry);
        assert!(plain.get("type").is_none());
        assert!(plain.get("enabled").is_none());
    }

    #[test]
    fn zcode_entry_keeps_minimal_shape() {
        let entry = McpCatalogEntry {
            name: "demo".into(),
            transport: "stdio".into(),
            command: "npx".into(),
            args: vec!["-y".into(), "x".into()],
            description: "描述".into(),
            ..Default::default()
        };
        let value = agent_entry_value("zcode", &entry);
        assert_eq!(value["command"], "npx");
        // zcode schema 未文档化扩展键：不写 description/type/enabled。
        assert!(value.get("description").is_none());
        assert!(value.get("type").is_none());
        // 用户手工配置的最小 command/args 形状应视为已安装。
        let configured = json!({"command": "npx", "args": ["-y", "x"]});
        assert!(entry_matches_config("zcode", &entry, &configured));
        // zcode 仅支持 stdio。
        assert!(supports_transport("zcode", "stdio"));
        assert!(!supports_transport("zcode", "sse"));
    }

    #[test]
    fn claude_desktop_remote_entry_uses_type_url() {
        let entry = McpCatalogEntry {
            name: "remote".into(),
            transport: "sse".into(),
            url: "https://example.com/sse".into(),
            ..Default::default()
        };
        let value = agent_entry_value("claude_desktop", &entry);
        assert_eq!(value["type"], "sse");
        assert_eq!(value["url"], "https://example.com/sse");
    }

    #[test]
    fn shape_comparison_ignores_noise_and_key_order() {
        let expected = json!({"type": "stdio", "command": "npx", "args": ["-y", "x"], "env": {"A": "1", "B": "2"}});
        // 配置侧：多了 enabled/description、env 键序不同、type 缺省 → 仍视为一致
        let configured = json!({"command": "npx", "args": ["-y", "x"], "env": {"B": "2", "A": "1"}, "enabled": true, "description": "whatever"});
        assert!(value_equal(
            &normalized_shape(&expected),
            &normalized_shape(&configured)
        ));
        // 参数不同 → 不一致
        let drifted = json!({"type": "stdio", "command": "npx", "args": ["-y", "y"], "env": {"A": "1", "B": "2"}});
        assert!(!value_equal(
            &normalized_shape(&expected),
            &normalized_shape(&drifted)
        ));
    }

    #[test]
    fn imports_stdio_and_remote_shapes() {
        let stdio = json!({"command": "npx", "args": ["-y", "@playwright/mcp"], "env": {"K": "V"}});
        let entry = entry_from_config("playwright", "claude_cli", &stdio).unwrap();
        assert_eq!(entry.transport, "stdio");
        assert_eq!(entry.command, "npx");
        assert_eq!(entry.env.get("K").map(String::as_str), Some("V"));
        assert_eq!(entry.assigned_agents, vec!["claude_cli".to_string()]);

        let remote = json!({"type": "sse", "url": "https://example.com/sse"});
        let entry = entry_from_config("remote", "claude_desktop", &remote).unwrap();
        assert_eq!(entry.transport, "sse");
        assert_eq!(entry.url, "https://example.com/sse");

        // 无命令的畸形条目不导入
        assert!(entry_from_config("broken", "kimi", &json!({"args": ["x"]})).is_none());
    }
}
