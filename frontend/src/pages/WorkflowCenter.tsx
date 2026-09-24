import { useCallback, useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { WorkflowBuilder } from './WorkflowBuilder'
import type { McpServer } from '../components/McpServersManager'

// 与旧 mcpAgentStore 相同的 localStorage 键，沿用用户已启用的服务器选择
const LS_ENABLED_SERVERS = 'mcp-enabled-servers'

function loadEnabledServers(): string[] {
  try {
    const raw = localStorage.getItem(LS_ENABLED_SERVERS)
    if (!raw) return []
    const parsed = JSON.parse(raw)
    return Array.isArray(parsed) ? parsed : []
  } catch {
    return []
  }
}

/** 工作流中心：DAG 画布、定时触发、外部 Hook 与运行的独立入口 */
export function WorkflowCenter() {
  const [mcpServers, setMcpServers] = useState<McpServer[]>([])
  const [enabledServers, setEnabledServers] = useState<string[]>(loadEnabledServers)

  useEffect(() => {
    invoke<McpServer[]>('list_mcp_servers').then(setMcpServers).catch(() => {})
  }, [])

  const toggleServer = useCallback((name: string) => {
    setEnabledServers(prev => {
      const next = prev.includes(name)
        ? prev.filter(n => n !== name)
        : [...prev, name]
      try { localStorage.setItem(LS_ENABLED_SERVERS, JSON.stringify(next)) } catch { /* ignore */ }
      return next
    })
  }, [])

  const selectedMcpServers = mcpServers.filter(s => enabledServers.includes(s.name))

  return (
    <WorkflowBuilder
      enabledServers={selectedMcpServers}
      allServers={mcpServers}
      enabledNames={enabledServers}
      onToggleServer={toggleServer}
    />
  )
}
