import { useCallback, useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { listen } from '@tauri-apps/api/event'
import {
  Send, Loader2, Bot, Wrench, ChevronDown, ChevronRight,
  AlertCircle, Settings, Plug, MessageSquare, Trash2,
  CheckCircle, XCircle, Eye, EyeOff, Plus, Minus, FolderOpen, Sparkles,
} from 'lucide-react'
import { WorkflowBuilder } from './WorkflowBuilder'
import { open } from '@tauri-apps/plugin-dialog'
import ReactMarkdown from 'react-markdown'
import remarkGfm from 'remark-gfm'
import { useTranslation } from 'react-i18next'
import { useMcpAgentStore, type AgentStep, type ChatMessage, type WorkflowStepSummary } from '../store/mcpAgentStore'

// ── Types ────────────────────────────────────────────────────────────────────

interface LlmProvider {
  id: string; name: string; model: string; base_url: string
  api_key: string; is_custom: boolean; enabled: boolean
  context_window?: number; max_output_tokens?: number
}

interface McpServer {
  name: string; command: string; args: string[]; env: Record<string, string>
  transport?: string; url?: string; headers?: Record<string, string>; description?: string
}

interface WorkflowNode {
  id: string; kind: string; label: string
  server: string; tool: string; arguments: Record<string, unknown>; prompt: string
}

interface Workflow {
  id: string; name: string; description: string
  nodes: WorkflowNode[]; created_at: string; updated_at: string
}

type Tab = 'chat' | 'workflow' | 'mcp' | 'llm'

// ── Main component ───────────────────────────────────────────────────────────

export function McpAgent() {
  const { t } = useTranslation()
  const [tab, setTab] = useState<Tab>('chat')
  const [providers, setProviders] = useState<LlmProvider[]>([])
  const [mcpServers, setMcpServers] = useState<McpServer[]>([])
  const [workflows, setWorkflows] = useState<Workflow[]>([])
  // Provider + enabled servers live in the store so they survive tab/route switches.
  const selectedProvider = useMcpAgentStore(s => s.selectedProvider)
  const setSelectedProvider = useMcpAgentStore(s => s.setSelectedProvider)
  const enabledServers = useMcpAgentStore(s => s.enabledServers)
  const toggleServer = useMcpAgentStore(s => s.toggleServer)

  const reload = useCallback(async () => {
    const [ps, ms, wfs] = await Promise.all([
      invoke<LlmProvider[]>('list_llm_providers'),
      invoke<McpServer[]>('list_mcp_servers'),
      invoke<Workflow[]>('list_workflows'),
    ])
    setProviders(ps)
    setMcpServers(ms)
    setWorkflows(wfs)
    const active = ps.find(p => p.id === selectedProvider && p.enabled && p.api_key.trim())
      ?? ps.find(p => p.enabled && p.api_key.trim())
    if (active?.id !== selectedProvider) setSelectedProvider(active?.id ?? '')
  }, [selectedProvider, setSelectedProvider])

  useEffect(() => { reload() }, [reload])

  const activeProvider = providers.find(
    p => p.id === selectedProvider && p.enabled && !!p.api_key.trim(),
  ) ?? null
  const selectedMcpServers = mcpServers.filter(s => enabledServers.includes(s.name))

  const TABS = [
    { id: 'chat' as Tab, label: t('mcpAgent.tabChat'), icon: <MessageSquare className="h-3.5 w-3.5" /> },
    { id: 'workflow' as Tab, label: t('mcpAgent.tabWorkflow'), icon: <Sparkles className="h-3.5 w-3.5" /> },
    { id: 'mcp'  as Tab, label: t('mcpAgent.tabMcpServers'), icon: <Plug className="h-3.5 w-3.5" /> },
    { id: 'llm'  as Tab, label: t('mcpAgent.tabLlmSettings'), icon: <Settings className="h-3.5 w-3.5" /> },
  ]

  return (
    <div className="flex h-full flex-col bg-white dark:bg-gray-900">
      {/* Header + tabs */}
      <div className="border-b border-gray-200 dark:border-gray-700 px-6 py-3">
        <div className="flex items-center justify-between mb-3">
          <div className="flex items-center gap-2">
            <Bot className="h-5 w-5 text-purple-500" />
            <span className="text-base font-semibold text-gray-900 dark:text-gray-100">MCP Agent</span>
          </div>
          {/* Quick status */}
          <div className="flex items-center gap-3 text-xs text-gray-500">
            {activeProvider
              ? <span className="flex items-center gap-1"><span className="h-1.5 w-1.5 rounded-full bg-green-400" />{activeProvider.name} · {activeProvider.model}</span>
              : <span className="flex items-center gap-1 text-yellow-600"><AlertCircle className="h-3 w-3" />{t('mcpAgent.noLlm')}</span>
            }
            <span>{t('mcpAgent.mcpToolCount', { count: enabledServers.length })}</span>
          </div>
        </div>
        <div className="flex gap-1">
          {TABS.map(tc => (
            <button key={tc.id} onClick={() => setTab(tc.id)}
              className={`flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-sm font-medium transition-colors ${
                tab === tc.id
                  ? 'bg-purple-50 text-purple-600 dark:bg-purple-900/20 dark:text-purple-400'
                  : 'text-gray-500 hover:bg-gray-100 dark:text-gray-400 dark:hover:bg-gray-800'
              }`}>
              {tc.icon}{tc.label}
            </button>
          ))}
        </div>
      </div>

      {/* Tab content */}
      <div className="flex-1 overflow-hidden">
        {tab === 'chat' && (
          <ChatPanel
            provider={activeProvider}
            mcpServers={selectedMcpServers}
            allProviders={providers.filter(p => p.enabled && p.api_key)}
            selectedProvider={selectedProvider}
            onSelectProvider={setSelectedProvider}
            enabledServers={enabledServers}
            allServers={mcpServers}
            onToggleServer={toggleServer}
            workflows={workflows}
          />
        )}
        {tab === 'workflow' && (
          <WorkflowBuilder
            enabledServers={selectedMcpServers}
            allServers={mcpServers}
            enabledNames={enabledServers}
            onToggleServer={toggleServer}
            onWorkflowsChange={setWorkflows}
          />
        )}
        {tab === 'mcp' && <McpTab servers={mcpServers} onReload={reload} activeProvider={activeProvider} />}
        {tab === 'llm' && <LlmTab providers={providers} onReload={reload} />}
      </div>
    </div>
  )
}

// ── Chat panel ───────────────────────────────────────────────────────────────

function ChatPanel({ provider, mcpServers, allProviders, selectedProvider, onSelectProvider,
  enabledServers, allServers, onToggleServer, workflows }: {
  provider: LlmProvider | null
  mcpServers: McpServer[]
  allProviders: LlmProvider[]
  selectedProvider: string
  onSelectProvider: (id: string) => void
  enabledServers: string[]
  allServers: McpServer[]
  onToggleServer: (name: string) => void
  workflows: Workflow[]
}) {
  const { t } = useTranslation()
  const messages = useMcpAgentStore(s => s.messages)
  const setMessages = useMcpAgentStore(s => s.setMessages)
  const clearMessages = useMcpAgentStore(s => s.clearMessages)
  const selectedWorkflowId = useMcpAgentStore(s => s.selectedWorkflowId)
  const setSelectedWorkflowId = useMcpAgentStore(s => s.setSelectedWorkflowId)
  const [input, setInput] = useState('')
  const [sending, setSending] = useState(false)
  const bottomRef = useRef<HTMLDivElement>(null)

  const selectedWorkflow = workflows.find(w => w.id === selectedWorkflowId) ?? null

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' })
  }, [messages, sending])

  async function send() {
    if (!input.trim() || sending) return
    if (!provider) return

    const userInput = input.trim()
    setInput('')
    setSending(true)

    const userMsg: ChatMessage = {
      role: 'user',
      content: selectedWorkflow
        ? `▶ ${selectedWorkflow.name}\n${userInput}`
        : userInput,
      isWorkflow: !!selectedWorkflow,
      workflowName: selectedWorkflow?.name,
    }
    setMessages(m => [...m, userMsg])

    try {
      if (selectedWorkflow) {
        // ── Workflow mode (streaming) ─────────────────────────────────────────
        // Add a placeholder assistant message immediately so the user sees
        // steps appearing in real time as each node finishes.
        const wfMsgIndex = messages.length + 1  // position after userMsg
        const placeholderMsg: ChatMessage = {
          role: 'assistant',
          content: '',
          isWorkflow: true,
          workflowName: selectedWorkflow.name,
          workflowSteps: [],
        }
        setMessages(m => [...m, placeholderMsg])

        type StepEvent = { node_id: string; label: string; kind: string; output: string; error: string | null }

        // Listen for step events and update the placeholder message live.
        const unlisten = await listen<StepEvent>('workflow-step', (event) => {
          const s = event.payload
          setMessages(m => m.map((msg, i) => {
            if (i !== wfMsgIndex) return msg
            const prev = msg.workflowSteps ?? []
            return {
              ...msg,
              workflowSteps: [...prev, {
                label: s.label,
                kind: s.kind,
                output: s.output,
                error: s.error ?? undefined,
              }],
            }
          }))
        })

        try {
          const res = await invoke<{
            steps: StepEvent[]
            final_output: string
            success: boolean
            error: string | null
          }>('run_workflow_stream', {
            request: {
              workflow: selectedWorkflow,
              provider,
              mcp_servers: mcpServers,
              input: userInput,
            },
          })

          // Replace placeholder with final complete message.
          setMessages(m => m.map((msg, i) => {
            if (i !== wfMsgIndex) return msg
            return {
              ...msg,
              content: res.final_output || (res.error ?? t('mcpAgent.workflowFailed')),
              workflowSteps: res.steps.map(s => ({
                label: s.label,
                kind: s.kind,
                output: s.output,
                error: s.error ?? undefined,
              })),
            }
          }))
        } finally {
          unlisten()
        }
      } else {
        // ── Normal chat mode ─────────────────────────────────────────────────
        const history = [...messages, userMsg].map(m => ({ role: m.role, content: m.content }))
        const result = await invoke<{ steps: AgentStep[]; reply: string; success: boolean; error?: string }>('chat_with_mcp', {
          request: { history, provider, mcp_servers: mcpServers, max_iterations: 10 }
        })
        setMessages(m => [...m, {
          role: 'assistant',
          content: result.reply || (result.error ?? ''),
          steps: result.steps.length ? result.steps : undefined,
        }])
      }
    } catch (e) {
      setMessages(m => [...m, { role: 'assistant', content: t('mcpAgent.errorPrefix', { error: String(e) }) }])
    } finally {
      setSending(false)
    }
  }

  return (
    <div className="flex h-full flex-col">
      {/* Toolbar */}
      <div className="flex items-center gap-3 border-b border-gray-100 dark:border-gray-800 bg-gray-50 dark:bg-gray-800/50 px-4 py-2 flex-wrap">
        {allProviders.length > 0 ? (
          <select value={selectedProvider} onChange={e => onSelectProvider(e.target.value)}
            className="rounded-lg border border-gray-200 bg-white px-2 py-1 text-xs dark:border-gray-700 dark:bg-gray-800 dark:text-gray-200 outline-none">
            {allProviders.map(p => <option key={p.id} value={p.id}>{p.name} · {p.model}</option>)}
          </select>
        ) : (
          <span className="text-xs text-yellow-600">⚠ {t('mcpAgent.configureLlmHint')}</span>
        )}
        <div className="flex items-center gap-1.5 flex-wrap">
          {allServers.map(s => (
            <button key={s.name} onClick={() => onToggleServer(s.name)}
              className={`rounded-md px-2 py-0.5 text-xs font-medium transition-colors ${
                enabledServers.includes(s.name)
                  ? 'bg-purple-100 text-purple-700 dark:bg-purple-900/40 dark:text-purple-300'
                  : 'bg-gray-100 text-gray-500 dark:bg-gray-700 dark:text-gray-400'
              }`}>
              🔌 {s.name}
            </button>
          ))}
          {allServers.length === 0 && <span className="text-xs text-gray-400">{t('mcpAgent.noMcpServersHint')}</span>}
        </div>
        {messages.length > 0 && (
          <button onClick={clearMessages} className="ml-auto flex items-center gap-1 text-xs text-gray-400 hover:text-red-500">
            <Trash2 className="h-3 w-3" /> {t('mcpAgent.clear')}
          </button>
        )}
      </div>

      {/* Messages */}
      <div className="flex-1 overflow-y-auto px-4 py-4 space-y-4">
        {messages.length === 0 && (
          <div className="flex h-full items-center justify-center">
            <div className="text-center space-y-2 text-gray-400">
              <Bot className="h-12 w-12 opacity-20 mx-auto" />
              <p className="text-sm">{t('mcpAgent.startConversation')}</p>
              <p className="text-xs">{t('mcpAgent.autoCallHint')}</p>
            </div>
          </div>
        )}

        {messages.map((msg, i) => (
          <div key={i}>
            {msg.role === 'assistant' && msg.steps && msg.steps.length > 0 && (
              <div className="mb-2 space-y-1.5">
                {msg.steps.map((step, j) => <InlineStep key={j} step={step} />)}
              </div>
            )}
            {msg.role === 'assistant' && msg.isWorkflow && msg.workflowSteps && (
              <WorkflowStepsDisplay steps={msg.workflowSteps} workflowName={msg.workflowName ?? ''} />
            )}
            <div className={`flex ${msg.role === 'user' ? 'justify-end' : 'justify-start'}`}>
              <div className={`max-w-[80%] rounded-2xl px-4 py-2.5 text-sm ${
                msg.role === 'user'
                  ? 'bg-purple-600 text-white rounded-br-sm'
                  : 'bg-gray-50 text-gray-900 dark:bg-gray-800 dark:text-gray-100 rounded-bl-sm'
              }`}>
                {msg.role === 'user'
                  ? <p className="whitespace-pre-wrap">{msg.content}</p>
                  : <MdContent content={msg.content} />
                }
              </div>
            </div>
          </div>
        ))}

        {sending && (
          <div className="flex justify-start">
            <div className="flex items-center gap-2 rounded-2xl rounded-bl-sm bg-gray-100 px-4 py-3 dark:bg-gray-800">
              <Loader2 className="h-4 w-4 animate-spin text-purple-500" />
              <span className="text-sm text-gray-500">
                {selectedWorkflow
                  ? t('mcpAgent.workflowRunning', { name: selectedWorkflow.name })
                  : t('mcpAgent.thinking')}
              </span>
            </div>
          </div>
        )}
        <div ref={bottomRef} />
      </div>

      {/* Workflow selector strip (shown when workflows exist) */}
      {workflows.length > 0 && (
        <div className="border-t border-gray-100 dark:border-gray-800 bg-gray-50/50 dark:bg-gray-800/30 px-4 py-2 flex items-center gap-2 flex-wrap">
          <Sparkles className="h-3.5 w-3.5 text-purple-400 shrink-0" />
          <span className="text-xs text-gray-400 shrink-0">{t('mcpAgent.workflowMode')}:</span>
          <button
            onClick={() => setSelectedWorkflowId('')}
            className={`rounded-full px-2.5 py-0.5 text-xs font-medium transition-colors ${
              !selectedWorkflowId
                ? 'bg-gray-200 text-gray-700 dark:bg-gray-600 dark:text-gray-200'
                : 'text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-700'
            }`}>
            {t('mcpAgent.workflowNone')}
          </button>
          {workflows.map(w => (
            <button key={w.id}
              onClick={() => setSelectedWorkflowId(selectedWorkflowId === w.id ? '' : w.id)}
              className={`rounded-full px-2.5 py-0.5 text-xs font-medium transition-colors ${
                selectedWorkflowId === w.id
                  ? 'bg-purple-100 text-purple-700 dark:bg-purple-900/40 dark:text-purple-300'
                  : 'text-gray-500 hover:bg-gray-100 dark:text-gray-400 dark:hover:bg-gray-700'
              }`}>
              ▶ {w.name}
            </button>
          ))}
        </div>
      )}

      {/* Input */}
      <div className="border-t border-gray-200 dark:border-gray-700 p-4">
        {selectedWorkflow && (
          <div className="mb-2 flex items-center gap-1.5 text-xs text-purple-600 dark:text-purple-400">
            <Sparkles className="h-3 w-3" />
            <span>{t('mcpAgent.workflowInputHint', { name: selectedWorkflow.name, steps: selectedWorkflow.nodes.length })}</span>
          </div>
        )}
        <div className="flex gap-2">
          <textarea
            value={input}
            onChange={e => setInput(e.target.value)}
            onKeyDown={e => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); send() } }}
            placeholder={selectedWorkflow
              ? t('mcpAgent.workflowInputPlaceholder', { name: selectedWorkflow.name })
              : t('mcpAgent.inputPlaceholder')}
            rows={2}
            disabled={sending || !provider}
            className="field-input flex-1 resize-none"
          />
          <button onClick={send} disabled={sending || !input.trim() || !provider}
            className={`self-end rounded-xl p-2.5 text-white disabled:opacity-50 ${
              selectedWorkflow ? 'bg-purple-700 hover:bg-purple-600' : 'bg-purple-600 hover:bg-purple-500'
            }`}>
            {sending ? <Loader2 className="h-4 w-4 animate-spin" /> : <Send className="h-4 w-4" />}
          </button>
        </div>
      </div>
    </div>
  )
}

// ── Inline step (tool call / result in chat) ─────────────────────────────────

function InlineStep({ step }: { step: AgentStep }) {
  const [expanded, setExpanded] = useState(false)

  if (step.kind === 'toolcall') {
    return (
      <button onClick={() => setExpanded(e => !e)}
        className="flex items-start gap-2 w-full rounded-xl border border-orange-200 bg-orange-50 dark:border-orange-800 dark:bg-orange-900/10 px-3 py-2 text-left">
        <Wrench className="h-3.5 w-3.5 text-orange-500 shrink-0 mt-0.5" />
        <div className="flex-1 min-w-0">
          <span className="text-xs font-semibold text-orange-700 dark:text-orange-400 font-mono">{step.tool}</span>
          {expanded && step.tool_input != null && (
            <pre className="mt-1 overflow-x-auto text-xs text-gray-600 dark:text-gray-300">
              {JSON.stringify(step.tool_input as object, null, 2)}
            </pre>
          )}
        </div>
        {expanded ? <ChevronDown className="h-3 w-3 text-gray-400 shrink-0" /> : <ChevronRight className="h-3 w-3 text-gray-400 shrink-0" />}
      </button>
    )
  }

  if (step.kind === 'toolresult') {
    return (
      <button onClick={() => setExpanded(e => !e)}
        className="flex items-start gap-2 w-full rounded-xl border border-gray-200 bg-gray-50 dark:border-gray-700 dark:bg-gray-800/50 px-3 py-2 text-left">
        <span className="text-xs text-gray-400 font-mono shrink-0 mt-0.5">↩</span>
        <div className="flex-1 min-w-0">
          <span className="text-xs text-gray-500 dark:text-gray-400 font-mono">{step.tool}</span>
          {expanded ? (
            <p className="mt-1 text-xs text-gray-600 dark:text-gray-300 whitespace-pre-wrap">{step.content}</p>
          ) : (
            <p className="text-xs text-gray-400 truncate">{step.content.slice(0, 80)}{step.content.length > 80 ? '…' : ''}</p>
          )}
        </div>
        {expanded ? <ChevronDown className="h-3 w-3 text-gray-400 shrink-0" /> : <ChevronRight className="h-3 w-3 text-gray-400 shrink-0" />}
      </button>
    )
  }

  if (step.kind === 'error') {
    return (
      <div className="flex items-center gap-2 rounded-xl border border-red-200 bg-red-50 dark:border-red-800 dark:bg-red-900/10 px-3 py-2">
        <AlertCircle className="h-3.5 w-3.5 text-red-500 shrink-0" />
        <p className="text-xs text-red-600 dark:text-red-400">{step.content}</p>
      </div>
    )
  }

  return null
}

// ── Workflow steps display (collapsible, shown before the assistant reply) ────

function WorkflowStepItem({ s, i }: { s: WorkflowStepSummary; i: number }) {
  const [expanded, setExpanded] = useState(false)
  const text = s.error || s.output || '(empty)'
  const long = text.length > 200

  return (
    <div className={`rounded-lg border px-3 py-2 text-xs ${
      s.error
        ? 'border-red-200 bg-red-50 dark:border-red-800 dark:bg-red-900/10'
        : 'border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-900'
    }`}>
      <p className="font-semibold text-gray-600 dark:text-gray-400 font-mono">
        {i + 1}. {s.label}
        <span className="ml-1.5 rounded bg-gray-100 px-1 py-0.5 text-[10px] dark:bg-gray-700">{s.kind}</span>
        {s.error && <span className="ml-1.5 text-red-500">⚠</span>}
      </p>
      <p className={`mt-0.5 text-gray-500 dark:text-gray-400 whitespace-pre-wrap ${!expanded && long ? 'line-clamp-3' : ''}`}>
        {text}
      </p>
      {long && (
        <button onClick={() => setExpanded(e => !e)}
          className="mt-1 text-[11px] text-purple-500 hover:text-purple-700">
          {expanded ? '▲ 收起' : '▼ 展开全部'}
        </button>
      )}
    </div>
  )
}

function WorkflowStepsDisplay({ steps, workflowName }: {
  steps: WorkflowStepSummary[]
  workflowName: string
}) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const hasError = steps.some(s => s.error)

  return (
    <div className="mb-2">
      <button onClick={() => setOpen(o => !o)}
        className={`flex items-center gap-2 w-full rounded-xl border px-3 py-2 text-left transition-colors ${
          hasError
            ? 'border-red-200 bg-red-50 dark:border-red-800 dark:bg-red-900/10'
            : 'border-purple-200 bg-purple-50 dark:border-purple-800 dark:bg-purple-900/10'
        }`}>
        <Sparkles className={`h-3.5 w-3.5 shrink-0 ${hasError ? 'text-red-500' : 'text-purple-500'}`} />
        <span className={`flex-1 text-xs font-semibold ${hasError ? 'text-red-700 dark:text-red-400' : 'text-purple-700 dark:text-purple-400'}`}>
          {workflowName} — {steps.length} {t('mcpAgent.step').toLowerCase()}
          {hasError && ` (${t('mcpAgent.workflowStepError')})`}
        </span>
        {open ? <ChevronDown className="h-3 w-3 text-gray-400 shrink-0" /> : <ChevronRight className="h-3 w-3 text-gray-400 shrink-0" />}
      </button>
      {open && (
        <div className="mt-1 ml-3 space-y-1 border-l-2 border-purple-200 dark:border-purple-800 pl-3">
          {steps.map((s, i) => (
            <WorkflowStepItem key={i} s={s} i={i} />
          ))}
        </div>
      )}
    </div>
  )
}

// ── MCP Servers tab ──────────────────────────────────────────────────────────

const EMPTY_SERVER: McpServer = {
  name: '', command: '', args: [], env: {},
  transport: 'stdio', url: '', headers: {}, description: '',
}

type AddMode = 'local' | 'text' | 'manual'

interface ScanResult {
  transport: string; name: string; command: string; args: string[]
  env: Record<string, string>; url: string; headers: Record<string, string>
  description: string; warnings: string[]; confidence: number
}

function McpTab({ servers, onReload, activeProvider }: {
  servers: McpServer[]
  onReload: () => void
  activeProvider: LlmProvider | null
}) {
  const { t } = useTranslation()
  const [editing, setEditing] = useState<McpServer | null>(null)
  const [isNew, setIsNew] = useState(false)
  const [addMode, setAddMode] = useState<AddMode>('local')
  const [saving, setSaving] = useState(false)
  const [scanning, setScanning] = useState(false)
  const [error, setError] = useState('')
  const [warnings, setWarnings] = useState<string[]>([])
  const [deleteConfirm, setDeleteConfirm] = useState<string | null>(null)
  const [configPath, setConfigPath] = useState('')
  const [localDir, setLocalDir] = useState('')
  const [parseText, setParseText] = useState('')

  useEffect(() => { invoke<string>('get_mcp_config_path').then(setConfigPath) }, [])

  function startNew() {
    setEditing({ ...EMPTY_SERVER })
    setIsNew(true)
    setError('')
    setWarnings([])
    setLocalDir('')
    setParseText('')
  }

  function startEdit(s: McpServer) {
    setEditing({ ...s, transport: s.transport || 'stdio', headers: s.headers || {}, url: s.url || '' })
    setIsNew(false)
    setError('')
    setWarnings([])
  }

  async function pickLocalDir() {
    const f = await open({ directory: true, multiple: false })
    if (f && typeof f === 'string') {
      setLocalDir(f)
      await scanLocal(f)
    }
  }

  async function scanLocal(dir: string) {
    setScanning(true); setError(''); setWarnings([])
    try {
      const result = await invoke<ScanResult>('scan_mcp_local', {
        dir,
        provider: activeProvider ?? null,
      })
      applyResult(result)
    } catch (e) {
      setError(String(e))
    } finally { setScanning(false) }
  }

  async function parseText_() {
    if (!parseText.trim()) { setError(t('mcpAgent.pasteTextFirst')); return }

    // Try direct JSON parse first (standard mcpServers / single-server format)
    try {
      const json = JSON.parse(parseText)
      // { mcpServers: { name: { command, args, env, ... } } }
      const serversMap: Record<string, unknown> = json.mcpServers ?? json
      const entries = Object.entries(serversMap)
      if (entries.length > 0) {
        const [name, cfg] = entries[0] as [string, Record<string, unknown>]
        if (cfg && typeof cfg === 'object' && ('command' in cfg || 'url' in cfg)) {
          const isSSE = (cfg.transport === 'sse') || typeof cfg.url === 'string'
          applyResult({
            name,
            command: String(cfg.command ?? ''),
            args: Array.isArray(cfg.args) ? cfg.args.map(String) : [],
            env: (cfg.env && typeof cfg.env === 'object' && !Array.isArray(cfg.env))
              ? cfg.env as Record<string, string> : {},
            transport: isSSE ? 'sse' : 'stdio',
            url: typeof cfg.url === 'string' ? cfg.url : '',
            headers: (cfg.headers && typeof cfg.headers === 'object' && !Array.isArray(cfg.headers))
              ? cfg.headers as Record<string, string> : {},
            description: typeof cfg.description === 'string' ? cfg.description : '',
            warnings: [],
            confidence: 1,
          })
          return
        }
      }
    } catch { /* not valid JSON, fall through to LLM */ }

    if (!activeProvider) { setError(t('mcpAgent.configureLlmFirst')); return }
    setScanning(true); setError(''); setWarnings([])
    try {
      const result = await invoke<ScanResult>('parse_mcp_text', {
        text: parseText,
        provider: activeProvider,
      })
      applyResult(result)
    } catch (e) {
      setError(String(e))
    } finally { setScanning(false) }
  }

  function applyResult(r: ScanResult) {
    setWarnings(r.warnings)
    setEditing({
      name: r.name, command: r.command, args: r.args, env: r.env,
      transport: r.transport, url: r.url, headers: r.headers, description: r.description,
    })
  }

  async function save() {
    if (!editing) return
    if (!editing.name.trim()) { setError(t('mcpAgent.nameRequired')); return }
    const isSSE = editing.transport === 'sse'
    if (isSSE && !(editing.url ?? '').trim()) { setError(t('mcpAgent.urlRequiredSse')); return }
    if (!isSSE && !editing.command.trim()) { setError(t('mcpAgent.commandRequiredStdio')); return }
    setSaving(true)
    try {
      await invoke('save_mcp_server', { server: editing })
      await onReload()
      setEditing(null)
    } catch (e) { setError(String(e)) }
    finally { setSaving(false) }
  }

  async function del(name: string) {
    await invoke('delete_mcp_server', { name })
    setDeleteConfirm(null)
    await onReload()
    if (editing?.name === name) setEditing(null)
  }

  const isSSE = editing?.transport === 'sse'

  return (
    <div className="flex h-full overflow-hidden">
      {/* Server list */}
      <div className="w-56 shrink-0 border-r border-gray-200 dark:border-gray-700 flex flex-col">
        <div className="flex items-center justify-between px-3 py-2 border-b border-gray-100 dark:border-gray-800">
          <span className="text-xs font-medium text-gray-500">{t('mcpAgent.serverCount', { count: servers.length })}</span>
          <button onClick={startNew} className="rounded p-1 text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-800">
            <Plus className="h-3.5 w-3.5" />
          </button>
        </div>
        <div className="flex-1 overflow-y-auto p-2 space-y-0.5">
          {servers.length === 0 && <p className="py-6 text-center text-xs text-gray-400">{t('mcpAgent.noServersYet')}</p>}
          {servers.map(s => (
            <button key={s.name} onClick={() => startEdit(s)}
              className={`group flex w-full items-center gap-2 rounded-lg px-3 py-2 text-left transition-colors ${
                editing?.name === s.name && !isNew
                  ? 'bg-purple-50 text-purple-700 dark:bg-purple-900/20 dark:text-purple-300'
                  : 'hover:bg-gray-50 dark:hover:bg-gray-800'
              }`}>
              <span>{s.transport === 'sse' ? '🌐' : '🔌'}</span>
              <div className="min-w-0 flex-1">
                <p className="truncate text-xs font-medium text-gray-900 dark:text-gray-100">{s.name}</p>
                <p className="truncate text-xs text-gray-400 font-mono">
                  {s.transport === 'sse' ? s.url : `${s.command} ${s.args.slice(0, 1).join(' ')}`}
                </p>
              </div>
              <button onClick={e => { e.stopPropagation(); setDeleteConfirm(s.name) }}
                className="shrink-0 opacity-0 group-hover:opacity-100 text-gray-300 hover:text-red-500">
                <Trash2 className="h-3 w-3" />
              </button>
            </button>
          ))}
        </div>
        <div className="p-2 border-t border-gray-100 dark:border-gray-800">
          <button onClick={startNew} className="flex w-full items-center justify-center gap-1.5 rounded-lg bg-purple-600 py-1.5 text-xs font-medium text-white hover:bg-purple-500">
            <Plus className="h-3.5 w-3.5" /> {t('mcpAgent.addServer')}
          </button>
        </div>
      </div>

      {/* Right panel */}
      <div className="flex-1 overflow-y-auto">
        {editing ? (
          <div className="max-w-xl p-5 space-y-4">
            <div className="flex items-center justify-between">
              <h3 className="text-sm font-semibold text-gray-900 dark:text-gray-100">
                {isNew ? t('mcpAgent.addMcpServer') : t('mcpAgent.editServer', { name: editing.name })}
              </h3>
            </div>

            {/* Add mode selector — only for new */}
            {isNew && (
              <div className="flex rounded-xl border border-gray-200 dark:border-gray-700 overflow-hidden">
                {([
                  { id: 'local' as AddMode, icon: <FolderOpen className="h-3.5 w-3.5" />, label: t('mcpAgent.modeLocalDir') },
                  { id: 'text'  as AddMode, icon: <Wrench className="h-3.5 w-3.5" />,      label: t('mcpAgent.modeSmartParse') },
                  { id: 'manual'as AddMode, icon: <Plus className="h-3.5 w-3.5" />,         label: t('mcpAgent.modeManual') },
                ]).map(m => (
                  <button key={m.id} onClick={() => setAddMode(m.id)}
                    className={`flex flex-1 items-center justify-center gap-1.5 py-2 text-xs font-medium transition-colors ${
                      addMode === m.id
                        ? 'bg-purple-50 text-purple-700 dark:bg-purple-900/20 dark:text-purple-400'
                        : 'text-gray-500 hover:bg-gray-50 dark:text-gray-400 dark:hover:bg-gray-800'
                    }`}>
                    {m.icon}{m.label}
                  </button>
                ))}
              </div>
            )}

            {/* Local directory mode */}
            {isNew && addMode === 'local' && (
              <div className="space-y-2">
                <F label={t('mcpAgent.localDirLabel')}>
                  <div className="flex gap-2">
                    <input value={localDir} onChange={e => setLocalDir(e.target.value)}
                      placeholder="D:/my-mcp-server" className="field-input flex-1 font-mono text-xs" />
                    <button onClick={pickLocalDir} className="rounded-lg border border-gray-200 px-2.5 text-gray-500 hover:bg-gray-50 dark:border-gray-700">
                      <FolderOpen className="h-4 w-4" />
                    </button>
                    <button onClick={() => localDir && scanLocal(localDir)} disabled={!localDir || scanning}
                      className="flex items-center gap-1 rounded-lg bg-purple-600 px-3 text-xs text-white hover:bg-purple-500 disabled:opacity-50">
                      {scanning ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : t('mcpAgent.scan')}
                    </button>
                  </div>
                </F>
                <p className="text-xs text-gray-400">{activeProvider ? t('mcpAgent.localDirHintAi') : t('mcpAgent.localDirHint')}</p>
              </div>
            )}

            {/* Smart parse mode */}
            {isNew && addMode === 'text' && (
              <div className="space-y-2">
                <F label={t('mcpAgent.pasteLabel')}>
                  <textarea value={parseText} onChange={e => setParseText(e.target.value)}
                    rows={5} placeholder={`npx @modelcontextprotocol/server-filesystem /path/to/dir\n\nor paste a JSON config snippet, README excerpt, SSE URL...`}
                    className="field-input font-mono text-xs resize-none" />
                </F>
                <button type="button" onClick={parseText_} disabled={scanning || !parseText.trim()}
                  className="flex items-center gap-1.5 rounded-lg bg-purple-600 px-3 py-1.5 text-xs text-white hover:bg-purple-500 disabled:opacity-50">
                  {scanning ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Wrench className="h-3.5 w-3.5" />}
                  {scanning ? t('mcpAgent.parsing') : t('mcpAgent.parseWithAi')}
                </button>
                {!activeProvider && <p className="text-xs text-yellow-600">⚠ {t('mcpAgent.smartParseLlmHint')}</p>}
              </div>
            )}

            {error && (
              <div className="flex items-center gap-2 rounded-lg bg-red-50 dark:bg-red-900/20 px-3 py-2 text-xs text-red-600 dark:text-red-400">
                <AlertCircle className="h-3.5 w-3.5 shrink-0" />{error}
              </div>
            )}

            {/* Warnings */}
            {warnings.length > 0 && (
              <div className="rounded-lg bg-yellow-50 dark:bg-yellow-900/20 border border-yellow-200 dark:border-yellow-800 p-3 space-y-1">
                {warnings.map((w, i) => (
                  <p key={i} className="text-xs text-yellow-700 dark:text-yellow-400 flex items-start gap-1.5">
                    <AlertCircle className="h-3.5 w-3.5 shrink-0 mt-0.5" />{w}
                  </p>
                ))}
              </div>
            )}

            {/* Form fields — shown after scan or in manual mode */}
            {(addMode === 'manual' || !isNew || editing.name || editing.command || editing.url) && (
              <>
                {/* Transport selector */}
                <div className="flex items-center gap-3">
                  <span className="text-xs font-medium text-gray-500">{t('mcpAgent.transport')}</span>
                  {['stdio', 'sse'].map(tp => (
                    <label key={tp} className="flex items-center gap-1.5 cursor-pointer">
                      <input type="radio" name="transport" value={tp}
                        checked={(editing.transport || 'stdio') === tp}
                        onChange={() => setEditing(v => v ? { ...v, transport: tp } : v)} />
                      <span className="text-xs text-gray-700 dark:text-gray-300">{tp === 'stdio' ? t('mcpAgent.transportStdio') : t('mcpAgent.transportSse')}</span>
                    </label>
                  ))}
                </div>

                <F label={t('mcpAgent.serverIdLabel')}>
                  <input value={editing.name} disabled={!isNew}
                    onChange={e => setEditing(v => v ? { ...v, name: e.target.value } : v)}
                    placeholder="filesystem" className="field-input disabled:opacity-50" />
                </F>

                <F label={t('mcpAgent.descriptionLabel')}>
                  <input value={editing.description || ''} onChange={e => setEditing(v => v ? { ...v, description: e.target.value } : v)}
                    placeholder={t('mcpAgent.descriptionPlaceholder')} className="field-input" />
                </F>

                {isSSE ? (
                  <>
                    <F label={t('mcpAgent.urlLabel')}>
                      <input value={editing.url || ''} onChange={e => setEditing(v => v ? { ...v, url: e.target.value } : v)}
                        placeholder="https://mcp.example.com/sse" className="field-input font-mono" />
                    </F>
                    <F label={t('mcpAgent.headersLabel')}>
                      <EnvEditor
                        env={editing.headers || {}}
                        onChange={h => setEditing(v => v ? { ...v, headers: h } : v)}
                        keyPlaceholder="Authorization"
                        valPlaceholder="Bearer token..."
                      />
                    </F>
                  </>
                ) : (
                  <>
                    <F label={t('mcpAgent.commandLabel')}>
                      <input value={editing.command} onChange={e => setEditing(v => v ? { ...v, command: e.target.value } : v)}
                        placeholder="npx / node / python / uvx" className="field-input font-mono" />
                    </F>
                    <F label={t('mcpAgent.argumentsLabel')}>
                      <div className="space-y-1.5">
                        {editing.args.map((a, i) => (
                          <div key={i} className="flex gap-2">
                            <input value={a} onChange={e => setEditing(v => v ? { ...v, args: v.args.map((x, j) => j === i ? e.target.value : x) } : v)}
                              className="field-input flex-1 font-mono text-xs" />
                            <button onClick={() => setEditing(v => v ? { ...v, args: v.args.filter((_, j) => j !== i) } : v)}
                              className="text-gray-400 hover:text-red-500"><Minus className="h-4 w-4" /></button>
                          </div>
                        ))}
                        <button onClick={() => setEditing(v => v ? { ...v, args: [...v.args, ''] } : v)}
                          className="flex items-center gap-1 text-xs text-gray-400 hover:text-gray-700">
                          <Plus className="h-3 w-3" /> {t('mcpAgent.addArg')}
                        </button>
                      </div>
                    </F>
                    <F label={t('mcpAgent.envLabel')}>
                      <EnvEditor
                        env={editing.env}
                        onChange={env => setEditing(v => v ? { ...v, env } : v)}
                      />
                    </F>
                  </>
                )}

                <div className="flex gap-2">
                  <button onClick={save} disabled={saving}
                    className="flex items-center gap-1.5 rounded-lg bg-purple-600 px-3 py-1.5 text-xs text-white hover:bg-purple-500 disabled:opacity-60">
                    {saving && <Loader2 className="h-3 w-3 animate-spin" />} {t('mcpAgent.save')}
                  </button>
                  <button onClick={() => setEditing(null)} className="rounded-lg px-3 py-1.5 text-xs text-gray-500 hover:bg-gray-100 dark:hover:bg-gray-800">{t('mcpAgent.cancel')}</button>
                </div>
                <p className="text-xs text-gray-400">{t('mcpAgent.configLabel')} <span className="font-mono">{configPath}</span></p>
              </>
            )}
          </div>
        ) : (
          <div className="flex h-full items-center justify-center text-gray-400">
            <div className="text-center space-y-2">
              <div className="text-4xl">🔌</div>
              <p className="text-sm">{t('mcpAgent.selectServerToEdit')}</p>
              <p className="text-xs text-gray-400">{t('mcpAgent.orClickAddServer')}</p>
            </div>
          </div>
        )}
      </div>

      {deleteConfirm && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40">
          <div className="rounded-2xl border border-gray-200 bg-white p-5 shadow-xl dark:border-gray-700 dark:bg-gray-900 w-72">
            <p className="text-sm font-semibold mb-1 text-gray-900 dark:text-gray-100">{t('mcpAgent.deletePrefix')} <span className="font-mono">{deleteConfirm}</span>?</p>
            <p className="text-xs text-gray-500 mb-4">{t('mcpAgent.deleteRemovesFrom')}</p>
            <div className="flex gap-2 justify-end">
              <button onClick={() => setDeleteConfirm(null)} className="px-3 py-1.5 text-xs text-gray-500 hover:bg-gray-100 rounded-lg">{t('mcpAgent.cancel')}</button>
              <button onClick={() => del(deleteConfirm)} className="px-3 py-1.5 text-xs bg-red-600 text-white rounded-lg hover:bg-red-500">{t('mcpAgent.delete')}</button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}

function EnvEditor({ env, onChange, keyPlaceholder = 'KEY', valPlaceholder = 'value' }: {
  env: Record<string, string>; onChange: (e: Record<string, string>) => void
  keyPlaceholder?: string; valPlaceholder?: string
}) {
  const [pairs, setPairs] = useState<[string, string][]>(() => Object.entries(env))
  const prevEnvRef = useRef(env)
  useEffect(() => {
    if (prevEnvRef.current !== env) {
      prevEnvRef.current = env
      setPairs(Object.entries(env))
    }
  }, [env])

  function update(newPairs: [string, string][]) {
    setPairs(newPairs)
    const obj: Record<string, string> = {}
    newPairs.forEach(([k, v]) => { if (k.trim()) obj[k.trim()] = v })
    onChange(obj)
  }

  return (
    <div className="space-y-1.5">
      {pairs.map(([k, v], i) => (
        <div key={i} className="flex gap-2">
          <input value={k} onChange={e => update(pairs.map((p, j) => j === i ? [e.target.value, p[1]] : p))}
            placeholder={keyPlaceholder} className="field-input flex-1 font-mono text-xs" />
          <input value={v} onChange={e => update(pairs.map((p, j) => j === i ? [p[0], e.target.value] : p))}
            placeholder={valPlaceholder} className="field-input flex-1 font-mono text-xs" />
          <button type="button" onClick={() => update(pairs.filter((_, j) => j !== i))} className="text-gray-400 hover:text-red-500">
            <Minus className="h-4 w-4" />
          </button>
        </div>
      ))}
      <button type="button" onClick={() => update([...pairs, ['', '']])}
        className="flex items-center gap-1 text-xs text-gray-400 hover:text-gray-700 dark:hover:text-gray-300">
        <Plus className="h-3 w-3" /> Add
      </button>
    </div>
  )
}

// ── LLM Settings tab ─────────────────────────────────────────────────────────

function LlmTab({ providers, onReload }: { providers: LlmProvider[]; onReload: () => void }) {
  const { t } = useTranslation()
  const [showCustom, setShowCustom] = useState(false)
  const [customForm, setCustomForm] = useState<LlmProvider>({ id: '', name: '', base_url: '', model: '', api_key: '', is_custom: true, enabled: true })
  const [editingId, setEditingId] = useState<string | null>(null)
  const [testing, setTesting] = useState<string | null>(null)
  const [testResult, setTestResult] = useState<Record<string, { ok: boolean; msg: string }>>({})
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState('')
  const [showKeys, setShowKeys] = useState<Record<string, boolean>>({})

  async function saveProvider(p: LlmProvider) {
    await invoke('save_llm_provider', { provider: p })
    onReload()
  }

  async function testProvider(p: LlmProvider) {
    setTesting(p.id)
    try {
      const msg = await invoke<string>('test_llm_provider', { provider: p })
      setTestResult(r => ({ ...r, [p.id]: { ok: true, msg } }))
    } catch (e) {
      setTestResult(r => ({ ...r, [p.id]: { ok: false, msg: String(e) } }))
    } finally { setTesting(null) }
  }

  async function saveCustom() {
    if (!customForm.name.trim() || !customForm.base_url.trim() || !customForm.model.trim() || !customForm.api_key.trim()) {
      setError(t('mcpAgent.allFieldsRequired')); return
    }
    const id = editingId ?? `custom_${Date.now()}`
    setSaving(true); setError('')
    try {
      await invoke('save_llm_provider', { provider: { ...customForm, id, is_custom: true } })
      onReload(); setShowCustom(false); setEditingId(null)
      setCustomForm({ id: '', name: '', base_url: '', model: '', api_key: '', is_custom: true, enabled: true })
    } catch (e) { setError(String(e)) }
    finally { setSaving(false) }
  }

  const builtins = providers.filter(p => !p.is_custom)
  const customs = providers.filter(p => p.is_custom)

  return (
    <div className="h-full overflow-y-auto p-5 space-y-5">
      <div>
        <h3 className="text-xs font-semibold uppercase tracking-wide text-gray-500 mb-3">{t('mcpAgent.builtinProviders')}</h3>
        <div className="space-y-3">
          {builtins.map(p => (
            <BuiltinRow key={p.id} provider={p}
              showKey={showKeys[p.id] ?? false}
              onToggleKey={() => setShowKeys(s => ({ ...s, [p.id]: !s[p.id] }))}
              testResult={testResult[p.id]}
              testing={testing === p.id}
              onSave={saveProvider}
              onTest={testProvider}
            />
          ))}
        </div>
      </div>

      <div>
        <div className="flex items-center justify-between mb-3">
          <h3 className="text-xs font-semibold uppercase tracking-wide text-gray-500">{t('mcpAgent.customModels')}</h3>
          <button onClick={() => { setShowCustom(true); setEditingId(null); setCustomForm({ id: '', name: '', base_url: '', model: '', api_key: '', is_custom: true, enabled: true }) }}
            className="flex items-center gap-1 text-xs text-purple-600 hover:text-purple-500">
            <Plus className="h-3.5 w-3.5" /> {t('mcpAgent.add')}
          </button>
        </div>
        <div className="space-y-2">
          {customs.map(p => (
            <div key={p.id} className="flex items-center gap-3 rounded-xl border border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-900 px-4 py-2.5">
              <div className="flex-1 min-w-0">
                <p className="text-sm font-medium text-gray-900 dark:text-gray-100">{p.name}</p>
                <p className="text-xs font-mono text-gray-400 truncate">{p.model} · {p.base_url}</p>
              </div>
              <button onClick={() => testProvider(p)} disabled={testing === p.id} className="text-xs text-gray-400 hover:text-blue-600">
                {testing === p.id ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : t('mcpAgent.test')}
              </button>
              <button onClick={() => { setCustomForm({ ...p }); setEditingId(p.id); setShowCustom(true) }}
                className="text-xs text-gray-400 hover:text-gray-700">{t('mcpAgent.edit')}</button>
              <button onClick={async () => { await invoke('delete_llm_provider', { id: p.id }); onReload() }}
                className="text-gray-300 hover:text-red-500"><Trash2 className="h-3.5 w-3.5" /></button>
              <Toggle enabled={p.enabled} onChange={v => saveProvider({ ...p, enabled: v })} />
              {testResult[p.id] && <TestBadge small result={testResult[p.id]} />}
            </div>
          ))}
          {customs.length === 0 && !showCustom && <p className="text-xs text-gray-400 py-3 text-center">{t('mcpAgent.noCustomModels')}</p>}
        </div>

        {showCustom && (
          <div className="mt-3 rounded-xl border border-purple-200 bg-white dark:border-purple-800 dark:bg-gray-900 p-4 space-y-3">
            <div className="grid grid-cols-2 gap-3">
              <F label={t('mcpAgent.nameLabel')}><input value={customForm.name} onChange={e => setCustomForm(f => ({ ...f, name: e.target.value }))} placeholder="Gemini / Kimi" className="field-input" /></F>
              <F label={t('mcpAgent.modelIdLabel')}><input value={customForm.model} onChange={e => setCustomForm(f => ({ ...f, model: e.target.value }))} placeholder="gemini-pro" className="field-input font-mono" /></F>
            </div>
            <F label={t('mcpAgent.baseUrlLabel')}><input value={customForm.base_url} onChange={e => setCustomForm(f => ({ ...f, base_url: e.target.value }))} placeholder="https://generativelanguage.googleapis.com/v1beta/openai" className="field-input font-mono" /></F>
            <F label={t('mcpAgent.apiKeyLabel')}><input type="password" value={customForm.api_key} onChange={e => setCustomForm(f => ({ ...f, api_key: e.target.value }))} placeholder="sk-..." className="field-input font-mono" /></F>
            {error && <p className="text-xs text-red-500">{error}</p>}
            <div className="flex gap-2">
              <button onClick={saveCustom} disabled={saving} className="flex items-center gap-1.5 rounded-lg bg-purple-600 px-3 py-1.5 text-xs text-white hover:bg-purple-500 disabled:opacity-60">
                {saving && <Loader2 className="h-3 w-3 animate-spin" />} {t('mcpAgent.save')}
              </button>
              <button onClick={() => testProvider({ ...customForm, id: editingId ?? 'temp' })} className="rounded-lg border border-gray-200 px-3 py-1.5 text-xs text-gray-600 hover:bg-gray-50 dark:border-gray-700">{t('mcpAgent.test')}</button>
              <button onClick={() => { setShowCustom(false); setError('') }} className="rounded-lg px-3 py-1.5 text-xs text-gray-500 hover:bg-gray-100">{t('mcpAgent.cancel')}</button>
            </div>
          </div>
        )}
      </div>
    </div>
  )
}

function BuiltinRow({ provider, showKey, onToggleKey, testResult, testing, onSave, onTest }: {
  provider: LlmProvider; showKey: boolean; onToggleKey: () => void
  testResult?: { ok: boolean; msg: string }; testing: boolean
  onSave: (p: LlmProvider) => void; onTest: (p: LlmProvider) => void
}) {
  const { t } = useTranslation()
  const [key, setKey] = useState(provider.api_key)
  const [model, setModel] = useState(provider.model)
  useEffect(() => { setKey(provider.api_key); setModel(provider.model) }, [provider])

  return (
    <div className="rounded-xl border border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-900 p-4 space-y-3">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <span className="text-sm font-semibold text-gray-900 dark:text-gray-100">{provider.name}</span>
          {provider.enabled && provider.api_key && <span className="rounded-full bg-green-100 px-2 py-0.5 text-xs text-green-700 dark:bg-green-900/30 dark:text-green-400">{t('mcpAgent.active')}</span>}
        </div>
        <Toggle enabled={provider.enabled} onChange={v => onSave({ ...provider, enabled: v })} />
      </div>
      <div className="grid grid-cols-2 gap-3">
        <F label={t('mcpAgent.modelLabel')}>
          <input value={model} onChange={e => setModel(e.target.value)} className="field-input text-xs font-mono" />
        </F>
        <F label={t('mcpAgent.apiKeyLabel')}>
          <div className="relative">
            <input type={showKey ? 'text' : 'password'} value={key} onChange={e => setKey(e.target.value)}
              placeholder="sk-..." className="field-input text-xs font-mono pr-8" />
            <button onClick={onToggleKey} className="absolute right-2 top-1/2 -translate-y-1/2 text-gray-400 hover:text-gray-600">
              {showKey ? <EyeOff className="h-3.5 w-3.5" /> : <Eye className="h-3.5 w-3.5" />}
            </button>
          </div>
        </F>
      </div>
      {testResult && <TestBadge result={testResult} />}
      <div className="flex gap-2">
        <button onClick={() => onSave({ ...provider, api_key: key, model, enabled: !!key })}
          className="rounded-lg bg-gray-900 px-3 py-1.5 text-xs text-white hover:bg-gray-700 dark:bg-gray-100 dark:text-gray-900">{t('mcpAgent.save')}</button>
        <button onClick={() => onTest({ ...provider, api_key: key, model })} disabled={testing || !key}
          className="flex items-center gap-1 rounded-lg border border-gray-200 px-3 py-1.5 text-xs text-gray-600 hover:bg-gray-50 disabled:opacity-40 dark:border-gray-700">
          {testing ? <Loader2 className="h-3 w-3 animate-spin" /> : null} {t('mcpAgent.test')}
        </button>
      </div>
    </div>
  )
}

function TestBadge({ result, small }: { result: { ok: boolean; msg: string }; small?: boolean }) {
  return (
    <div className={`flex items-center gap-1.5 rounded-lg px-3 py-1.5 ${small ? 'text-xs' : 'text-xs'} ${
      result.ok ? 'bg-green-50 text-green-700 dark:bg-green-900/20 dark:text-green-400'
                : 'bg-red-50 text-red-600 dark:bg-red-900/20 dark:text-red-400'
    }`}>
      {result.ok ? <CheckCircle className="h-3.5 w-3.5 shrink-0" /> : <XCircle className="h-3.5 w-3.5 shrink-0" />}
      <span className="truncate">{result.msg}</span>
    </div>
  )
}

function Toggle({ enabled, onChange }: { enabled: boolean; onChange: (v: boolean) => void }) {
  return (
    <button onClick={() => onChange(!enabled)}
      className={`relative h-5 w-9 rounded-full transition-colors ${enabled ? 'bg-purple-600' : 'bg-gray-200 dark:bg-gray-700'}`}>
      <span className={`absolute top-0.5 h-4 w-4 rounded-full bg-white shadow transition-transform ${enabled ? 'translate-x-4' : 'translate-x-0.5'}`} />
    </button>
  )
}

// ── Markdown renderer ────────────────────────────────────────────────────────

function MdContent({ content }: { content: string }) {
  // Strip <think>...</think> blocks (chain-of-thought from some models)
  const clean = content.replace(/<think>[\s\S]*?<\/think>/g, '').trim()

  return (
    <ReactMarkdown
      remarkPlugins={[remarkGfm]}
      components={{
        // Paragraphs
        p: ({ children }) => <p className="mb-2 last:mb-0 leading-relaxed">{children}</p>,
        // Headings
        h1: ({ children }) => <h1 className="text-base font-bold mt-3 mb-1">{children}</h1>,
        h2: ({ children }) => <h2 className="text-sm font-bold mt-3 mb-1">{children}</h2>,
        h3: ({ children }) => <h3 className="text-sm font-semibold mt-2 mb-1">{children}</h3>,
        // Code blocks
        code: ({ className, children, ...props }) => {
          const isBlock = className?.includes('language-')
          return isBlock ? (
            <code className="block overflow-x-auto rounded-lg bg-gray-900 dark:bg-black px-3 py-2 text-xs font-mono text-green-300 my-2 whitespace-pre">
              {children}
            </code>
          ) : (
            <code className="rounded px-1 py-0.5 bg-gray-200 dark:bg-gray-700 text-xs font-mono text-purple-700 dark:text-purple-300" {...props}>
              {children}
            </code>
          )
        },
        pre: ({ children }) => <>{children}</>,
        // Lists
        ul: ({ children }) => <ul className="list-disc list-inside mb-2 space-y-0.5 text-sm">{children}</ul>,
        ol: ({ children }) => <ol className="list-decimal list-inside mb-2 space-y-0.5 text-sm">{children}</ol>,
        li: ({ children }) => <li className="leading-relaxed">{children}</li>,
        // Table
        table: ({ children }) => (
          <div className="overflow-x-auto my-2">
            <table className="text-xs border-collapse w-full">{children}</table>
          </div>
        ),
        thead: ({ children }) => <thead className="bg-gray-200 dark:bg-gray-700">{children}</thead>,
        th: ({ children }) => <th className="border border-gray-300 dark:border-gray-600 px-2 py-1 text-left font-semibold">{children}</th>,
        td: ({ children }) => <td className="border border-gray-300 dark:border-gray-600 px-2 py-1">{children}</td>,
        // Blockquote
        blockquote: ({ children }) => (
          <blockquote className="border-l-2 border-purple-400 pl-3 italic text-gray-500 dark:text-gray-400 my-2">
            {children}
          </blockquote>
        ),
        // HR
        hr: () => <hr className="border-gray-300 dark:border-gray-600 my-3" />,
        // Strong / em
        strong: ({ children }) => <strong className="font-semibold">{children}</strong>,
        em: ({ children }) => <em className="italic">{children}</em>,
        // Links
        a: ({ href, children }) => (
          <a href={href} target="_blank" rel="noreferrer" className="text-purple-600 dark:text-purple-400 underline hover:no-underline">
            {children}
          </a>
        ),
      }}
    >
      {clean}
    </ReactMarkdown>
  )
}

function F({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div>
      <label className="mb-1 block text-xs font-medium text-gray-500 dark:text-gray-400">{label}</label>
      {children}
    </div>
  )
}
