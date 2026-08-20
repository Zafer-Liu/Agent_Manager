//! 受支持 Agent 的统一数据源注册表。
//!
//! 每种 Agent 的转录目录、MCP 配置文件与 Hook 能力集中在这里定义。
//! 默认路径按当前操作系统用户自动探测；不同设备上安装位置不同时，用户可以在
//! 记忆中心把某个 Agent 的目录改到真实位置，覆盖值持久化在
//! `agent_source_paths.json`（属于当前操作系统用户，不属于任何项目目录）。

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// 全部受支持的会话/Token 来源。
pub const AGENT_SOURCE_IDS: [&str; 6] =
    ["codex", "claude", "qoder", "workbuddy", "minimax", "kimi"];

pub fn is_supported_source(id: &str) -> bool {
    AGENT_SOURCE_IDS.contains(&id)
}

pub fn agent_label(id: &str) -> &'static str {
    match id {
        "codex" => "Codex",
        "claude" => "Claude Code",
        "qoder" => "Qoder",
        "workbuddy" => "WorkBuddy",
        "minimax" => "MiniMax Code",
        "kimi" => "Kimi",
        _ => "Agent",
    }
}

/// 该 Agent 是否支持写入命令式 Hook。MiniMax Code 与 Kimi 没有文档化的
/// 命令 Hook 入口，会话监听依靠本地转录扫描，不需要 Hook。
pub fn supports_hooks(id: &str) -> bool {
    matches!(id, "codex" | "claude" | "qoder" | "workbuddy")
}

// ── 跨设备路径覆盖 ────────────────────────────────────────────────────────────

#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct AgentSourceOverride {
    /// 自定义转录根目录；None 表示使用自动探测的默认目录。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub transcript_roots: Option<Vec<String>>,
    /// 自定义配置主目录（用于定位 MCP 配置等）；None 表示使用默认目录。
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub config_home: Option<String>,
}

fn overrides_path() -> PathBuf {
    dirs_next::config_dir()
        .unwrap_or_else(|| PathBuf::from("."))
        .join("agent-manager")
        .join("agent_source_paths.json")
}

fn read_overrides() -> HashMap<String, AgentSourceOverride> {
    std::fs::read_to_string(overrides_path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default()
}

fn write_overrides(overrides: &HashMap<String, AgentSourceOverride>) -> Result<(), String> {
    let path = overrides_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::write(
        &path,
        serde_json::to_string_pretty(overrides).map_err(|error| error.to_string())?,
    )
    .map_err(|error| format!("写入 {} 失败：{error}", path.display()))
}

fn override_for(id: &str) -> AgentSourceOverride {
    read_overrides().remove(id).unwrap_or_default()
}

// ── 默认目录探测 ──────────────────────────────────────────────────────────────

fn home() -> Option<PathBuf> {
    dirs_next::home_dir()
}

/// Kimi 桌面端把数据根登记在 `%APPDATA%/kimi-desktop/daimon-storage.json` 的
/// `shareDir` 字段；会话转录在 `<shareDir>/daimon/runtime/kimi-code/home` 下。
/// 独立 Kimi Code CLI 则使用 `~/.kimi-code`。两者都会探测，跨设备无需改代码。
fn kimi_code_home_candidates() -> Vec<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(app_data) = std::env::var("APPDATA") {
        let marker = PathBuf::from(app_data)
            .join("kimi-desktop")
            .join("daimon-storage.json");
        if let Some(share_dir) = std::fs::read_to_string(&marker)
            .ok()
            .and_then(|text| serde_json::from_str::<serde_json::Value>(&text).ok())
            .and_then(|value| {
                value
                    .get("shareDir")
                    .and_then(serde_json::Value::as_str)
                    .map(str::to_string)
            })
        {
            candidates.push(
                PathBuf::from(share_dir)
                    .join("daimon")
                    .join("runtime")
                    .join("kimi-code")
                    .join("home"),
            );
        }
    }
    if let Some(home) = home() {
        candidates.push(home.join(".kimi-code"));
    }
    candidates
}

fn kimi_code_home() -> Option<PathBuf> {
    let candidates = kimi_code_home_candidates();
    // 优先返回真实存在的候选；都不存在时返回第一个，让界面显示默认探测目标。
    candidates
        .iter()
        .find(|path| path.is_dir())
        .cloned()
        .or_else(|| candidates.into_iter().next())
}

/// Qoder 国际版数据在 `~/.qoder`，国内版在 `~/.qoder-cn`，两个版本可能
/// 同时安装。转录扫描会覆盖两个候选；配置定位则从其中选出活跃的一个。
fn qoder_home_candidates() -> Vec<PathBuf> {
    home()
        .map(|home| vec![home.join(".qoder"), home.join(".qoder-cn")])
        .unwrap_or_default()
}

/// 目录下是否存在真实会话转录（`projects/**/*.jsonl`），遍历有上限。
fn qoder_has_transcripts(home_dir: &Path) -> bool {
    let mut pending = vec![home_dir.join("projects")];
    let mut visited = 0usize;
    while let Some(directory) = pending.pop() {
        if visited >= 2_000 {
            break;
        }
        let Ok(entries) = std::fs::read_dir(&directory) else {
            continue;
        };
        for entry in entries.flatten() {
            visited += 1;
            let path = entry.path();
            if path.is_dir() {
                pending.push(path);
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) == Some("jsonl") {
                return true;
            }
        }
    }
    false
}

/// 选出实际在用的 Qoder 配置主目录：有真实会话转录的优先；都有或都没有
/// 时取 settings.json 最近修改的；完全无法判断时回退到国际版路径。
fn qoder_home() -> Option<PathBuf> {
    let score = |dir: &Path| -> (bool, u64) {
        let modified = std::fs::metadata(dir.join("settings.json"))
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|time| time.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|duration| duration.as_secs())
            .unwrap_or(0);
        (qoder_has_transcripts(dir), modified)
    };
    let mut best: Option<PathBuf> = None;
    for candidate in qoder_home_candidates() {
        best = match best {
            None => Some(candidate),
            Some(current) if score(&candidate) > score(&current) => Some(candidate),
            current => current,
        };
    }
    best
}

fn default_transcript_roots(id: &str) -> Vec<PathBuf> {
    let Some(home) = home() else {
        return Vec::new();
    };
    match id {
        "codex" => vec![
            home.join(".codex").join("sessions"),
            home.join(".codex").join("archived_sessions"),
        ],
        "claude" => vec![home.join(".claude").join("projects")],
        "qoder" => qoder_home_candidates()
            .into_iter()
            .map(|home| home.join("projects"))
            .collect(),
        "workbuddy" => vec![home.join(".workbuddy").join("projects")],
        "minimax" => vec![home.join(".minimax").join("v2").join("sessions")],
        "kimi" => kimi_code_home_candidates()
            .into_iter()
            .map(|home| home.join("sessions"))
            .collect(),
        _ => Vec::new(),
    }
}

fn default_config_home(id: &str) -> Option<PathBuf> {
    match id {
        "qoder" => qoder_home(),
        "minimax" => home().map(|home| home.join(".minimax")),
        "kimi" => kimi_code_home(),
        _ => None,
    }
}

// ── 对外解析 ─────────────────────────────────────────────────────────────────

/// 该 Agent 当前生效的转录根目录（用户覆盖优先，其次自动探测默认）。
pub fn transcript_roots(id: &str) -> Vec<PathBuf> {
    let override_value = override_for(id).transcript_roots;
    if let Some(roots) = override_value {
        if !roots.is_empty() {
            return roots.into_iter().map(PathBuf::from).collect();
        }
    }
    default_transcript_roots(id)
}

/// 该 Agent 的配置主目录（MCP 配置等），同样支持跨设备覆盖。
pub fn config_home(id: &str) -> Option<PathBuf> {
    if let Some(config_home) = override_for(id).config_home {
        if !config_home.trim().is_empty() {
            return Some(PathBuf::from(config_home));
        }
    }
    default_config_home(id)
}

/// 文件型 MCP 配置的位置。返回 None 表示该 Agent 通过 CLI 注册。
pub fn mcp_config_path(id: &str) -> Option<PathBuf> {
    match id {
        "qoder" | "minimax" | "kimi" => config_home(id).map(|home| home.join("mcp.json")),
        "workbuddy" => home().map(|home| home.join(".workbuddy").join(".mcp.json")),
        _ => None,
    }
}

/// 从转录文件路径推导稳定会话 id。
pub fn session_id_for_path(source: &str, path: &Path) -> Option<String> {
    let stem = path.file_stem()?.to_str()?.trim();
    match source {
        "codex" => {
            // Codex 文件名 rollout-<timestamp>-<uuid>.jsonl，末尾 UUID 即会话 id。
            if stem.len() >= 36 {
                let suffix = &stem[stem.len() - 36..];
                if uuid::Uuid::parse_str(suffix).is_ok() {
                    return Some(suffix.to_string());
                }
            }
            (!stem.is_empty()).then_some(stem.to_string())
        }
        "minimax" => {
            // <root>/YYYY/MM/DD/<时间>-session_<id>/messages.jsonl
            if stem == "messages" {
                return path
                    .parent()
                    .and_then(|dir| dir.file_name())
                    .and_then(|name| name.to_str())
                    .map(str::to_string);
            }
            (!stem.is_empty()).then_some(stem.to_string())
        }
        "kimi" => {
            // <home>/sessions/<workspace>/<conv-id>/agents/main/wire.jsonl
            if stem == "wire" {
                return path
                    .parent() // main
                    .and_then(Path::parent) // agents
                    .and_then(Path::parent) // <conv-id>
                    .and_then(|dir| dir.file_name())
                    .and_then(|name| name.to_str())
                    .map(str::to_string);
            }
            (!stem.is_empty()).then_some(stem.to_string())
        }
        _ => (!stem.is_empty()).then_some(stem.to_string()),
    }
}

// ── Tauri 命令：记忆中心的数据目录设置 ────────────────────────────────────────

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct AgentSourcePathInfo {
    pub path: String,
    pub exists: bool,
    pub is_override: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "snake_case")]
pub struct AgentSourceInfo {
    pub id: String,
    pub label: String,
    pub supports_hooks: bool,
    pub transcript_roots: Vec<AgentSourcePathInfo>,
    pub default_transcript_roots: Vec<String>,
    pub mcp_config_path: Option<String>,
}

pub fn agent_source_info(id: &str) -> Option<AgentSourceInfo> {
    if !is_supported_source(id) {
        return None;
    }
    let has_override = override_for(id).transcript_roots.is_some();
    Some(AgentSourceInfo {
        id: id.into(),
        label: agent_label(id).into(),
        supports_hooks: supports_hooks(id),
        transcript_roots: transcript_roots(id)
            .into_iter()
            .map(|path| AgentSourcePathInfo {
                exists: path.is_dir(),
                is_override: has_override,
                path: path.to_string_lossy().to_string(),
            })
            .collect(),
        default_transcript_roots: default_transcript_roots(id)
            .into_iter()
            .map(|path| path.to_string_lossy().to_string())
            .collect(),
        mcp_config_path: mcp_config_path(id).map(|path| path.to_string_lossy().to_string()),
    })
}

#[tauri::command]
pub fn agent_sources_list() -> Vec<AgentSourceInfo> {
    AGENT_SOURCE_IDS
        .iter()
        .filter_map(|id| agent_source_info(id))
        .collect()
}

/// 设置或清除某个 Agent 的跨设备路径覆盖。两个字段都传 None/空即恢复自动探测。
#[tauri::command]
pub fn agent_source_set_override(
    id: String,
    transcript_roots: Option<Vec<String>>,
    config_home: Option<String>,
) -> Result<AgentSourceInfo, String> {
    if !is_supported_source(&id) {
        return Err(format!("暂不支持的 Agent 来源：{id}"));
    }
    let mut overrides = read_overrides();
    let cleaned_roots = transcript_roots
        .map(|roots| {
            roots
                .into_iter()
                .map(|root| root.trim().to_string())
                .filter(|root| !root.is_empty())
                .collect::<Vec<_>>()
        })
        .filter(|roots| !roots.is_empty());
    let cleaned_home = config_home
        .map(|home| home.trim().to_string())
        .filter(|home| !home.is_empty());
    if cleaned_roots.is_none() && cleaned_home.is_none() {
        overrides.remove(&id);
    } else {
        overrides.insert(
            id.clone(),
            AgentSourceOverride {
                transcript_roots: cleaned_roots,
                config_home: cleaned_home,
            },
        );
    }
    write_overrides(&overrides)?;
    agent_source_info(&id).ok_or_else(|| format!("暂不支持的 Agent 来源：{id}"))
}

#[cfg(test)]
mod tests {
    use super::session_id_for_path;
    use std::path::Path;

    #[test]
    fn minimax_session_id_comes_from_the_session_directory() {
        let path = Path::new(
            "C:/Users/test/.minimax/v2/sessions/2026/08/18/08-51-37-739-session_bXZzXzE5/messages.jsonl",
        );
        assert_eq!(
            session_id_for_path("minimax", path).as_deref(),
            Some("08-51-37-739-session_bXZzXzE5")
        );
    }

    #[test]
    fn kimi_session_id_comes_from_the_conversation_directory() {
        let path = Path::new(
            "F:/KimiData/daimon-share/daimon/runtime/kimi-code/home/sessions/wd_demo/conv-abc123/agents/main/wire.jsonl",
        );
        assert_eq!(
            session_id_for_path("kimi", path).as_deref(),
            Some("conv-abc123")
        );
    }

    #[test]
    fn codex_session_id_keeps_the_trailing_uuid() {
        let path = Path::new(
            "C:/Users/test/.codex/sessions/2026/08/18/rollout-2026-08-18T08-00-00-019f8e90-36f5-71a2-bddf-f0fe9ee23b57.jsonl",
        );
        assert_eq!(
            session_id_for_path("codex", path).as_deref(),
            Some("019f8e90-36f5-71a2-bddf-f0fe9ee23b57")
        );
    }
}
