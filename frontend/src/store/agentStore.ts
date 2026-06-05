import { invoke } from '@tauri-apps/api/core'
import type { AgentConfig, AgentState, LogEntry } from '../types/agent'
import { create } from 'zustand'

const ORDER_KEY = 'agent-order'

function loadOrder(): string[] {
  try { return JSON.parse(localStorage.getItem(ORDER_KEY) ?? '[]') } catch { return [] }
}
function saveOrder(ids: string[]) {
  localStorage.setItem(ORDER_KEY, JSON.stringify(ids))
}

function applyOrder(agents: AgentState[], order: string[]): AgentState[] {
  if (!order.length) return agents
  const map = new Map(agents.map(a => [a.config.id, a]))
  const sorted: AgentState[] = []
  for (const id of order) {
    const a = map.get(id)
    if (a) sorted.push(a)
  }
  // 追加 order 里没有的新 agent
  for (const a of agents) {
    if (!order.includes(a.config.id)) sorted.push(a)
  }
  return sorted
}

interface AgentStore {
  agents: AgentState[]
  order: string[]
  selectedId: string | null
  logs: Record<string, LogEntry[]>
  loading: boolean
  fetchAgents: () => Promise<void>
  selectAgent: (id: string) => void
  reorderAgents: (newOrder: string[]) => void
  startAgent: (id: string) => Promise<void>
  stopAgent: (id: string) => Promise<void>
  saveAgent: (config: Partial<AgentConfig>) => Promise<void>
  deleteAgent: (id: string) => Promise<void>
  fetchLogs: (id: string) => Promise<void>
}

export const useAgentStore = create<AgentStore>((set, get) => ({
  agents: [],
  order: loadOrder(),
  selectedId: null,
  logs: {},
  loading: false,

  fetchAgents: async () => {
    set({ loading: true })
    try {
      const raw = await invoke<AgentState[]>('list_agents')
      const { order } = get()
      const agents = applyOrder(raw, order)
      set({ agents, loading: false })
    } catch {
      set({ loading: false })
    }
  },

  selectAgent: (id) => {
    set({ selectedId: id })
    get().fetchLogs(id)
  },

  reorderAgents: (newOrder) => {
    saveOrder(newOrder)
    set(s => ({ order: newOrder, agents: applyOrder(s.agents, newOrder) }))
  },

  startAgent: async (id) => {
    await invoke('start_agent', { id })
    await get().fetchAgents()
  },

  stopAgent: async (id) => {
    await invoke('stop_agent', { id })
    await get().fetchAgents()
  },

  saveAgent: async (config) => {
    await invoke('save_agent_config', { config })
    await get().fetchAgents()
  },

  deleteAgent: async (id) => {
    await invoke('delete_agent', { id })
    set(s => ({
      agents: s.agents.filter(a => a.config.id !== id),
      selectedId: s.selectedId === id ? null : s.selectedId,
      order: s.order.filter(oid => oid !== id),
    }))
    saveOrder(get().order)
  },

  fetchLogs: async (id) => {
    const logs = await invoke<LogEntry[]>('get_agent_logs', { id })
    set(s => ({ logs: { ...s.logs, [id]: logs } }))
  },
}))
