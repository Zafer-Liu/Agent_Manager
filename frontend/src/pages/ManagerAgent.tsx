import { useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import {
  Send, Loader2, Crown, AlertCircle, Settings,
  Trash2, ChevronDown, ChevronUp,
  Play, Square, ExternalLink, Zap, Moon, Bot,
  MessageSquare, TerminalSquare, Navigation, Cpu,
  CheckCircle2, Activity,
} from 'lucide-react'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { useTranslation } from 'react-i18next'
import type { AgentState } from '../types/agent'
import { stripThinkingBlocks } from '../lib/thinking'

// ── Types ────────────────────────────────────────────────────────────────────

export interface LlmProvider {
  id: string; name: string; model: string; base_url: string
  api_key: string; is_custom: boolean; enabled: boolean
}

export interface AgentStep {
  kind: 'thought' | 'toolcall' | 'toolresult' | 'answer' | 'error'
  content: string; tool?: string; tool_input?: unknown
}

export type ManagerAction =
  | { type: 'open_ui'; agentId: string; agentName: string }
  | { type: 'open_terminal'; agentId: string; agentName: string }
  | { type: 'start_agent'; agentId: string; agentName: string }
  | { type: 'stop_agent'; agentId: string; agentName: string }
  | { type: 'navigate'; page: string }
  | { type: 'send_message'; agentId: string; agentName: string; message: string; port: number }

export interface ManagerChatMessage {
  role: 'user' | 'assistant'
  content: string
  steps?: AgentStep[]
  actions?: ManagerAction[]
}

export interface ManagerSessionState {
  messages: ManagerChatMessage[]
  selectedProvider: string
}

interface ManagerAgentProps {
  agents: AgentState[]
  session: ManagerSessionState
  onSessionChange: (s: ManagerSessionState) => void
  onOpenAgentUI: (agent: AgentState) => void
  onOpenAgentTerminal: (agent: AgentState) => void
  onStartAgent: (id: string) => Promise<void>
  onStopAgent: (id: string) => Promise<void>
  onNavigate: (page: string) => void
}

// ── Action extractor (from tool result steps) ────────────────────────────────

function extractActionsFromSteps(steps: AgentStep[]): ManagerAction[] {
  const actions: ManagerAction[] = []
  for (const step of steps) {
    if (step.kind !== 'toolresult') continue
    const c = step.content
    if (!c.startsWith('__action__:')) continue
    // format: __action__:type:id:name  (navigate has empty id, name=page)
    const parts = c.slice('__action__:'.length).split(':')
    const type = parts[0]
    const agentId = parts[1] ?? ''
    const rest = parts.slice(2).join(':')  // name or page may contain colons
    if (type === 'start_agent')   actions.push({ type: 'start_agent',   agentId, agentName: rest })
    if (type === 'stop_agent')    actions.push({ type: 'stop_agent',    agentId, agentName: rest })
    if (type === 'open_ui')       actions.push({ type: 'open_ui',       agentId, agentName: rest })
    if (type === 'open_terminal') actions.push({ type: 'open_terminal', agentId, agentName: rest })
    if (type === 'navigate')      actions.push({ type: 'navigate',      page: rest })
  }
  return actions
}

// ── ManagerAgent ─────────────────────────────────────────────────────────────

export function ManagerAgent({
  agents,
  session,
  onSessionChange,
  onOpenAgentUI,
  onOpenAgentTerminal,
  onStartAgent,
  onStopAgent,
  onNavigate,
}: ManagerAgentProps) {
  const { t } = useTranslation()
  const [providers, setProviders] = useState<LlmProvider[]>([])
  const [input, setInput] = useState('')
  const [sending, setSending] = useState(false)
  const [executingActions, setExecutingActions] = useState(false)
  const [showSettings, setShowSettings] = useState(false)
  const bottomRef = useRef<HTMLDivElement>(null)
  const textareaRef = useRef<HTMLTextAreaElement>(null)

  const { messages, selectedProvider } = session

  function setSelectedProvider(id: string) {
    onSessionChange({ ...session, selectedProvider: id })
  }

  useEffect(() => {
    invoke<LlmProvider[]>('list_llm_providers').then(ps => {
      setProviders(ps)
      if (!session.selectedProvider) {
        const active = ps.find(p => p.enabled && p.api_key)
        if (active) setSelectedProvider(active.id)
      }
    })
    // Provider discovery is initialization-only; session updates must not
    // refetch providers or overwrite the user's current selection.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' })
  }, [messages, sending])

  const activeProvider = providers.find(p => p.id === selectedProvider) ?? null
  const availableProviders = providers.filter(p => p.enabled && p.api_key)

  function resolveAgent(agentId: string, agentName?: string): AgentState | undefined {
    return (
      agents.find(a => a.config.id === agentId) ??
      agents.find(a => a.config.name === agentName) ??
      agents.find(a => a.config.id === agentName) ??
      agents.find(a => a.config.name.toLowerCase().replace(/\s+/g, '_') === agentId.toLowerCase()) ??
      agents.find(a => a.config.name.toLowerCase().includes(agentId.toLowerCase())) ??
      agents.find(a => agentId.toLowerCase().includes(a.config.name.toLowerCase()))
    )
  }

  async function executeActions(actions: ManagerAction[]): Promise<string[]> {
    setExecutingActions(true)
    let shouldNavigateToAgents = false
    const results: string[] = []
    try {
      for (const action of actions) {
        if (action.type === 'open_ui') {
          const agent = resolveAgent(action.agentId, action.agentName)
          if (agent) {
            onOpenAgentUI(agent)
            shouldNavigateToAgents = true
            results.push(`✓ ${t('manager.actionOpenUi')} · ${action.agentName}: ${t('manager.actionSuccess')}`)
          } else {
            results.push(`✗ ${t('manager.actionOpenUi')} · ${action.agentName}: ${t('manager.actionAgentNotFound')}`)
          }
        } else if (action.type === 'open_terminal') {
          const agent = resolveAgent(action.agentId, action.agentName)
          if (agent) {
            onOpenAgentTerminal(agent)
            shouldNavigateToAgents = true
            results.push(`✓ ${t('manager.actionOpenTerminal')} · ${action.agentName}: ${t('manager.actionSuccess')}`)
          } else {
            results.push(`✗ ${t('manager.actionOpenTerminal')} · ${action.agentName}: ${t('manager.actionAgentNotFound')}`)
          }
        } else if (action.type === 'start_agent') {
          const agent = resolveAgent(action.agentId, action.agentName)
          if (agent) {
            try {
              await onStartAgent(agent.config.id)
              results.push(`✓ ${t('manager.actionStart')} · ${action.agentName}: ${t('manager.actionSuccess')}`)
            } catch (e) {
              results.push(`✗ ${t('manager.actionStart')} · ${action.agentName}: ${t('manager.actionFailed', { error: String(e) })}`)
            }
          } else {
            results.push(`✗ ${t('manager.actionStart')} · ${action.agentName}: ${t('manager.actionAgentNotFound')}`)
          }
        } else if (action.type === 'stop_agent') {
          const agent = resolveAgent(action.agentId, action.agentName)
          if (agent) {
            try {
              await onStopAgent(agent.config.id)
              results.push(`✓ ${t('manager.actionStop')} · ${action.agentName}: ${t('manager.actionSuccess')}`)
            } catch (e) {
              results.push(`✗ ${t('manager.actionStop')} · ${action.agentName}: ${t('manager.actionFailed', { error: String(e) })}`)
            }
          } else {
            results.push(`✗ ${t('manager.actionStop')} · ${action.agentName}: ${t('manager.actionAgentNotFound')}`)
          }
        } else if (action.type === 'navigate') {
          onNavigate(action.page)
          results.push(`✓ ${t('manager.actionNavigate')} · ${action.page}`)
        } else if (action.type === 'send_message') {
          try {
            const resp = await fetch(`http://localhost:${action.port}/message`, {
              method: 'POST',
              headers: { 'Content-Type': 'application/json' },
              body: JSON.stringify({ message: action.message }),
            })
            if (resp.ok) {
              results.push(`✓ ${t('manager.actionSendMessage')} · ${action.agentName}: ${t('manager.actionSuccess')}`)
            } else {
              results.push(`✗ ${t('manager.actionSendMessage')} · ${action.agentName}: ${t('manager.actionFailed', { error: `HTTP ${resp.status}` })}`)
            }
          } catch (e) {
            results.push(`✗ ${t('manager.actionSendMessage')} · ${action.agentName}: ${t('manager.actionFailed', { error: String(e) })}`)
          }
        }
      }
    } finally {
      setExecutingActions(false)
      if (shouldNavigateToAgents) onNavigate('agents')
    }
    return results
  }

  async function send() {
    if (!input.trim() || !activeProvider || sending) return
    const text = input.trim()
    setInput('')
    // auto-resize textarea back
    if (textareaRef.current) textareaRef.current.style.height = 'auto'

    const userMsg: ManagerChatMessage = { role: 'user', content: text }
    const nextMessages = [...messages, userMsg]
    onSessionChange({ ...session, messages: nextMessages })
    setSending(true)

    try {
      const history = nextMessages.map(m => ({ role: m.role, content: m.content }))
      const agentInfos = agents.map(a => ({
        id: a.config.id,
        name: a.config.name,
        status: a.status,
        port: a.config.port ?? null,
        description: a.config.description,
        port_open: a.port_open,
        working_dir: a.config.working_dir,
        command: a.config.command,
        args: a.config.args,
      }))

      const result = await invoke<{ steps: AgentStep[]; reply: string; success: boolean; error?: string }>('manager_chat', {
        request: { history, provider: activeProvider, agents: agentInfos, max_iterations: 8 },
      })

      const rawReply = stripThinkingBlocks(result.reply || (result.error ?? t('manager.noResponse')))

      // Extract actions from tool results (embedded as __action__:type:id:name)
      const toolActions = extractActionsFromSteps(result.steps)

      const assistantMsg: ManagerChatMessage = {
        role: 'assistant',
        content: rawReply,
        steps: result.steps.length ? result.steps : undefined,
        actions: toolActions.length ? toolActions : undefined,
      }

      if (toolActions.length > 0) {
        const actionResults = await executeActions(toolActions)
        if (actionResults.length > 0) {
          const feedbackMsg: ManagerChatMessage = {
            role: 'assistant',
            content: `${t('manager.actionResultsTitle')}\n\n${actionResults.join('\n')}`,
          }
          onSessionChange({ ...session, messages: [...nextMessages, assistantMsg, feedbackMsg] })
        } else {
          onSessionChange({ ...session, messages: [...nextMessages, assistantMsg] })
        }
      } else {
        onSessionChange({ ...session, messages: [...nextMessages, assistantMsg] })
      }
    } catch (e) {
      const errMsg: ManagerChatMessage = { role: 'assistant', content: t('manager.errorPrefix', { error: String(e) }) }
      onSessionChange({ ...session, messages: [...nextMessages, errMsg] })
    } finally {
      setSending(false)
    }
  }

  function handleKeyDown(e: React.KeyboardEvent<HTMLTextAreaElement>) {
    if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); send() }
  }

  function autoResize(e: React.ChangeEvent<HTMLTextAreaElement>) {
    setInput(e.target.value)
    e.target.style.height = 'auto'
    e.target.style.height = Math.min(e.target.scrollHeight, 160) + 'px'
  }

  const runningCount = agents.filter(a => a.status === 'running').length

  return (
    <div className="flex h-full flex-col bg-gray-50 dark:bg-gray-950">

      {/* ── Top bar ── */}
      <div className="shrink-0 border-b border-gray-200 dark:border-gray-800 bg-white dark:bg-gray-900 px-5 py-3">
        <div className="flex items-center justify-between">
          {/* Left: title + agent summary */}
          <div className="flex items-center gap-3">
            <div className="flex h-8 w-8 items-center justify-center rounded-xl bg-gradient-to-br from-amber-400 to-orange-500 shadow-sm">
              <Crown className="h-4 w-4 text-white" />
            </div>
            <div>
              <p className="text-sm font-semibold text-gray-900 dark:text-gray-100 leading-none">Manager Agent</p>
              <p className="mt-0.5 text-xs text-gray-400 dark:text-gray-500">
                {agents.length} agents · {runningCount} {t('manager.running')}
              </p>
            </div>
          </div>

          {/* Right: LLM status + controls */}
          <div className="flex items-center gap-2">
            {activeProvider ? (
              <span className="hidden sm:flex items-center gap-1.5 rounded-full bg-green-50 dark:bg-green-900/20 px-2.5 py-1 text-xs font-medium text-green-700 dark:text-green-400">
                <span className="h-1.5 w-1.5 rounded-full bg-green-400" />
                {activeProvider.name}
              </span>
            ) : (
              <span className="flex items-center gap-1 rounded-full bg-yellow-50 dark:bg-yellow-900/20 px-2.5 py-1 text-xs font-medium text-yellow-700 dark:text-yellow-400">
                <AlertCircle className="h-3 w-3" />{t('manager.notConfigured')}
              </span>
            )}
            {messages.length > 0 && (
              <button
                onClick={() => onSessionChange({ ...session, messages: [] })}
                className="rounded-lg p-1.5 text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-800 hover:text-red-500 transition-colors"
                title={t('manager.clearChat')}
              >
                <Trash2 className="h-4 w-4" />
              </button>
            )}
            <button
              onClick={() => setShowSettings(s => !s)}
              className={`rounded-lg p-1.5 transition-colors ${showSettings ? 'bg-gray-100 dark:bg-gray-800 text-gray-700 dark:text-gray-300' : 'text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-800'}`}
              title={t('manager.settings')}
            >
              <Settings className="h-4 w-4" />
            </button>
          </div>
        </div>

        {/* Settings dropdown */}
        {showSettings && (
          <div className="mt-3 flex items-center gap-3 rounded-xl bg-gray-50 dark:bg-gray-800/60 border border-gray-200 dark:border-gray-700 p-3">
            <span className="text-xs font-medium text-gray-500 shrink-0">{t('manager.model')}</span>
            {availableProviders.length > 0 ? (
              <select
                value={selectedProvider}
                onChange={e => setSelectedProvider(e.target.value)}
                className="flex-1 rounded-lg border border-gray-200 bg-white px-2 py-1.5 text-xs dark:border-gray-700 dark:bg-gray-900 dark:text-gray-200 outline-none"
              >
                {availableProviders.map(p => (
                  <option key={p.id} value={p.id}>{p.name} · {p.model}</option>
                ))}
              </select>
            ) : (
              <span className="text-xs text-yellow-600">{t('manager.configureHint')}</span>
            )}
          </div>
        )}
      </div>

      {/* ── Agent pills ── */}
      {agents.length > 0 && (
        <div className="shrink-0 border-b border-gray-100 dark:border-gray-800/60 bg-white/60 dark:bg-gray-900/60 px-5 py-2 flex items-center gap-1.5 flex-wrap">
          {agents.map(a => <AgentPill key={a.config.id} agent={a} />)}
        </div>
      )}

      {/* ── Messages ── */}
      <div className="flex-1 overflow-y-auto">
        {messages.length === 0 ? (
          <EmptyState agents={agents} onPrompt={p => { setInput(p); textareaRef.current?.focus() }} />
        ) : (
          <div className="px-6 py-6 space-y-6">
            {messages.map((msg, i) => (
              <MessageBubble key={i} msg={msg} />
            ))}

            {(sending || executingActions) && (
              <div className="flex items-start gap-3">
                <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-xl bg-gradient-to-br from-amber-400 to-orange-500 shadow-sm">
                  <Crown className="h-4 w-4 text-white" />
                </div>
                <div className="flex items-center gap-2 rounded-2xl rounded-tl-sm bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 px-4 py-3 shadow-sm">
                  <Loader2 className="h-4 w-4 animate-spin text-amber-500" />
                  <span className="text-sm text-gray-500 dark:text-gray-400">
                    {executingActions ? t('manager.executing') : t('manager.thinking')}
                  </span>
                </div>
              </div>
            )}
            <div ref={bottomRef} />
          </div>
        )}
      </div>

      {/* ── Input area ── */}
      <div className="shrink-0 border-t border-gray-200 dark:border-gray-800 bg-white dark:bg-gray-900 px-6 py-4">
        <div>
          <div className={`flex items-end gap-2 rounded-2xl border bg-white dark:bg-gray-800 px-4 py-3 shadow-sm transition-all ${
            activeProvider
              ? 'border-gray-200 dark:border-gray-700 focus-within:border-amber-400 dark:focus-within:border-amber-600'
              : 'border-gray-200 dark:border-gray-700 opacity-60'
          }`}>
            <textarea
              ref={textareaRef}
              value={input}
              onChange={autoResize}
              onKeyDown={handleKeyDown}
              placeholder={activeProvider ? t('manager.inputPlaceholderFull') : t('manager.inputPlaceholderNoLlm')}
              rows={1}
              disabled={sending || !activeProvider}
              className="flex-1 resize-none bg-transparent text-sm text-gray-800 dark:text-gray-100 placeholder-gray-400 outline-none disabled:cursor-not-allowed"
              style={{ minHeight: '24px', maxHeight: '160px' }}
            />
            <button
              onClick={send}
              disabled={sending || !input.trim() || !activeProvider}
              className="shrink-0 flex h-8 w-8 items-center justify-center rounded-xl bg-amber-500 text-white hover:bg-amber-400 disabled:opacity-40 disabled:cursor-not-allowed transition-all hover:scale-105 active:scale-95"
            >
              {sending ? <Loader2 className="h-4 w-4 animate-spin" /> : <Send className="h-4 w-4" />}
            </button>
          </div>
          <p className="mt-1.5 text-center text-[11px] text-gray-400 dark:text-gray-600">
            {t('manager.footerHint')}
          </p>
        </div>
      </div>
    </div>
  )
}

// ── EmptyState ────────────────────────────────────────────────────────────────

function EmptyState({ agents, onPrompt }: { agents: AgentState[]; onPrompt: (p: string) => void }) {
  const { t } = useTranslation()
  const examples = [
    { icon: <Activity className="h-4 w-4" />, text: t('manager.exampleStatus') },
    { icon: <Play className="h-4 w-4" />, text: t('manager.exampleStartAll') },
    { icon: <ExternalLink className="h-4 w-4" />, text: agents[0] ? t('manager.exampleOpenUi', { name: agents[0].config.name }) : t('manager.exampleOpenUiGeneric') },
    { icon: <Navigation className="h-4 w-4" />, text: t('manager.exampleNavigate') },
  ]

  return (
    <div className="flex h-full flex-col items-center justify-center gap-8 px-4 py-12">
      {/* Icon */}
      <div className="relative">
        <div className="flex h-24 w-24 items-center justify-center rounded-3xl bg-gradient-to-br from-amber-400 to-orange-500 shadow-xl">
          <Crown className="h-12 w-12 text-white" />
        </div>
        <div className="absolute -bottom-1 -right-1 flex h-7 w-7 items-center justify-center rounded-full bg-green-400 shadow-md">
          <CheckCircle2 className="h-4 w-4 text-white" />
        </div>
      </div>

      {/* Text */}
      <div className="text-center space-y-2">
        <h2 className="text-lg font-semibold text-gray-800 dark:text-gray-100">{t('manager.ready')}</h2>
        <p className="text-sm text-gray-500 dark:text-gray-400 max-w-xs">
          {t('manager.readyDesc')}
        </p>
      </div>

      {/* Example prompts */}
      <div className="w-full max-w-sm space-y-2">
        <p className="text-xs font-medium text-gray-400 dark:text-gray-600 uppercase tracking-wide mb-3">{t('manager.tryThese')}</p>
        {examples.map((ex, i) => (
          <button
            key={i}
            onClick={() => onPrompt(ex.text)}
            className="flex w-full items-center gap-3 rounded-xl border border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-800/60 px-4 py-3 text-sm text-left text-gray-700 dark:text-gray-300 hover:bg-amber-50 dark:hover:bg-amber-900/10 hover:border-amber-300 dark:hover:border-amber-700 transition-all group"
          >
            <span className="text-gray-400 dark:text-gray-500 group-hover:text-amber-500 transition-colors">
              {ex.icon}
            </span>
            {ex.text}
          </button>
        ))}
      </div>
    </div>
  )
}

// ── MessageBubble ─────────────────────────────────────────────────────────────

function MessageBubble({ msg }: { msg: ManagerChatMessage }) {
  if (msg.role === 'user') {
    return (
      <div className="flex justify-end">
        <div className="max-w-[60%] rounded-2xl rounded-tr-sm bg-amber-500 px-4 py-3 shadow-sm">
          <p className="text-sm text-white whitespace-pre-wrap leading-relaxed">{msg.content}</p>
        </div>
      </div>
    )
  }

  // assistant
  return (
    <div className="flex items-start gap-3">
      {/* Avatar */}
      <div className="mt-0.5 flex h-8 w-8 shrink-0 items-center justify-center rounded-xl bg-gradient-to-br from-amber-400 to-orange-500 shadow-sm">
        <Crown className="h-4 w-4 text-white" />
      </div>

      <div className="flex-1 space-y-2 min-w-0 max-w-[85%]">
        {/* Tool call steps */}
        {msg.steps && msg.steps.length > 0 && (
          <ToolCallGroup steps={msg.steps} />
        )}

        {/* Main reply */}
        {msg.content && (
          <div className="rounded-2xl rounded-tl-sm bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 px-4 py-3 shadow-sm text-sm text-gray-800 dark:text-gray-100">
            <MdContent content={msg.content} />
          </div>
        )}

        {/* Action chips */}
        {msg.actions && msg.actions.length > 0 && (
          <ActionRow actions={msg.actions} />
        )}
      </div>
    </div>
  )
}

// ── ToolCallGroup ─────────────────────────────────────────────────────────────

function ToolCallGroup({ steps }: { steps: AgentStep[] }) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)

  const toolCalls = steps.filter(s => s.kind === 'toolcall')
  const toolResults = steps.filter(s => s.kind === 'toolresult')
  const errors = steps.filter(s => s.kind === 'error')

  if (toolCalls.length === 0 && errors.length === 0) return null

  return (
    <div className="rounded-xl border border-amber-200 dark:border-amber-800/60 bg-amber-50/60 dark:bg-amber-900/10 overflow-hidden">
      {/* Header row */}
      <button
        onClick={() => setExpanded(e => !e)}
        className="flex w-full items-center gap-2 px-3 py-2 text-left hover:bg-amber-100/50 dark:hover:bg-amber-900/20 transition-colors"
      >
        <Activity className="h-3.5 w-3.5 text-amber-600 dark:text-amber-400 shrink-0" />
        <span className="flex-1 text-xs font-medium text-amber-700 dark:text-amber-400">
          {toolCalls.map(tc => tc.tool).filter(Boolean).join(' · ')}
          {errors.length > 0 && <span className="ml-1 text-red-500">· {t('manager.errorsCount', { count: errors.length })}</span>}
        </span>
        {expanded
          ? <ChevronUp className="h-3.5 w-3.5 text-amber-500 shrink-0" />
          : <ChevronDown className="h-3.5 w-3.5 text-amber-500 shrink-0" />
        }
      </button>

      {/* Expanded content */}
      {expanded && (
        <div className="border-t border-amber-200 dark:border-amber-800/60 px-3 py-2 space-y-2">
          {errors.map((s, i) => (
            <div key={i} className="flex items-start gap-2 rounded-lg bg-red-50 dark:bg-red-900/20 px-3 py-2">
              <AlertCircle className="h-3.5 w-3.5 text-red-500 shrink-0 mt-0.5" />
              <p className="text-xs text-red-600 dark:text-red-400">{s.content}</p>
            </div>
          ))}
          {toolResults.map((s, i) => (
            <div key={i} className="space-y-0.5">
              <p className="text-[10px] font-mono text-amber-600 dark:text-amber-500">{s.tool} {t('manager.resultSuffix')}</p>
              <pre className="overflow-x-auto rounded-lg bg-gray-900 dark:bg-black px-3 py-2 text-[11px] font-mono text-green-300 whitespace-pre-wrap">{s.content}</pre>
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

// ── ActionRow ─────────────────────────────────────────────────────────────────

const ACTION_CFG: Record<string, { icon: React.ReactNode; labelKey: string; color: string }> = {
  open_ui:       { icon: <ExternalLink className="h-3.5 w-3.5" />, labelKey: 'manager.actionOpenUi',       color: 'bg-blue-100 text-blue-700 border-blue-200 dark:bg-blue-900/30 dark:text-blue-400 dark:border-blue-800' },
  open_terminal: { icon: <TerminalSquare className="h-3.5 w-3.5" />, labelKey: 'manager.actionOpenTerminal', color: 'bg-green-100 text-green-700 border-green-200 dark:bg-green-900/30 dark:text-green-400 dark:border-green-800' },
  start_agent:   { icon: <Play className="h-3.5 w-3.5" />,         labelKey: 'manager.actionStart',         color: 'bg-emerald-100 text-emerald-700 border-emerald-200 dark:bg-emerald-900/30 dark:text-emerald-400 dark:border-emerald-800' },
  stop_agent:    { icon: <Square className="h-3.5 w-3.5" />,       labelKey: 'manager.actionStop',          color: 'bg-red-100 text-red-600 border-red-200 dark:bg-red-900/30 dark:text-red-400 dark:border-red-800' },
  navigate:      { icon: <Navigation className="h-3.5 w-3.5" />,   labelKey: 'manager.actionNavigate',      color: 'bg-purple-100 text-purple-700 border-purple-200 dark:bg-purple-900/30 dark:text-purple-400 dark:border-purple-800' },
  send_message:  { icon: <MessageSquare className="h-3.5 w-3.5" />, labelKey: 'manager.actionSendMessage',  color: 'bg-amber-100 text-amber-700 border-amber-200 dark:bg-amber-900/30 dark:text-amber-400 dark:border-amber-800' },
}

function ActionRow({ actions }: { actions: ManagerAction[] }) {
  const { t } = useTranslation()
  return (
    <div className="flex flex-wrap gap-1.5">
      {actions.map((action, i) => {
        const cfg = ACTION_CFG[action.type] ?? ACTION_CFG['navigate']
        const target = 'agentName' in action ? action.agentName : ('page' in action ? action.page : '')
        return (
          <span
            key={i}
            className={`inline-flex items-center gap-1.5 rounded-lg border px-2.5 py-1 text-xs font-medium ${cfg.color}`}
          >
            {cfg.icon}
            <span>{t(cfg.labelKey)}</span>
            {target && <span className="opacity-70">· {target}</span>}
          </span>
        )
      })}
    </div>
  )
}

// ── AgentPill ─────────────────────────────────────────────────────────────────

function AgentPill({ agent }: { agent: AgentState }) {
  const cfg: Record<string, { dot: string; text: string; icon: React.ReactNode }> = {
    running:  { dot: 'bg-green-400', text: 'text-green-700 dark:text-green-400', icon: <Zap className="h-2.5 w-2.5" /> },
    stopped:  { dot: 'bg-gray-300 dark:bg-gray-600', text: 'text-gray-500 dark:text-gray-400', icon: <Moon className="h-2.5 w-2.5" /> },
    error:    { dot: 'bg-red-400', text: 'text-red-500 dark:text-red-400', icon: <AlertCircle className="h-2.5 w-2.5" /> },
    starting: { dot: 'bg-yellow-400 animate-pulse', text: 'text-yellow-600 dark:text-yellow-400', icon: <Loader2 className="h-2.5 w-2.5 animate-spin" /> },
  }
  const s = cfg[agent.status] ?? cfg['stopped']
  return (
    <span className={`inline-flex items-center gap-1 text-[10px] font-medium ${s.text}`}>
      <span className={`h-1.5 w-1.5 rounded-full shrink-0 ${s.dot}`} />
      {agent.config.name}
    </span>
  )
}

// ── MdContent ─────────────────────────────────────────────────────────────────

function MdContent({ content }: { content: string }) {
  const clean = stripThinkingBlocks(content)
  return (
    <ReactMarkdown
      remarkPlugins={[remarkGfm]}
      components={{
        p: ({ children }) => <p className="mb-2 last:mb-0 leading-relaxed">{children}</p>,
        h1: ({ children }) => <h1 className="text-base font-bold mt-3 mb-1">{children}</h1>,
        h2: ({ children }) => <h2 className="text-sm font-bold mt-3 mb-1">{children}</h2>,
        h3: ({ children }) => <h3 className="text-sm font-semibold mt-2 mb-1">{children}</h3>,
        code: ({ className, children, ...props }) => {
          const isBlock = className?.includes('language-')
          return isBlock ? (
            <code className="block overflow-x-auto rounded-lg bg-gray-900 dark:bg-black px-3 py-2 text-xs font-mono text-green-300 my-2 whitespace-pre">{children}</code>
          ) : (
            <code className="rounded px-1 py-0.5 bg-amber-100 dark:bg-amber-900/40 text-xs font-mono text-amber-800 dark:text-amber-300" {...props}>{children}</code>
          )
        },
        pre: ({ children }) => <>{children}</>,
        ul: ({ children }) => <ul className="list-disc list-inside mb-2 space-y-0.5">{children}</ul>,
        ol: ({ children }) => <ol className="list-decimal list-inside mb-2 space-y-0.5">{children}</ol>,
        li: ({ children }) => <li className="leading-relaxed text-sm">{children}</li>,
        table: ({ children }) => <div className="overflow-x-auto my-2"><table className="text-xs border-collapse w-full">{children}</table></div>,
        thead: ({ children }) => <thead className="bg-gray-100 dark:bg-gray-700">{children}</thead>,
        th: ({ children }) => <th className="border border-gray-300 dark:border-gray-600 px-3 py-1.5 text-left font-semibold">{children}</th>,
        td: ({ children }) => <td className="border border-gray-300 dark:border-gray-600 px-3 py-1.5">{children}</td>,
        strong: ({ children }) => <strong className="font-semibold">{children}</strong>,
        a: ({ href, children }) => <a href={href} target="_blank" rel="noreferrer" className="text-amber-600 dark:text-amber-400 underline hover:no-underline">{children}</a>,
      }}
    >
      {clean}
    </ReactMarkdown>
  )
}

// suppress unused import warning
void Bot
void Cpu
