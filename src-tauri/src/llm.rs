use serde::{Deserialize, Serialize};
use std::collections::HashMap;

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
pub fn builtin_defaults() -> HashMap<&'static str, (&'static str, &'static str, &'static str, u32, u32)> {
    // id -> (name, base_url, model, context_window, max_output_tokens)
    let mut m = HashMap::new();
    m.insert("deepseek",  ("DeepSeek",  "https://api.deepseek.com",    "deepseek-chat",  64000,  8192));
    m.insert("openai",    ("OpenAI",    "https://api.openai.com/v1",    "gpt-4o-mini",   128000, 16384));
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
    std::fs::read_to_string(&path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

fn save_providers(providers: &[LlmProvider]) {
    let path = config_path();
    if let Some(parent) = path.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    if let Ok(json) = serde_json::to_string_pretty(providers) {
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
            providers.insert(0, LlmProvider {
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
    let url = format!("{}/chat/completions", provider.base_url.trim_end_matches('/'));

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
        Ok(format!("✓ Connected — {} ({})", provider.name, provider.model))
    } else {
        let text = resp.text().await.unwrap_or_default();
        let msg = serde_json::from_str::<serde_json::Value>(&text)
            .ok()
            .and_then(|v| v["error"]["message"].as_str().map(|s| s.to_string()))
            .unwrap_or(text);
        Err(format!("HTTP {}: {}", status, msg))
    }
}
