mod agent;
mod agent_http;
mod commands;
mod github;
mod llm;
mod mcp;
mod mcp_agent;
mod ports;
mod proxy;
mod pty;
mod scheduler;
mod sweeper;
mod ui_window;
mod updater;
mod workflow;
mod workflow_events;
mod workflow_store;

use commands::*;
use agent_http::*;
use github::*;
use llm::*;
use mcp::*;
use mcp_agent::*;
use ports::*;
use proxy::*;
use pty::*;
use ui_window::*;
use updater::*;
use workflow::*;
use workflow_events::*;
use workflow_store::*;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(agent::AgentStore::new())
        .manage(pty::PtyStore::new())
        .manage(ui_window::UiWebviewStore::new())
        .manage(proxy::TunnelStore::new())
        .manage(workflow_store::WorkflowRunStore::new())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_dialog::init())
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

            // 阶段四 P2：启动定时调度器
            scheduler::start_scheduler();

            // 阶段三 3c：启动 Sweeper 巡检
            sweeper::start_sweeper(_app.handle().clone());

            // 阶段三 3d：初始化事件写入器
            workflow_events::init_event_writer();

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
            // Workflow Run history (phase 1b)
            list_workflow_runs,
            list_hook_triggered_runs,
            get_workflow_run,
            // Workflow Events (phase 3d)
            list_workflow_events,
            // Workflow Metrics (phase 3f)
            get_workflow_metrics,
            // Workflow Acceptance/Rework (phase 2)
            approve_run,
            reject_run,
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
