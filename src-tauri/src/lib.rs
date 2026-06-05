mod agent;
mod commands;
mod mcp;
mod ports;

use commands::*;
use mcp::*;
use ports::*;

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .manage(agent::AgentStore::new())
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
            // Ports
            list_listening_ports,
            kill_port,
        ])
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}
