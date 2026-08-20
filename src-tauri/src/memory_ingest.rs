//! 自动沉淀管道：接收 Agent hook 回调，自动提取记忆。
//!
//! 数据流：Agent（Claude Code / Codex 等）hook 回调 → 本模块按会话聚合
//! 对话与工具轨迹 → 会话结束（Stop）时批量调用记忆引擎：
//!   - 对话片段 → 本地 L1 记忆账本（LLM 抽取后直接、可恢复地写入）
//!   - 原始 Hook 事件 → 本地遥测账本（供 Token 统计与审计）
//!
//! 全部异步执行，不阻塞 hook 回调；带节流（按会话聚合，避免逐条调用 LLM）。

use serde_json::{json, Value};
use sha2::Digest;
use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::{BufRead, BufReader};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use crate::llm;
use crate::memory_backend::MemoryBackend;
use crate::thinking::strip_thinking_blocks;

/// All harnesses serve the same local user.  Memories must stay portable when
/// a conversation continues from Codex to Claude or another connected agent.
const GLOBAL_MEMORY_OWNER: &str = "agent-manager";
const ORGANIZE_BATCH_LIMIT: u32 = 10;
const ORGANIZE_ONE_CONVERSATION_TIMEOUT: Duration = Duration::from_secs(150);
const IMPORT_MAX_FILES: usize = 100;
const IMPORT_MAX_FILE_CHARS: usize = 48_000;
const IMPORT_MAX_TOTAL_CHARS: usize = 240_000;
const IMPORT_CHUNK_CHARS: usize = 18_000;
const IMPORT_MAX_CHUNKS: usize = 14;
const NATIVE_SCAN_MAX_FILES_PER_SOURCE: usize = 100;
const NATIVE_SCAN_MAX_ENTRIES: usize = 20_000;
const NATIVE_SESSION_MAX_MESSAGES: usize = 200;
const NATIVE_SESSION_MAX_CHARS: usize = 120_000;
// This caps only the final profile that is persisted. It must comfortably fit
// a complete compact Markdown profile; reasoning payloads are removed before
// this check and are not part of the persisted document.
//
// L2/L3 预算统一为 10000 cl100k token（与注入侧
// memory_mcp::MEMORY_LAYER_INJECTION_TOKENS 同源）。字符上限按 3 字符/token
// 放缩，保证 10000 token 的中英混排内容不会被字符闸误杀。
const MEMORY_LAYER_TOKEN_BUDGET: usize = 10_000;
const MEMORY_LAYER_DOC_MAX_CHARS: usize = MEMORY_LAYER_TOKEN_BUDGET * 3;
const L3_PROFILE_MAX_CHARS: usize = MEMORY_LAYER_DOC_MAX_CHARS;
const L3_PROFILE_TARGET_CHARS: usize = MEMORY_LAYER_DOC_MAX_CHARS;
const L3_L1_EVIDENCE_LIMIT: usize = 80;

/// 全局单例（供 agent_http 的 route 无 State 上下文使用）。
static INGEST: OnceLock<IngestStore> = OnceLock::new();

pub fn init_ingest(store: IngestStore) {
    let _ = INGEST.set(store);
}

pub fn ingest_store() -> Option<&'static IngestStore> {
    INGEST.get()
}

/// 单个会话的聚合缓冲
#[derive(Clone, Debug)]
struct SessionBuffer {
    agent_id: String,
    /// Hook 来源标识（如 claude / codex），用于在整理日志中显示所属 Agent。
    source: String,
    messages: Vec<(String, String)>, // (role, content)
    last_active: Instant,
}

#[derive(Default)]
struct IngestInner {
    sessions: HashMap<String, SessionBuffer>,
    /// 最近提取记录（前端展示）
    recent: Vec<IngestLog>,
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub struct IngestLog {
    pub at: String,
    pub agent_id: String,
    pub kind: String,  // memory | skill
    pub state: String, // working | stored | retrying
    pub detail: String,
}

#[derive(Clone)]
pub struct IngestStore {
    inner: Arc<Mutex<IngestInner>>,
    /// 会话静默超时（秒），超时自动冲刷
    idle_timeout_secs: u64,
    enabled: Arc<Mutex<bool>>,
}

impl IngestStore {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Mutex::new(IngestInner::default())),
            idle_timeout_secs: 180,
            enabled: Arc::new(Mutex::new(true)),
        }
    }

    pub fn set_enabled(&self, on: bool) {
        *self.enabled.lock().unwrap() = on;
    }

    pub fn is_enabled(&self) -> bool {
        *self.enabled.lock().unwrap()
    }

    /// 处理一条 hook 回调（不阻塞；耗时的记忆提取在后台执行）。
    pub fn handle_hook(
        &self,
        backend: Arc<MemoryBackend>,
        body: &str,
        source: &str,
    ) -> Result<Value, String> {
        let parsed: Value =
            serde_json::from_str(body).map_err(|e| format!("invalid hook JSON: {e}"))?;
        if let Some(store) = crate::telemetry_store::shared_store() {
            store.record_hook(source, &parsed)?;
        }
        let event = parsed
            .get("hook_event_name")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let session_id = parsed
            .get("session_id")
            .and_then(|v| v.as_str())
            .unwrap_or("")
            .to_string();
        let agent_id = GLOBAL_MEMORY_OWNER.to_string();

        if session_id.is_empty() {
            return Ok(
                json!({"status": "ok", "note": "no session_id, retained in telemetry only"}),
            );
        }

        match event {
            // 用户提交提示词 → 记录对话
            "UserPromptSubmit" => {
                let prompt = parsed
                    .get("prompt")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                self.push_message(&session_id, &agent_id, source, "user", &prompt);
            }
            // 工具轨迹属于同一段对话上下文。保留一个有大小上限的摘要，
            // 但绝不在此处提取，统一等待会话 Stop。
            "PostToolUse" => {
                self.touch_session(&session_id, &agent_id, source);
                let tool_name = parsed
                    .get("tool_name")
                    .and_then(Value::as_str)
                    .unwrap_or("tool");
                let input = value_excerpt(parsed.get("tool_input"), 2_000);
                let output = value_excerpt(
                    parsed
                        .get("tool_response")
                        .or_else(|| parsed.get("tool_output")),
                    4_000,
                );
                let detail = format!("工具 {tool_name}\n输入：{input}\n结果：{output}");
                self.push_message(&session_id, &agent_id, source, "tool", &detail);
            }
            // 会话结束 → 冲刷：批量提取记忆。
            "Stop" => {
                // Agent hooks provide operational events, while their local
                // transcripts hold the actual user/assistant dialogue.  Read
                // the one completed turn so tool traffic never becomes memory.
                let transcript_replaced = read_hook_transcript(source, &parsed);
                let mut conversation_state = "unavailable";
                let mut conversation_messages = Vec::new();
                if let Some(messages) = transcript_replaced {
                    conversation_state = "full";
                    conversation_messages = messages.clone();
                    self.replace_session_messages(&session_id, &agent_id, source, messages);
                } else if let Some(reply) = parsed
                    .get("last_assistant_message")
                    .or_else(|| parsed.get("assistant_message"))
                    .or_else(|| parsed.get("final_response"))
                    .and_then(Value::as_str)
                {
                    // Adapters without a readable transcript can still supply
                    // a final assistant response as a useful fallback.
                    self.push_message(&session_id, &agent_id, source, "assistant", reply);
                    conversation_state = "partial";
                    conversation_messages.push(("assistant".to_string(), reply.to_string()));
                }
                if let Some(store) = crate::telemetry_store::shared_store() {
                    if let Err(error) = store.record_conversation(
                        source,
                        &parsed,
                        conversation_state,
                        &conversation_messages,
                    ) {
                        eprintln!("[memory-ingest] failed to annotate hook receipt: {error}");
                    }
                }
                self.flush_session(backend, &session_id);
            }
            _ => {}
        }

        Ok(json!({"status": "ok"}))
    }

    fn push_message(&self, session_id: &str, agent_id: &str, source: &str, role: &str, content: &str) {
        let content = strip_memory_thinking(role, content);
        if content.is_empty() {
            return;
        }
        {
            let mut inner = self.inner.lock().unwrap();
            let buf = inner
                .sessions
                .entry(session_id.to_string())
                .or_insert_with(|| SessionBuffer {
                    agent_id: agent_id.to_string(),
                    source: source.to_string(),
                    messages: Vec::new(),
                    last_active: Instant::now(),
                });
            buf.messages
                .push((role.to_string(), truncate_text(&content, 12_000)));
            buf.last_active = Instant::now();
        }
    }

    fn touch_session(&self, session_id: &str, agent_id: &str, source: &str) {
        let mut inner = self.inner.lock().unwrap();
        let buf = inner
            .sessions
            .entry(session_id.to_string())
            .or_insert_with(|| SessionBuffer {
                agent_id: agent_id.to_string(),
                source: source.to_string(),
                messages: Vec::new(),
                last_active: Instant::now(),
            });
        buf.last_active = Instant::now();
    }

    fn replace_session_messages(
        &self,
        session_id: &str,
        agent_id: &str,
        source: &str,
        messages: Vec<(String, String)>,
    ) {
        let messages = messages
            .into_iter()
            .map(|(role, content)| {
                let content = strip_memory_thinking(&role, &content);
                (role, content)
            })
            .filter(|(_, content)| !content.is_empty())
            .collect();
        let mut inner = self.inner.lock().unwrap();
        inner.sessions.insert(
            session_id.to_string(),
            SessionBuffer {
                agent_id: agent_id.to_string(),
                source: source.to_string(),
                messages,
                last_active: Instant::now(),
            },
        );
    }

    /// 冲刷一个会话：取出缓冲 → 后台提取记忆 + 沉淀 skill。
    pub fn flush_session(&self, backend: Arc<MemoryBackend>, session_id: &str) {
        if !self.is_enabled() {
            return;
        }
        let snapshot = {
            let mut inner = self.inner.lock().unwrap();
            inner.sessions.remove(session_id)
        };
        let Some(buf) = snapshot else { return };
        if buf.messages.is_empty() {
            return;
        }

        let store = self.clone();
        let session_id = session_id.to_string();
        tauri::async_runtime::spawn(async move {
            store.ingest_memory(&backend, &session_id, &buf).await;
        });
    }

    /// One completed hook session is one extraction unit.  The configured LLM
    /// sees the full buffered transcript only after Stop (or the idle timeout).
    async fn ingest_memory(&self, _backend: &MemoryBackend, session_id: &str, buf: &SessionBuffer) {
        if buf.messages.is_empty() {
            return;
        }
        let provider = match llm::memory_extraction_provider() {
            Ok(provider) => provider,
            Err(error) => {
                self.restore_session_for(session_id, buf.clone());
                self.log(IngestLog {
                    at: now_str(),
                    agent_id: buf.agent_id.clone(),
                    kind: "memory".into(),
                    state: "retrying".into(),
                    detail: format!("{error}；已保留完整会话等待配置完成"),
                });
                return;
            }
        };
        let transcript = buf
            .messages
            .iter()
            .map(|(role, content)| format!("[{role}]\n{content}"))
            .collect::<Vec<_>>()
            .join("\n\n");
        let extraction_messages = l1_extraction_messages(&transcript);
        let extracted = match llm::complete_text(&provider, &extraction_messages).await {
            Ok(text) => match parse_typed_l1_candidates(&text) {
                Ok(candidates) => candidates,
                Err(error) => {
                    if let Some(telemetry) = crate::telemetry_store::shared_store() {
                        let _ = telemetry.set_l1_failure_for_session(session_id, &error);
                    }
                    self.restore_session_for(session_id, buf.clone());
                    self.log(IngestLog {
                        at: now_str(),
                        agent_id: buf.agent_id.clone(),
                        kind: "memory".into(),
                        state: "failed".into(),
                        detail: format!(
                            "L1 标签校验失败：{error}；完整会话已保留，可修正模型输出后重跑"
                        ),
                    });
                    return;
                }
            },
            Err(error) => {
                self.restore_session_for(session_id, buf.clone());
                self.log(IngestLog {
                    at: now_str(),
                    agent_id: buf.agent_id.clone(),
                    kind: "memory".into(),
                    state: "retrying".into(),
                    detail: format!("{}；已保留完整会话等待重试", error),
                });
                return;
            }
        };
        match crate::telemetry_store::shared_store()
            .ok_or_else(|| "本地记忆账本尚未初始化".to_string())
            .and_then(|telemetry| {
                telemetry.store_typed_l1_memories_for_session(session_id, &extracted)
            }) {
            Ok(n) => {
                crate::memory_backend::queue_semantic_l1_index(
                    extracted.iter().map(|item| item.content.clone()).collect(),
                );
                self.log(IngestLog {
                    at: now_str(),
                    agent_id: buf.agent_id.clone(),
                    kind: "memory".into(),
                    state: "stored".into(),
                    detail: if buf.source.is_empty() {
                        format!(
                            "{} 已分析完整会话；已写入本地记忆库 {} 条（无需等待记忆服务二次提取）",
                            provider.name, n
                        )
                    } else {
                        format!(
                            "{} 已分析 {} 的完整会话；已写入本地记忆库 {} 条（无需等待记忆服务二次提取）",
                            provider.name,
                            crate::agent_sources::agent_label(&buf.source),
                            n
                        )
                    },
                });
            }
            Err(e) => {
                eprintln!("[memory-ingest] local L1 write failed: {e}");
                if let Some(telemetry) = crate::telemetry_store::shared_store() {
                    let _ = telemetry.set_l1_failure_for_session(session_id, &e);
                }
                self.restore_session_for(session_id, buf.clone());
                self.log(IngestLog {
                    at: now_str(),
                    agent_id: buf.agent_id.clone(),
                    kind: "memory".into(),
                    state: "failed".into(),
                    detail: format!("L1 写入校验失败：{e}；完整会话已保留，可重跑"),
                });
            }
        }
    }

    fn restore_session_for(&self, session_id: &str, mut previous: SessionBuffer) {
        previous.last_active = Instant::now();
        let mut inner = self.inner.lock().unwrap();
        if let Some(current) = inner.sessions.remove(session_id) {
            previous.messages.extend(current.messages);
        }
        inner.sessions.insert(session_id.to_string(), previous);
    }

    fn log(&self, entry: IngestLog) {
        let mut inner = self.inner.lock().unwrap();
        inner.recent.insert(0, entry);
        inner.recent.truncate(30);
    }

    /// 前端展示用：最近提取记录。
    pub fn recent_logs(&self) -> Vec<IngestLog> {
        self.inner.lock().unwrap().recent.clone()
    }

    /// 当前缓冲中的会话数。
    pub fn buffered_sessions(&self) -> usize {
        self.inner.lock().unwrap().sessions.len()
    }

    pub fn flush_pending(&self, backend: Arc<MemoryBackend>) -> usize {
        if !self.is_enabled() {
            return 0;
        }
        let session_ids = self
            .inner
            .lock()
            .unwrap()
            .sessions
            .keys()
            .cloned()
            .collect::<Vec<_>>();
        let count = session_ids.len();
        for session_id in session_ids {
            self.flush_session(Arc::clone(&backend), &session_id);
        }
        count
    }
}

fn l1_extraction_messages(transcript: &str) -> Vec<Value> {
    let language = output_language_directive(transcript);
    vec![
        json!({"role": "system", "content": format!("你是本地 Agent Manager 的高质量会话记忆整理器。每个完整会话最多输出 3 条、通常 1–2 条。第一条必须是 `summary`，且 durability 必须为 `session`；其余只保留可跨后续任务复用的事实、已确认决定、偏好候选或约束。不要逐步复述操作、工具调用、短暂报错、无结论讨论、重复表述或大段代码。每条须可独立理解，明确主体和结论；不得臆测用户偏好。\n\n必须为每条写准确 durability：`session` 仅当前会话摘要、进度或临时排查；`short_term` 是当前项目/近期任务可能持续数天到数周的决定；`long_term` 仅限用户明确表达、跨项目且 90 天后仍适用的偏好或硬约束。助手自行做出的代码修改、模型/端点/Token 配置、UI 实现、构建/测试记录一律不得标为 `long_term`。不得猜测、遗漏或自动降级：若无法明确分类，请输出 `durability: \"undetermined\"`；该整批结果会被拒绝并标记失败。{language}\n\n严格输出 JSON：{{\"memories\":[{{\"content\":\"...\",\"type\":\"summary|fact|decision|constraint|preference_candidate|open_item\",\"durability\":\"session|short_term|long_term|undetermined\"}}]}}。没有可长期复用的信息时只输出一个 summary。")}),
        json!({"role": "user", "content": format!("以下是一段已经结束的会话，请整体理解后整理为 L1：\n\n{transcript}")}),
    ]
}

/// Keep generated memory in the conversation's primary language.  This is
/// intentionally determined from the captured content instead of the app UI
/// locale: a Chinese conversation still needs Chinese memory in an English UI.
fn output_language_directive(source: &str) -> &'static str {
    let mut han = 0usize;
    let mut latin = 0usize;
    let mut japanese = 0usize;
    let mut korean = 0usize;
    for ch in source.chars() {
        match ch {
            '\u{4e00}'..='\u{9fff}' => han += 1,
            '\u{3040}'..='\u{30ff}' => japanese += 1,
            '\u{ac00}'..='\u{d7af}' => korean += 1,
            _ if ch.is_ascii_alphabetic() => latin += 1,
            _ => {}
        }
    }
    if japanese > 0 {
        "输出必须使用日语；不得翻译成其他语言。"
    } else if korean > 0 {
        "输出必须使用韩语；不得翻译成其他语言。"
    // Technical conversations often contain many ASCII identifiers, file
    // names, and product names inside otherwise Chinese prose. Require Latin
    // text to substantially outweigh Han text before treating it as English.
    } else if han.saturating_mul(2) > latin {
        "输出必须使用中文；不得翻译成英文或其他语言。"
    } else if latin > 0 {
        "Output must be in English; do not translate it into another language."
    } else {
        "输出必须使用原始内容的主要语言；不得自行翻译。"
    }
}

/// Only assistant output is filtered for thinking: a user may deliberately
/// discuss the literal marker syntax, while assistant reasoning must never
/// enter L0.  Injected harness context (system-reminder/meta 等) 对两个角色
/// 都剥离——它是宿主对模型说的话，不是对话内容。
fn strip_memory_thinking(role: &str, content: &str) -> String {
    let cleaned = crate::telemetry_store::strip_injected_context(content);
    if role == "assistant" {
        strip_thinking_blocks(&cleaned)
    } else {
        cleaned.trim().to_string()
    }
}

async fn extract_l1_conversation(
    transcript: &str,
) -> Result<Vec<crate::telemetry_store::L1MemoryCandidate>, String> {
    let provider = llm::memory_extraction_provider()?;
    let text = llm::complete_text(&provider, &l1_extraction_messages(transcript)).await?;
    parse_typed_l1_candidates(&text)
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub struct MemoryImportResult {
    pub folder: String,
    pub scanned_files: u32,
    pub recognized_files: u32,
    pub skipped_files: u32,
    pub imported_memories: u32,
    pub message: String,
}

fn is_importable_memory_file(path: &Path) -> bool {
    matches!(
        path.extension()
            .and_then(|extension| extension.to_str())
            .map(|extension| extension.to_ascii_lowercase())
            .as_deref(),
        Some("md" | "txt" | "json" | "jsonl" | "yaml" | "yml")
    )
}

fn is_ignored_import_directory(path: &Path) -> bool {
    matches!(
        path.file_name().and_then(|name| name.to_str()),
        Some(
            ".git"
                | "node_modules"
                | "target"
                | "dist"
                | "build"
                | ".venv"
                | "venv"
                | "__pycache__"
                | "cache"
                | "blobs"
                | "binaries"
                | "vendor"
        )
    )
}

fn collect_import_files(
    directory: &Path,
    files: &mut Vec<PathBuf>,
    scanned: &mut u32,
    skipped: &mut u32,
    depth: u8,
) -> Result<(), String> {
    if depth > 12 || files.len() >= IMPORT_MAX_FILES {
        return Ok(());
    }
    let entries = fs::read_dir(directory).map_err(|error| format!("无法读取文件夹：{error}"))?;
    for entry in entries {
        if files.len() >= IMPORT_MAX_FILES {
            break;
        }
        let entry = entry.map_err(|error| format!("无法枚举文件夹内容：{error}"))?;
        let file_type = entry
            .file_type()
            .map_err(|error| format!("无法读取文件类型：{error}"))?;
        if file_type.is_symlink() {
            *skipped += 1;
            continue;
        }
        let path = entry.path();
        if file_type.is_dir() {
            if is_ignored_import_directory(&path) {
                *skipped += 1;
            } else {
                collect_import_files(&path, files, scanned, skipped, depth + 1)?;
            }
        } else if file_type.is_file() {
            *scanned += 1;
            if is_importable_memory_file(&path) {
                files.push(path);
            } else {
                *skipped += 1;
            }
        }
    }
    Ok(())
}

fn take_import_chunks(text: &str) -> Vec<String> {
    let chars = text.chars().collect::<Vec<_>>();
    chars
        .chunks(IMPORT_CHUNK_CHARS)
        .take(IMPORT_MAX_CHUNKS)
        .map(|chunk| chunk.iter().collect::<String>())
        .collect()
}

/// Manually import a folder of exported conversations or memory documents.
/// Only safe text formats are read and the selected source is never treated as
/// an executable configuration file.
#[tauri::command]
pub async fn memory_import_folder(
    telemetry: tauri::State<'_, crate::telemetry_store::TelemetryStore>,
    folder: String,
) -> Result<MemoryImportResult, String> {
    let root = PathBuf::from(&folder)
        .canonicalize()
        .map_err(|error| format!("无法访问选择的文件夹：{error}"))?;
    if !root.is_dir() {
        return Err("请选择一个文件夹，而不是单个文件".into());
    }

    let mut paths = Vec::new();
    let mut scanned = 0;
    let mut skipped = 0;
    collect_import_files(&root, &mut paths, &mut scanned, &mut skipped, 0)?;
    paths.sort();
    if paths.is_empty() {
        return Ok(MemoryImportResult {
            folder: root.display().to_string(),
            scanned_files: scanned,
            recognized_files: 0,
            skipped_files: skipped,
            imported_memories: 0,
            message: "未在该文件夹中找到可识别的记忆或会话文本文件".into(),
        });
    }

    let mut source = String::new();
    let mut recognized = 0u32;
    for path in paths {
        let metadata = match fs::metadata(&path) {
            Ok(metadata) => metadata,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        if metadata.len() > 1_500_000 {
            skipped += 1;
            continue;
        }
        let content = match fs::read_to_string(&path) {
            Ok(content) => content,
            Err(_) => {
                skipped += 1;
                continue;
            }
        };
        let content = truncate_text(&content, IMPORT_MAX_FILE_CHARS);
        if content.trim().is_empty() {
            skipped += 1;
            continue;
        }
        let relative = path.strip_prefix(&root).unwrap_or(&path).display();
        let addition = format!("\n\n--- 文件：{relative} ---\n{content}");
        if source.chars().count() + addition.chars().count() > IMPORT_MAX_TOTAL_CHARS {
            break;
        }
        source.push_str(&addition);
        recognized += 1;
    }
    if source.trim().is_empty() {
        return Ok(MemoryImportResult {
            folder: root.display().to_string(),
            scanned_files: scanned,
            recognized_files: 0,
            skipped_files: skipped,
            imported_memories: 0,
            message: "识别到的文件没有可提取的文本内容".into(),
        });
    }

    let mut unique = HashSet::new();
    let mut candidates = Vec::new();
    for chunk in take_import_chunks(&source) {
        for candidate in extract_l1_conversation(&chunk).await? {
            let normalized = candidate
                .content
                .split_whitespace()
                .collect::<String>()
                .to_lowercase();
            if candidate.content.trim().len() >= 4 && unique.insert(normalized) {
                candidates.push(candidate);
            }
        }
    }
    if candidates.is_empty() {
        return Err("记忆模型未能从所选文件中提取有效记忆".into());
    }

    let source_key_digest = sha2::Sha256::digest(root.to_string_lossy().as_bytes());
    let import_key = format!(
        "manual-import:{}",
        crate::telemetry_store::hex(&source_key_digest)[..24].to_string()
    );
    let imported =
        telemetry.store_imported_memories(&import_key, &root.display().to_string(), &candidates)?;
    Ok(MemoryImportResult {
        folder: root.display().to_string(),
        scanned_files: scanned,
        recognized_files: recognized,
        skipped_files: skipped,
        imported_memories: imported as u32,
        message: format!("已从 {recognized} 个文本文件提取并写入 {imported} 条共享记忆"),
    })
}

fn truncate_text(text: &str, max_chars: usize) -> String {
    if text.chars().count() <= max_chars {
        return text.to_string();
    }
    let truncated = text.chars().take(max_chars).collect::<String>();
    format!("{truncated}\n[内容已截断]")
}

fn value_excerpt(value: Option<&Value>, max_chars: usize) -> String {
    let text = value
        .and_then(Value::as_str)
        .map(str::to_string)
        .or_else(|| value.and_then(|item| serde_json::to_string(item).ok()))
        .unwrap_or_else(|| "（未提供）".into());
    truncate_text(&text, max_chars)
}

/// Read a transcript turn and turn parser failures into an observable log
/// while preserving the hook payload's fallback response.
fn read_transcript_turn(
    path: Option<&str>,
    turn_id: Option<&str>,
    reader: fn(&Path, &str) -> Result<Vec<(String, String)>, String>,
    agent_name: &str,
) -> Option<Vec<(String, String)>> {
    let (Some(path), Some(turn_id)) = (path, turn_id) else {
        return None;
    };
    match reader(Path::new(path), turn_id) {
        Ok(messages) if !messages.is_empty() => Some(messages),
        Ok(_) => {
            eprintln!("[memory-ingest] no {agent_name} transcript messages for turn {turn_id}");
            None
        }
        Err(error) => {
            eprintln!("[memory-ingest] {agent_name} transcript read failed: {error}");
            None
        }
    }
}

fn read_hook_transcript(source: &str, payload: &Value) -> Option<Vec<(String, String)>> {
    match source {
        "claude" => read_transcript_turn(
            payload.get("transcript_path").and_then(Value::as_str),
            payload.get("prompt_id").and_then(Value::as_str),
            read_claude_turn,
            "Claude",
        ),
        "codex" => read_transcript_turn(
            payload.get("transcript_path").and_then(Value::as_str),
            payload.get("turn_id").and_then(Value::as_str),
            read_codex_turn,
            "Codex",
        ),
        _ => None,
    }
}

/// Populate newly introduced receipt previews from retained Stop records.
/// This never calls the LLM or memory backend; it only reads the original
/// local transcript and preserves user/assistant messages in the local ledger.
#[tauri::command]
pub fn telemetry_backfill_conversations() -> Result<u32, String> {
    let Some(store) = crate::telemetry_store::shared_store() else {
        return Ok(0);
    };
    let mut updated = 0;
    for (source, payload) in store.unannotated_stop_payloads(50)? {
        if let Some(messages) = read_hook_transcript(&source, &payload) {
            store.record_conversation(&source, &payload, "full", &messages)?;
            updated += 1;
        }
    }
    updated += scan_native_transcripts(&store, false)?;
    Ok(updated)
}

/// Import native transcripts from every locally supported Agent into the same
/// durable conversation ledger used by Hook traffic. This only stages L1
/// candidates; model extraction remains governed by the existing explicit
/// “organize conversations” action and its configured provider.
/// 转录读取器清洗规则的版本。每次修改结构化/文本清洗规则时递增，
/// 存量扫描状态会被清空并触发一次全量重扫，已污染的 L0 行随内容寻址
/// 重录自动修复（同 event_key 覆盖正文、重新进入待提取队列）。
/// v2：结构化块只保留 text 类；v3：注入包裹剥离下沉到读取器，
///     保证内容指纹反映清洗后文本（否则指纹不变的旧行永不重录）。
const TRANSCRIPT_SCANNER_VERSION: i64 = 3;

fn scan_native_transcripts(
    store: &crate::telemetry_store::TelemetryStore,
    retry_failed: bool,
) -> Result<u32, String> {
    if store
        .app_setting_get::<i64>("transcript_scanner_version")
        .unwrap_or(0)
        != TRANSCRIPT_SCANNER_VERSION
    {
        store.reset_transcript_scan_states()?;
        store.app_setting_set("transcript_scanner_version", &TRANSCRIPT_SCANNER_VERSION)?;
    }
    let mut imported = 0u32;
    // 转录根目录来自统一的 Agent 数据源注册表，支持按设备覆盖。
    for source in crate::agent_sources::AGENT_SOURCE_IDS {
        for root in crate::agent_sources::transcript_roots(source) {
        let mut pending = vec![root];
        let mut visited = 0usize;
        let mut candidates = Vec::new();
        while let Some(directory) = pending.pop() {
            if visited >= NATIVE_SCAN_MAX_ENTRIES {
                break;
            }
            let Ok(entries) = fs::read_dir(directory) else {
                continue;
            };
            for entry in entries.flatten() {
                visited += 1;
                let path = entry.path();
                if path.is_dir() {
                    pending.push(path);
                    continue;
                }
                if path.extension().and_then(|extension| extension.to_str()) != Some("jsonl") {
                    continue;
                }
                let modified = entry
                    .metadata()
                    .ok()
                    .and_then(|metadata| metadata.modified().ok())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                candidates.push((modified, path));
            }
        }
        candidates.sort_by(|left, right| right.0.cmp(&left.0));
        for (_, path) in candidates
            .into_iter()
            .take(NATIVE_SCAN_MAX_FILES_PER_SOURCE)
        {
            if !store.should_scan_transcript(source, &path, retry_failed) {
                continue;
            }
            let Some(session_id) = native_session_id(source, &path) else {
                continue;
            };
            let messages = match read_native_session(source, &path) {
                Ok(messages) => messages,
                Err(error) => {
                    eprintln!(
                        "[memory-ingest] {source} transcript read failed ({}): {error}",
                        path.display()
                    );
                    store.record_transcript_scan(source, &path, Some(&session_id), Err(&error));
                    continue;
                }
            };
            if messages.is_empty() {
                store.record_transcript_scan(source, &path, Some(&session_id), Ok(()));
                continue;
            }
            let occurred_at = fs::metadata(&path)
                .ok()
                .and_then(|metadata| metadata.modified().ok())
                .map(chrono::DateTime::<chrono::Utc>::from)
                .map(|time| time.to_rfc3339())
                .unwrap_or_else(now_str);
            if store.record_native_conversation(
                source,
                &session_id,
                &path,
                &occurred_at,
                &messages,
            )? {
                imported += 1;
            }
            store.record_transcript_scan(source, &path, Some(&session_id), Ok(()));
        }
        }
    }
    Ok(imported)
}

fn native_session_id(source: &str, path: &Path) -> Option<String> {
    crate::agent_sources::session_id_for_path(source, path)
}

fn read_native_session(source: &str, path: &Path) -> Result<Vec<(String, String)>, String> {
    match source {
        "codex" => read_codex_session(path),
        "claude" => read_claude_session(path),
        "qoder" => read_qoder_session(path),
        "workbuddy" => read_workbuddy_session(path),
        "minimax" => read_minimax_session(path),
        "kimi" => read_kimi_session(path),
        _ => Ok(Vec::new()),
    }
}

/// MiniMax Code 会话位于 `v2/sessions/**/messages.jsonl`，每行
/// `{message_id, turn_id, message:{role, content:[...]}}`。只保留
/// user/assistant 的 text 片段；thinking 与工具载荷不进入记忆。
fn read_minimax_session(path: &Path) -> Result<Vec<(String, String)>, String> {
    let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
    if metadata.len() > 16 * 1024 * 1024 {
        return Ok(Vec::new());
    }
    let file = fs::File::open(path).map_err(|error| error.to_string())?;
    let mut seen = HashSet::new();
    let mut messages = Vec::new();
    let mut total_chars = 0usize;
    for line in BufReader::new(file).lines() {
        if messages.len() >= NATIVE_SESSION_MAX_MESSAGES || total_chars >= NATIVE_SESSION_MAX_CHARS
        {
            break;
        }
        let line = line.map_err(|error| error.to_string())?;
        let Ok(entry) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(message) = entry.get("message") else {
            continue;
        };
        let Some(role) = message
            .get("role")
            .and_then(Value::as_str)
            .filter(|role| matches!(*role, "user" | "assistant"))
        else {
            continue;
        };
        // 同一 message_id 可能分多行追加（流式片段），按 message_id 去重。
        let identity = entry
            .get("message_id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or_else(|| line.clone());
        if !seen.insert(identity) {
            continue;
        }
        let text = qoder_transcript_text(message.get("content"));
        if text.is_empty() {
            continue;
        }
        append_native_message(&mut messages, &mut total_chars, role, text);
    }
    Ok(messages)
}

/// Kimi（Kimi Code / Kimi Work）会话是 `<home>/sessions/**/wire.jsonl`。
/// 用户消息在 `context.append_message`，助手正文在 loop 事件的
/// `content.part`（`part.type == "text"`；`think` 不进入记忆）。
fn read_kimi_session(path: &Path) -> Result<Vec<(String, String)>, String> {
    let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
    if metadata.len() > 16 * 1024 * 1024 {
        return Ok(Vec::new());
    }
    let file = fs::File::open(path).map_err(|error| error.to_string())?;
    let mut messages = Vec::new();
    let mut total_chars = 0usize;
    for line in BufReader::new(file).lines() {
        if messages.len() >= NATIVE_SESSION_MAX_MESSAGES || total_chars >= NATIVE_SESSION_MAX_CHARS
        {
            break;
        }
        let line = line.map_err(|error| error.to_string())?;
        let Ok(entry) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        match entry.get("type").and_then(Value::as_str) {
            Some("context.append_message") => {
                let Some(message) = entry.get("message") else {
                    continue;
                };
                if message.get("role").and_then(Value::as_str) != Some("user") {
                    continue;
                }
                let text = qoder_transcript_text(message.get("content"));
                if text.is_empty() {
                    continue;
                }
                append_native_message(&mut messages, &mut total_chars, "user", text);
            }
            Some("context.append_loop_event") => {
                let event = entry.get("event").cloned().unwrap_or(Value::Null);
                if event.get("type").and_then(Value::as_str) != Some("content.part") {
                    continue;
                }
                let Some(part) = event.get("part") else {
                    continue;
                };
                if part.get("type").and_then(Value::as_str) != Some("text") {
                    continue;
                }
                let Some(text) = part.get("text").and_then(Value::as_str) else {
                    continue;
                };
                let text = text.trim().to_string();
                if text.is_empty() {
                    continue;
                }
                append_native_message(&mut messages, &mut total_chars, "assistant", text);
            }
            _ => {}
        }
    }
    Ok(messages)
}

/// Extract only user/assistant text from a Qoder project JSONL. Tool payloads
/// are intentionally excluded from memories and token estimates. The limits
/// keep historical imports bounded before an LLM ever sees the transcript.
fn read_qoder_session(path: &Path) -> Result<Vec<(String, String)>, String> {
    let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
    if metadata.len() > 16 * 1024 * 1024 {
        return Ok(Vec::new());
    }
    let file = fs::File::open(path).map_err(|error| error.to_string())?;
    let mut seen = HashSet::new();
    let mut messages = Vec::new();
    let mut total_chars = 0usize;
    for line in BufReader::new(file).lines() {
        if messages.len() >= NATIVE_SESSION_MAX_MESSAGES || total_chars >= NATIVE_SESSION_MAX_CHARS
        {
            break;
        }
        let line = line.map_err(|error| error.to_string())?;
        let Ok(entry) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let Some(role) = entry
            .get("type")
            .and_then(Value::as_str)
            .filter(|role| matches!(*role, "user" | "assistant"))
        else {
            continue;
        };
        let identity = entry
            .get("uuid")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or(line);
        if !seen.insert(identity) {
            continue;
        }
        let text = qoder_transcript_text(
            entry
                .get("message")
                .and_then(|message| message.get("content")),
        );
        if text.is_empty() {
            continue;
        }
        append_native_message(&mut messages, &mut total_chars, role, text);
    }
    Ok(messages)
}

/// 从消息 content 中提取纯文本。结构化块只接受文本类（`text` /
/// `input_text` / `output_text`）；`tool_use`、`tool_result`、thinking、
/// 图片等载荷一律不进入 L0——它们是 Agent 的操作细节，不是对话内容。
/// 带 `tool_use_id` 的无 type 块同样视为工具结果剔除。
fn qoder_transcript_text(value: Option<&Value>) -> String {
    match value {
        Some(Value::String(text)) => text.trim().to_string(),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| qoder_transcript_text(Some(item)))
            .filter(|text| !text.is_empty())
            .collect::<Vec<_>>()
            .join("\n"),
        Some(Value::Object(object)) => {
            if object.contains_key("tool_use_id") {
                return String::new();
            }
            if let Some(kind) = object.get("type").and_then(Value::as_str) {
                if !matches!(kind, "text" | "input_text" | "output_text") {
                    return String::new();
                }
                return object
                    .get("text")
                    .or_else(|| object.get("content"))
                    .map(|text| qoder_transcript_text(Some(text)))
                    .unwrap_or_default();
            }
            object
                .get("text")
                .or_else(|| object.get("content"))
                .map(|child| qoder_transcript_text(Some(child)))
                .unwrap_or_default()
        }
        _ => String::new(),
    }
}

fn append_native_message(
    messages: &mut Vec<(String, String)>,
    total_chars: &mut usize,
    role: &str,
    text: String,
) {
    let text = strip_memory_thinking(role, &text);
    if text.is_empty()
        || messages.len() >= NATIVE_SESSION_MAX_MESSAGES
        || *total_chars >= NATIVE_SESSION_MAX_CHARS
    {
        return;
    }
    let remaining = NATIVE_SESSION_MAX_CHARS.saturating_sub(*total_chars);
    let text = truncate_text(&text, remaining.min(24_000));
    *total_chars += text.chars().count();
    messages.push((role.to_string(), text));
}

fn read_claude_session(path: &Path) -> Result<Vec<(String, String)>, String> {
    let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
    if metadata.len() > 16 * 1024 * 1024 {
        return Ok(Vec::new());
    }
    let file = fs::File::open(path).map_err(|error| error.to_string())?;
    let mut seen = HashSet::new();
    let mut messages = Vec::new();
    let mut total_chars = 0usize;
    for line in BufReader::new(file).lines() {
        if messages.len() >= NATIVE_SESSION_MAX_MESSAGES || total_chars >= NATIVE_SESSION_MAX_CHARS
        {
            break;
        }
        let line = line.map_err(|error| error.to_string())?;
        let Ok(entry) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let event_type = entry.get("type").and_then(Value::as_str).unwrap_or("");
        if !matches!(event_type, "user" | "assistant")
            || entry.get("sourceToolAssistantUUID").is_some()
        {
            continue;
        }
        let message = entry.get("message");
        let role = message
            .and_then(|message| message.get("role"))
            .and_then(Value::as_str)
            .unwrap_or(event_type);
        if !matches!(role, "user" | "assistant") {
            continue;
        }
        let identity = entry
            .get("uuid")
            .and_then(Value::as_str)
            .or_else(|| {
                message
                    .and_then(|message| message.get("id"))
                    .and_then(Value::as_str)
            })
            .map(str::to_string)
            .unwrap_or(line);
        if !seen.insert(identity) {
            continue;
        }
        append_native_message(
            &mut messages,
            &mut total_chars,
            role,
            transcript_text(message.and_then(|message| message.get("content"))),
        );
    }
    Ok(messages)
}

fn read_codex_session(path: &Path) -> Result<Vec<(String, String)>, String> {
    let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
    if metadata.len() > 16 * 1024 * 1024 {
        return Ok(Vec::new());
    }
    let file = fs::File::open(path).map_err(|error| error.to_string())?;
    let mut seen = HashSet::new();
    let mut messages = Vec::new();
    let mut total_chars = 0usize;
    for line in BufReader::new(file).lines() {
        if messages.len() >= NATIVE_SESSION_MAX_MESSAGES || total_chars >= NATIVE_SESSION_MAX_CHARS
        {
            break;
        }
        let line = line.map_err(|error| error.to_string())?;
        let Ok(entry) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        if entry.get("type").and_then(Value::as_str) != Some("response_item") {
            continue;
        }
        let Some(payload) = entry.get("payload").filter(|payload| payload.is_object()) else {
            continue;
        };
        if payload.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        let Some(role) = payload
            .get("role")
            .and_then(Value::as_str)
            .filter(|role| matches!(*role, "user" | "assistant"))
        else {
            continue;
        };
        let identity = payload
            .get("id")
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or(line);
        if !seen.insert(identity) {
            continue;
        }
        append_native_message(
            &mut messages,
            &mut total_chars,
            role,
            codex_transcript_text(payload.get("content")),
        );
    }
    Ok(messages)
}

fn read_workbuddy_session(path: &Path) -> Result<Vec<(String, String)>, String> {
    let metadata = fs::metadata(path).map_err(|error| error.to_string())?;
    if metadata.len() > 16 * 1024 * 1024 {
        return Ok(Vec::new());
    }
    let file = fs::File::open(path).map_err(|error| error.to_string())?;
    let mut seen = HashSet::new();
    let mut messages = Vec::new();
    let mut total_chars = 0usize;
    for line in BufReader::new(file).lines() {
        if messages.len() >= NATIVE_SESSION_MAX_MESSAGES || total_chars >= NATIVE_SESSION_MAX_CHARS
        {
            break;
        }
        let line = line.map_err(|error| error.to_string())?;
        let Ok(entry) = serde_json::from_str::<Value>(&line) else {
            continue;
        };
        let message = entry.get("message");
        let role = entry
            .get("role")
            .or_else(|| message.and_then(|message| message.get("role")))
            .or_else(|| entry.get("type"))
            .and_then(Value::as_str)
            .filter(|role| matches!(*role, "user" | "assistant"));
        let Some(role) = role else {
            continue;
        };
        let identity = entry
            .get("id")
            .or_else(|| entry.get("uuid"))
            .and_then(Value::as_str)
            .map(str::to_string)
            .unwrap_or(line);
        if !seen.insert(identity) {
            continue;
        }
        let content = message
            .and_then(|message| message.get("content"))
            .or_else(|| entry.get("content"))
            .or_else(|| entry.get("text"));
        append_native_message(
            &mut messages,
            &mut total_chars,
            role,
            qoder_transcript_text(content),
        );
    }
    Ok(messages)
}

/// Read the single Claude turn that begins with this hook's `prompt_id`.
/// Transcript lines for tool results are intentionally skipped: they are
/// operational detail, whereas the user prompt and assistant text form the
/// conversation we want the memory model to understand.
fn read_claude_turn(path: &Path, prompt_id: &str) -> Result<Vec<(String, String)>, String> {
    let metadata = std::fs::metadata(path).map_err(|error| error.to_string())?;
    if metadata.len() > 16 * 1024 * 1024 {
        return Err("Claude transcript exceeds 16 MB safety limit".into());
    }
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut collecting = false;
    let mut messages = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line.map_err(|error| error.to_string())?;
        let entry: Value = match serde_json::from_str(&line) {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        let entry_type = entry.get("type").and_then(Value::as_str).unwrap_or("");
        let is_prompt = entry_type == "user"
            && entry.get("promptId").and_then(Value::as_str).is_some()
            && entry.get("sourceToolAssistantUUID").is_none();
        if is_prompt {
            let entry_prompt_id = entry.get("promptId").and_then(Value::as_str).unwrap_or("");
            if collecting && entry_prompt_id != prompt_id {
                break;
            }
            if entry_prompt_id == prompt_id {
                collecting = true;
            }
        }
        if !collecting || !matches!(entry_type, "user" | "assistant") {
            continue;
        }
        let role = entry
            .get("message")
            .and_then(|message| message.get("role"))
            .and_then(Value::as_str)
            .unwrap_or(entry_type);
        let content = entry
            .get("message")
            .and_then(|message| message.get("content"));
        let text = transcript_text(content);
        let text = strip_memory_thinking(role, &text);
        if !text.is_empty() {
            messages.push((role.to_string(), truncate_text(&text, 24_000)));
        }
    }
    Ok(messages)
}

/// Read the Codex desktop JSONL records belonging to a hook's `turn_id`.
/// Codex stores its dialogue as `response_item/message` payloads, each with
/// `internal_chat_message_metadata_passthrough.turn_id`; tool calls and
/// reasoning are separate response-item types and are intentionally ignored.
fn read_codex_turn(path: &Path, turn_id: &str) -> Result<Vec<(String, String)>, String> {
    let metadata = std::fs::metadata(path).map_err(|error| error.to_string())?;
    if metadata.len() > 16 * 1024 * 1024 {
        return Err("Codex transcript exceeds 16 MB safety limit".into());
    }
    let file = std::fs::File::open(path).map_err(|error| error.to_string())?;
    let mut messages = Vec::new();
    for line in BufReader::new(file).lines() {
        let line = line.map_err(|error| error.to_string())?;
        let entry: Value = match serde_json::from_str(&line) {
            Ok(entry) => entry,
            Err(_) => continue,
        };
        if entry.get("type").and_then(Value::as_str) != Some("response_item") {
            continue;
        }
        let payload = match entry.get("payload") {
            Some(Value::Object(_)) => entry.get("payload").unwrap(),
            _ => continue,
        };
        if payload.get("type").and_then(Value::as_str) != Some("message") {
            continue;
        }
        let message_turn_id = payload
            .get("internal_chat_message_metadata_passthrough")
            .and_then(|metadata| metadata.get("turn_id"))
            .and_then(Value::as_str);
        if message_turn_id != Some(turn_id) {
            continue;
        }
        let role = payload.get("role").and_then(Value::as_str).unwrap_or("");
        if !matches!(role, "user" | "assistant") {
            continue;
        }
        let text = codex_transcript_text(payload.get("content"));
        let text = strip_memory_thinking(role, &text);
        if !text.is_empty() {
            messages.push((role.to_string(), truncate_text(&text, 24_000)));
        }
    }
    Ok(messages)
}

fn transcript_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.trim().to_string(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|block| {
                (block.get("type").and_then(Value::as_str) == Some("text"))
                    .then(|| block.get("text").and_then(Value::as_str))
                    .flatten()
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn codex_transcript_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.trim().to_string(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter_map(|block| {
                matches!(
                    block.get("type").and_then(Value::as_str),
                    Some("input_text" | "output_text")
                )
                .then(|| block.get("text").and_then(Value::as_str))
                .flatten()
            })
            .collect::<Vec<_>>()
            .join("\n"),
        _ => String::new(),
    }
}

fn parse_typed_l1_candidates(
    text: &str,
) -> Result<Vec<crate::telemetry_store::L1MemoryCandidate>, String> {
    let normalized = text
        .trim()
        .trim_start_matches("```json")
        .trim_start_matches("```")
        .trim_end_matches("```")
        .trim();
    let Some(values) = serde_json::from_str::<Value>(normalized)
        .ok()
        .and_then(|value| value.get("memories").and_then(Value::as_array).cloned())
    else {
        return Err("模型输出未符合 L1 JSON 契约，缺少 memories 数组".into());
    };
    if values.is_empty() {
        return Err("模型未输出任何 L1 记忆".into());
    }
    if values.len() > 3 {
        return Err("模型输出超过 L1 最多 3 条的契约".into());
    }
    let mut candidates = Vec::with_capacity(values.len());
    for (index, value) in values.into_iter().enumerate() {
        let content = value
            .get("content")
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|content| !content.is_empty())
            .ok_or_else(|| format!("第 {} 条 L1 缺少 content", index + 1))?;
        let memory_type = value
            .get("type")
            .or_else(|| value.get("memory_type"))
            .and_then(Value::as_str)
            .map(str::trim)
            .filter(|memory_type| !memory_type.is_empty())
            .ok_or_else(|| format!("第 {} 条 L1 缺少 type", index + 1))?;
        let durability = value
            .get("durability")
            .and_then(Value::as_str)
            .map(str::trim)
            .ok_or_else(|| format!("第 {} 条 L1 漏标 durability", index + 1))?;
        crate::telemetry_store::validate_l1_durability(durability)
            .map_err(|error| format!("第 {} 条 L1 {error}", index + 1))?;
        if index == 0 && memory_type != "summary" {
            return Err("第 1 条 L1 必须为 summary".into());
        }
        if memory_type == "summary" && durability != "session" {
            return Err(format!(
                "第 {} 条 summary 的 durability 必须为 session",
                index + 1
            ));
        }
        candidates.push(crate::telemetry_store::L1MemoryCandidate {
            content: content.into(),
            memory_type: memory_type.to_ascii_lowercase(),
            durability: durability.to_ascii_lowercase(),
        });
    }
    Ok(candidates)
}

fn now_str() -> String {
    chrono::Local::now().format("%H:%M:%S").to_string()
}

#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub struct MemoryLayerRunResult {
    pub document: crate::telemetry_store::MemoryLayerDocument,
    pub selected_l1_count: u32,
    pub stage_count: u32,
    pub message: String,
}

/// Select a bounded and diverse evidence set.  Decisions, constraints and
/// preference candidates are never dropped merely because a session is long;
/// remaining items are capped per session to prevent one transcript from
/// monopolising the L2 input.
fn select_l2_evidence(
    items: Vec<crate::telemetry_store::LocalMemorySnapshot>,
) -> Vec<crate::telemetry_store::LocalMemorySnapshot> {
    let mut selected = Vec::new();
    let mut selected_ids = HashSet::new();
    for item in &items {
        if matches!(
            item.memory_type.as_str(),
            "decision" | "constraint" | "preference_candidate"
        ) && selected_ids.insert(item.id.clone())
        {
            selected.push(item.clone());
        }
    }
    let mut per_session: HashMap<String, u8> = HashMap::new();
    for item in items {
        if selected.len() >= 240 {
            break;
        }
        let count = per_session.entry(item.session_id.clone()).or_default();
        if *count >= 2 || !selected_ids.insert(item.id.clone()) {
            continue;
        }
        *count += 1;
        selected.push(item);
    }
    selected.truncate(240);
    selected
}

fn l2_evidence_text(items: &[crate::telemetry_store::LocalMemorySnapshot]) -> String {
    items
        .iter()
        .map(|item| {
            format!(
                "[{} | {} | {} | {}]\n{}",
                item.id,
                item.memory_type,
                item.source,
                item.event_time.as_deref().unwrap_or("unknown-time"),
                item.memory
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

/// L3 must not rely exclusively on a single recent L2. Feed it direct L1
/// decisions, constraints, and explicit preference candidates as auditable
/// long-term evidence, while retaining the L2 for contextual synthesis.
fn select_l3_l1_evidence(
    items: Vec<crate::telemetry_store::LocalMemorySnapshot>,
) -> Vec<crate::telemetry_store::LocalMemorySnapshot> {
    items
        .into_iter()
        .filter(|item| {
            matches!(
                item.memory_type.as_str(),
                "decision" | "constraint" | "preference_candidate"
            )
        })
        .filter(|item| item.durability == "long_term")
        .filter(|item| !is_l3_volatile_bullet(&item.memory))
        .take(L3_L1_EVIDENCE_LIMIT)
        .collect()
}

fn l3_l1_evidence_text(items: &[crate::telemetry_store::LocalMemorySnapshot]) -> String {
    items
        .iter()
        .map(|item| {
            format!(
                "[L1:{} | {} | {} | {}]\n{}",
                item.id,
                item.memory_type,
                item.durability,
                item.event_time.as_deref().unwrap_or("unknown-time"),
                item.memory
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n")
}

fn l2_map_messages(evidence: &str) -> Vec<Value> {
    vec![
        json!({"role":"system","content":"You are performing evidence-preserving L1-to-L2 memory compression. Return concise Markdown in the input language. Preserve confirmed decisions, constraints, explicit preferences, unresolved commitments, and their qualifiers. Do not invent facts, merge conflicting claims, or turn one-off activity into a user preference. Keep source ids in square brackets after every retained claim. This is an intermediate map; do not write a profile."}),
        json!({"role":"user","content":format!("L1 evidence:\n{evidence}")}),
    ]
}

fn l2_reduce_messages(parts: &[String]) -> Vec<Value> {
    vec![
        json!({"role":"system","content":format!("Create the current L2 short-term working memory from evidence-preserving map summaries. Output ONLY the finished memory document in concise Markdown in the original language, at most {MEMORY_LAYER_DOC_MAX_CHARS} characters. Begin directly with the memory; never describe the task, map summaries, source material, your plan, or reasoning. Organize: Current focus, Confirmed decisions, Constraints/preferences, Open items/risks. Retain source ids in square brackets for every factual bullet. Resolve neither ambiguity nor conflicts: label them explicitly. Exclude routine actions and stale details.")}),
        json!({"role":"user","content":format!("Map summaries:\n{}", parts.join("\n\n---\n\n"))}),
    ]
}

fn is_l2_final_heading(line: &str) -> bool {
    let title = line
        .trim_start()
        .trim_start_matches('#')
        .trim_start()
        .to_ascii_lowercase();
    title.starts_with("当前聚焦") || title.starts_with("current focus")
}

fn unwrap_markdown_document_fence(content: &str) -> String {
    let trimmed = content.trim();
    let Some((opening, remainder)) = trimmed.split_once('\n') else {
        return trimmed.to_string();
    };
    let language = opening
        .trim()
        .strip_prefix("```")
        .map(str::trim)
        .map(str::to_ascii_lowercase);
    if !matches!(language.as_deref(), Some("" | "markdown" | "md" | "text")) {
        return trimmed.to_string();
    }
    let body = remainder
        .rsplit_once("\n```")
        .map(|(body, _)| body)
        .unwrap_or(remainder)
        .trim();
    let is_markdown_document = body.lines().any(|line| {
        let line = line.trim_start();
        line.starts_with('#')
            || line.starts_with("- ")
            || line.starts_with("* ")
            || line.starts_with("| ")
    });
    if is_markdown_document {
        body.to_string()
    } else {
        trimmed.to_string()
    }
}

/// Reasoning models sometimes expose a natural-language planning preamble even
/// when no explicit `<think>` tags are present.  The L2 contract requires the
/// document to start with the Current Focus heading, so retain only the final
/// document when that heading appears later in the response.
fn normalize_l2_document(content: &str) -> String {
    let content = unwrap_markdown_document_fence(&strip_thinking_blocks(content));
    let mut byte_offset = 0;
    let mut final_heading_offset = None;
    for part in content.split_inclusive('\n') {
        if is_l2_final_heading(part) {
            // A leaked plan often contains an outline headed "Current focus"
            // before the actual document. The final occurrence is the only
            // candidate that can start the publishable L2 result.
            final_heading_offset = Some(byte_offset);
        }
        byte_offset += part.len();
    }
    if let Some(offset) = final_heading_offset {
        return content[offset..].trim().to_string();
    }
    // `split_inclusive` does not yield an empty final part, but it does yield a
    // non-newline-terminated final line. This fallback documents that a valid
    // heading is mandatory rather than silently publishing model narration.
    if let Some(last_line) = content
        .lines()
        .last()
        .filter(|line| is_l2_final_heading(line))
    {
        return last_line.trim().to_string();
    }
    content.trim().to_string()
}

fn is_l2_analysis_preamble(content: &str) -> bool {
    let opening = content
        .trim_start()
        .chars()
        .take(240)
        .collect::<String>()
        .to_ascii_lowercase();
    [
        "the user wants",
        "let me analyze",
        "let me review",
        "let me think",
        "let me synthesize",
        "i need to ",
        "i will ",
        "analysis:",
        "用户希望",
        "让我",
        "我需要",
        "我将",
        "分析：",
        "思考：",
    ]
    .iter()
    .any(|prefix| opening.starts_with(prefix))
}

fn has_l2_final_heading(content: &str) -> bool {
    content
        .lines()
        .find(|line| !line.trim().is_empty())
        .is_some_and(is_l2_final_heading)
}

fn normalize_l3_profile(content: &str) -> String {
    unwrap_markdown_document_fence(&strip_thinking_blocks(content))
}

fn has_l3_category_heading(content: &str) -> bool {
    content.lines().any(|line| {
        let trimmed = line.trim_start();
        trimmed.starts_with("## ") || trimmed.starts_with("### ")
    })
}

/// L2 also carries recent implementation decisions. These markers identify
/// material that may be useful in a work log but must not become user-level
/// L3 memory merely because a model described it as a constraint.
fn is_l3_volatile_bullet(line: &str) -> bool {
    let lower = line.to_ascii_lowercase();
    [
        "minimax",
        "endpoint",
        "api.minimax",
        "tls",
        "reasoning_split",
        "reasoning_content",
        "max_completion",
        "token",
        "模型",
        "端点",
        "输出预算",
        "i18n",
        "tauri",
        "window.confirm",
        "window.alert",
        "ui ",
        "ui、",
        "前端日期",
        "rust 9",
        "纳秒",
        "cargo check",
        "单元测试",
        "test command",
        "dev 进程",
        "dev process",
        "构建命令",
        "全局 json",
        "thinking",
        "当前任务",
        "当前项目状态",
    ]
    .iter()
    .any(|marker| lower.contains(marker))
}

/// Keep only categorized, durable bullets. L3 is a user profile, not a
/// project changelog; an empty filtered section is deliberately removed.
fn filter_l3_profile(content: &str) -> String {
    let mut sections = Vec::<(String, Vec<String>)>::new();
    let mut heading: Option<String> = None;
    let mut bullets = Vec::<String>::new();
    let flush = |heading: &mut Option<String>,
                 bullets: &mut Vec<String>,
                 sections: &mut Vec<(String, Vec<String>)>| {
        if let Some(title) = heading.take().filter(|_| !bullets.is_empty()) {
            sections.push((title, std::mem::take(bullets)));
        } else {
            bullets.clear();
        }
    };
    for line in content.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with("## ") || trimmed.starts_with("### ") {
            flush(&mut heading, &mut bullets, &mut sections);
            heading = Some(trimmed.to_string());
        } else if (trimmed.starts_with("- ") || trimmed.starts_with("* "))
            && !is_l3_volatile_bullet(trimmed)
        {
            bullets.push(format!("- {}", trimmed[2..].trim()));
        }
    }
    flush(&mut heading, &mut bullets, &mut sections);
    sections
        .into_iter()
        .flat_map(|(heading, bullets)| std::iter::once(heading).chain(bullets))
        .collect::<Vec<_>>()
        .join("\n")
}

fn source_prefers_chinese(source: &str) -> bool {
    let han = source
        .chars()
        .filter(|character| matches!(*character, '\u{4e00}'..='\u{9fff}'))
        .count();
    let latin = source
        .chars()
        .filter(|character| character.is_ascii_alphabetic())
        .count();
    han.saturating_mul(2) > latin
}

fn is_usable_l3_profile(content: &str, source: &str) -> bool {
    let trimmed = content.trim();
    if trimmed.is_empty()
        || trimmed.chars().count() > L3_PROFILE_MAX_CHARS
        || is_l2_analysis_preamble(trimmed)
        || !has_l3_category_heading(trimmed)
        || trimmed.lines().any(is_l3_volatile_bullet)
    {
        return false;
    }
    let lower = trimmed.to_ascii_lowercase();
    if lower.contains("[内容已截断]")
        || lower.contains("[truncated]")
        || lower.contains("pending human approval")
    {
        return false;
    }
    !source_prefers_chinese(source)
        || trimmed
            .chars()
            .filter(|character| matches!(*character, '\u{4e00}'..='\u{9fff}'))
            .count()
            >= 8
}

fn l3_profile_messages(baseline: &str, evidence: &str, language: &str) -> Vec<Value> {
    vec![
        json!({"role":"system","content":format!("You write an L3 user Profile for long-term reuse across agent sessions. The existing published profile in the user message is the AUTHORITATIVE baseline and your most important input: carry its preferences, constraints and decisions forward by default, including any manual edits the user made, and only drop or change an existing item when the new evidence clearly contradicts it. {language}\n\nOutput ONLY the finished categorized Markdown document: no document title, preamble, code fence, approval notice, or explanation. Use 2–6 short category headings in the source language (each as `## Category`), with relevant factual bullets below each heading. Stop after the final bullet. The final document MUST be no more than {L3_PROFILE_TARGET_CHARS} characters.\n\nYou receive both L2 working-memory context and filtered direct L1 evidence. Prefer explicit L1 preferences and constraints, using L2 only to corroborate or add durable context. Apply a strict 90-day, cross-project test: retain only an explicit user preference or a hard constraint that would still apply to unrelated future work. Exclude current tasks, project status, change logs, dates, paths, source ids, secrets, model/provider names, endpoints, token budgets, framework APIs, UI conventions, i18n file rules, date-parsing quirks, build/test commands, temporary incidents, and implementation details. If evidence is merely a project decision or current engineering practice, omit it. Do not infer preferences. Clearly label uncertainty instead of guessing.")}),
        json!({"role":"user","content":format!("Existing published profile (may be empty):\n{baseline}\n\nL2 evidence:\n{evidence}")}),
    ]
}

fn l3_profile_retry_messages(baseline: &str, evidence: &str, language: &str) -> Vec<Value> {
    vec![
        json!({"role":"system","content":format!("Write the final long-term L3 Profile now. The existing published profile is the authoritative baseline and your most important input: carry its items forward by default, including manual user edits, and only change them when the new evidence clearly contradicts them. {language} Output only Markdown with 2–6 `## Category` headings in the source language and concise bullets under them: no document title, no preamble, no code fence, no explanation. Each bullet should stay one or two sentences and the entire final body must be under {L3_PROFILE_TARGET_CHARS} characters. Stop immediately after the final bullet. Use explicit filtered L1 preferences and constraints plus corroborating L2 context. Keep only facts that still apply after 90 days across unrelated projects. Remove model/provider names, endpoints, token budgets, framework/UI/i18n rules, build/test commands, current work, implementation detail, tool lists, status updates, source ids, and anything temporary. Do not infer preferences.")}),
        json!({"role":"user","content":format!("Existing published profile (may be empty):\n{baseline}\n\nL2 evidence:\n{evidence}")}),
    ]
}

fn l2_final_retry_messages(parts: &[String]) -> Vec<Value> {
    vec![
        json!({"role":"system","content":"Return the FINAL L2 working-memory document now. Do not plan, analyze, acknowledge, explain, or mention this instruction, the user, map summaries, or source documents. The first non-whitespace line MUST be the `## Current focus` heading in the evidence language; emit no text before it. Your entire response must be the finished concise Markdown memory in the evidence language. Use headings: Current focus, Confirmed decisions, Constraints/preferences, Open items/risks. Every retained factual bullet must keep its square-bracket source id. If there is no supported content for a heading, omit it."}),
        json!({"role":"user","content":format!("Evidence-preserving map summaries:\n{}", parts.join("\n\n---\n\n"))}),
    ]
}

/// Manual L2 generation only reads a bounded recent L1 window.  It uses a
/// map-reduce compression pass and stores all selected evidence ids, rather
/// than feeding the entire memory library into one unreviewable prompt.
#[tauri::command]
pub async fn memory_short_term_consolidate(
    telemetry: tauri::State<'_, crate::telemetry_store::TelemetryStore>,
    days: Option<i64>,
) -> Result<MemoryLayerRunResult, String> {
    let days = days.unwrap_or(30).clamp(1, 90);
    let mut evidence = select_l2_evidence(telemetry.recent_l1_memories(days, 500)?);
    // 归入 L2 的用户自定义记忆强制进入整理证据：不受时间窗与类型筛选
    // 影响；L3 层自定义记忆只进 Profile，两者相互独立。
    let selected_ids = evidence
        .iter()
        .map(|item| item.id.clone())
        .collect::<std::collections::HashSet<_>>();
    for item in telemetry.user_defined_l1_memories_for_scope("l2")? {
        if !selected_ids.contains(&item.id) {
            evidence.push(item);
        }
    }
    if evidence.is_empty() {
        return Err(format!("最近 {days} 天没有可用于 L2 的已提取会话记忆"));
    }
    let provider = llm::memory_extraction_provider()?;
    let mut maps = Vec::new();
    for chunk in evidence.chunks(50) {
        let text = l2_evidence_text(chunk);
        maps.push(truncate_text(
            &llm::complete_text(&provider, &l2_map_messages(&text)).await?,
            8_000,
        ));
    }
    let mut content =
        normalize_l2_document(&llm::complete_text(&provider, &l2_reduce_messages(&maps)).await?);
    if is_l2_analysis_preamble(&content) || !has_l2_final_heading(&content) {
        content = normalize_l2_document(
            &llm::complete_text(&provider, &l2_final_retry_messages(&maps)).await?,
        );
    }
    // Normalise before truncation.  Otherwise a long exposed planning preamble
    // can consume the L2 character budget and discard the actual memory.
    content = truncate_text(&content, MEMORY_LAYER_DOC_MAX_CHARS);
    if content.trim().is_empty() {
        return Err("L2 记忆模型返回空内容，未写入任何数据".into());
    }
    if is_l2_analysis_preamble(&content) || !has_l2_final_heading(&content) {
        return Err("L2 记忆模型返回任务分析而非最终记忆；已停止发布，请稍后重新生成".into());
    }
    let source_ids = evidence
        .iter()
        .map(|item| item.id.clone())
        .collect::<Vec<_>>();
    let now = chrono::Utc::now().to_rfc3339();
    let start = (chrono::Utc::now() - chrono::Duration::days(days)).to_rfc3339();
    let document = telemetry.save_memory_layer_document(
        "l2",
        &content,
        "published",
        "l1",
        &source_ids,
        Some(start),
        Some(now),
    )?;
    Ok(MemoryLayerRunResult {
        document,
        selected_l1_count: source_ids.len() as u32,
        stage_count: maps.len() as u32,
        message: format!(
            "L2 已由 {} 条 L1 证据分 {} 组压缩并发布；每条保留来源 id",
            source_ids.len(),
            maps.len()
        ),
    })
}

/// Build, but never automatically publish, a compact L3 Profile. Existing
/// Profile text is input as a baseline so updates preserve stable preferences
/// unless newer L2 evidence explicitly supports a change.
#[tauri::command]
pub async fn memory_long_term_profile_draft(
    telemetry: tauri::State<'_, crate::telemetry_store::TelemetryStore>,
) -> Result<MemoryLayerRunResult, String> {
    let l2 = telemetry
        .memory_layer_documents("l2", 12)?
        .into_iter()
        .filter(|doc| doc.state == "published")
        .collect::<Vec<_>>();
    if l2.is_empty() {
        return Err("请先手动生成 L2 近 30 天工作记忆，再创建 L3 Profile 草案".into());
    }
    let current = telemetry.active_memory_layer_document("l3")?;
    let mut l1 = select_l3_l1_evidence(telemetry.local_l1_memory_snapshot()?);
    // 归入 L3 的用户自定义记忆强制进入 Profile 证据：类型/易变性筛选
    // 可能把它们挡在外面，但用户手写条目本身就是明确的长期偏好与约束。
    let selected_ids = l1
        .iter()
        .map(|item| item.id.clone())
        .collect::<std::collections::HashSet<_>>();
    for item in telemetry.user_defined_l1_memories_for_scope("l3")? {
        if !selected_ids.contains(&item.id) {
            l1.push(item);
        }
    }
    let mut source_references = l2
        .iter()
        .map(|doc| ("l2".to_string(), doc.id.clone()))
        .collect::<Vec<_>>();
    source_references.extend(l1.iter().map(|item| ("l1".to_string(), item.id.clone())));
    let l2_input = l2
        .iter()
        .map(|doc| {
            format!(
                "[L2:{} | {}]\n{}",
                doc.id,
                doc.created_at,
                normalize_l2_document(&doc.content)
            )
        })
        .collect::<Vec<_>>()
        .join("\n\n---\n\n");
    let input = format!(
        "Published L2 context:\n{l2_input}\n\n---\n\nFiltered direct L1 evidence:\n{}",
        l3_l1_evidence_text(&l1)
    );
    let provider = llm::memory_extraction_provider()?;
    let baseline = current
        .as_ref()
        .map(|doc| normalize_l3_profile(&doc.content))
        .unwrap_or_else(|| "(no published profile yet)".to_string());
    let language = output_language_directive(&input);
    // Use the user's configured provider generation limit here. A narrow
    // caller-side ceiling can end an M3 response before it emits `content`,
    // even when the final persisted Profile itself is deliberately short.
    // Treat any empty/invalid first response like malformed output and retry
    // once with a much smaller final-body contract.
    let initial = llm::complete_text_with_limit(
        &provider,
        &l3_profile_messages(&baseline, &input, language),
        None,
    )
    .await;
    let initial_error = initial.as_ref().err().cloned();
    let mut content = initial
        .ok()
        .map(|text| filter_l3_profile(&normalize_l3_profile(&text)))
        .unwrap_or_default();
    if !is_usable_l3_profile(&content, &input) {
        let retry = llm::complete_text_with_limit(
            &provider,
            &l3_profile_retry_messages(&baseline, &input, language),
            None,
        )
        .await;
        content = match retry {
            Ok(text) => filter_l3_profile(&normalize_l3_profile(&text)),
            Err(retry_error) => {
                return Err(initial_error
                    .map(|error| format!("L3 首次生成失败：{error}；紧凑重试失败：{retry_error}"))
                    .unwrap_or(retry_error))
            }
        };
    }
    if !is_usable_l3_profile(&content, &input) {
        return Err(
            "L3 Profile 未满足语言、长度或长期记忆格式要求，未保存草案；请稍后重新生成".into(),
        );
    }
    // 永久写入 MCP 调用提示：确定性追加而非依赖生成模型，重生成后提示
    // 依然存在；已含标记时 with_l3_mcp_hint 原样返回。
    let content = crate::memory_mcp::with_l3_mcp_hint(&content);
    let document = telemetry.save_memory_layer_document_with_sources(
        "l3",
        &content,
        "draft",
        &source_references,
        None,
        None,
    )?;
    Ok(MemoryLayerRunResult {
        document,
        selected_l1_count: l1.len() as u32,
        stage_count: l2.len() as u32,
        message: format!(
            "已基于 {} 份已发布 L2 与 {} 条筛选 L1 证据创建 L3 Profile 草案；请检查后手动发布",
            l2.len(),
            l1.len()
        ),
    })
}

#[tauri::command]
pub fn memory_long_term_profile_publish(
    telemetry: tauri::State<'_, crate::telemetry_store::TelemetryStore>,
    document_id: String,
    content: Option<String>,
) -> Result<crate::telemetry_store::MemoryLayerDocument, String> {
    let document = telemetry.publish_memory_layer_document(&document_id, content.as_deref())?;
    if document.layer != "l3" {
        return Err("只能发布 L3 Profile 草案".into());
    }
    Ok(document)
}

/// 手动编辑已发布的 L2 工作记忆正文：L2 生成即发布、没有发布闸门，
/// 保存即覆盖当前发布版，下一次注入与 L3 草案立刻生效。
#[tauri::command]
pub fn memory_layer_document_update(
    telemetry: tauri::State<'_, crate::telemetry_store::TelemetryStore>,
    document_id: String,
    content: String,
) -> Result<crate::telemetry_store::MemoryLayerDocument, String> {
    telemetry.update_published_memory_layer_content(&document_id, &content)
}

#[tauri::command]
pub fn memory_long_term_profile_delete_draft(
    telemetry: tauri::State<'_, crate::telemetry_store::TelemetryStore>,
    document_id: String,
) -> Result<(), String> {
    telemetry.delete_l3_draft(&document_id)
}

#[tauri::command]
pub fn memory_layer_documents(
    telemetry: tauri::State<'_, crate::telemetry_store::TelemetryStore>,
    layer: String,
) -> Result<Vec<crate::telemetry_store::MemoryLayerDocument>, String> {
    if !matches!(layer.as_str(), "l2" | "l3") {
        return Err("仅支持 L2 或 L3 记忆层".into());
    }
    telemetry.memory_layer_documents(&layer, 12)
}

// ────────────────────────────────────────────────────────────────────────────
// Hook 自动配置：向支持 hook 的 Agent 写入回调（Claude Code settings.json）
// ────────────────────────────────────────────────────────────────────────────

/// Stable marker used to recognise entries injected by Agent Manager.  Do not
/// use the HTTP address here: the hook server port is user configurable.
const HOOK_MARKER: &str = "X-Agent-Manager-Hook";
const CLAUDE_SETTINGS: &str = ".claude/settings.json";
const CODEX_HOOKS: &str = ".codex/hooks.json";
const WORKBUDDY_SETTINGS: &str = ".workbuddy/settings.json";

fn agent_settings_path(agent_type: &str) -> Option<PathBuf> {
    if agent_type == "qoder" {
        // Qoder 国际版在 ~/.qoder，国内版在 ~/.qoder-cn；由注册表按实际
        // 活动目录（含用户覆盖）选出配置主目录。
        return crate::agent_sources::config_home("qoder")
            .map(|home| home.join("settings.json"));
    }
    let relative = match agent_type {
        "claude" => CLAUDE_SETTINGS,
        "codex" => CODEX_HOOKS,
        "workbuddy" => WORKBUDDY_SETTINGS,
        _ => return None,
    };
    dirs_next::home_dir().map(|h| h.join(relative))
}

fn hook_command(agent_type: &str) -> Result<String, String> {
    let url = format!(
        "http://127.0.0.1:{}/memory/hook?source={agent_type}",
        crate::agent_http::AGENT_HTTP_PORT
    );
    let marker = format!(r#" -H "{HOOK_MARKER}: 1""#);

    if cfg!(windows) {
        Ok(format!(
            r#"curl.exe -fsS --max-time 5 -X POST "{url}" -H "Content-Type: application/json" --data-binary "@-" -o NUL{marker}"#
        ))
    } else {
        Ok(format!(
            r#"curl -fsS --max-time 5 -X POST '{url}' -H 'Content-Type: application/json' --data-binary '@-' -o /dev/null{marker}"#
        ))
    }
}

/// SessionStart 注入 hook：拉取共享记忆上下文并写到 stdout。端点已按来源
/// 返回 Claude 形态的结构化 JSON（hookSpecificOutput.additionalContext），
/// curl 只需透传，harness 解析后注入模型上下文。应用未运行时 curl 静默
/// 失败，不阻塞会话启动。URL 携带 source 标识，端点据此把注入计入记忆
/// 注入摘要。
fn hook_inject_command(agent_type: &str) -> String {
    let url = format!(
        "http://127.0.0.1:{}/memory/context?source={agent_type}",
        crate::agent_http::AGENT_HTTP_PORT
    );
    let marker = format!(r#" -H "{HOOK_MARKER}: 1""#);
    if cfg!(windows) {
        format!(r#"curl.exe -fsS --max-time 5 "{url}"{marker}"#)
    } else {
        format!(r#"curl -fsS --max-time 5 '{url}'{marker}"#)
    }
}

/// Install hooks for an adapter that implements the command-hook shape.  The
/// configuration paths stay explicit per harness; one Agent is never written
/// into another Agent's settings file.
#[tauri::command]
pub fn memory_hook_install(agent_type: String) -> Result<Vec<String>, String> {
    match agent_type.as_str() {
        "claude" | "qoder" | "codex" | "workbuddy" => install_command_hooks(&agent_type),
        other => Err(format!("暂不支持的 Agent 类型: {other}")),
    }
}

fn install_command_hooks(agent_type: &str) -> Result<Vec<String>, String> {
    let path = agent_settings_path(agent_type).ok_or("无法定位用户主目录")?;
    let mut settings: Value = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_else(|| json!({}));

    let hooks = settings
        .get_mut("hooks")
        .map(|h| h.take())
        .unwrap_or_else(|| json!({}));
    let mut hooks = if hooks.is_object() { hooks } else { json!({}) };

    let command = hook_command(agent_type)?;
    let inject_command = hook_inject_command(agent_type);
    let mut installed = Vec::new();
    // 出站沉淀（Agent → 本应用）与入站注入（SessionStart → 模型上下文）
    for (event, command) in [
        ("UserPromptSubmit", &command),
        ("PostToolUse", &command),
        ("Stop", &command),
        ("SessionStart", &inject_command),
    ] {
        // 已有的该事件配置保留；若未包含我们的 hook 则追加
        let existing = hooks.get(event).cloned().unwrap_or_else(|| json!([]));
        let contains_ours = serde_json::to_string(&existing)
            .map(|s| s.contains(HOOK_MARKER))
            .unwrap_or(false);
        if contains_ours && event != "SessionStart" {
            continue;
        }
        let mut arr = if existing.is_array() {
            existing.as_array().unwrap().clone()
        } else {
            vec![]
        };
        // 注入命令可能随版本演进（例如补充 source 标识）；始终把我们旧版的
        // SessionStart hook 替换为最新命令，其余事件的已有配置保持不动。
        if event == "SessionStart" {
            arr.retain(|item| {
                serde_json::to_string(item)
                    .map(|s| !s.contains(HOOK_MARKER))
                    .unwrap_or(true)
            });
        }
        arr.push(json!({
            "hooks": [{"type": "command", "command": command}]
        }));
        hooks[event] = Value::Array(arr);
        installed.push(event.to_string());
    }

    settings["hooks"] = hooks;
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("写入 {} 失败: {e}", path.display()))?;
    Ok(installed)
}

/// 卸载本工具注入的 hook 回调（只移除包含 memory/hook 的命令，保留其他配置）。
#[tauri::command]
pub fn memory_hook_uninstall(agent_type: String) -> Result<(), String> {
    if !matches!(
        agent_type.as_str(),
        "claude" | "qoder" | "codex" | "workbuddy"
    ) {
        return Ok(());
    }
    let path = agent_settings_path(&agent_type).ok_or("无法定位用户主目录")?;
    let Ok(text) = std::fs::read_to_string(&path) else {
        return Ok(());
    };
    let Ok(mut settings) = serde_json::from_str::<Value>(&text) else {
        return Ok(());
    };
    let Some(hooks) = settings.get_mut("hooks") else {
        return Ok(());
    };
    if !hooks.is_object() {
        return Ok(());
    }
    for event in ["UserPromptSubmit", "PostToolUse", "Stop", "SessionStart"] {
        if let Some(arr) = hooks.get_mut(event) {
            if let Some(list) = arr.as_array_mut() {
                list.retain(|item| {
                    serde_json::to_string(item)
                        .map(|s| !s.contains(HOOK_MARKER))
                        .unwrap_or(true)
                });
            }
        }
    }
    std::fs::write(
        &path,
        serde_json::to_string_pretty(&settings).map_err(|e| e.to_string())?,
    )
    .map_err(|e| format!("写入 {} 失败: {e}", path.display()))?;
    Ok(())
}

#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub struct HookStatus {
    pub installed: bool,
    pub agent_type: String,
    pub events: Vec<String>,
}

/// 检查某类 Agent 是否已安装记忆 hook。
#[tauri::command]
pub fn memory_hook_status(agent_type: String) -> HookStatus {
    if matches!(
        agent_type.as_str(),
        "claude" | "qoder" | "codex" | "workbuddy"
    ) {
        if let Some(path) = agent_settings_path(&agent_type) {
            if let Ok(text) = std::fs::read_to_string(&path) {
                let Ok(settings) = serde_json::from_str::<Value>(&text) else {
                    return HookStatus {
                        installed: false,
                        agent_type,
                        events: vec![],
                    };
                };
                let events = ["UserPromptSubmit", "PostToolUse", "Stop", "SessionStart"]
                    .iter()
                    .filter(|e| {
                        settings
                            .get("hooks")
                            .and_then(|hooks| hooks.get(**e))
                            .and_then(Value::as_array)
                            .map(|items| {
                                items.iter().any(|item| {
                                    serde_json::to_string(item)
                                        .map(|item| {
                                            item.contains(HOOK_MARKER)
                                                // 旧版注入命令不带 source 标识，无法计入
                                                // 记忆注入摘要；视为未启用以引导升级。
                                                && (**e != "SessionStart"
                                                    || item.contains("memory/context?source="))
                                        })
                                        .unwrap_or(false)
                                })
                            })
                            .unwrap_or(false)
                    })
                    .map(|s| s.to_string())
                    .collect::<Vec<_>>();
                return HookStatus {
                    installed: !events.is_empty(),
                    agent_type,
                    events,
                };
            }
        }
    }
    HookStatus {
        installed: false,
        agent_type,
        events: vec![],
    }
}

/// 沉淀管道状态（前端展示）。
#[derive(serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub struct IngestStatus {
    pub enabled: bool,
    pub buffered_sessions: usize,
    pub model_provider_id: Option<String>,
    pub model_ready: bool,
    pub recent: Vec<IngestLog>,
}

#[tauri::command]
pub fn memory_ingest_status(state: tauri::State<'_, IngestStore>) -> IngestStatus {
    let model_provider_id = crate::llm::memory_extraction_config().provider_id;
    let model_ready = crate::llm::memory_extraction_provider().is_ok();
    IngestStatus {
        enabled: state.is_enabled(),
        buffered_sessions: state.buffered_sessions(),
        model_provider_id,
        model_ready,
        recent: state.recent_logs(),
    }
}

#[tauri::command]
pub fn memory_ingest_set_enabled(state: tauri::State<'_, IngestStore>, enabled: bool) {
    state.set_enabled(enabled);
}

#[tauri::command]
pub fn memory_ingest_flush_pending(state: tauri::State<'_, IngestStore>) -> usize {
    crate::memory_backend::shared_backend()
        .map(|backend| state.flush_pending(backend))
        .unwrap_or(0)
}

/// Backfill complete conversations received before memory extraction was
/// configured, or that failed during a prior extraction attempt.
#[derive(Clone, serde::Serialize)]
#[serde(rename_all = "snake_case")]
pub struct OrganizeConversationsResult {
    pub attempted: u32,
    pub succeeded: u32,
    pub failed: u32,
    pub failure_reasons: Vec<String>,
}

#[tauri::command]
pub async fn memory_ingest_organize_conversations() -> Result<OrganizeConversationsResult, String> {
    let ingest = ingest_store().ok_or("自动沉淀模块尚未初始化")?;
    let telemetry = crate::telemetry_store::shared_store().ok_or("本地对话账本尚未初始化")?;
    let native_imported = scan_native_transcripts(&telemetry, true)?;
    let pending = telemetry.pending_l1_conversations(1_000)?;
    ingest.log(IngestLog { at: now_str(), agent_id: GLOBAL_MEMORY_OWNER.into(), kind: "memory".into(), state: "working".into(), detail: format!("整理队列开始：原生扫描新增 {native_imported}；待处理 {} 个会话，每批 {ORGANIZE_BATCH_LIMIT} 个", pending.len()) });
    let mut result = OrganizeConversationsResult {
        attempted: 0,
        succeeded: 0,
        failed: 0,
        failure_reasons: Vec::new(),
    };
    for (batch_index, batch) in pending.chunks(ORGANIZE_BATCH_LIMIT as usize).enumerate() {
        ingest.log(IngestLog {
            at: now_str(),
            agent_id: GLOBAL_MEMORY_OWNER.into(),
            kind: "memory".into(),
            state: "working".into(),
            detail: format!(
                "正在执行第 {} 批（{} 个会话）",
                batch_index + 1,
                batch.len()
            ),
        });
        for conversation in batch {
            result.attempted += 1;
            let mut last_error = String::new();
            let mut stored = false;
            for retry in 0..=3 {
                ingest.log(IngestLog {
                    at: now_str(),
                    agent_id: GLOBAL_MEMORY_OWNER.into(),
                    kind: "memory".into(),
                    state: "working".into(),
                    detail: format!("正在提取会话要点（第 {}/4 次）", retry + 1),
                });
                match tokio::time::timeout(
                    ORGANIZE_ONE_CONVERSATION_TIMEOUT,
                    extract_l1_conversation(&conversation.conversation_text),
                )
                .await
                {
                    Ok(Ok(candidates)) => {
                        let count = telemetry
                            .store_typed_l1_memories(&conversation.event_key, &candidates)?;
                        crate::memory_backend::queue_semantic_l1_index(
                            candidates.iter().map(|item| item.content.clone()).collect(),
                        );
                        result.succeeded += 1;
                        stored = true;
                        ingest.log(IngestLog {
                            at: now_str(),
                            agent_id: GLOBAL_MEMORY_OWNER.into(),
                            kind: "memory".into(),
                            state: "stored".into(),
                            detail: format!("整理完成：写入 {count} 条记忆"),
                        });
                        break;
                    }
                    Err(_) => {
                        last_error = format!(
                            "记忆模型在 {} 秒内未完成",
                            ORGANIZE_ONE_CONVERSATION_TIMEOUT.as_secs()
                        )
                    }
                    Ok(Err(error)) => last_error = error,
                }
                if retry < 3 {
                    tokio::time::sleep(std::time::Duration::from_millis(500)).await;
                }
            }
            if !stored {
                telemetry.set_l1_failure(&conversation.event_key, &last_error)?;
                result.failed += 1;
                let reason = format!("{}：{}", conversation.event_key, last_error);
                result.failure_reasons.push(reason.clone());
                ingest.log(IngestLog {
                    at: now_str(),
                    agent_id: GLOBAL_MEMORY_OWNER.into(),
                    kind: "memory".into(),
                    state: "retrying".into(),
                    detail: format!("整理失败，已重试 3 次：{reason}"),
                });
            }
        }
    }
    ingest.log(IngestLog {
        at: now_str(),
        agent_id: GLOBAL_MEMORY_OWNER.into(),
        kind: "memory".into(),
        state: "stored".into(),
        detail: format!(
            "整理队列结束：成功 {}，失败 {}",
            result.succeeded, result.failed
        ),
    });
    Ok(result)
}

/// 「待提取记忆」面板的会话列表：已完成但尚未成功提炼的会话，
/// 只带开头摘要，不返回完整对话正文。
#[tauri::command]
pub fn memory_pending_l1_sessions(
    limit: Option<u32>,
) -> Result<Vec<crate::telemetry_store::PendingMemorySession>, String> {
    let telemetry = crate::telemetry_store::shared_store().ok_or("本地对话账本尚未初始化")?;
    telemetry.pending_memory_session_list(limit.unwrap_or(200))
}

/// 「已整理对话」面板的会话列表：已成功提炼为记忆的完整会话，
/// 只带开头摘要，不返回完整对话正文。
#[tauri::command]
pub fn memory_organized_l1_sessions(
    limit: Option<u32>,
) -> Result<Vec<crate::telemetry_store::PendingMemorySession>, String> {
    let telemetry = crate::telemetry_store::shared_store().ok_or("本地对话账本尚未初始化")?;
    telemetry.organized_memory_session_list(limit.unwrap_or(200))
}

/// 面板弹窗用的完整 sanitized 对话正文（user/assistant 轮次）。
#[tauri::command]
pub fn memory_l1_conversation_detail(
    event_key: String,
) -> Result<crate::telemetry_store::MemoryConversationDetail, String> {
    let telemetry = crate::telemetry_store::shared_store().ok_or("本地对话账本尚未初始化")?;
    telemetry
        .conversation_detail_by_event_key(&event_key)?
        .ok_or_else(|| "未找到该会话的完整对话记录".to_string())
}

/// Organize a single staged conversation from the pending panel.  Extraction
/// still goes through the configured memory model; nothing is written when
/// the model is unavailable, and the row is marked failed with the reason.
#[tauri::command]
pub async fn memory_ingest_organize_session(
    event_key: String,
) -> Result<OrganizeConversationsResult, String> {
    let ingest = ingest_store().ok_or("自动沉淀模块尚未初始化")?;
    let telemetry = crate::telemetry_store::shared_store().ok_or("本地对话账本尚未初始化")?;
    let conversation = telemetry
        .conversation_by_event_key(&event_key)?
        .ok_or("该会话不在待提取列表中（可能已整理完成）")?;
    let mut result = OrganizeConversationsResult {
        attempted: 1,
        succeeded: 0,
        failed: 0,
        failure_reasons: Vec::new(),
    };
    let mut last_error = String::new();
    let mut stored = false;
    for retry in 0..=1 {
        ingest.log(IngestLog {
            at: now_str(),
            agent_id: GLOBAL_MEMORY_OWNER.into(),
            kind: "memory".into(),
            state: "working".into(),
            detail: format!("正在提取单个会话要点（第 {}/2 次）", retry + 1),
        });
        match tokio::time::timeout(
            ORGANIZE_ONE_CONVERSATION_TIMEOUT,
            extract_l1_conversation(&conversation.conversation_text),
        )
        .await
        {
            Ok(Ok(candidates)) => {
                let count =
                    telemetry.store_typed_l1_memories(&conversation.event_key, &candidates)?;
                crate::memory_backend::queue_semantic_l1_index(
                    candidates.iter().map(|item| item.content.clone()).collect(),
                );
                result.succeeded = 1;
                stored = true;
                ingest.log(IngestLog {
                    at: now_str(),
                    agent_id: GLOBAL_MEMORY_OWNER.into(),
                    kind: "memory".into(),
                    state: "stored".into(),
                    detail: format!("单会话整理完成：写入 {count} 条记忆"),
                });
                break;
            }
            Err(_) => {
                last_error = format!(
                    "记忆模型在 {} 秒内未完成",
                    ORGANIZE_ONE_CONVERSATION_TIMEOUT.as_secs()
                )
            }
            Ok(Err(error)) => last_error = error,
        }
        if retry < 1 {
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        }
    }
    if !stored {
        telemetry.set_l1_failure(&conversation.event_key, &last_error)?;
        result.failed = 1;
        result
            .failure_reasons
            .push(format!("{}：{}", conversation.event_key, last_error));
    }
    Ok(result)
}

/// Native L1 storage is the reliable source of completed-conversation memory.
/// The optional semantic service remains available for legacy/hand-written
/// items, but its failure must never make captured conversations disappear.
#[tauri::command]
pub fn local_memory_list(
    telemetry: tauri::State<'_, crate::telemetry_store::TelemetryStore>,
    limit: Option<u32>,
) -> Result<Vec<crate::telemetry_store::LocalMemory>, String> {
    telemetry.local_l1_memories(limit.unwrap_or(200))
}

#[tauri::command]
pub fn local_memory_stats(
    telemetry: tauri::State<'_, crate::telemetry_store::TelemetryStore>,
) -> Result<crate::telemetry_store::LocalMemoryStats, String> {
    telemetry.local_l1_memory_stats()
}

/// Explicit, user-initiated L1 rebuild.  This never deletes L0 transcripts;
/// it only clears derived layers and requeues completed conversations.
#[tauri::command]
pub fn local_memory_reset_for_reextraction(
    telemetry: tauri::State<'_, crate::telemetry_store::TelemetryStore>,
) -> Result<crate::telemetry_store::L1ResetResult, String> {
    telemetry.reset_l1_for_reextraction()
}

#[tauri::command]
pub fn local_memory_search(
    telemetry: tauri::State<'_, crate::telemetry_store::TelemetryStore>,
    query: String,
    limit: Option<u32>,
) -> Result<Vec<crate::telemetry_store::LocalMemory>, String> {
    telemetry.search_local_l1_memories(&query, limit.unwrap_or(50))
}

#[tauri::command]
pub fn local_memory_update(
    telemetry: tauri::State<'_, crate::telemetry_store::TelemetryStore>,
    id: String,
    content: String,
) -> Result<(), String> {
    telemetry.update_local_l1_memory(&id, &content)
}

#[tauri::command]
pub fn local_memory_delete(
    telemetry: tauri::State<'_, crate::telemetry_store::TelemetryStore>,
    id: String,
) -> Result<(), String> {
    telemetry.delete_local_l1_memory(&id)
}

/// 用户手动添加自定义长期记忆：固定 long_term 并标记 user_defined，
/// 自动整理与 L1 重建都会跳过它，只有用户能在面板删改；scope 指定
/// 归属层级（l2 工作记忆 / l3 长期 Profile，默认 l3），两层相互独立。
#[tauri::command]
pub fn local_memory_add_user(
    telemetry: tauri::State<'_, crate::telemetry_store::TelemetryStore>,
    content: String,
    memory_type: Option<String>,
    scope: Option<String>,
) -> Result<crate::telemetry_store::LocalMemory, String> {
    telemetry.add_user_defined_l1_memory(
        &content,
        memory_type.as_deref().unwrap_or("fact"),
        scope.as_deref().unwrap_or("l3"),
    )
}

// ────────────────────────────────────────────────────────────────────────────
// 节流巡检：定期冲刷静默超时的会话（由 lib.rs 启动的定时任务调用）
// ────────────────────────────────────────────────────────────────────────────
pub fn start_idle_flusher(backend: Arc<MemoryBackend>, store: IngestStore) {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(Duration::from_secs(30)).await;
            let idle = Duration::from_secs(store.idle_timeout_secs);
            let stale: Vec<String> = {
                let inner = store.inner.lock().unwrap();
                inner
                    .sessions
                    .iter()
                    .filter(|(_, b)| b.last_active.elapsed() > idle)
                    .map(|(k, _)| k.clone())
                    .collect()
            };
            for sid in stale {
                store.flush_session(Arc::clone(&backend), &sid);
            }
        }
    });
}

#[cfg(test)]
mod tests {
    use super::{
        filter_l3_profile, has_l2_final_heading, is_l2_analysis_preamble, is_usable_l3_profile,
        normalize_l2_document, output_language_directive, parse_typed_l1_candidates,
        read_claude_session, read_claude_turn, read_codex_session, read_codex_turn,
        read_qoder_session, read_workbuddy_session, select_l2_evidence, strip_memory_thinking,
        unwrap_markdown_document_fence,
    };

    #[test]
    fn assistant_thinking_is_removed_before_l0_and_l1_processing() {
        assert_eq!(
            strip_memory_thinking("assistant", "<think>内部推理</think>最终结论"),
            "最终结论"
        );
        assert_eq!(
            strip_memory_thinking("user", "<think>这是用户原文</think>"),
            "<think>这是用户原文</think>"
        );
    }

    #[test]
    fn typed_l1_parser_preserves_the_model_type() {
        let memories = parse_typed_l1_candidates(
            r#"{"memories":[{"content":"会话摘要","type":"summary","durability":"session"},{"content":"用户要求中文输出","type":"preference_candidate","durability":"long_term"}]}"#,
        ).unwrap();
        assert_eq!(memories.len(), 2);
        assert_eq!(memories[1].memory_type, "preference_candidate");
        assert_eq!(memories[1].durability, "long_term");
    }

    #[test]
    fn typed_l1_parser_rejects_missing_or_undetermined_durability() {
        assert!(parse_typed_l1_candidates(
            r#"{"memories":[{"content":"会话摘要","type":"summary"}]}"#,
        )
        .is_err());
        assert!(parse_typed_l1_candidates(
            r#"{"memories":[{"content":"会话摘要","type":"summary","durability":"undetermined"}]}"#,
        )
        .is_err());
    }

    #[test]
    fn l2_selection_retains_critical_evidence_before_session_cap() {
        let row = |id: &str, memory_type: &str, session_id: &str| {
            crate::telemetry_store::LocalMemorySnapshot {
                id: id.into(),
                source_event_key: "event".into(),
                ordinal: 0,
                memory: id.into(),
                memory_type: memory_type.into(),
                durability: "short_term".into(),
                source: "codex".into(),
                session_id: session_id.into(),
                user_defined: false,
                event_time: Some("2026-08-16T00:00:00Z".into()),
                created_at: "2026-08-16T00:00:00Z".into(),
                updated_at: "2026-08-16T00:00:00Z".into(),
            }
        };
        let output = select_l2_evidence(vec![
            row("a", "fact", "s1"),
            row("b", "fact", "s1"),
            row("c", "fact", "s1"),
            row("d", "constraint", "s1"),
        ]);
        assert!(output.iter().any(|item| item.id == "d"));
        assert!(output.len() <= 3);
    }

    #[test]
    fn l2_rejects_task_narration_but_keeps_final_memory_markdown() {
        assert!(is_l2_analysis_preamble("The user wants me to create an L2 short-term working memory from the provided map summaries."));
        assert!(is_l2_analysis_preamble("Let me analyze the three sources:"));
        assert!(!is_l2_analysis_preamble(
            "## Current focus\n\n- 完成 L2 记忆整理 [l1:123]"
        ));
    }

    #[test]
    fn l2_discards_untagged_reasoning_before_the_final_heading() {
        let model_output = "The user wants me to create an L2 memory. Let me analyze the sources.\n\n## Current focus\nThis is only the planned outline.\n\nLet me draft it now.\n\n## 当前聚焦\n- 修复记忆提取 [l1:1]";
        let document = normalize_l2_document(model_output);
        assert_eq!(document, "## 当前聚焦\n- 修复记忆提取 [l1:1]");
        assert!(has_l2_final_heading(&document));
        assert!(!is_l2_analysis_preamble(&document));
    }

    #[test]
    fn unwraps_a_fence_that_wraps_an_entire_memory_document() {
        assert_eq!(
            unwrap_markdown_document_fence("```markdown\n# Profile\n\n- 使用 SQLite\n```"),
            "# Profile\n\n- 使用 SQLite",
        );
    }

    #[test]
    fn l3_rejects_truncated_or_wrong_language_profiles() {
        let chinese_evidence = "用户偏好中文，长期使用 SQLite";
        assert!(!is_usable_l3_profile(
            "## Collaboration\n- Use SQLite\n[内容已截断]",
            chinese_evidence
        ));
        assert!(!is_usable_l3_profile(
            "## Collaboration\n- Use SQLite as the durable ledger.",
            chinese_evidence
        ));
        assert!(is_usable_l3_profile(
            "## 协作偏好\n- 默认使用中文沟通。",
            chinese_evidence
        ));
        assert!(!is_usable_l3_profile(
            "- 默认使用中文沟通。",
            chinese_evidence
        ));
    }

    #[test]
    fn l3_accepts_a_complete_final_body_within_its_persisted_limit() {
        let chinese_evidence = "用户偏好中文，长期使用 SQLite";
        let complete_profile = format!("## 协作偏好\n- {}", "持久偏好。".repeat(900));
        assert!(is_usable_l3_profile(&complete_profile, chinese_evidence));
    }

    #[test]
    fn l3_removes_volatile_project_details_before_persistence() {
        let profile = "## 模型与端点\n- MiniMax 默认端点 api.minimaxi.com。\n## 长期约束\n- 助手只能通过本地项目和数据库进行诊断。\n## 代码约定\n- i18n 键必须同时维护。";
        assert_eq!(
            filter_l3_profile(profile),
            "## 长期约束\n- 助手只能通过本地项目和数据库进行诊断。"
        );
    }

    #[test]
    fn memory_output_language_follows_the_captured_conversation() {
        assert!(output_language_directive("请把记忆整理成中文").contains("中文"));
        assert!(
            output_language_directive("Please retain this project decision").contains("English")
        );
        assert!(output_language_directive("用户偏好中文并使用 SQLite MCP Server").contains("中文"));
    }

    #[test]
    fn claude_transcript_reader_uses_one_prompt_turn_and_skips_tool_result_rows() {
        let path = std::env::temp_dir().join(format!(
            "agent-manager-transcript-{}.jsonl",
            uuid::Uuid::new_v4()
        ));
        let rows = [
            serde_json::json!({"type":"user", "promptId":"p-1", "message":{"role":"user", "content":"remember this project decision"}}),
            serde_json::json!({"type":"assistant", "message":{"role":"assistant", "content":[{"type":"text", "text":"I will use SQLite."}]}}),
            serde_json::json!({"type":"user", "sourceToolAssistantUUID":"tool-1", "message":{"role":"user", "content":[{"type":"tool_result", "content":"large output"}]}}),
            serde_json::json!({"type":"assistant", "message":{"role":"assistant", "content":[{"type":"text", "text":"The migration is complete."}]}}),
            serde_json::json!({"type":"user", "promptId":"p-2", "message":{"role":"user", "content":"a different turn"}}),
        ];
        std::fs::write(
            &path,
            rows.iter()
                .map(serde_json::Value::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        assert_eq!(
            read_claude_turn(&path, "p-1").unwrap(),
            vec![
                ("user".into(), "remember this project decision".into()),
                ("assistant".into(), "I will use SQLite.".into()),
                ("assistant".into(), "The migration is complete.".into()),
            ]
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn codex_transcript_reader_uses_only_the_hook_turn_dialogue() {
        let path = std::env::temp_dir().join(format!(
            "agent-manager-codex-transcript-{}.jsonl",
            uuid::Uuid::new_v4()
        ));
        let rows = [
            serde_json::json!({"type":"event_msg", "payload":{"type":"task_started", "turn_id":"turn-1"}}),
            serde_json::json!({"type":"response_item", "payload":{"type":"message", "role":"user", "content":[{"type":"input_text", "text":"Persist this decision"}], "internal_chat_message_metadata_passthrough":{"turn_id":"turn-1"}}}),
            serde_json::json!({"type":"response_item", "payload":{"type":"function_call", "name":"shell", "arguments":"secret tool details", "internal_chat_message_metadata_passthrough":{"turn_id":"turn-1"}}}),
            serde_json::json!({"type":"response_item", "payload":{"type":"message", "role":"assistant", "content":[{"type":"output_text", "text":"I will use the shared skill library."}], "internal_chat_message_metadata_passthrough":{"turn_id":"turn-1"}}}),
            serde_json::json!({"type":"response_item", "payload":{"type":"message", "role":"user", "content":[{"type":"input_text", "text":"Different request"}], "internal_chat_message_metadata_passthrough":{"turn_id":"turn-2"}}}),
        ];
        std::fs::write(
            &path,
            rows.iter()
                .map(serde_json::Value::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        assert_eq!(
            read_codex_turn(&path, "turn-1").unwrap(),
            vec![
                ("user".into(), "Persist this decision".into()),
                (
                    "assistant".into(),
                    "I will use the shared skill library.".into()
                ),
            ]
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn qoder_native_reader_keeps_dialogue_and_skips_non_dialogue_rows() {
        let path = std::env::temp_dir().join(format!(
            "agent-manager-qoder-transcript-{}.jsonl",
            uuid::Uuid::new_v4()
        ));
        let rows = [
            serde_json::json!({"type":"user", "uuid":"u-1", "message":{"content":[{"content":"记住本项目使用 SQLite"}]}}),
            serde_json::json!({"type":"tool", "uuid":"tool-1", "message":{"content":"secret tool payload"}}),
            serde_json::json!({"type":"assistant", "uuid":"a-1", "message":{"content":[{"content":"已记录并会写入本地账本。"}]}}),
            serde_json::json!({"type":"assistant", "uuid":"a-1", "message":{"content":[{"content":"重复行"}]}}),
        ];
        std::fs::write(
            &path,
            rows.iter()
                .map(serde_json::Value::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        assert_eq!(
            read_qoder_session(&path).unwrap(),
            vec![
                ("user".into(), "记住本项目使用 SQLite".into()),
                ("assistant".into(), "已记录并会写入本地账本。".into()),
            ]
        );
        let _ = std::fs::remove_file(path);
    }

    #[test]
    fn native_session_readers_extract_only_dialogue_across_agent_formats() {
        let claude = std::env::temp_dir().join(format!(
            "agent-manager-claude-session-{}.jsonl",
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&claude, [
            serde_json::json!({"type":"user","uuid":"u-1","message":{"role":"user","content":"保留这个决定"}}).to_string(),
            serde_json::json!({"type":"assistant","uuid":"a-1","message":{"role":"assistant","content":[{"type":"text","text":"使用 SQLite"}]}}).to_string(),
            serde_json::json!({"type":"user","sourceToolAssistantUUID":"tool-1","message":{"role":"user","content":"工具回显"}}).to_string(),
        ].join("\n")).unwrap();
        assert_eq!(read_claude_session(&claude).unwrap().len(), 2);

        let codex = std::env::temp_dir().join(format!(
            "agent-manager-codex-session-{}.jsonl",
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&codex, [
            serde_json::json!({"type":"response_item","payload":{"type":"message","id":"u-1","role":"user","content":[{"type":"input_text","text":"记住配置"}]}}).to_string(),
            serde_json::json!({"type":"response_item","payload":{"type":"function_call","id":"tool-1","arguments":"secret"}}).to_string(),
            serde_json::json!({"type":"response_item","payload":{"type":"message","id":"a-1","role":"assistant","content":[{"type":"output_text","text":"已完成"}]}}).to_string(),
        ].join("\n")).unwrap();
        assert_eq!(read_codex_session(&codex).unwrap().len(), 2);

        let workbuddy = std::env::temp_dir().join(format!(
            "agent-manager-workbuddy-session-{}.jsonl",
            uuid::Uuid::new_v4()
        ));
        std::fs::write(&workbuddy, [
            serde_json::json!({"id":"u-1","type":"user","message":{"content":"讨论需求"}}).to_string(),
            serde_json::json!({"id":"tool-1","type":"tool","message":{"content":"工具数据"}}).to_string(),
            serde_json::json!({"id":"a-1","type":"assistant","message":{"content":[{"text":"给出方案"}]}}).to_string(),
        ].join("\n")).unwrap();
        assert_eq!(read_workbuddy_session(&workbuddy).unwrap().len(), 2);

        for path in [claude, codex, workbuddy] {
            let _ = std::fs::remove_file(path);
        }
    }

    #[test]
    fn qoder_reader_drops_tool_calls_results_and_thinking_blocks() {
        let path = std::env::temp_dir().join(format!(
            "agent-manager-qoder-clean-{}.jsonl",
            uuid::Uuid::new_v4()
        ));
        let rows = [
            // 真实用户消息：纯字符串 content。
            serde_json::json!({"type":"user", "uuid":"u-1", "message":{"role":"user", "content":"记忆中心只保留对话"}}),
            // 工具调用：type=tool_use，携带完整参数 JSON。
            serde_json::json!({"type":"assistant", "uuid":"a-1", "message":{"role":"assistant", "content":[{"type":"tool_use", "id":"call_1", "name":"Bash", "input":{"command":"secret command"}}]}}),
            // 助手思考块。
            serde_json::json!({"type":"assistant", "uuid":"a-2", "message":{"role":"assistant", "content":[{"type":"thinking", "thinking":"secret reasoning"}]}}),
            // 助手正文。
            serde_json::json!({"type":"assistant", "uuid":"a-3", "message":{"role":"assistant", "content":[{"type":"text", "text":"好的，只保留用户与回复。"}]}}),
            // 工具结果：伪装成 user，带 tool_use_id / type=tool_result。
            serde_json::json!({"type":"user", "uuid":"u-2", "message":{"role":"user", "content":[{"type":"tool_result", "tool_use_id":"call_1", "is_error":false, "content":"Command completed. secret output"}]}}),
        ];
        std::fs::write(
            &path,
            rows.iter()
                .map(serde_json::Value::to_string)
                .collect::<Vec<_>>()
                .join("\n"),
        )
        .unwrap();
        assert_eq!(
            read_qoder_session(&path).unwrap(),
            vec![
                ("user".into(), "记忆中心只保留对话".into()),
                ("assistant".into(), "好的，只保留用户与回复。".into()),
            ]
        );
        let _ = std::fs::remove_file(path);
    }
}
