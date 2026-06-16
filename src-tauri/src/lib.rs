mod agent;
mod commands;
mod mcp;
mod ports;
mod llm;
mod mcp_agent;
mod workflow;
mod pty;
mod proxy;
mod github;
mod ui_window;

use commands::*;
use mcp::*;
use ports::*;
use llm::*;
use mcp_agent::*;
use workflow::*;
use pty::*;
use proxy::*;
use github::*;
use ui_window::*;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(agent::AgentStore::new())
        .manage(pty::PtyStore::new())
        .manage(ui_window::UiWebviewStore::new())
        .manage(proxy::TunnelStore::new())
        .plugin(tauri_plugin_shell::init())
        .plugin(tauri_plugin_fs::init())
        .plugin(tauri_plugin_http::init())
        .plugin(tauri_plugin_dialog::init())
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
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
