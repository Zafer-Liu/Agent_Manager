mod agent;
mod agent_http;
mod commands;
mod github;
mod llm;
mod mcp;
mod mcp_agent;
mod memory_backend;
mod memory_ingest;
mod memory_mcp;
mod ports;
mod proxy;
mod pty;
mod skill_registry;
mod sweeper;
mod telemetry_store;
mod thinking;
mod ui_window;
mod updater;
mod workflow;
mod workflow_events;
mod workflow_store;

use agent_http::*;
use commands::*;
use github::*;
use llm::*;
use mcp::*;
use mcp_agent::*;
use memory_backend::*;
use memory_ingest::*;
use memory_mcp::*;
use ports::*;
use proxy::*;
use pty::*;
use skill_registry::*;
use tauri::Manager;
use telemetry_store::*;
use ui_window::*;
use updater::*;
use workflow::*;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(agent::AgentStore::new())
        .manage(pty::PtyStore::new())
        .manage(ui_window::UiWebviewStore::new())
        .manage(proxy::TunnelStore::new())
        .manage(workflow_store::WorkflowRunStore::new())
        .manage(memory_backend::MemoryBackend::new())
        .manage(memory_ingest::IngestStore::new())
        .manage(telemetry_store::TelemetryStore::new().expect("initialize telemetry store"))
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_dialog::init())
        .on_window_event(|window, event| {
            // `Child` does not terminate its process on Drop.  If the optional
            // memory backend was explicitly started, stop only the children we
            // own when the main window is destroyed.
            if matches!(event, tauri::WindowEvent::Destroyed) {
                window
                    .app_handle()
                    .state::<memory_backend::MemoryBackend>()
                    .stop();
            }
        })
        .setup(|_app| {
            // 阶段四：初始化全局 AgentHttpStore 并从配置启动 HTTP server
            let store = agent_http::init_store();
            let config = agent_http::read_hook_server_config();
            if config.enabled {
                let port = config.port;
                let token = config.auth_token.clone();
                let h = tauri::async_runtime::spawn(async move {
                    agent_http::start_agent_http_server(port, token, store).await;
                });
                // 记录 server handle（restart 用）
                let handle = agent_http::get_server_handle();
                {
                    let mut guard = handle.lock().unwrap();
                    *guard = Some(h);
                }
            } else {
                eprintln!("[agent-http] disabled by config");
            }

            // 阶段三 3c：启动 Sweeper 巡检
            sweeper::start_sweeper(_app.handle().clone());

            // Memory extraction is optional.  Keep event collection local and
            // durable even when its external vector/graph services are absent;
            // starting those services is an explicit user action from the UI.
            let memory = _app
                .state::<memory_backend::MemoryBackend>()
                .inner()
                .clone();
            let shared = memory_backend::init_shared(memory);
            let ingest = _app.state::<memory_ingest::IngestStore>().inner().clone();
            memory_ingest::init_ingest(ingest.clone());
            let telemetry = _app
                .state::<telemetry_store::TelemetryStore>()
                .inner()
                .clone();
            telemetry_store::init_shared(telemetry);
            // Populate local indexes after the window can paint.  These jobs
            // are idempotent and write their result to SQLite, so opening a
            // dashboard never needs to re-walk agent directories.
            tauri::async_runtime::spawn_blocking(|| {
                let _ = skill_registry::skill_list();
                let _ = memory_ingest::telemetry_backfill_conversations();
                if let Some(store) = telemetry_store::shared_store() {
                    let _ = store.refresh_transcript_usage();
                }
            });
            let ingest2 = ingest.clone();
            // 静默会话节流巡检
            memory_ingest::start_idle_flusher(shared, ingest2);

            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_agents,
            start_agent,
            stop_agent,
            get_agent_logs,
            save_agent_config,
            delete_agent,
            get_port_status,
            scan_project_dir,
            // MCP
            list_mcp_servers,
            save_mcp_server,
            delete_mcp_server,
            get_mcp_config_path,
            scan_mcp_local,
            parse_mcp_text,
            // Ports
            list_listening_ports,
            kill_port,
            // LLM config
            list_llm_providers,
            save_llm_provider,
            delete_llm_provider,
            test_llm_provider,
            memory_extraction_config_get,
            memory_extraction_config_set,
            // MCP Agent
            run_mcp_agent,
            chat_with_mcp,
            manager_chat,
            // Workflow
            list_workflows,
            save_workflow,
            delete_workflow,
            list_mcp_tools,
            run_workflow,
            run_workflow_stream,
            // Agent HTTP spike (phase 4)
            dispatch_agent_task,
            list_agent_tasks,
            list_agent_results,
            // Agent HTTP config (phase 4 - 4h)
            get_hook_server_config,
            set_hook_server_config,
            restart_hook_server,
            // PTY terminal
            pty_start,
            pty_write,
            pty_resize,
            pty_stop,
            pty_resolve_debug,
            // Proxy (Caddy)
            proxy_get_config,
            proxy_save_config,
            proxy_check_caddy,
            proxy_hash_password,
            proxy_apply,
            proxy_stop,
            proxy_status,
            proxy_get_caddyfile,
            proxy_preview_caddyfile,
            // Cloudflare Tunnel
            tunnel_check_cloudflared,
            tunnel_start,
            tunnel_stop,
            tunnel_stop_all,
            tunnel_list,
            tunnel_alive,
            // GitHub install
            github_fetch_repo_info,
            github_clone_repo,
            github_check_git,
            github_get_proxy,
            github_save_token,
            github_token_status,
            // Agent UI window
            open_agent_ui_webview,
            update_agent_ui_webview,
            fullscreen_agent_ui_webview,
            close_agent_ui_webview,
            // Updater
            check_for_update,
            get_app_version,
            // Memory engine
            memory_backend_status,
            memory_backend_start,
            memory_backend_stop,
            memory_backend_request,
            memory_backend::memory_consolidate,
            memory_backend::memory_consolidation_restore,
            memory_backend::memory_importance_refresh,
            memory_backend::memory_importance_list,
            memory_backend::memory_importance_set_pinned,
            // Memory ingest (hook 自动沉淀)
            memory_hook_install,
            memory_hook_uninstall,
            memory_hook_status,
            memory_ingest_status,
            memory_ingest_set_enabled,
            memory_ingest_flush_pending,
            memory_ingest::memory_ingest_organize_conversations,
            memory_ingest::memory_import_folder,
            memory_ingest::local_memory_list,
            memory_ingest::local_memory_stats,
            memory_ingest::local_memory_reset_for_reextraction,
            memory_ingest::local_memory_search,
            memory_ingest::local_memory_update,
            memory_ingest::local_memory_delete,
            memory_ingest::memory_workspace_summary_get,
            memory_ingest::memory_workspace_summary_refresh,
            memory_ingest::memory_profile_summary_get,
            memory_ingest::memory_profile_summary_refresh,
            memory_ingest::memory_short_term_consolidate,
            memory_ingest::memory_long_term_profile_draft,
            memory_ingest::memory_long_term_profile_publish,
            memory_ingest::memory_long_term_profile_delete_draft,
            memory_ingest::memory_layer_documents,
            // Shared memory MCP (Codex / Claude one-click setup)
            memory_mcp_status,
            memory_mcp_install,
            memory_mcp_uninstall,
            // Usage and event telemetry
            telemetry_summary,
            telemetry_refresh_usage,
            telemetry_live_status,
            telemetry_recent_events,
            telemetry_usage_records,
            telemetry_usage_analytics,
            telemetry_search_conversations,
            memory_mcp_access_logs,
            memory_ingest::telemetry_backfill_conversations,
            // Shared Skill registry
            skill_scan,
            skill_list,
            skill_read,
            skill_sync_preview,
            skill_sync_apply,
            skill_set_status,
            skill_set_assignment,
            skill_rollback_latest,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

/// Separate from the Tauri desktop runtime so the same installed executable
/// can act as a stdio MCP server for Codex and Claude.
pub fn run_memory_mcp_stdio() -> Result<(), String> {
    memory_mcp::run_stdio()
}
