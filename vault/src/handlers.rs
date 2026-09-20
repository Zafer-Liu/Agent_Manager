use axum::{
    extract::{Path, Query, State},
    http::{HeaderMap, StatusCode},
    response::{IntoResponse, Response},
    routing::get,
    Json, Router,
};
use serde::Deserialize;
use serde_json::json;

use crate::store::{self, CasError, SharedStore};

const MAX_PAYLOAD: usize = 256 * 1024;

#[derive(Deserialize)]
pub struct PutBody {
    pub content_hash: String,
    #[serde(with = "serde_bytes_base64")]
    pub payload: Vec<u8>,
    pub updated_at: String,
    pub device_id: String,
}

/// payload 用标准 base64（无填充也接受）传输；为避免引入 base64 crate，
/// 这里手写一个宽松的 serde 辅助。Phase 1 数据量小，性能足够。
mod serde_bytes_base64 {
    use serde::{Deserialize, Deserializer};

    fn decode_b64(s: &str) -> Option<Vec<u8>> {
        const TABLE: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
        let filtered: Vec<u8> = s.bytes().filter(|b| !b.is_ascii_whitespace()).collect();
        let trimmed: &[u8] = {
            let end = filtered
                .iter()
                .rposition(|b| *b != b'=')
                .map_or(0, |i| i + 1);
            &filtered[..end]
        };
        if trimmed.len() % 4 == 1 {
            return None;
        }
        let mut out = Vec::with_capacity(trimmed.len() * 3 / 4);
        let mut buf: u32 = 0;
        let mut bits = 0u32;
        for byte in trimmed {
            let v = TABLE.iter().position(|c| *c == *byte)? as u32;
            buf = (buf << 6) | v;
            bits += 6;
            if bits >= 8 {
                bits -= 8;
                out.push((buf >> bits) as u8);
            }
        }
        Some(out)
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<Vec<u8>, D::Error>
    where
        D: Deserializer<'de>,
    {
        let s = String::deserialize(deserializer)?;
        decode_b64(&s).ok_or_else(|| serde::de::Error::custom("invalid base64 payload"))
    }
}

#[derive(Deserialize)]
struct SinceQuery {
    since_rev: Option<i64>,
}

pub fn router(store: SharedStore) -> Router {
    Router::new()
        .route("/objects", get(list_objects))
        .route("/objects/:key", get(get_object).put(put_object).delete(delete_object))
        .route_layer(axum::middleware::from_fn_with_state(
            store.clone(),
            auth,
        ))
        // /health 免认证：连接测试按钮只探测存活，不携带 PAT。
        .route("/health", get(health))
        .with_state(store)
}

async fn auth(
    State(store): State<SharedStore>,
    req: axum::extract::Request,
    next: axum::middleware::Next,
) -> Response {
    let token = req
        .headers()
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "));
    let Some(token) = token else {
        return (StatusCode::UNAUTHORIZED, "missing bearer token").into_response();
    };
    if !store::check_token(&store, token) {
        return (StatusCode::UNAUTHORIZED, "invalid token").into_response();
    }
    if let Some(device) = req.headers().get("x-device-id").and_then(|v| v.to_str().ok()) {
        if !device.is_empty() && device.len() <= 128 {
            store::touch_device(&store, device);
        }
    }
    next.run(req).await
}

async fn health() -> impl IntoResponse {
    Json(json!({ "status": "ok", "service": "agent-manager-vault", "version": env!("CARGO_PKG_VERSION") }))
}

async fn list_objects(
    State(store): State<SharedStore>,
    Query(q): Query<SinceQuery>,
) -> impl IntoResponse {
    let since = q.since_rev.unwrap_or(0);
    let (objects, tombstones, revision) = store::objects_since(&store, since);
    Json(json!({
        "objects": objects,
        "tombstones": tombstones,
        "revision": revision,
    }))
}

async fn get_object(
    State(store): State<SharedStore>,
    Path(key): Path<String>,
) -> Response {
    match store::get_object(&store, &key) {
        Some(obj) => Json(obj).into_response(),
        None => (StatusCode::NOT_FOUND, "object not found").into_response(),
    }
}

async fn put_object(
    State(store): State<SharedStore>,
    Path(key): Path<String>,
    headers: HeaderMap,
    body: axum::body::Bytes,
) -> Response {
    if body.len() > 1024 * 1024 {
        return (StatusCode::PAYLOAD_TOO_LARGE, "body too large").into_response();
    }
    let parsed: PutBody = match serde_json::from_slice(&body) {
        Ok(v) => v,
        Err(e) => return (StatusCode::BAD_REQUEST, format!("bad body: {e}")).into_response(),
    };
    if parsed.payload.len() > MAX_PAYLOAD {
        return (StatusCode::PAYLOAD_TOO_LARGE, "payload exceeds 256KB").into_response();
    }
    if key.is_empty() || key.len() > 256 {
        return (StatusCode::BAD_REQUEST, "invalid key").into_response();
    }
    let base_revision: i64 = headers
        .get("if-match")
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.trim().parse().ok())
        .unwrap_or(-1);
    if base_revision < 0 {
        return (StatusCode::PRECONDITION_FAILED, "missing If-Match revision").into_response();
    }
    match store::put_object(
        &store,
        &key,
        base_revision,
        parsed.content_hash,
        parsed.payload,
        &parsed.updated_at,
        &parsed.device_id,
    ) {
        Ok(rev) => Json(json!({ "revision": rev })).into_response(),
        Err(CasError::Conflict { current_revision }) => (
            StatusCode::CONFLICT,
            Json(json!({ "error": "revision conflict", "current_revision": current_revision })),
        )
            .into_response(),
    }
}

async fn delete_object(
    State(store): State<SharedStore>,
    Path(key): Path<String>,
    headers: HeaderMap,
) -> Response {
    let device = headers
        .get("x-device-id")
        .and_then(|v| v.to_str().ok())
        .unwrap_or("")
        .to_string();
    if store::delete_object(&store, &key, &device) {
        StatusCode::NO_CONTENT.into_response()
    } else {
        (StatusCode::NOT_FOUND, "object not found").into_response()
    }
}
