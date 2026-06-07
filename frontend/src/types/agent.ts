export type AgentStatus = 'running' | 'stopped' | 'error' | 'starting'

export interface AgentConfig {
  id: string
  name: string
  description: string
  command: string
  args: string[]
  working_dir: string
  env: Record<string, string>
  port?: number
  ui_token?: string
  auto_restart: boolean
  created_at: string
  updated_at: string
}

export interface AgentState {
  config: AgentConfig
  status: AgentStatus
  pid?: number
  started_at?: string
  port_open: boolean
}

export interface LogEntry {
  timestamp: string
  level: 'info' | 'warn' | 'error' | 'debug'
  message: string
}
