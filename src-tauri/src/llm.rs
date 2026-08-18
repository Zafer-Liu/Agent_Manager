use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use crate::thinking::strip_thinking_blocks;
use aes_gcm::aead::{Aead, AeadCore, Key, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};
use sha2::{Digest, Sha256};

/// 基于应用标识 + 机器名派生固定 32 字节 AES-256 密钥
fn derive_key() -> [u8; 32] {
    let machine_id = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "agent-manager".to_string());
    let mut hasher = Sha256::new();
    hasher.update(b"agent-manager-key-v1");
    hasher.update(machine_id.as_bytes());
    let result = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&result);
    key
}

/// 将字节数组编码为十六进制字符串
fn hex_encode(data: &[u8]) -> String {
    data.iter().map(|b| format!("{:02x}", b)).collect()
}

/// 将十六进制字符串解码为字节数组，失败返回 None
fn hex_decode(s: &str) -> Option<Vec<u8>> {
    if s.len() % 2 != 0 {
        return None;
    }
    (0..s.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&s[i..i + 2], 16).ok())
        .collect()
}

const ENCRYPTED_KEY_PREFIX: &str = "v2:";
const GCM_NONCE_BYTES: usize = 12;
const LLM_PROVIDERS_SETTING_KEY: &str = "llm_providers";
const MEMORY_EXTRACTION_SETTING_KEY: &str = "memory_extraction_config";

/// Encrypt a key with a fresh GCM nonce.  The nonce is stored alongside the
/// ciphertext so each saved provider is independently decryptable.
fn encrypt_api_key(plain: &str) -> Result<String, String> {
    if plain.is_empty() {
        return Ok(String::new());
    }
    let key = derive_key();
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let ciphertext = cipher
        .encrypt(&nonce, plain.as_bytes())
        .map_err(|_| "无法加密 API Key".to_string())?;
    Ok(format!(
        "{ENCRYPTED_KEY_PREFIX}{}:{}",
        hex_encode(nonce.as_slice()),
        hex_encode(&ciphertext)
    ))
}

/// Decrypt v2 keys.  Legacy values remain plaintext-compatible; malformed v2
/// data becomes empty rather than being exposed as a pretend API key.
fn decrypt_api_key(cipher_text: &str) -> String {
    if cipher_text.is_empty() {
        return String::new();
    }
    let Some(payload) = cipher_text.strip_prefix(ENCRYPTED_KEY_PREFIX) else {
        return cipher_text.to_string();
    };
    let Some((nonce_hex, cipher_hex)) = payload.split_once(':') else {
        return String::new();
    };
    let Some(nonce_bytes) = hex_decode(nonce_hex).filter(|bytes| bytes.len() == GCM_NONCE_BYTES)
    else {
        return String::new();
    };
    let Some(ciphertext) = hex_decode(cipher_hex).filter(|bytes| !bytes.is_empty()) else {
        return String::new();
    };
    let key = derive_key();
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let nonce = Nonce::from_slice(&nonce_bytes);
    cipher
        .decrypt(nonce, ciphertext.as_ref())
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_default()
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LlmProvider {
    pub id: String,
    pub name: String,
    pub base_url: String,
    pub model: String,
    pub api_key: String,
    pub is_custom: bool,
    pub enabled: bool,
    pub context_window: Option<u32>,
    pub max_output_tokens: Option<u32>,
}

/// The one configured provider used to turn a completed Agent conversation
/// into durable memory candidates.  Credentials stay with the provider record;
/// this config only stores its stable id.
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub struct MemoryExtractionConfig {
    pub provider_id: Option<String>,
}

// 内置 provider 默认值
pub fn builtin_defaults(
) -> HashMap<&'static str, (&'static str, &'static str, &'static str, u32, u32)> {
    // id -> (name, base_url, model, context_window, max_output_tokens)
    let mut m = HashMap::new();
    m.insert(
        "deepseek",
        (
            "DeepSeek",
            "https://api.deepseek.com",
            "deepseek-chat",
            64000,
            8192,
        ),
    );
    m.insert(
        "openai",
        (
            "OpenAI",
            "https://api.openai.com/v1",
            "gpt-4o-mini",
            128000,
            16384,
        ),
    );
    m
}

fn config_path() -> std::path::PathBuf {
    dirs_next::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("agent-manager")
        .join("llm_config.json")
}

fn memory_extraction_config_path() -> std::path::PathBuf {
    dirs_next::data_dir()
        .unwrap_or_else(|| std::path::PathBuf::from("."))
        .join("agent-manager")
        .join("memory_extraction.json")
}

pub fn memory_extraction_config() -> MemoryExtractionConfig {
    if let Some(store) = crate::telemetry_store::shared_store() {
        if let Some(config) = store.app_setting_get(MEMORY_EXTRACTION_SETTING_KEY) {
            return config;
        }
    }
    let config = std::fs::read_to_string(memory_extraction_config_path())
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_default();
    if let Some(store) = crate::telemetry_store::shared_store() {
        let _ = store.app_setting_set(MEMORY_EXTRACTION_SETTING_KEY, &config);
    }
    config
}

fn save_memory_extraction_config(config: &MemoryExtractionConfig) -> Result<(), String> {
    if let Some(store) = crate::telemetry_store::shared_store() {
        store.app_setting_set(MEMORY_EXTRACTION_SETTING_KEY, config)?;
    }
    // Compatibility mirror for older versions; SQLite is the authoritative
    // location once it has been initialized.
    let path = memory_extraction_config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    let text = serde_json::to_string_pretty(config).map_err(|error| error.to_string())?;
    std::fs::write(path, text).map_err(|error| error.to_string())
}

fn load_providers() -> Vec<LlmProvider> {
    let from_primary: Option<Vec<LlmProvider>> = crate::telemetry_store::shared_store()
        .and_then(|store| store.app_setting_get(LLM_PROVIDERS_SETTING_KEY));
    let migrated_from_legacy = from_primary.is_none();
    let mut providers: Vec<LlmProvider> = from_primary.unwrap_or_else(|| {
        let path = config_path();
        std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
            .unwrap_or_default()
    });
    if crate::telemetry_store::shared_store().is_some() && migrated_from_legacy {
        if let Some(store) = crate::telemetry_store::shared_store() {
            let _ = store.app_setting_set(LLM_PROVIDERS_SETTING_KEY, &providers);
        }
    }
    // 解密所有 api_key（向后兼容旧版明文）
    for provider in &mut providers {
        provider.api_key = decrypt_api_key(&provider.api_key);
    }
    providers
}

fn save_providers(providers: &[LlmProvider]) -> Result<(), String> {
    let path = config_path();
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    // 加密所有 api_key 后再序列化
    let encrypted: Vec<LlmProvider> = providers
        .iter()
        .map(|p| -> Result<LlmProvider, String> {
            let mut cloned = p.clone();
            cloned.api_key = encrypt_api_key(&p.api_key)?;
            Ok(cloned)
        })
        .collect::<Result<Vec<_>, _>>()?;
    let json = serde_json::to_string_pretty(&encrypted).map_err(|error| error.to_string())?;
    if let Some(store) = crate::telemetry_store::shared_store() {
        store.app_setting_set(LLM_PROVIDERS_SETTING_KEY, &encrypted)?;
    }
    // Mirror is only for downgrade compatibility; the primary database is
    // used by current releases.
    std::fs::write(path, json).map_err(|error| error.to_string())
}

#[tauri::command]
pub fn list_llm_providers() -> Vec<LlmProvider> {
    let mut providers = load_providers();
    let defaults = builtin_defaults();

    // 确保内置 provider 始终存在（api_key 可为空）
    for (id, (name, base_url, model, ctx, max_out)) in &defaults {
        if !providers.iter().any(|p| p.id == *id) {
            providers.insert(
                0,
                LlmProvider {
                    id: id.to_string(),
                    name: name.to_string(),
                    base_url: base_url.to_string(),
                    model: model.to_string(),
                    api_key: String::new(),
                    is_custom: false,
                    enabled: false,
                    context_window: Some(*ctx),
                    max_output_tokens: Some(*max_out),
                },
            );
        }
    }
    providers
}

#[tauri::command]
pub fn memory_extraction_config_get() -> MemoryExtractionConfig {
    memory_extraction_config()
}

#[tauri::command]
pub fn memory_extraction_config_set(config: MemoryExtractionConfig) -> Result<(), String> {
    if let Some(provider_id) = config.provider_id.as_deref() {
        let provider = list_llm_providers()
            .into_iter()
            .find(|candidate| candidate.id == provider_id)
            .ok_or("所选记忆提取模型不存在")?;
        if !provider.enabled || provider.api_key.trim().is_empty() {
            return Err("所选记忆提取模型尚未启用或未配置 API Key".into());
        }
    }
    save_memory_extraction_config(&config)
}

/// Resolve the configured provider inside the native process.  Hook requests
/// never receive credentials from the webview.
pub fn memory_extraction_provider() -> Result<LlmProvider, String> {
    let config = memory_extraction_config();
    let provider_id = config.provider_id.ok_or("请先在设置中选择记忆提取模型")?;
    let provider = list_llm_providers()
        .into_iter()
        .find(|candidate| candidate.id == provider_id)
        .ok_or("记忆提取模型不存在，请在设置中重新选择")?;
    if !provider.enabled || provider.api_key.trim().is_empty() {
        return Err("记忆提取模型未启用或缺少 API Key".into());
    }
    Ok(provider)
}

/// Call an OpenAI-compatible provider for a text-only completion.
pub async fn complete_text(
    provider: &LlmProvider,
    messages: &[serde_json::Value],
) -> Result<String, String> {
    complete_text_with_limit(provider, messages, None).await
}

/// A bounded variant for narrow, structured work such as duplicate-memory
/// review.  Giving a reasoning model its entire general-purpose budget for a
/// tiny JSON decision can lead to a very long hidden-reasoning pass and a
/// request timeout, so callers may set a smaller ceiling deliberately.
pub async fn complete_text_with_limit(
    provider: &LlmProvider,
    messages: &[serde_json::Value],
    output_limit: Option<u32>,
) -> Result<String, String> {
    let is_minimax = provider.base_url.to_ascii_lowercase().contains("minimax")
        || provider.model.to_ascii_lowercase().contains("minimax");
    // Reasoning models consume their output budget for both thoughts and the
    // final answer.  Keep the configured budget, but ensure an older custom
    // MiniMax record without a value still has enough room for JSON tasks.
    let configured_limit =
        provider
            .max_output_tokens
            .unwrap_or(if is_minimax { 16_384 } else { 1024 });
    let max_output_tokens = output_limit
        .map(|limit| configured_limit.min(limit))
        .unwrap_or(configured_limit);
    let mut body = serde_json::json!({
        "model": provider.model,
        "messages": messages,
        "temperature": 0.1,
        "max_tokens": max_output_tokens,
    });
    if is_minimax {
        body["max_completion_tokens"] = serde_json::json!(max_output_tokens);
        // MiniMax's OpenAI-compatible interface can return thought text in
        // `content` unless this flag is set.  Splitting it keeps the JSON/text
        // answer intact and prevents our safety filter from discarding it.
        body["reasoning_split"] = serde_json::json!(true);
    }
    let client = reqwest::Client::builder()
        .connect_timeout(std::time::Duration::from_secs(20))
        .pool_max_idle_per_host(0)
        .build()
        .map_err(|error| format!("无法创建记忆提取模型连接: {error}"))?;
    let configured_url = format!(
        "{}/chat/completions",
        provider.base_url.trim_end_matches('/')
    );
    // Both official MiniMax compatibility gateways are valid.  Network paths
    // can intermittently prefer one DNS/CDN route over the other, so a
    // connection failure should transparently try the sibling endpoint before
    // surfacing an error to the user.  API/validation errors are not retried.
    let mut urls = vec![configured_url];
    if is_minimax {
        let alternate_base = if provider
            .base_url
            .to_ascii_lowercase()
            .contains("minimaxi.com")
        {
            "https://api.minimax.io/v1"
        } else if provider
            .base_url
            .to_ascii_lowercase()
            .contains("minimax.io")
        {
            "https://api.minimaxi.com/v1"
        } else {
            "https://api.minimax.io/v1"
        };
        let alternate = format!("{alternate_base}/chat/completions");
        if !urls.contains(&alternate) {
            urls.push(alternate);
        }
    }
    let mut connection_errors = Vec::new();
    let mut response = None;
    for url in urls {
        match client
            .post(&url)
            .header("Authorization", format!("Bearer {}", provider.api_key))
            .header("Content-Type", "application/json")
            .json(&body)
            .timeout(std::time::Duration::from_secs(120))
            .send()
            .await
        {
            Ok(value) => {
                response = Some(value);
                break;
            }
            Err(error) if error.is_connect() || error.is_timeout() => {
                connection_errors.push(format!("{url}: {}", explain_request_error(&error)));
                tokio::time::sleep(std::time::Duration::from_millis(400)).await;
            }
            Err(error) => {
                return Err(format!(
                    "记忆提取模型请求失败（{}）: {}",
                    provider.name,
                    explain_request_error(&error)
                ))
            }
        }
    }
    let response = response.ok_or_else(|| {
        format!(
            "记忆提取模型请求失败（{}）：两个 MiniMax 官方端点均无法连接。{}",
            provider.name,
            connection_errors.join("；"),
        )
    })?;
    let status = response.status();
    let value = response
        .json::<serde_json::Value>()
        .await
        .map_err(|error| error.to_string())?;
    if !status.is_success() {
        let message = value
            .pointer("/error/message")
            .and_then(serde_json::Value::as_str)
            .unwrap_or("unknown LLM error");
        return Err(format!("记忆提取模型返回 HTTP {status}: {message}"));
    }
    let finish_reason = value
        .pointer("/choices/0/finish_reason")
        .and_then(serde_json::Value::as_str)
        .unwrap_or("unknown");
    let content = value
        .pointer("/choices/0/message/content")
        .and_then(serde_json::Value::as_str)
        .map(str::to_string)
        .filter(|content| !content.trim().is_empty())
        // MiniMax reasoning models usually send their chain-of-thought in
        // `reasoning_content`; it must never be treated as the final answer.
        .ok_or_else(|| {
            if is_minimax {
                format!("MiniMax 未返回最终文本（结束原因：{finish_reason}）。已启用 reasoning_split；请检查模型名称与服务端额度后重试")
            } else {
                "记忆提取模型未返回文本内容".to_string()
            }
        })?;
    let answer = strip_thinking_blocks(&content);
    (!answer.trim().is_empty()).then_some(answer).ok_or_else(|| {
        if is_minimax {
            format!("MiniMax 最终文本在思考过滤后为空（结束原因：{finish_reason}）。请提高最大输出 Token 或稍后重试")
        } else {
            "记忆提取模型仅返回了思考内容，未返回最终文本".to_string()
        }
    })
}

fn explain_request_error(error: &reqwest::Error) -> String {
    if error.is_timeout() {
        "请求超时，请稍后重试".into()
    } else if error.is_connect() {
        format!("无法建立 HTTPS 连接（{}）", error)
    } else if error.is_request() {
        format!("请求发送失败（{}）", error)
    } else {
        error.to_string()
    }
}

/// Some OpenAI-compatible gateways flatten a reasoning model's thought text
/// into `message.content` instead of exposing `reasoning_content`.  Strip
/// only explicit protocol tags, leaving ordinary XML/Markdown untouched.
#[tauri::command]
pub fn save_llm_provider(provider: LlmProvider) -> Result<(), String> {
    let mut providers = load_providers();
    // 确保内置 provider 的初始记录存在
    let defaults = builtin_defaults();
    for (id, (name, base_url, model, ctx, max_out)) in &defaults {
        if !providers.iter().any(|p| p.id == *id) {
            providers.push(LlmProvider {
                id: id.to_string(),
                name: name.to_string(),
                base_url: base_url.to_string(),
                model: model.to_string(),
                api_key: String::new(),
                is_custom: false,
                enabled: false,
                context_window: Some(*ctx),
                max_output_tokens: Some(*max_out),
            });
        }
    }

    if let Some(existing) = providers.iter_mut().find(|p| p.id == provider.id) {
        *existing = provider;
    } else {
        providers.push(provider);
    }
    save_providers(&providers)
}

#[tauri::command]
pub fn delete_llm_provider(id: String) -> Result<(), String> {
    let defaults = builtin_defaults();
    if defaults.contains_key(id.as_str()) {
        return Err("Cannot delete built-in provider".to_string());
    }
    let mut providers = load_providers();
    providers.retain(|p| p.id != id);
    save_providers(&providers)
}

#[tauri::command]
pub async fn test_llm_provider(provider: LlmProvider) -> Result<String, String> {
    if provider.api_key.is_empty() {
        return Err("API key is empty".to_string());
    }

    let client = reqwest::Client::new();
    let url = format!(
        "{}/chat/completions",
        provider.base_url.trim_end_matches('/')
    );

    let is_minimax = provider.base_url.to_ascii_lowercase().contains("minimax")
        || provider.model.to_ascii_lowercase().contains("minimax");
    let mut body = serde_json::json!({
        "model": provider.model,
        "messages": [{"role": "user", "content": "Hi"}],
        "max_tokens": if is_minimax { 256 } else { 5 },
    });
    if is_minimax {
        body["max_completion_tokens"] = serde_json::json!(256);
        body["reasoning_split"] = serde_json::json!(true);
    }

    let resp = client
        .post(&url)
        .header("Authorization", format!("Bearer {}", provider.api_key))
        .header("Content-Type", "application/json")
        .json(&body)
        .timeout(std::time::Duration::from_secs(15))
        .send()
        .await
        .map_err(|e| format!("Request failed: {}", e))?;

    let status = resp.status();
    if status.is_success() {
        Ok(format!(
            "✓ Connected — {} ({})",
            provider.name, provider.model
        ))
    } else {
        let text = resp.text().await.unwrap_or_default();
        let msg = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v["error"]["message"].as_str().map(|s| s.to_string()))
            .unwrap_or(text);
        Err(format!("HTTP {}: {}", status, msg))
    }
}

#[cfg(test)]
mod tests {
    use super::{decrypt_api_key, encrypt_api_key, ENCRYPTED_KEY_PREFIX};
    use crate::thinking::strip_thinking_blocks;

    #[test]
    fn api_key_round_trips_with_a_valid_gcm_nonce() {
        let encrypted = encrypt_api_key("sk-test-secret").expect("encrypt API key");
        assert!(encrypted.starts_with(ENCRYPTED_KEY_PREFIX));
        assert_ne!(encrypted, "sk-test-secret");
        assert_eq!(decrypt_api_key(&encrypted), "sk-test-secret");
    }

    #[test]
    fn malformed_v2_key_does_not_panic_or_echo_ciphertext() {
        assert_eq!(decrypt_api_key("v2:0011:ffee"), "");
    }

    #[test]
    fn minimax_thinking_tags_are_removed_before_parsing_the_answer() {
        assert_eq!(
            strip_thinking_blocks("<think>分析 JSON</think>\n{\"actions\":[]}"),
            "{\"actions\":[]}"
        );
        assert_eq!(
            strip_thinking_blocks("<THINKING>reasoning</THINKING>final"),
            "final"
        );
    }
}
