#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if std::env::args().any(|argument| argument == "--mcp-memory") {
        if let Err(error) = agent_manager_lib::run_memory_mcp_stdio() {
            eprintln!("[memory-mcp] {error}");
            std::process::exit(1);
        }
        return;
    }
    agent_manager_lib::run();
}
