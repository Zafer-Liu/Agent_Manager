use serde::{Deserialize, Serialize};
use std::collections::HashMap;

use aes_gcm::aead::{Aead, Key, KeyInit};
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

/// 使用 AES-256-GCM 加密 API Key，返回 hex 编码密文
fn encrypt_api_key(plain: &str) -> String {
    if plain.is_empty() {
        return String::new();
    }
    let key = derive_key();
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let nonce = Nonce::from_slice(b"agent-mgr-nonce-01");
    let ciphertext = cipher.encrypt(nonce, plain.as_bytes()).unwrap_or_default();
    hex_encode(&ciphertext)
}

/// 使用 AES-256-GCM 解密 API Key。解密失败或非 hex 格式时返回原文（向后兼容旧版明文）
fn decrypt_api_key(cipher_text: &str) -> String {
    if cipher_text.is_empty() {
        return String::new();
    }
    let ciphertext = match hex_decode(cipher_text) {
        Some(data) if !data.is_empty() => data,
        _ => return cipher_text.to_string(), // 不是 hex 格式，可能是旧版明文，直接返回
    };
    let key = derive_key();
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
    let nonce = Nonce::from_slice(b"agent-mgr-nonce-01");
    cipher
        .decrypt(nonce, ciphertext.as_ref())
        .ok()
        .and_then(|bytes| String::from_utf8(bytes).ok())
        .unwrap_or_else(|| cipher_text.to_string()) // 解密失败，返回原文（向后兼容）
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

fn load_providers() -> Vec<LlmProvider> {
    let path = config_path();
    let mut providers: Vec<LlmProvider> = std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default();
    // 解密所有 api_key（向后兼容旧版明文）
    for provider in &mut providers {
        provider.api_key = decrypt_api_key(&provider.api_key);
    }
    providers
}

fn save_providers(providers: &[LlmProvider]) {
    let path = config_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    // 加密所有 api_key 后再序列化
    let encrypted: Vec<LlmProvider> = providers
        .iter()
        .map(|p| {
            let mut cloned = p.clone();
            cloned.api_key = encrypt_api_key(&p.api_key);
            cloned
        })
        .collect();
    if let Ok(json) = serde_json::to_string_pretty(&encrypted) {
        let _ = std::fs::write(path, json);
    }
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
    save_providers(&providers);
    Ok(())
}

#[tauri::command]
pub fn delete_llm_provider(id: String) -> Result<(), String> {
    let defaults = builtin_defaults();
    if defaults.contains_key(id.as_str()) {
        return Err("Cannot delete built-in provider".to_string());
    }
    let mut providers = load_providers();
    providers.retain(|p| p.id != id);
    save_providers(&providers);
    Ok(())
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

    let body = serde_json::json!({
        "model": provider.model,
        "messages": [{"role": "user", "content": "Hi"}],
        "max_tokens": 5
    });

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
