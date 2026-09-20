mod handlers;
mod store;

use std::sync::Arc;

#[tokio::main]
async fn main() {
    let db_path = std::env::var("VAULT_DB").unwrap_or_else(|_| "vault.sqlite3".to_string());
    let store = Arc::new(store::open(&db_path).expect("failed to open vault database"));

    if let Some(token) = store::bootstrap_tokens(&store, std::env::var("VAULT_TOKENS").ok().as_deref()) {
        println!("=== Cloud Vault PAT (shown once, store it now) ===");
        println!("{token}");
        println!("=====================================================");
    }

    let port: u16 = std::env::var("PORT")
        .ok()
        .and_then(|p| p.parse().ok())
        .unwrap_or(8787);
    let app = handlers::router(store);
    let addr = format!("0.0.0.0:{port}");
    let listener = tokio::net::TcpListener::bind(&addr)
        .await
        .expect("failed to bind listen address");
    println!("agent-manager-vault listening on http://{addr}");
    axum::serve(listener, app).await.expect("server error");
}
