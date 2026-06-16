import { create } from 'zustand'

// ── Shared types (kept in sync with McpAgent.tsx) ────────────────────────────

export interface AgentStep {
  kind: 'thought' | 'toolcall' | 'toolresult' | 'answer' | 'error'
  content: string
  tool?: string
  tool_input?: unknown
}

export interface ChatMessage {
  role: 'user' | 'assistant'
  content: string
  steps?: AgentStep[]
  // Workflow-mode metadata
  isWorkflow?: boolean
  workflowName?: string
  workflowSteps?: WorkflowStepSummary[]
}

export interface WorkflowStepSummary {
  label: string
  kind: string
  output: string
  error?: string
}

// ── Persistence helpers ───────────────────────────────────────────────────────

const LS_ENABLED_SERVERS    = 'mcp-enabled-servers'
const LS_SELECTED_PROVIDER  = 'mcp-selected-provider'
const LS_SELECTED_WORKFLOW  = 'mcp-selected-workflow'

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

function saveEnabledServers(servers: string[]) {
  try { localStorage.setItem(LS_ENABLED_SERVERS, JSON.stringify(servers)) } catch { /* ignore */ }
}

function loadSelectedProvider(): string {
  try { return localStorage.getItem(LS_SELECTED_PROVIDER) ?? '' } catch { return '' }
}

function saveSelectedProvider(id: string) {
  try { localStorage.setItem(LS_SELECTED_PROVIDER, id) } catch { /* ignore */ }
}

function loadSelectedWorkflow(): string {
  try { return localStorage.getItem(LS_SELECTED_WORKFLOW) ?? '' } catch { return '' }
}

function saveSelectedWorkflow(id: string) {
  try { localStorage.setItem(LS_SELECTED_WORKFLOW, id) } catch { /* ignore */ }
}

// ── Store ────────────────────────────────────────────────────────────────────

interface McpAgentStore {
  messages: ChatMessage[]
  selectedProvider: string
  enabledServers: string[]
  selectedWorkflowId: string   // '' = no workflow (plain chat)

  setMessages: (updater: ChatMessage[] | ((prev: ChatMessage[]) => ChatMessage[])) => void
  clearMessages: () => void
  setSelectedProvider: (id: string) => void
  setEnabledServers: (updater: string[] | ((prev: string[]) => string[])) => void
  toggleServer: (name: string) => void
  setSelectedWorkflowId: (id: string) => void
}

export const useMcpAgentStore = create<McpAgentStore>((set) => ({
  messages: [],
  selectedProvider: loadSelectedProvider(),
  enabledServers: loadEnabledServers(),
  selectedWorkflowId: loadSelectedWorkflow(),

  setMessages: (updater) =>
    set((s) => ({
      messages: typeof updater === 'function' ? updater(s.messages) : updater,
    })),

  clearMessages: () => set({ messages: [] }),

  setSelectedProvider: (id) => {
    saveSelectedProvider(id)
    set({ selectedProvider: id })
  },

  setEnabledServers: (updater) =>
    set((s) => {
      const next = typeof updater === 'function' ? updater(s.enabledServers) : updater
      saveEnabledServers(next)
      return { enabledServers: next }
    }),

  toggleServer: (name) =>
    set((s) => {
      const next = s.enabledServers.includes(name)
        ? s.enabledServers.filter((n) => n !== name)
        : [...s.enabledServers, name]
      saveEnabledServers(next)
      return { enabledServers: next }
    }),

  setSelectedWorkflowId: (id) => {
    saveSelectedWorkflow(id)
    set({ selectedWorkflowId: id })
  },
}))
