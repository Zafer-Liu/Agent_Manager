use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::process::Child;
use std::sync::{Arc, Mutex};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentConfig {
    pub id: String,
    pub name: String,
    pub description: String,
    pub command: String,
    pub args: Vec<String>,
    pub working_dir: String,
    pub env: HashMap<String, String>,
    pub port: Option<u16>,
    pub auto_restart: bool,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum AgentStatus {
    Running,
    Stopped,
    Error,
    Starting,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AgentState {
    pub config: AgentConfig,
    pub status: AgentStatus,
    pub pid: Option<u32>,
    pub started_at: Option<String>,
    pub port_open: bool,
}

pub struct AgentStore {
    pub agents: Mutex<HashMap<String, AgentState>>,
    pub logs: Arc<Mutex<HashMap<String, Vec<LogEntry>>>>,
    pub processes: Mutex<HashMap<String, Child>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LogEntry {
    pub timestamp: String,
    pub level: LogLevel,
    pub message: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum LogLevel {
    Info,
    Warn,
    Error,
    Debug,
}

impl AgentStore {
    pub fn new() -> Self {
        Self {
            agents: Mutex::new(HashMap::new()),
            logs: Arc::new(Mutex::new(HashMap::new())),
            processes: Mutex::new(HashMap::new()),
        }
    }
}
