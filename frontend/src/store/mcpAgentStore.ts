import { create } from 'zustand'

// ── Shared types (kept in sync with McpAgent.tsx) ────────────────────────────

export interface AgentStep {
  kind: 'thought' | 'toolcall' | 'toolresult' | 'answer' | 'error'
  content: string
  tool?: string
  tool_input?: unknown
}

/** 阶段二：对话流内嵌验收请求（来自 workflow-acceptance-requested 事件） */
export interface ChatAcceptance {
  runId: string
  nodeId: string
  label: string
  allowRejectTo: string[]
  executedNodeIds: string[]
}

export interface ChatMessage {
  role: 'user' | 'assistant'
  content: string
  steps?: AgentStep[]
  // Workflow-mode metadata
  isWorkflow?: boolean
  workflowName?: string
  workflowSteps?: WorkflowStepSummary[]
  // 验收卡片（assistant 消息附带）
  acceptance?: ChatAcceptance
  // 验收结果反馈
  acceptanceResult?: 'approved' | 'rejected'
  acceptanceRejectTo?: string
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
  pendingAcceptance: ChatAcceptance | null  // 当前待验收请求

  setMessages: (updater: ChatMessage[] | ((prev: ChatMessage[]) => ChatMessage[])) => void
  clearMessages: () => void
  setSelectedProvider: (id: string) => void
  setEnabledServers: (updater: string[] | ((prev: string[]) => string[])) => void
  toggleServer: (name: string) => void
  setSelectedWorkflowId: (id: string) => void
  setPendingAcceptance: (req: ChatAcceptance | null) => void
}

export const useMcpAgentStore = create<McpAgentStore>((set) => ({
  messages: [],
  selectedProvider: loadSelectedProvider(),
  enabledServers: loadEnabledServers(),
  selectedWorkflowId: loadSelectedWorkflow(),
  pendingAcceptance: null,

  setMessages: (updater) =>
    set((s) => ({
      messages: typeof updater === 'function' ? updater(s.messages) : updater,
    })),

  clearMessages: () => set({ messages: [], pendingAcceptance: null }),

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

  setPendingAcceptance: (req) => set({ pendingAcceptance: req }),
}))
