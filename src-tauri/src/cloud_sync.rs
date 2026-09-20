//! 云端记忆库（Cloud Memory Vault）Phase 1 客户端同步引擎。
//! 设计见 docs/16-cloud-memory-vault-design.md：本地 SQLite 权威、
//! 端到端 AES-256-GCM 加密、CAS 修订号仲裁、`updated_at` 较新者胜出。

use aes_gcm::aead::{Aead, AeadCore, Key, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tauri::State;

use crate::telemetry_store::{TelemetryStore, UsageDayAggregate};

const SETTING_URL: &str = "cloud_vault.url";
const SETTING_PAT: &str = "cloud_vault.pat";
const SETTING_ENABLED: &str = "cloud_vault.enabled";
const SETTING_DEVICE_ID: &str = "cloud_vault.device_id";
const SETTING_LAST_SYNC: &str = "cloud_vault.last_sync";
const SETTING_PASSWORD: &str = "cloud_vault.password";
const SETTING_AUTO_INTERVAL_MIN: &str = "cloud_vault.auto_interval_min";
const SETTING_LAST_AUTO_SYNC: &str = "cloud_vault.last_auto_sync";

const SYNC_LAYERS: [&str; 2] = ["l3", "l2"];

// ---------- 加密 ----------

fn vault_key(password: &str) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(password.as_bytes());
    let out = hasher.finalize();
    let mut key = [0u8; 32];
    key.copy_from_slice(&out);
    key
}

fn sha256_hex(data: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().iter().map(|b| format!("{:02x}", b)).collect()
}

fn vault_encrypt(key: &[u8; 32], plain: &[u8]) -> Result<Vec<u8>, String> {
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    let nonce = Aes256Gcm::generate_nonce(&mut OsRng);
    let sealed = cipher
        .encrypt(&nonce, plain)
        .map_err(|_| "云端加密失败".to_string())?;
    let mut out = nonce.to_vec();
    out.extend_from_slice(&sealed);
    Ok(out)
}

fn vault_decrypt(key: &[u8; 32], blob: &[u8]) -> Result<Vec<u8>, String> {
    if blob.len() <= 12 {
        return Err("云端密文损坏".into());
    }
    let (nonce, ciphertext) = blob.split_at(12);
    let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(key));
    cipher
        .decrypt(Nonce::from_slice(nonce), ciphertext)
        .map_err(|_| "云端解密失败（同步密码是否正确？）".to_string())
}

// ---------- base64（payload 传输编码，避免新增 crate） ----------

const B64_TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";

fn b64_encode(data: &[u8]) -> String {
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    let mut buf: u32 = 0;
    let mut bits = 0u32;
    for byte in data {
        buf = (buf << 8) | *byte as u32;
        bits += 8;
        while bits >= 6 {
            bits -= 6;
            out.push(B64_TABLE[((buf >> bits) & 0x3f) as usize] as char);
        }
    }
    if bits > 0 {
        out.push(B64_TABLE[((buf << (6 - bits)) & 0x3f) as usize] as char);
    }
    out
}

fn b64_decode(s: &str) -> Option<Vec<u8>> {
    let trimmed: Vec<u8> = s
        .bytes()
        .filter(|b| !b.is_ascii_whitespace() && *b != b'=')
        .collect();
    if trimmed.len() % 4 == 1 {
        return None;
    }
    let mut out = Vec::with_capacity(trimmed.len() * 3 / 4);
    let mut buf: u32 = 0;
    let mut bits = 0u32;
    for byte in trimmed {
        let v = B64_TABLE.iter().position(|c| *c == byte)? as u32;
        buf = (buf << 6) | v;
        bits += 6;
        if bits >= 8 {
            bits -= 8;
            out.push((buf >> bits) as u8);
        }
    }
    Some(out)
}

// ---------- 类型 ----------

#[derive(Serialize)]
pub struct CloudSyncReport {
    pub pushed: u32,
    pub pulled: u32,
    pub skipped: u32,
    pub conflicts: u32,
    pub usage_imported: u32,
    pub errors: Vec<String>,
}

#[derive(Serialize)]
pub struct CloudVaultSettingsView {
    pub url: String,
    pub enabled: bool,
    pub pat_set: bool,
    pub password_set: bool,
    pub auto_interval_min: i64,
}

#[derive(Serialize)]
pub struct CloudVaultStatus {
    pub configured: bool,
    pub enabled: bool,
    pub dirty: u32,
    pub last_sync_at: Option<String>,
    pub last_report: Option<String>,
    pub auto_interval_min: i64,
}

#[derive(Deserialize)]
struct RemoteObject {
    key: String,
    revision: i64,
    content_hash: String,
    payload: String,
    updated_at: String,
    #[allow(dead_code)]
    device_id: String,
}

#[derive(Deserialize)]
struct RemoteTombstone {
    key: String,
    #[allow(dead_code)]
    deleted_at: String,
}

#[derive(Deserialize)]
struct RemoteList {
    objects: Vec<RemoteObject>,
    #[serde(default)]
    tombstones: Vec<RemoteTombstone>,
    #[allow(dead_code)]
    revision: i64,
}

/// 自定义 L1 的云端载荷：内容 + 类型 + 时间戳（冲突裁决用）。
#[derive(Serialize, Deserialize)]
struct L1Payload {
    content: String,
    memory_type: String,
    updated_at: String,
}

#[derive(Serialize)]
struct PutBody<'a> {
    content_hash: &'a str,
    payload: String,
    updated_at: String,
    device_id: String,
}

// ---------- 配置 ----------

fn settings(store: &TelemetryStore) -> Result<(String, String, bool, String), String> {
    let url: Option<String> = store.app_setting_get(SETTING_URL);
    let pat_enc: Option<String> = store.app_setting_get(SETTING_PAT);
    let enabled: Option<bool> = store.app_setting_get(SETTING_ENABLED);
    let password_enc: Option<String> = store.app_setting_get(SETTING_PASSWORD);
    let decrypt = |enc: Option<String>| {
        enc.map(|v| crate::llm::decrypt_api_key(&v)).unwrap_or_default()
    };
    Ok((
        url.unwrap_or_default(),
        decrypt(pat_enc),
        enabled.unwrap_or(false),
        decrypt(password_enc),
    ))
}

fn device_id(store: &TelemetryStore) -> String {
    if let Some(id) = store.app_setting_get::<String>(SETTING_DEVICE_ID) {
        if !id.is_empty() {
            return id;
        }
    }
    let machine = std::env::var("COMPUTERNAME")
        .or_else(|_| std::env::var("HOSTNAME"))
        .unwrap_or_else(|_| "device".to_string());
    let mut raw = [0u8; 4];
    use aes_gcm::aead::rand_core::RngCore;
    OsRng.fill_bytes(&mut raw);
    let suffix: String = raw.iter().map(|b| format!("{:02x}", b)).collect();
    let id = format!("{machine}-{suffix}");
    let _ = store.app_setting_set(SETTING_DEVICE_ID, &id);
    id
}

fn http_client() -> reqwest::blocking::Client {
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(15))
        .build()
        .expect("failed to build http client")
}

// ---------- 同步引擎 ----------

fn parse_ts(ts: &str) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::parse_from_rfc3339(ts)
        .map(|t| t.with_timezone(&chrono::Utc))
        .unwrap_or_else(|_| chrono::DateTime::UNIX_EPOCH)
}

fn push_layer(
    store: &TelemetryStore,
    client: &reqwest::blocking::Client,
    url: &str,
    pat: &str,
    device: &str,
    key: &str,
    layer: &str,
    vault_key_bytes: &[u8; 32],
    report: &mut CloudSyncReport,
) {
    let Some(doc) = store.active_memory_layer_document(layer).ok().flatten() else {
        return;
    };
    let meta = store.cloud_sync_meta_row(key).ok().flatten();
    let base_revision = meta.as_ref().map(|(rev, _, _, _)| *rev).unwrap_or(0);
    let local_hash = sha256_hex(doc.content.as_bytes());
    if let Some((_, Some(hash), state, _)) = meta.as_ref() {
        if *hash == local_hash && state == "clean" {
            report.skipped += 1;
            return;
        }
    }
    let payload = match vault_encrypt(vault_key_bytes, doc.content.as_bytes()) {
        Ok(p) => p,
        Err(e) => {
            report.errors.push(format!("{key}: {e}"));
            return;
        }
    };
    let body = PutBody {
        content_hash: &local_hash,
        payload: b64_encode(&payload),
        updated_at: doc
            .published_at
            .clone()
            .unwrap_or_else(|| chrono::Utc::now().to_rfc3339()),
        device_id: device.to_string(),
    };
    let resp = client
        .put(format!("{url}/objects/{key}"))
        .bearer_auth(pat)
        .header("x-device-id", device)
        .header("If-Match", base_revision.to_string())
        .json(&body)
        .send();
    let resp = match resp {
        Ok(r) => r,
        Err(e) => {
            report.errors.push(format!("{key}: 推送失败 {e}"));
            return;
        }
    };
    match resp.status().as_u16() {
        200..=299 => {
            if let Ok(json) = resp.json::<serde_json::Value>() {
                let rev = json["revision"].as_i64().unwrap_or(base_revision);
                let _ = store.cloud_sync_meta_set(key, rev, Some(&local_hash), "clean");
            }
            report.pushed += 1;
        }
        409 => {
            // CAS 冲突：updated_at 较新者胜出（Phase 1 简化裁决）。
            let current_rev: i64 = resp
                .json::<serde_json::Value>()
                .ok()
                .and_then(|v| v["current_revision"].as_i64())
                .unwrap_or(-1);
            let remote = fetch_object(client, url, pat, key);
            let remote_newer = remote
                .as_ref()
                .map(|o| parse_ts(&o.updated_at) > parse_ts(&body.updated_at))
                .unwrap_or(false);
            if remote_newer {
                if apply_remote(store, layer, vault_key_bytes, remote.unwrap(), report) {
                    report.conflicts += 1;
                }
            } else if current_rev >= 0 {
                let forced = client
                    .put(format!("{url}/objects/{key}"))
                    .bearer_auth(pat)
                    .header("x-device-id", device)
                    .header("If-Match", current_rev.to_string())
                    .json(&body)
                    .send();
                match forced {
                    Ok(r) if r.status().is_success() => {
                        let rev = r
                            .json::<serde_json::Value>()
                            .ok()
                            .and_then(|v| v["revision"].as_i64())
                            .unwrap_or(current_rev);
                        let _ = store.cloud_sync_meta_set(key, rev, Some(&local_hash), "clean");
                        report.pushed += 1;
                        report.conflicts += 1;
                    }
                    Ok(r) => report.errors.push(format!("{key}: 强制推送失败 HTTP {}", r.status())),
                    Err(e) => report.errors.push(format!("{key}: 强制推送失败 {e}")),
                }
            } else {
                report.errors.push(format!("{key}: 冲突但无法获取远端修订号"));
            }
        }
        status => report.errors.push(format!("{key}: 推送失败 HTTP {status}")),
    }
}

fn fetch_object(
    client: &reqwest::blocking::Client,
    url: &str,
    pat: &str,
    key: &str,
) -> Option<RemoteObject> {
    client
        .get(format!("{url}/objects/{key}"))
        .bearer_auth(pat)
        .send()
        .ok()?
        .error_for_status()
        .ok()?
        .json::<RemoteObject>()
        .ok()
}

fn apply_remote(
    store: &TelemetryStore,
    layer: &str,
    vault_key_bytes: &[u8; 32],
    obj: RemoteObject,
    report: &mut CloudSyncReport,
) -> bool {
    let key = obj.key.as_str();
    let blob = match b64_decode(&obj.payload) {
        Some(b) => b,
        None => {
            report.errors.push(format!("{key}: 远端 payload 编码损坏"));
            return false;
        }
    };
    let plain = match vault_decrypt(vault_key_bytes, &blob) {
        Ok(p) => p,
        Err(e) => {
            report.errors.push(format!("{key}: {e}"));
            return false;
        }
    };
    if sha256_hex(&plain) != obj.content_hash {
        report.errors.push(format!("{key}: 远端内容哈希校验失败，已跳过"));
        return false;
    }
    let content = String::from_utf8_lossy(&plain).to_string();
    if let Err(e) = store.upsert_published_layer_content(layer, &content) {
        report.errors.push(format!("{key}: 落地失败 {e}"));
        return false;
    }
    let _ = store.cloud_sync_meta_set(key, obj.revision, Some(&obj.content_hash), "clean");
    report.pulled += 1;
    true
}

/// 自定义 L1 推送：快照 diff——比对当前用户自定义条目与已知水位，
/// 内容变化（或新增）的对象走 CAS PUT；本地已删除但云端还有的对象
/// 推 DELETE（服务端落墓碑），使删除跨设备传播。
fn push_l1(
    store: &TelemetryStore,
    client: &reqwest::blocking::Client,
    url: &str,
    pat: &str,
    device: &str,
    vault_key_bytes: &[u8; 32],
    report: &mut CloudSyncReport,
) {
    let items = match store.user_defined_l1_memories() {
        Ok(items) => items,
        Err(e) => {
            report.errors.push(format!("l1: 读取自定义记忆失败 {e}"));
            return;
        }
    };
    let metas: Vec<(String, i64, Option<String>, String, Option<String>)> = store
        .cloud_sync_meta_rows()
        .unwrap_or_default()
        .into_iter()
        .filter(|(key, _, _, _, _)| key.starts_with("l1-user:"))
        .collect();
    for item in &items {
        let key = format!("l1-user:{}", item.id);
        let payload = L1Payload {
            content: item.memory.clone(),
            memory_type: item.memory_type.clone(),
            updated_at: item.updated_at.clone(),
        };
        let plain = serde_json::to_string(&payload).unwrap_or_default();
        let local_hash = sha256_hex(plain.as_bytes());
        let meta = metas.iter().find(|(k, _, _, _, _)| *k == key);
        if let Some((_, _, Some(hash), state, _)) = meta {
            if *hash == local_hash && state == "clean" {
                continue;
            }
        }
        let base_revision = meta.map(|(_, rev, _, _, _)| *rev).unwrap_or(0);
        let blob = match vault_encrypt(vault_key_bytes, plain.as_bytes()) {
            Ok(b) => b,
            Err(e) => {
                report.errors.push(format!("{key}: {e}"));
                continue;
            }
        };
        let body = PutBody {
            content_hash: &local_hash,
            payload: b64_encode(&blob),
            updated_at: payload.updated_at.clone(),
            device_id: device.to_string(),
        };
        let resp = client
            .put(format!("{url}/objects/{key}"))
            .bearer_auth(pat)
            .header("x-device-id", device)
            .header("If-Match", base_revision.to_string())
            .json(&body)
            .send();
        match resp {
            Ok(r) if r.status().is_success() => {
                let rev = r
                    .json::<serde_json::Value>()
                    .ok()
                    .and_then(|v| v["revision"].as_i64())
                    .unwrap_or(base_revision);
                let _ = store.cloud_sync_meta_set(&key, rev, Some(&local_hash), "clean");
                report.pushed += 1;
            }
            // 409：远端被其他设备改过。拉取阶段（紧随其后）会做 updated_at 裁决，
            // 本地较新时下次同步仍会以新 revision 覆盖，这里不重复处理。
            Ok(r) if r.status().as_u16() == 409 => {
                report.conflicts += 1;
            }
            Ok(r) => report.errors.push(format!("{key}: 推送失败 HTTP {}", r.status())),
            Err(e) => report.errors.push(format!("{key}: 推送失败 {e}")),
        }
    }
    // 本地删除传播：水位里有、本地已无对应条目 → 推墓碑。
    let live_ids: Vec<String> = items
        .iter()
        .map(|item| format!("l1-user:{}", item.id))
        .collect();
    for (key, _, _, _, _) in metas {
        if !live_ids.contains(&key) {
            match client
                .delete(format!("{url}/objects/{key}"))
                .bearer_auth(pat)
                .header("x-device-id", device)
                .send()
            {
                Ok(r) if r.status().is_success() || r.status().as_u16() == 404 => {
                    let _ = store.cloud_sync_meta_delete(&key);
                    report.pushed += 1;
                }
                Ok(r) => report.errors.push(format!("{key}: 删除推送失败 HTTP {}", r.status())),
                Err(e) => report.errors.push(format!("{key}: 删除推送失败 {e}")),
            }
        }
    }
}

/// Token 用量推送：每台设备只写一个小对象 `usage-agg:{device}`，
/// 内容是「天 × 来源」的总量统计（不含逐条明细）。单写者，无并发冲突；
/// 内容变化由 hash 比对发现后整体重推。
fn push_usage(
    store: &TelemetryStore,
    client: &reqwest::blocking::Client,
    url: &str,
    pat: &str,
    device: &str,
    vault_key_bytes: &[u8; 32],
    report: &mut CloudSyncReport,
) {
    let key = format!("usage-agg:{}", device.replace(':', "-"));
    let rows = match store.usage_day_aggregates() {
        Ok(rows) => rows,
        Err(e) => {
            report.errors.push(format!("{key}: 统计读取失败 {e}"));
            return;
        }
    };
    let plain = match serde_json::to_vec(&rows) {
        Ok(p) => p,
        Err(e) => {
            report.errors.push(format!("{key}: 序列化失败 {e}"));
            return;
        }
    };
    let local_hash = sha256_hex(&plain);
    if let Some((_, Some(hash), state, _)) = store.cloud_sync_meta_row(&key).ok().flatten() {
        if hash == local_hash && state == "clean" {
            return;
        }
    }
    let base_revision = store
        .cloud_sync_meta_row(&key)
        .ok()
        .flatten()
        .map(|(rev, _, _, _)| rev)
        .unwrap_or(0);
    let blob = match vault_encrypt(vault_key_bytes, &plain) {
        Ok(b) => b,
        Err(e) => {
            report.errors.push(format!("{key}: {e}"));
            return;
        }
    };
    if blob.len() > 250 * 1024 {
        report.errors.push(format!("{key}: 统计超过 250KB 上限，已跳过"));
        return;
    }
    let body = PutBody {
        content_hash: &local_hash,
        payload: b64_encode(&blob),
        updated_at: chrono::Utc::now().to_rfc3339(),
        device_id: device.to_string(),
    };
    let resp = client
        .put(format!("{url}/objects/{key}"))
        .bearer_auth(pat)
        .header("x-device-id", device)
        .header("If-Match", base_revision.to_string())
        .json(&body)
        .send();
    match resp {
        Ok(r) if r.status().is_success() => {
            let rev = r
                .json::<serde_json::Value>()
                .ok()
                .and_then(|v| v["revision"].as_i64())
                .unwrap_or(base_revision);
            let _ = store.cloud_sync_meta_set(&key, rev, Some(&local_hash), "clean");
            report.pushed += 1;
        }
        // 本设备单写者，409 只可能是水位错位；按远端修订强制重推一次。
        Ok(r) if r.status().as_u16() == 409 => {
            let current_rev = r
                .json::<serde_json::Value>()
                .ok()
                .and_then(|v| v["current_revision"].as_i64())
                .unwrap_or(-1);
            if current_rev < 0 {
                report.errors.push(format!("{key}: 冲突且无法获取远端修订号"));
                return;
            }
            let forced = client
                .put(format!("{url}/objects/{key}"))
                .bearer_auth(pat)
                .header("x-device-id", device)
                .header("If-Match", current_rev.to_string())
                .json(&body)
                .send();
            match forced {
                Ok(r) if r.status().is_success() => {
                    let rev = r
                        .json::<serde_json::Value>()
                        .ok()
                        .and_then(|v| v["revision"].as_i64())
                        .unwrap_or(current_rev);
                    let _ = store.cloud_sync_meta_set(&key, rev, Some(&local_hash), "clean");
                    report.pushed += 1;
                }
                Ok(r) => report.errors.push(format!("{key}: 用量推送失败 HTTP {}", r.status())),
                Err(e) => report.errors.push(format!("{key}: 用量推送失败 {e}")),
            }
        }
        Ok(r) => report.errors.push(format!("{key}: 用量推送失败 HTTP {}", r.status())),
        Err(e) => report.errors.push(format!("{key}: 用量推送失败 {e}")),
    }
}

fn pull(
    store: &TelemetryStore,
    client: &reqwest::blocking::Client,
    url: &str,
    pat: &str,
    vault_key_bytes: &[u8; 32],
    report: &mut CloudSyncReport,
) {
    let since = store.cloud_sync_max_revision().unwrap_or(0);
    let resp = match client
        .get(format!("{url}/objects"))
        .query(&[("since_rev", since.to_string())])
        .bearer_auth(pat)
        .send()
    {
        Ok(r) => r,
        Err(e) => {
            report.errors.push(format!("拉取失败 {e}"));
            return;
        }
    };
    if !resp.status().is_success() {
        report.errors.push(format!("拉取失败 HTTP {}", resp.status()));
        return;
    }
    let list = match resp.json::<RemoteList>() {
        Ok(l) => l,
        Err(e) => {
            report.errors.push(format!("拉取响应解析失败 {e}"));
            return;
        }
    };
    let device = device_id(store);
    for obj in list.objects {
        if let Some(id) = obj.key.strip_prefix("l1-user:") {
            apply_remote_l1(store, id.to_string(), vault_key_bytes, obj, report);
            continue;
        }
        if let Some(rest) = obj.key.strip_prefix("usage-agg:") {
            apply_remote_usage(store, rest.to_string(), vault_key_bytes, obj, report);
            continue;
        }
        if obj.key.starts_with("usage:") {
            // 旧版逐条账本分片：不再同步，直接从服务端清除。
            let _ = client
                .delete(format!("{url}/objects/{}", obj.key))
                .bearer_auth(pat)
                .header("x-device-id", &device)
                .send();
            let _ = store.cloud_sync_meta_delete(&obj.key);
            continue;
        }
        let Some(layer) = obj.key.strip_suffix(":published").map(str::to_string) else {
            continue;
        };
        if !SYNC_LAYERS.contains(&layer.as_str()) {
            continue;
        }
        let meta = store.cloud_sync_meta_row(&obj.key).ok().flatten();
        if let Some((_, Some(hash), _, _)) = meta.as_ref() {
            if *hash == obj.content_hash {
                let _ = store.cloud_sync_meta_set(&obj.key, obj.revision, Some(&obj.content_hash), "clean");
                report.skipped += 1;
                continue;
            }
        }
        let dirty = meta
            .as_ref()
            .map(|(_, _, state, _)| state == "dirty")
            .unwrap_or(false);
        if !dirty {
            apply_remote(store, &layer, vault_key_bytes, obj, report);
        } else {
            // 本地 dirty 但推送阶段已处理过 409 裁决；此处远端仍是更新版本时采用远端。
            let local = store.active_memory_layer_document(&layer).ok().flatten();
            let local_ts = local
                .and_then(|d| d.published_at)
                .map(|t| parse_ts(&t))
                .unwrap_or(chrono::DateTime::UNIX_EPOCH);
            if parse_ts(&obj.updated_at) > local_ts {
                if apply_remote(store, &layer, vault_key_bytes, obj, report) {
                    report.conflicts += 1;
                }
            }
        }
    }
    // 墓碑：其他设备删除的自定义 L1 在本地同步删除（仅 user_defined 条目）。
    for tomb in list.tombstones {
        let Some(id) = tomb.key.strip_prefix("l1-user:") else {
            continue;
        };
        match store.delete_synced_l1_memory(id) {
            Ok(true) => report.pulled += 1,
            Ok(false) => {}
            Err(e) => report.errors.push(format!("{}: 删除落地失败 {e}", tomb.key)),
        }
        let _ = store.cloud_sync_meta_delete(&tomb.key);
    }
}

/// 远端用量总量统计落地：解密验哈希 → 整设备覆盖 usage_remote_totals。
/// 本设备自己的对象跳过落地（推送阶段保证本地为源），只对齐水位。
fn apply_remote_usage(
    store: &TelemetryStore,
    device_suffix: String,
    vault_key_bytes: &[u8; 32],
    obj: RemoteObject,
    report: &mut CloudSyncReport,
) -> bool {
    let device = device_id(store).replace(':', "-");
    if device_suffix == device {
        let _ = store.cloud_sync_meta_set(&obj.key, obj.revision, Some(&obj.content_hash), "clean");
        report.skipped += 1;
        return true;
    }
    let blob = match b64_decode(&obj.payload) {
        Some(b) => b,
        None => {
            report.errors.push(format!("usage-agg: 远端 payload 编码损坏 ({})", obj.key));
            return false;
        }
    };
    let plain = match vault_decrypt(vault_key_bytes, &blob) {
        Ok(p) => p,
        Err(e) => {
            report.errors.push(format!("usage-agg: {e} ({})", obj.key));
            return false;
        }
    };
    if sha256_hex(&plain) != obj.content_hash {
        report.errors.push(format!("usage-agg: 远端哈希校验失败，已跳过 ({})", obj.key));
        return false;
    }
    let rows: Vec<UsageDayAggregate> = match serde_json::from_slice(&plain) {
        Ok(r) => r,
        Err(e) => {
            report.errors.push(format!("usage-agg: 载荷解析失败 {e} ({})", obj.key));
            return false;
        }
    };
    match store.replace_remote_usage_totals(&device_suffix, &rows) {
        Ok(imported) => {
            let _ = store.cloud_sync_meta_set(&obj.key, obj.revision, Some(&obj.content_hash), "clean");
            report.pulled += 1;
            report.usage_imported += imported as u32;
        }
        Err(e) => {
            report.errors.push(format!("usage-agg: 落地失败 {e} ({})", obj.key));
            return false;
        }
    }
    true
}

/// 远端自定义 L1 落地：解密验哈希 → 与本地比对 → upsert 或按 updated_at 裁决。
fn apply_remote_l1(
    store: &TelemetryStore,
    id: String,
    vault_key_bytes: &[u8; 32],
    obj: RemoteObject,
    report: &mut CloudSyncReport,
) -> bool {
    let blob = match b64_decode(&obj.payload) {
        Some(b) => b,
        None => {
            report.errors.push(format!("l1-user:{id}: 远端 payload 编码损坏"));
            return false;
        }
    };
    let plain = match vault_decrypt(vault_key_bytes, &blob) {
        Ok(p) => p,
        Err(e) => {
            report.errors.push(format!("l1-user:{id}: {e}"));
            return false;
        }
    };
    if sha256_hex(&plain) != obj.content_hash {
        report.errors.push(format!("l1-user:{id}: 远端内容哈希校验失败，已跳过"));
        return false;
    }
    let payload: L1Payload = match serde_json::from_slice(&plain) {
        Ok(p) => p,
        Err(e) => {
            report.errors.push(format!("l1-user:{id}: 载荷解析失败 {e}"));
            return false;
        }
    };
    // 本地已有同 hash → 只对齐水位。
    if let Ok(Some(item)) = store.find_user_defined_l1(&id) {
        let local_payload = L1Payload {
            content: item.memory.clone(),
            memory_type: item.memory_type.clone(),
            updated_at: item.updated_at.clone(),
        };
        if let Ok(local_plain) = serde_json::to_string(&local_payload) {
            if sha256_hex(local_plain.as_bytes()) == obj.content_hash {
                let _ = store.cloud_sync_meta_set(&obj.key, obj.revision, Some(&obj.content_hash), "clean");
                report.skipped += 1;
                return true;
            }
        }
        // 内容不同：updated_at 较新者胜出。本地较新则保留，下次推送覆盖远端。
        if parse_ts(&item.updated_at) > parse_ts(&payload.updated_at) {
            report.conflicts += 1;
            return false;
        }
    }
    if let Err(e) = store.upsert_synced_l1_memory(&id, &payload.content, &payload.memory_type, &payload.updated_at) {
        report.errors.push(format!("l1-user:{id}: 落地失败 {e}"));
        return false;
    }
    let _ = store.cloud_sync_meta_set(&obj.key, obj.revision, Some(&obj.content_hash), "clean");
    report.pulled += 1;
    true
}

fn run_sync(store: &TelemetryStore) -> Result<CloudSyncReport, String> {
    let (url, pat, enabled, password) = settings(store)?;
    if !enabled {
        return Err("云端记忆库未启用".into());
    }
    if url.trim().is_empty() || pat.is_empty() {
        return Err("云端记忆库未配置 URL 或 PAT".into());
    }
    if password.is_empty() {
        return Err("未设置同步密码：请到「设置 → 云端记忆库」保存同步密码".into());
    }
    let url = url.trim().trim_end_matches('/').to_string();
    let key = vault_key(&password);
    let client = http_client();
    let device = device_id(store);
    let mut report = CloudSyncReport {
        pushed: 0,
        pulled: 0,
        skipped: 0,
        conflicts: 0,
        usage_imported: 0,
        errors: Vec::new(),
    };
    for layer in SYNC_LAYERS {
        let key_name = format!("{layer}:published");
        push_layer(store, &client, &url, &pat, &device, &key_name, layer, &key, &mut report);
    }
    push_l1(store, &client, &url, &pat, &device, &key, &mut report);
    push_usage(store, &client, &url, &pat, &device, &key, &mut report);
    pull(store, &client, &url, &pat, &key, &mut report);
    let summary = format!(
        "推送 {} · 拉取 {} · 用量 {} 条 · 跳过 {} · 冲突 {} · 错误 {}",
        report.pushed, report.pulled, report.usage_imported, report.skipped, report.conflicts, report.errors.len()
    );
    let _ = store.app_setting_set(SETTING_LAST_SYNC, &chrono::Utc::now().to_rfc3339());
    let _ = store.app_setting_set("cloud_vault.last_report", &summary);
    Ok(report)
}

// ---------- Tauri 命令 ----------

#[tauri::command]
pub fn cloud_vault_get_settings(
    telemetry: State<'_, TelemetryStore>,
) -> Result<CloudVaultSettingsView, String> {
    let (url, pat, enabled, password) = settings(&telemetry)?;
    Ok(CloudVaultSettingsView {
        url,
        enabled,
        pat_set: !pat.is_empty(),
        password_set: !password.is_empty(),
        auto_interval_min: telemetry
            .app_setting_get::<i64>(SETTING_AUTO_INTERVAL_MIN)
            .unwrap_or(0),
    })
}

#[tauri::command]
pub fn cloud_vault_save_settings(
    telemetry: State<'_, TelemetryStore>,
    url: String,
    pat: Option<String>,
    password: Option<String>,
    auto_interval_min: i64,
    enabled: bool,
) -> Result<(), String> {
    let url = url.trim().trim_end_matches('/').to_string();
    telemetry.app_setting_set(SETTING_URL, &url)?;
    if let Some(pat) = pat {
        let enc = if pat.is_empty() {
            String::new()
        } else {
            crate::llm::encrypt_api_key(&pat)?
        };
        telemetry.app_setting_set(SETTING_PAT, &enc)?;
    }
    if let Some(password) = password {
        let enc = if password.is_empty() {
            String::new()
        } else {
            crate::llm::encrypt_api_key(&password)?
        };
        telemetry.app_setting_set(SETTING_PASSWORD, &enc)?;
    }
    telemetry.app_setting_set(SETTING_AUTO_INTERVAL_MIN, &auto_interval_min.clamp(0, 1440))?;
    telemetry.app_setting_set(SETTING_ENABLED, &enabled)?;
    Ok(())
}

#[tauri::command]
pub async fn cloud_vault_test_connection(url: String, pat: String) -> Result<String, String> {
    let url = url.trim().trim_end_matches('/').to_string();
    if url.is_empty() {
        return Err("请先填写服务端 URL".into());
    }
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client
        .get(format!("{url}/health"))
        .bearer_auth(pat.trim())
        .send()
        .await
        .map_err(|e| format!("连接失败: {e}"))?;
    if !resp.status().is_success() {
        return Err(format!("服务端返回 HTTP {}", resp.status()));
    }
    let body: serde_json::Value = resp.json().await.map_err(|e| e.to_string())?;
    Ok(body["version"].as_str().unwrap_or("unknown").to_string())
}

#[tauri::command]
pub async fn cloud_vault_sync() -> Result<CloudSyncReport, String> {
    let store = crate::telemetry_store::shared_store().ok_or("telemetry store 未初始化")?;
    tauri::async_runtime::spawn_blocking(move || run_sync(&store))
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
pub fn cloud_vault_status(
    telemetry: State<'_, TelemetryStore>,
) -> Result<CloudVaultStatus, String> {
    let (url, pat, enabled, password) = settings(&telemetry)?;
    let dirty = telemetry
        .cloud_sync_meta_rows()?
        .iter()
        .filter(|(_, _, _, state, _)| state == "dirty")
        .count() as u32;
    Ok(CloudVaultStatus {
        configured: !url.is_empty() && !pat.is_empty() && !password.is_empty(),
        enabled,
        dirty,
        last_sync_at: telemetry.app_setting_get(SETTING_LAST_SYNC),
        last_report: telemetry.app_setting_get("cloud_vault.last_report"),
        auto_interval_min: telemetry
            .app_setting_get::<i64>(SETTING_AUTO_INTERVAL_MIN)
            .unwrap_or(0),
    })
}

/// 定时同步调度：每分钟检查一次设置（改设置无需重启），
/// 到期则触发一轮完整推拉。失败静默，下个周期重试。
pub fn start_cloud_sync_scheduler() {
    tauri::async_runtime::spawn(async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(60)).await;
            let Some(store) = crate::telemetry_store::shared_store() else {
                continue;
            };
            let interval_min = store
                .app_setting_get::<i64>(SETTING_AUTO_INTERVAL_MIN)
                .unwrap_or(0);
            if interval_min <= 0 {
                continue;
            }
            if store.app_setting_get::<bool>(SETTING_ENABLED) != Some(true) {
                continue;
            }
            let now = chrono::Utc::now();
            let due = match store.app_setting_get::<String>(SETTING_LAST_AUTO_SYNC) {
                Some(last) => {
                    parse_ts(&last) + chrono::Duration::minutes(interval_min) <= now
                }
                None => true,
            };
            if !due {
                continue;
            }
            let _ = store.app_setting_set(SETTING_LAST_AUTO_SYNC, &now.to_rfc3339());
            let task_store = store.clone();
            let _ = tauri::async_runtime::spawn_blocking(move || run_sync(&task_store)).await;
        }
    });
}
