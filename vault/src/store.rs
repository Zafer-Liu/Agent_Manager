use std::sync::{Arc, Mutex};

use rusqlite::Connection;

const SCHEMA: &str = "
CREATE TABLE IF NOT EXISTS objects (
  key TEXT PRIMARY KEY,
  revision INTEGER NOT NULL,
  content_hash TEXT NOT NULL,
  payload BLOB NOT NULL,
  updated_at TEXT NOT NULL,
  device_id TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS tombstones (
  key TEXT PRIMARY KEY,
  deleted_at TEXT NOT NULL,
  device_id TEXT NOT NULL
);
CREATE TABLE IF NOT EXISTS devices (
  device_id TEXT PRIMARY KEY,
  name TEXT,
  last_seen_at TEXT
);
CREATE TABLE IF NOT EXISTS tokens (
  token_hash TEXT PRIMARY KEY,
  label TEXT,
  created_at TEXT
);
CREATE TABLE IF NOT EXISTS meta (rev INTEGER NOT NULL);
INSERT INTO meta (rev) SELECT 0 WHERE NOT EXISTS (SELECT 1 FROM meta);
";

pub struct Store {
    conn: Mutex<Connection>,
}

pub type SharedStore = Arc<Store>;

#[derive(Debug, Clone, serde::Serialize)]
pub struct CloudObject {
    pub key: String,
    pub revision: i64,
    pub content_hash: String,
    /// base64 编码的密文（serde 的 Vec<u8> 会序列化成 JSON 数字数组，体积大且不便客户端解析）
    pub payload: String,
    pub updated_at: String,
    pub device_id: String,
}

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

#[derive(Debug, Clone, serde::Serialize)]
pub struct Tombstone {
    pub key: String,
    pub deleted_at: String,
    pub device_id: String,
}

#[derive(Debug)]
pub enum CasError {
    Conflict { current_revision: i64 },
}

fn now_rfc3339() -> String {
    chrono::Utc::now().to_rfc3339()
}

pub fn open(path: &str) -> Result<Store, String> {
    let conn = Connection::open(path).map_err(|e| e.to_string())?;
    conn.pragma_update(None, "journal_mode", "WAL")
        .map_err(|e| e.to_string())?;
    conn.execute_batch(SCHEMA).map_err(|e| e.to_string())?;
    Ok(Store {
        conn: Mutex::new(conn),
    })
}

pub fn sha256_hex(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let mut hasher = Sha256::new();
    hasher.update(data);
    let out = hasher.finalize();
    out.iter().map(|b| format!("{:02x}", b)).collect()
}

/// 首次启动时初始化 PAT；已存在 tokens 则跳过。
pub fn bootstrap_tokens(store: &Store, env_tokens: Option<&str>) -> Option<String> {
    let conn = store.conn.lock().unwrap();
    let existing: i64 = conn
        .query_row("SELECT COUNT(*) FROM tokens", [], |r| r.get(0))
        .unwrap_or(0);
    if existing > 0 {
        return None;
    }
    let created = now_rfc3339();
    if let Some(list) = env_tokens {
        for tok in list.split(',').map(str::trim).filter(|t| !t.is_empty()) {
            let _ = conn.execute(
                "INSERT OR IGNORE INTO tokens (token_hash, label, created_at) VALUES (?1, 'env', ?2)",
                rusqlite::params![sha256_hex(tok.as_bytes()), created],
            );
        }
        return None;
    }
    use rand::RngCore;
    let mut raw = [0u8; 24];
    rand::thread_rng().fill_bytes(&mut raw);
    let token = format!(
        "vault-{}",
        raw.iter().map(|b| format!("{:02x}", b)).collect::<String>()
    );
    conn.execute(
        "INSERT INTO tokens (token_hash, label, created_at) VALUES (?1, 'bootstrap', ?2)",
        rusqlite::params![sha256_hex(token.as_bytes()), created],
    )
    .ok()?;
    Some(token)
}

pub fn check_token(store: &Store, token: &str) -> bool {
    let hash = sha256_hex(token.as_bytes());
    let conn = store.conn.lock().unwrap();
    conn.query_row("SELECT 1 FROM tokens WHERE token_hash = ?1", [&hash], |_| Ok(()))
        .is_ok()
}

pub fn touch_device(store: &Store, device_id: &str) {
    let conn = store.conn.lock().unwrap();
    let _ = conn.execute(
        "INSERT INTO devices (device_id, last_seen_at) VALUES (?1, ?2)
         ON CONFLICT(device_id) DO UPDATE SET last_seen_at = ?2",
        rusqlite::params![device_id, now_rfc3339()],
    );
}

#[allow(dead_code)]
pub fn current_revision(store: &Store) -> i64 {
    let conn = store.conn.lock().unwrap();
    conn.query_row("SELECT rev FROM meta", [], |r| r.get(0)).unwrap_or(0)
}

pub fn get_object(store: &Store, key: &str) -> Option<CloudObject> {
    let conn = store.conn.lock().unwrap();
    conn.query_row(
        "SELECT key, revision, content_hash, payload, updated_at, device_id FROM objects WHERE key = ?1",
        [key],
        |r| {
            Ok(CloudObject {
                key: r.get(0)?,
                revision: r.get(1)?,
                content_hash: r.get(2)?,
                payload: b64_encode(&r.get::<_, Vec<u8>>(3)?),
                updated_at: r.get(4)?,
                device_id: r.get(5)?,
            })
        },
    )
    .ok()
}

/// CAS 写入：If-Match 与当前修订不符返回 Conflict（携带当前修订供强制重试）。
pub fn put_object(
    store: &Store,
    key: &str,
    base_revision: i64,
    content_hash: String,
    payload: Vec<u8>,
    updated_at: &str,
    device_id: &str,
) -> Result<i64, CasError> {
    let mut conn = store.conn.lock().unwrap();
    let tx = conn
        .transaction()
        .map_err(|_| CasError::Conflict { current_revision: 0 })?;
    let current: Option<i64> = tx
        .query_row(
            "SELECT revision FROM objects WHERE key = ?1",
            [key],
            |r| r.get(0),
        )
        .ok();
    match current {
        Some(rev) if rev != base_revision => {
            return Err(CasError::Conflict {
                current_revision: rev,
            })
        }
        None if base_revision != 0 => {
            return Err(CasError::Conflict { current_revision: 0 })
        }
        _ => {}
    }
    tx.execute("UPDATE meta SET rev = rev + 1", [])
        .map_err(|_| CasError::Conflict { current_revision: 0 })?;
    let new_rev: i64 = tx
        .query_row("SELECT rev FROM meta", [], |r| r.get(0))
        .map_err(|_| CasError::Conflict { current_revision: 0 })?;
    tx.execute(
        "INSERT INTO objects (key, revision, content_hash, payload, updated_at, device_id)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6)
         ON CONFLICT(key) DO UPDATE SET revision = ?2, content_hash = ?3,
           payload = ?4, updated_at = ?5, device_id = ?6",
        rusqlite::params![key, new_rev, content_hash, payload, updated_at, device_id],
    )
    .map_err(|_| CasError::Conflict { current_revision: 0 })?;
    tx.execute("DELETE FROM tombstones WHERE key = ?1", [key])
        .map_err(|_| CasError::Conflict { current_revision: 0 })?;
    tx.commit().map_err(|_| CasError::Conflict { current_revision: 0 })?;
    Ok(new_rev)
}

pub fn delete_object(store: &Store, key: &str, device_id: &str) -> bool {
    let mut conn = store.conn.lock().unwrap();
    if let Ok(tx) = conn.transaction() {
        let n = tx.execute("DELETE FROM objects WHERE key = ?1", [key]).unwrap_or(0);
        if n > 0 {
            let _ = tx.execute(
                "INSERT INTO tombstones (key, deleted_at, device_id) VALUES (?1, ?2, ?3)
                 ON CONFLICT(key) DO UPDATE SET deleted_at = ?2, device_id = ?3",
                rusqlite::params![key, now_rfc3339(), device_id],
            );
            let _ = tx.execute("UPDATE meta SET rev = rev + 1", []);
            let _ = tx.commit();
            return true;
        }
        let _ = tx.commit();
    }
    false
}

pub fn objects_since(
    store: &Store,
    since_rev: i64,
) -> (Vec<CloudObject>, Vec<Tombstone>, i64) {
    let conn = store.conn.lock().unwrap();
    let mut stmt = conn
        .prepare(
            "SELECT key, revision, content_hash, payload, updated_at, device_id
             FROM objects WHERE revision > ?1 ORDER BY revision",
        )
        .unwrap();
    let objects = stmt
        .query_map([since_rev], |r| {
            Ok(CloudObject {
                key: r.get(0)?,
                revision: r.get(1)?,
                content_hash: r.get(2)?,
                payload: b64_encode(&r.get::<_, Vec<u8>>(3)?),
                updated_at: r.get(4)?,
                device_id: r.get(5)?,
            })
        })
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default();
    let mut stmt = conn
        .prepare("SELECT key, deleted_at, device_id FROM tombstones")
        .unwrap();
    let tombstones = stmt
        .query_map([], |r| {
            Ok(Tombstone {
                key: r.get(0)?,
                deleted_at: r.get(1)?,
                device_id: r.get(2)?,
            })
        })
        .map(|rows| rows.filter_map(Result::ok).collect())
        .unwrap_or_default();
    let rev: i64 = conn
        .query_row("SELECT rev FROM meta", [], |r| r.get(0))
        .unwrap_or(0);
    (objects, tombstones, rev)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp_store() -> Store {
        open(":memory:").unwrap()
    }

    #[test]
    fn cas_second_write_conflicts() {
        let store = tmp_store();
        let r1 = put_object(&store, "l3:published", 0, "h1".into(), b"c1".to_vec(), "2026-01-01T00:00:00Z", "a").unwrap();
        assert_eq!(r1, 1);
        let err = put_object(&store, "l3:published", 0, "h2".into(), b"c2".to_vec(), "2026-01-02T00:00:00Z", "b").unwrap_err();
        match err {
            CasError::Conflict { current_revision } => assert_eq!(current_revision, 1),
        }
        let r2 = put_object(&store, "l3:published", 1, "h2".into(), b"c2".to_vec(), "2026-01-02T00:00:00Z", "b").unwrap();
        assert_eq!(r2, 2);
    }

    #[test]
    fn objects_since_returns_increment() {
        let store = tmp_store();
        put_object(&store, "l3:published", 0, "h".into(), b"c".to_vec(), "t", "a").unwrap();
        put_object(&store, "l2:published", 0, "h".into(), b"c".to_vec(), "t", "a").unwrap();
        let (objs, _, rev) = objects_since(&store, 1);
        assert_eq!(objs.len(), 1);
        assert_eq!(objs[0].key, "l2:published");
        assert_eq!(rev, 2);
    }

    #[test]
    fn delete_writes_tombstone() {
        let store = tmp_store();
        put_object(&store, "l3:published", 0, "h".into(), b"c".to_vec(), "t", "a").unwrap();
        assert!(delete_object(&store, "l3:published", "a"));
        let (_, tombs, _) = objects_since(&store, 0);
        assert_eq!(tombs.len(), 1);
        assert_eq!(tombs[0].key, "l3:published");
    }
}
