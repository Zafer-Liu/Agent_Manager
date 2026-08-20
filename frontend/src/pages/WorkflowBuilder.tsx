import { useCallback, useEffect, useMemo, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import {
  DndContext, useDraggable, DragOverlay,
  PointerSensor, useSensor, useSensors, type DragStartEvent, type DragEndEvent,
} from '@dnd-kit/core'
import {
  Plus, Trash2, Save, Loader2, Wrench, Sparkles,
  ArrowUp, ArrowDown, AlertCircle, ChevronDown, ChevronRight, X, Bot, Play, GitBranch, Shuffle,
} from 'lucide-react'
import { useTranslation } from 'react-i18next'

// ── Types (mirror backend workflow.rs) ───────────────────────────────────────

interface McpServer {
  name: string; command: string; args: string[]; env: Record<string, string>
  transport?: string; url?: string; headers?: Record<string, string>; description?: string
}

interface McpToolInfo {
  server: string
  name: string
  description: string
  input_schema: unknown
}

interface FanOutConfig {
  split: { by_field?: { field: string }; static?: { items: string[] }; llm_split?: { count: number } }
  child_node_id: string
  converge: 'and' | 'or'
  on_child_fail: 'fail_parent' | 'continue' | 'cancel_siblings'
}

interface AgentCandidate {
  id: string
  url: string
  capabilities?: string[]
  priority?: number
}

interface DispatchConfig {
  strategy: 'fixed' | 'failover' | 'capability_match' | 'random'
  candidates: AgentCandidate[]
  required_capabilities?: string[]
  timeout_secs?: number
}

interface AcceptanceConfig {
  notify: string[]
  allow_reject_to: string[]
  timeout_secs: number | null
  timeout_action: 'remind' | 'auto_pass' | 'auto_fail'
}

interface WorkflowNode {
  id: string
  kind: 'tool' | 'llm' | 'mcp_agent' | 'agent_task' | 'fan_out' | 'acceptance'
  label: string
  server: string
  tool: string
  arguments: Record<string, unknown>
  prompt: string
  position?: { x: number; y: number }
  acceptance?: AcceptanceConfig | null
  output_contract?: Record<string, unknown> | null
  fan_out?: FanOutConfig | null
  dispatch?: DispatchConfig | null
}

interface WorkflowEdge {
  from: string
  to: string
  condition?: { verdict?: 'pass' | 'fail' | 'blocked' }
}

interface Workflow {
  id: string
  name: string
  description: string
  nodes: WorkflowNode[]
  edges?: WorkflowEdge[]
  callback_url?: string | null
  schedule?: string | null
  entry_node_id?: string
  end_node_ids?: string[]
  created_at: string
  updated_at: string
}

function uid() {
  return `n_${Date.now()}_${Math.random().toString(36).slice(2, 7)}`
}

/// 计算新节点的默认位置（水平排列，向右偏移）
function nextPosition(nodes: WorkflowNode[]): { x: number; y: number } {
  if (nodes.length === 0) return { x: 40, y: 120 }
  const last = nodes[nodes.length - 1]
  return { x: (last.position?.x ?? 40) + 220, y: last.position?.y ?? 120 }
}

/// 轻量 cron 校验：5 字段（分 时 日 月 周），每字段支持 * / N / */N / 数字列表 / 范围
function validateCron(expr: string): string {
  const parts = expr.trim().split(/\s+/)
  if (parts.length !== 5) return 'invalid'
  const ranges = [[0, 59], [0, 23], [1, 31], [1, 12], [0, 6]]
  for (let i = 0; i < 5; i++) {
    const field = parts[i]
    const [lo, hi] = ranges[i]
    // Allow * or */N
    if (field === '*' || /^\*\/\d+$/.test(field)) continue
    // Allow comma-separated values/ranges
    const items = field.split(',')
    for (const item of items) {
      const m = item.match(/^(\d+)-(\d+)$/)
      if (m) {
        const a = parseInt(m[1]), b = parseInt(m[2])
        if (a < lo || b > hi || a > b) return 'invalid'
        continue
      }
      if (/^\d+\/\d+$/.test(item)) {
        const n = parseInt(item.split('/')[0])
        if (n < lo || n > hi) return 'invalid'
        continue
      }
      const n = parseInt(item)
      if (isNaN(n) || n < lo || n > hi) return 'invalid'
    }
  }
  return ''
}

/// 添加节点并自动连边：如果有上一个节点，自动连一条 OnVerdict:Pass 边
function addNodeWithEdge(wf: Workflow, node: WorkflowNode): Workflow {
  const edges = [...(wf.edges ?? [])]
  const lastNode = wf.nodes.length > 0 ? wf.nodes[wf.nodes.length - 1] : null
  if (lastNode) {
    edges.push({ from: lastNode.id, to: node.id, condition: { verdict: 'pass' } })
  }
  return { ...wf, nodes: [...wf.nodes, node], edges }
}

// ── Main component ───────────────────────────────────────────────────────────

export function WorkflowBuilder({ enabledServers, allServers, enabledNames, onToggleServer, onWorkflowsChange }: {
  enabledServers: McpServer[]
  allServers: McpServer[]
  enabledNames: string[]
  onToggleServer: (name: string) => void
  onWorkflowsChange?: (wfs: Workflow[]) => void
}) {
  const { t } = useTranslation()
  const [workflows, setWorkflows] = useState<Workflow[]>([])
  const [editing, setEditing] = useState<Workflow | null>(null)
  const [tools, setTools] = useState<McpToolInfo[]>([])
  const [loadingTools, setLoadingTools] = useState(false)
  const [error, setError] = useState('')
  const [saving, setSaving] = useState(false)
  const [activeDrag, setActiveDrag] = useState<McpToolInfo | null>(null)
  const [testing, setTesting] = useState(false)
  const [testInput, setTestInput] = useState('')
  const [testResult, setTestResult] = useState<{ steps: { kind: string; content: string; tool?: string }[]; finalAnswer: string; error?: string } | null>(null)
  const [showTestDialog, setShowTestDialog] = useState(false)
  const [cronError, setCronError] = useState('')

  const sensors = useSensors(useSensor(PointerSensor, { activationConstraint: { distance: 6 } }))

  const loadWorkflows = useCallback(async () => {
    const list = await invoke<Workflow[]>('list_workflows')
    setWorkflows(list)
    onWorkflowsChange?.(list)
  }, [onWorkflowsChange])
  useEffect(() => { loadWorkflows() }, [loadWorkflows])

  // Load tools whenever the enabled server set changes.
  const serverKey = enabledServers.map(s => s.name).join(',')
  useEffect(() => {
    let cancelled = false
    if (enabledServers.length === 0) { setTools([]); return }
    setLoadingTools(true)
    invoke<McpToolInfo[]>('list_mcp_tools', { servers: enabledServers })
      .then(ts => { if (!cancelled) setTools(ts) })
      .catch(() => { if (!cancelled) setTools([]) })
      .finally(() => { if (!cancelled) setLoadingTools(false) })
    return () => { cancelled = true }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [serverKey])

  function startNew() {
    setEditing({
      id: uid(), name: '', description: '', nodes: [],
      created_at: '', updated_at: '',
    })
    setError('')
    setCronError('')
  }

  function startEdit(w: Workflow) {
    setEditing(JSON.parse(JSON.stringify(w)))
    setError('')
    setCronError('')
  }

  function addToolNode(tool: McpToolInfo) {
    setEditing(w => {
      if (!w) return w
      const pos = nextPosition(w.nodes)
      const node: WorkflowNode = {
        id: uid(), kind: 'tool', label: `${tool.server}__${tool.name}`,
        server: tool.server, tool: tool.name, arguments: {}, prompt: '', position: pos,
      }
      return addNodeWithEdge(w, node)
    })
  }

  function addLlmNode() {
    setEditing(w => {
      if (!w) return w
      const pos = nextPosition(w.nodes)
      const node: WorkflowNode = {
        id: uid(), kind: 'llm', label: t('mcpAgent.llmNode'),
        server: '', tool: '', arguments: {}, prompt: '', position: pos,
      }
      return addNodeWithEdge(w, node)
    })
  }

  function addMcpAgentNode(server: McpServer) {
    setEditing(w => {
      if (!w) return w
      const pos = nextPosition(w.nodes)
      const node: WorkflowNode = {
        id: uid(), kind: 'mcp_agent', label: server.name,
        server: server.name, tool: '', arguments: {}, prompt: '', position: pos,
      }
      return addNodeWithEdge(w, node)
    })
  }

  function addAgentTaskNode() {
    setEditing(w => {
      if (!w) return w
      const pos = nextPosition(w.nodes)
      const node: WorkflowNode = {
        id: uid(), kind: 'agent_task', label: t('wfBuilder.agentTask'),
        server: '', tool: '', arguments: {}, prompt: '', position: pos,
      }
      return addNodeWithEdge(w, node)
    })
  }

  function addAcceptanceNode() {
    setEditing(w => {
      if (!w) return w
      const pos = nextPosition(w.nodes)
      const node: WorkflowNode = {
        id: uid(), kind: 'acceptance', label: t('wfBuilder.acceptance'),
        server: '', tool: '', arguments: {}, prompt: '', position: pos,
        acceptance: { notify: [], allow_reject_to: [], timeout_secs: null, timeout_action: 'remind' },
      }
      return addNodeWithEdge(w, node)
    })
  }

  function addFanOutNode() {
    setEditing(w => {
      if (!w) return w
      const pos = nextPosition(w.nodes)
      const node: WorkflowNode = {
        id: uid(), kind: 'fan_out', label: t('wfBuilder.fanOut'),
        server: '', tool: '', arguments: {}, prompt: '', position: pos,
        fan_out: {
          split: { static: { items: ['task1', 'task2'] } },
          child_node_id: '',
          converge: 'and',
          on_child_fail: 'fail_parent',
        },
      }
      return addNodeWithEdge(w, node)
    })
  }

  function updateNode(id: string, patch: Partial<WorkflowNode>) {
    setEditing(w => w ? { ...w, nodes: w.nodes.map(n => n.id === id ? { ...n, ...patch } : n) } : w)
  }
  function removeNode(id: string) {
    setEditing(w => w ? {
      ...w,
      nodes: w.nodes.filter(n => n.id !== id),
      edges: (w.edges ?? []).filter(e => e.from !== id && e.to !== id),
    } : w)
  }
  function updatePosition(id: string, x: number, y: number) {
    setEditing(w => w ? { ...w, nodes: w.nodes.map(n => n.id === id ? { ...n, position: { x, y } } : n) } : w)
  }
  function moveNode(id: string, dir: -1 | 1) {
    setEditing(w => {
      if (!w) return w
      const idx = w.nodes.findIndex(n => n.id === id)
      const next = idx + dir
      if (idx < 0 || next < 0 || next >= w.nodes.length) return w
      const nodes = [...w.nodes]
      ;[nodes[idx], nodes[next]] = [nodes[next], nodes[idx]]
      return { ...w, nodes }
    })
  }

  async function save() {
    if (!editing) return
    if (!editing.name.trim()) { setError(t('mcpAgent.workflowNameRequired')); return }
    if (editing.nodes.length === 0) { setError(t('mcpAgent.workflowNeedsNodes')); return }
    if (editing.schedule) {
      const err = validateCron(editing.schedule)
      if (err) { setError(t('wfBuilder.cronInvalid')); return }
    }
    setSaving(true); setError('')
    try {
      await invoke('save_workflow', { workflow: editing })
      await loadWorkflows()
    } catch (e) { setError(String(e)) }
    finally { setSaving(false) }
  }

  async function del(id: string) {
    await invoke('delete_workflow', { id })
    await loadWorkflows()
    if (editing?.id === id) setEditing(null)
  }

  async function testRun() {
    if (!editing || editing.nodes.length === 0) return
    setTesting(true)
    setTestResult(null)
    try {
      const result = await invoke<{ steps: { kind: string; content: string; tool?: string }[]; final_answer: string; success: boolean; error?: string }>('run_workflow', {
        workflow: editing,
        input: testInput || '',
      })
      setTestResult({
        steps: result.steps || [],
        finalAnswer: result.final_answer || '',
        error: result.error,
      })
    } catch (e) {
      setTestResult({ steps: [], finalAnswer: '', error: String(e) })
    } finally {
      setTesting(false)
    }
  }

  function onDragStart(e: DragStartEvent) {
    const tool = e.active.data.current?.tool as McpToolInfo | undefined
    setActiveDrag(tool ?? null)
  }
  function onDragEnd(e: DragEndEvent) {
    setActiveDrag(null)
    // DAG 画布不支持 drop target，拖拽后直接添加（与点击行为一致）
    const tool = e.active.data.current?.tool as McpToolInfo | undefined
    if (tool && editing) addToolNode(tool)
  }

  const groupedTools = useMemo(() => {
    const map: Record<string, McpToolInfo[]> = {}
    for (const tl of tools) (map[tl.server] ??= []).push(tl)
    return map
  }, [tools])

  return (
    <DndContext sensors={sensors} onDragStart={onDragStart} onDragEnd={onDragEnd}>
      <div className="flex h-full overflow-hidden">
        {/* Left: workflow list + tool palette */}
        <div className="w-64 shrink-0 border-r border-gray-200 dark:border-gray-700 flex flex-col">
          {/* Workflow list */}
          <div className="flex items-center justify-between px-3 py-2 border-b border-gray-100 dark:border-gray-800">
            <span className="text-xs font-medium text-gray-500">{t('mcpAgent.workflowList')}</span>
            <button onClick={startNew} className="rounded p-1 text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-800">
              <Plus className="h-3.5 w-3.5" />
            </button>
          </div>
          <div className="max-h-44 overflow-y-auto p-2 space-y-0.5 border-b border-gray-100 dark:border-gray-800">
            {workflows.length === 0 && <p className="py-3 text-center text-xs text-gray-400">{t('mcpAgent.noWorkflows')}</p>}
            {workflows.map(w => (
              <button key={w.id} onClick={() => startEdit(w)}
                className={`group flex w-full items-center gap-2 rounded-lg px-2.5 py-1.5 text-left transition-colors ${
                  editing?.id === w.id
                    ? 'bg-purple-50 text-purple-700 dark:bg-purple-900/20 dark:text-purple-300'
                    : 'hover:bg-gray-50 dark:hover:bg-gray-800'
                }`}>
                <Sparkles className="h-3.5 w-3.5 shrink-0 text-purple-400" />
                <div className="min-w-0 flex-1">
                  <p className="truncate text-xs font-medium text-gray-900 dark:text-gray-100">{w.name || '(untitled)'}</p>
                  <p className="truncate text-xs text-gray-400">{w.nodes.length} {t('mcpAgent.step').toLowerCase()}</p>
                </div>
                <button onClick={e => { e.stopPropagation(); del(w.id) }}
                  className="shrink-0 opacity-0 group-hover:opacity-100 text-gray-300 hover:text-red-500">
                  <Trash2 className="h-3 w-3" />
                </button>
              </button>
            ))}
          </div>

          {/* Server toggles */}
          {allServers.length > 0 && (
            <div className="px-2.5 py-2 border-b border-gray-100 dark:border-gray-800 flex flex-wrap gap-1">
              {allServers.map(s => (
                <button key={s.name} onClick={() => onToggleServer(s.name)}
                  className={`rounded-md px-2 py-0.5 text-[11px] font-medium transition-colors ${
                    enabledNames.includes(s.name)
                      ? 'bg-purple-100 text-purple-700 dark:bg-purple-900/40 dark:text-purple-300'
                      : 'bg-gray-100 text-gray-500 dark:bg-gray-700 dark:text-gray-400'
                  }`}>
                  🔌 {s.name}
                </button>
              ))}
            </div>
          )}

          {/* Tool palette */}
          <div className="px-3 py-2 border-b border-gray-100 dark:border-gray-800">
            <span className="text-xs font-medium text-gray-500">{t('mcpAgent.toolPalette')}</span>
          </div>
          <div className="flex-1 overflow-y-auto p-2 space-y-2">
            {enabledServers.length === 0 ? (
              <p className="px-1 py-3 text-xs text-gray-400">{t('mcpAgent.paletteHintEnable')}</p>
            ) : loadingTools ? (
              <div className="flex items-center gap-2 px-1 py-3 text-xs text-gray-400">
                <Loader2 className="h-3.5 w-3.5 animate-spin" /> {t('mcpAgent.loadingTools')}
              </div>
            ) : (
              <>
                <p className="px-1 text-[11px] text-gray-400">{t('mcpAgent.paletteHintDrag')}</p>
                {Object.entries(groupedTools).map(([server, list]) => (
                  <div key={server} className="space-y-1">
                    <div className="flex items-center justify-between px-1">
                      <p className="text-[11px] font-semibold uppercase tracking-wide text-gray-400">🔌 {server}</p>
                      <button
                        onClick={() => {
                          const srv = allServers.find(s => s.name === server)
                          if (srv && editing) addMcpAgentNode(srv)
                        }}
                        disabled={!editing}
                        title={t('mcpAgent.addMcpAgentNode')}
                        className="flex items-center gap-0.5 rounded px-1.5 py-0.5 text-[10px] font-medium text-purple-500 hover:bg-purple-50 disabled:opacity-40 dark:hover:bg-purple-900/20">
                        <Bot className="h-3 w-3" />
                        <Plus className="h-2.5 w-2.5" />
                      </button>
                    </div>
                    {list.map(tl => (
                      <PaletteTool key={`${server}.${tl.name}`} tool={tl} onAdd={() => editing && addToolNode(tl)} />
                    ))}
                  </div>
                ))}
              </>
            )}
          </div>
        </div>

        {/* Right: canvas / editor */}
        <div className="flex-1 overflow-y-auto">
          {editing ? (
            <div className="max-w-3xl p-5 space-y-4">
              {/* Header: name + actions */}
              <div className="flex items-start gap-2">
                <div className="flex-1 space-y-1.5">
                  <input value={editing.name}
                    onChange={e => setEditing(w => w ? { ...w, name: e.target.value } : w)}
                    placeholder={t('mcpAgent.workflowNamePlaceholder')}
                    className="field-input font-medium" />
                  <input value={editing.description}
                    onChange={e => setEditing(w => w ? { ...w, description: e.target.value } : w)}
                    placeholder={t('mcpAgent.workflowDescPlaceholder')}
                    className="field-input text-xs" />
                </div>
                <button onClick={() => setEditing(null)} className="mt-1 text-gray-400 hover:text-gray-600">
                  <X className="h-5 w-5" />
                </button>
              </div>

              {/* Trigger & callback config */}
              <div className="grid grid-cols-2 gap-2">
                <div>
                  <label className="mb-0.5 block text-[10px] font-medium text-gray-400">{t('wfBuilder.callbackUrl')}</label>
                  <input
                    value={editing.callback_url ?? ''}
                    onChange={e => setEditing(w => w ? { ...w, callback_url: e.target.value || null } : w)}
                    placeholder="https://…"
                    className="field-input text-[11px] font-mono" />
                </div>
                <div>
                  <label className="mb-0.5 block text-[10px] font-medium text-gray-400">{t('wfBuilder.scheduleLabel')}</label>
                  <input
                    value={editing.schedule ?? ''}
                    onChange={e => {
                      const v = e.target.value || null
                      setEditing(w => w ? { ...w, schedule: v } : w)
                      setCronError(v ? validateCron(v) : '')
                    }}
                    placeholder={t('wfBuilder.schedulePlaceholder')}
                    className={`field-input text-[11px] font-mono ${cronError ? 'border-red-400' : ''}`} />
                  {cronError && <p className="mt-0.5 text-[10px] text-red-500">{t('wfBuilder.cronInvalid')}</p>}
                </div>
              </div>

              <div className="flex flex-wrap items-center gap-2">
                <button onClick={addLlmNode}
                  className="flex items-center gap-1.5 rounded-lg border border-gray-200 px-2.5 py-1.5 text-xs text-gray-600 hover:bg-gray-50 dark:border-gray-700 dark:text-gray-300 dark:hover:bg-gray-800">
                  <Sparkles className="h-3.5 w-3.5 text-purple-500" /> {t('mcpAgent.addLlmNode')}
                </button>
                <button onClick={addAgentTaskNode}
                  className="flex items-center gap-1.5 rounded-lg border border-gray-200 px-2.5 py-1.5 text-xs text-gray-600 hover:bg-gray-50 dark:border-gray-700 dark:text-gray-300 dark:hover:bg-gray-800">
                  <Bot className="h-3.5 w-3.5 text-blue-500" /> {t('wfBuilder.addAgentTask')}
                </button>
                <button onClick={addAcceptanceNode}
                  className="flex items-center gap-1.5 rounded-lg border border-gray-200 px-2.5 py-1.5 text-xs text-gray-600 hover:bg-gray-50 dark:border-gray-700 dark:text-gray-300 dark:hover:bg-gray-800">
                  <AlertCircle className="h-3.5 w-3.5 text-amber-500" /> {t('wfBuilder.addAcceptance')}
                </button>
                <button onClick={addFanOutNode}
                  className="flex items-center gap-1.5 rounded-lg border border-gray-200 px-2.5 py-1.5 text-xs text-gray-600 hover:bg-gray-50 dark:border-gray-700 dark:text-gray-300 dark:hover:bg-gray-800">
                  <GitBranch className="h-3.5 w-3.5 text-green-500" /> {t('wfBuilder.addFanOut')}
                </button>
                <button onClick={save} disabled={saving}
                  className="flex items-center gap-1.5 rounded-lg bg-gray-900 px-2.5 py-1.5 text-xs text-white hover:bg-gray-700 disabled:opacity-60 dark:bg-gray-100 dark:text-gray-900">
                  {saving ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Save className="h-3.5 w-3.5" />} {t('mcpAgent.saveWorkflow')}
                </button>
                <button
                  onClick={() => { setShowTestDialog(true); setTestResult(null) }}
                  disabled={testing || !editing || editing.nodes.length === 0}
                  className="flex items-center gap-1.5 rounded-lg border border-purple-200 px-2.5 py-1.5 text-xs text-purple-600 hover:bg-purple-50 disabled:opacity-40 dark:border-purple-800 dark:text-purple-400 dark:hover:bg-purple-900/20"
                >
                  <Play className="h-3.5 w-3.5" /> {t('mcpAgent.testRun')}
                </button>
                <span className="text-xs text-gray-400">{t('mcpAgent.workflowRunInChat')}</span>
              </div>

              {error && (
                <div className="flex items-center gap-2 rounded-lg bg-red-50 dark:bg-red-900/20 px-3 py-2 text-xs text-red-600 dark:text-red-400">
                  <AlertCircle className="h-3.5 w-3.5 shrink-0" />{error}
                </div>
              )}

              {/* Canvas (SVG DAG 画布) */}
              <Canvas
                nodes={editing.nodes}
                edges={editing.edges ?? []}
                onUpdate={updateNode}
                onRemove={removeNode}
                onMove={moveNode}
                onMovePos={updatePosition}
              />

              {/* Test run dialog */}
              {showTestDialog && (
                <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40">
                  <div className="rounded-2xl border border-gray-200 bg-white p-5 shadow-xl dark:border-gray-700 dark:bg-gray-900 w-[480px] max-h-[80vh] flex flex-col">
                    <div className="flex items-center justify-between mb-3">
                      <h3 className="text-sm font-semibold text-gray-900 dark:text-gray-100">{t('mcpAgent.testRunTitle')}</h3>
                      <button onClick={() => setShowTestDialog(false)} className="text-gray-400 hover:text-gray-600">
                        <X className="h-4 w-4" />
                      </button>
                    </div>

                    <div className="space-y-3 flex-1 overflow-y-auto">
                      {/* Input */}
                      <div>
                        <label className="mb-1 block text-xs font-medium text-gray-500">{t('mcpAgent.testInput')}</label>
                        <textarea
                          value={testInput}
                          onChange={e => setTestInput(e.target.value)}
                          rows={2}
                          placeholder={t('mcpAgent.testInputPlaceholder')}
                          className="field-input resize-none text-xs"
                        />
                      </div>

                      {/* Run button */}
                      <button
                        onClick={testRun}
                        disabled={testing}
                        className="flex items-center gap-1.5 rounded-lg bg-purple-600 px-3 py-1.5 text-xs text-white hover:bg-purple-500 disabled:opacity-60"
                      >
                        {testing ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Play className="h-3.5 w-3.5" />}
                        {testing ? t('mcpAgent.running') : t('mcpAgent.run')}
                      </button>

                      {/* Results */}
                      {testResult && (
                        <div className="space-y-2">
                          {testResult.error && (
                            <div className="rounded-lg bg-red-50 dark:bg-red-900/20 px-3 py-2 text-xs text-red-600 dark:text-red-400">
                              {testResult.error}
                            </div>
                          )}
                          {testResult.steps.map((step, i) => (
                            <div key={i} className="rounded-lg border border-gray-200 dark:border-gray-700 px-3 py-2">
                              <p className="text-[10px] font-mono text-purple-500 mb-1">
                                Step {i + 1}: {step.kind}
                                {step.tool && ` · ${step.tool}`}
                              </p>
                              <pre className="overflow-x-auto text-[11px] font-mono text-gray-600 dark:text-gray-400 whitespace-pre-wrap max-h-32 overflow-y-auto">
                                {step.content}
                              </pre>
                            </div>
                          ))}
                          {testResult.finalAnswer && (
                            <div className="rounded-lg bg-green-50 dark:bg-green-900/20 px-3 py-2">
                              <p className="text-[10px] font-mono text-green-600 mb-1">{t('mcpAgent.finalAnswer')}</p>
                              <p className="text-xs text-gray-700 dark:text-gray-300 whitespace-pre-wrap">{testResult.finalAnswer}</p>
                            </div>
                          )}
                        </div>
                      )}
                    </div>
                  </div>
                </div>
              )}
            </div>
          ) : (
            <div className="flex h-full items-center justify-center text-gray-400">
              <div className="text-center space-y-2">
                <Sparkles className="mx-auto h-10 w-10 opacity-20" />
                <p className="text-sm">{t('mcpAgent.selectWorkflowHint')}</p>
              </div>
            </div>
          )}
        </div>
      </div>

      <DragOverlay>
        {activeDrag && (
          <div className="rounded-lg border border-purple-300 bg-white px-2.5 py-1.5 text-xs shadow-lg dark:bg-gray-800">
            <Wrench className="mr-1 inline h-3 w-3 text-orange-500" />{activeDrag.name}
          </div>
        )}
      </DragOverlay>
    </DndContext>
  )
}

// ── Palette tool (draggable) ─────────────────────────────────────────────────

function PaletteTool({ tool, onAdd }: { tool: McpToolInfo; onAdd: () => void }) {
  const { attributes, listeners, setNodeRef, isDragging } = useDraggable({
    id: `tool:${tool.server}:${tool.name}`,
    data: { tool },
  })
  return (
    <div ref={setNodeRef} {...attributes} {...listeners}
      onClick={onAdd}
      className={`group flex cursor-grab items-start gap-1.5 rounded-lg border border-gray-200 bg-white px-2 py-1.5 dark:border-gray-700 dark:bg-gray-900 ${
        isDragging ? 'opacity-40' : 'hover:border-purple-300'
      }`}
      title={tool.description}>
      <Wrench className="mt-0.5 h-3 w-3 shrink-0 text-orange-500" />
      <div className="min-w-0 flex-1">
        <p className="truncate text-xs font-medium text-gray-800 dark:text-gray-200">{tool.name}</p>
        {tool.description && <p className="truncate text-[11px] text-gray-400">{tool.description}</p>}
      </div>
      <Plus className="mt-0.5 h-3 w-3 shrink-0 text-gray-300 group-hover:text-purple-500" />
    </div>
  )
}

// ── Canvas (SVG DAG 画布) ─────────────────────────────────────────────────────

const NODE_W = 180
const NODE_H = 56

function Canvas({ nodes, edges, onUpdate, onRemove, onMove, onMovePos }: {
  nodes: WorkflowNode[]
  edges: WorkflowEdge[]
  onUpdate: (id: string, patch: Partial<WorkflowNode>) => void
  onRemove: (id: string) => void
  onMove: (id: string, dir: -1 | 1) => void
  onMovePos: (id: string, x: number, y: number) => void
}) {
  const { t } = useTranslation()
  const [selectedId, setSelectedId] = useState<string | null>(null)

  // 计算画布尺寸
  const maxX = Math.max(...nodes.map(n => (n.position?.x ?? 0) + NODE_W), 600)
  const maxY = Math.max(...nodes.map(n => (n.position?.y ?? 0) + NODE_H), 400)

  return (
    <div className="relative overflow-auto rounded-xl border-2 border-dashed border-gray-200 dark:border-gray-700 bg-gray-50/30 dark:bg-gray-900/30"
      style={{ minHeight: 300 }}
      onClick={() => setSelectedId(null)}
    >
      {nodes.length === 0 ? (
        <div className="flex h-48 items-center justify-center text-center text-xs text-gray-400">
          {t('mcpAgent.canvasEmpty')}
        </div>
      ) : (
        <div style={{ width: maxX + 80, height: maxY + 80, position: 'relative' }}>
          {/* SVG 连线层 */}
          <svg className="absolute inset-0 pointer-events-none" width={maxX + 80} height={maxY + 80}>
            <defs>
              <marker id="arrowhead" markerWidth="8" markerHeight="6" refX="7" refY="3" orient="auto">
                <polygon points="0 0, 8 3, 0 6" fill="#a78bfa" />
              </marker>
            </defs>
            {edges.map((e, i) => {
              const from = nodes.find(n => n.id === e.from)
              const to = nodes.find(n => n.id === e.to)
              if (!from || !to) return null
              const x1 = (from.position?.x ?? 0) + NODE_W
              const y1 = (from.position?.y ?? 0) + NODE_H / 2
              const x2 = to.position?.x ?? 0
              const y2 = (to.position?.y ?? 0) + NODE_H / 2
              const midX = (x1 + x2) / 2
              const color = e.condition?.verdict === 'fail' ? '#ef4444'
                : e.condition?.verdict === 'blocked' ? '#f59e0b'
                : '#a78bfa'
              return (
                <g key={i}>
                  <path
                    d={`M ${x1} ${y1} C ${midX} ${y1}, ${midX} ${y2}, ${x2} ${y2}`}
                    stroke={color}
                    strokeWidth={2}
                    fill="none"
                    markerEnd="url(#arrowhead)"
                  />
                  {e.condition?.verdict && e.condition.verdict !== 'pass' && (
                    <text x={midX} y={(y1 + y2) / 2 - 4} className="text-[10px]" fill={color}>
                      {e.condition.verdict}
                    </text>
                  )}
                </g>
              )
            })}
          </svg>

          {/* 节点层 */}
          {nodes.map((n, i) => (
            <DagNode
              key={n.id}
              node={n}
              index={i}
              total={nodes.length}
              selected={selectedId === n.id}
              onSelect={() => { setSelectedId(n.id) }}
              onUpdate={onUpdate}
              onRemove={onRemove}
              onMove={onMove}
              onMovePos={onMovePos}
            />
          ))}
        </div>
      )}
    </div>
  )
}

/// 可拖拽 DAG 节点
function DagNode({ node, index, total, selected, onSelect, onUpdate, onRemove, onMove, onMovePos }: {
  node: WorkflowNode
  index: number
  total: number
  selected: boolean
  onSelect: () => void
  onUpdate: (id: string, patch: Partial<WorkflowNode>) => void
  onRemove: (id: string) => void
  onMove: (id: string, dir: -1 | 1) => void
  onMovePos: (id: string, x: number, y: number) => void
}) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(false)
  const [dragging, setDragging] = useState(false)
  const dragRef = useRef<{ startX: number; startY: number; origX: number; origY: number } | null>(null)

  const isTool = node.kind === 'tool'
  const isMcpAgent = node.kind === 'mcp_agent'
  const isAgentTask = node.kind === 'agent_task'
  const isAcceptance = node.kind === 'acceptance'
  const isFanOut = node.kind === 'fan_out'

  const kindLabel = isTool ? t('mcpAgent.nodeKindTool')
    : isMcpAgent ? t('mcpAgent.nodeKindMcpAgent')
    : isAgentTask ? 'Agent'
    : isAcceptance ? 'Acceptance'
    : isFanOut ? 'Fan-out'
    : t('mcpAgent.nodeKindLlm')

  const nodeIcon = isTool ? <Wrench className="h-3.5 w-3.5 text-orange-500" />
    : isMcpAgent ? <Bot className="h-3.5 w-3.5 text-purple-500" />
    : isAcceptance ? <AlertCircle className="h-3.5 w-3.5 text-blue-500" />
    : <Sparkles className="h-3.5 w-3.5 text-purple-500" />

  const nodeTitle = isTool ? `${node.server}__${node.tool}`
    : isMcpAgent ? `🔌 ${node.server}`
    : isAgentTask ? `🤖 ${node.label || 'agent'}`
    : isAcceptance ? '✓ Acceptance'
    : isFanOut ? '⏃ Fan-out'
    : t('mcpAgent.llmNode')

  function onMouseDown(e: React.MouseEvent) {
    e.stopPropagation()
    onSelect()
    const startX = e.clientX
    const startY = e.clientY
    const origX = node.position?.x ?? 0
    const origY = node.position?.y ?? 0
    dragRef.current = { startX, startY, origX, origY }
    setDragging(true)

    const onMove2 = (ev: MouseEvent) => {
      if (!dragRef.current) return
      const dx = ev.clientX - dragRef.current.startX
      const dy = ev.clientY - dragRef.current.startY
      onMovePos(node.id, dragRef.current.origX + dx, dragRef.current.origY + dy)
    }
    const onUp = () => {
      setDragging(false)
      dragRef.current = null
      window.removeEventListener('mousemove', onMove2)
      window.removeEventListener('mouseup', onUp)
    }
    window.addEventListener('mousemove', onMove2)
    window.addEventListener('mouseup', onUp)
  }

  return (
    <div
      className={`absolute rounded-xl border bg-white dark:bg-gray-900 shadow-sm transition-shadow ${
        selected ? 'border-purple-400 ring-2 ring-purple-200 dark:ring-purple-900/40'
        : 'border-gray-200 dark:border-gray-700 hover:shadow-md'
      } ${dragging ? 'cursor-grabbing opacity-80' : 'cursor-grab'}`}
      style={{ left: node.position?.x ?? 0, top: node.position?.y ?? 0, width: NODE_W }}
      onMouseDown={onMouseDown}
    >
      <div className="flex items-center gap-1.5 px-2.5 py-1.5">
        <span className="flex h-4 w-4 shrink-0 items-center justify-center rounded-full bg-purple-100 text-[10px] font-semibold text-purple-700 dark:bg-purple-900/40 dark:text-purple-300">{index + 1}</span>
        {nodeIcon}
        <span className="flex-1 truncate text-xs font-medium text-gray-800 dark:text-gray-200 font-mono">{nodeTitle}</span>
        <button onClick={(e) => { e.stopPropagation(); setOpen(o => !o) }} className="text-gray-400 hover:text-gray-600">
          {open ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />}
        </button>
      </div>

      <div className="px-2.5 pb-1">
        <span className="rounded px-1 py-0.5 text-[9px] font-semibold uppercase bg-gray-100 text-gray-500 dark:bg-gray-700 dark:text-gray-400">{kindLabel}</span>
      </div>

      {open && (
        <div className="border-t border-gray-100 dark:border-gray-800 px-2.5 py-2 space-y-1.5" onClick={e => e.stopPropagation()}>
          {isTool ? (
            <div>
              <label className="mb-0.5 block text-[11px] font-medium text-gray-500">{t('mcpAgent.argumentsLabel')}</label>
              <JsonArgsEditor
                value={node.arguments}
                onChange={args => onUpdate(node.id, { arguments: args })}
              />
            </div>
          ) : (isMcpAgent || isAgentTask) ? (
            <div>
              <label className="mb-0.5 block text-[11px] font-medium text-gray-500">{t('mcpAgent.mcpAgentPromptLabel')}</label>
              <textarea value={node.prompt}
                onChange={e => onUpdate(node.id, { prompt: e.target.value })}
                rows={2} placeholder={t('mcpAgent.mcpAgentPromptPlaceholder')}
                className="field-input resize-none text-xs" />
              {isAgentTask && (
                <>
                  <input value={node.server} onChange={e => onUpdate(node.id, { server: e.target.value })}
                    placeholder="http://localhost:8501/task"
                    className="field-input mt-1 text-[11px] font-mono" />
                  <DispatchEditor
                    dispatch={node.dispatch}
                    onChange={d => onUpdate(node.id, { dispatch: d })}
                  />
                </>
              )}
            </div>
          ) : isAcceptance ? (
            <div className="space-y-1.5">
              <div>
                <label className="mb-0.5 block text-[11px] font-medium text-gray-500">{t('wfBuilder.rejectToNodes')}</label>
                <input
                  value={(node.acceptance?.allow_reject_to ?? []).join(', ')}
                  onChange={e => {
                    const arr = e.target.value.split(',').map(s => s.trim()).filter(Boolean)
                    onUpdate(node.id, { acceptance: { ...(node.acceptance ?? { notify: [], allow_reject_to: [], timeout_secs: null, timeout_action: 'remind' }), allow_reject_to: arr } })
                  }}
                  placeholder="node-id-1, node-id-2"
                  className="field-input text-[11px] font-mono" />
              </div>
              <div className="flex gap-2">
                <div className="flex-1">
                  <label className="mb-0.5 block text-[11px] font-medium text-gray-500">{t('wfBuilder.timeoutSecs')}</label>
                  <input
                    type="number"
                    value={node.acceptance?.timeout_secs ?? ''}
                    onChange={e => {
                      const v = e.target.value ? parseInt(e.target.value) : null
                      onUpdate(node.id, { acceptance: { ...(node.acceptance ?? { notify: [], allow_reject_to: [], timeout_secs: null, timeout_action: 'remind' }), timeout_secs: v } })
                    }}
                    placeholder="—"
                    className="field-input text-[11px]" />
                </div>
                <div className="flex-1">
                  <label className="mb-0.5 block text-[11px] font-medium text-gray-500">{t('wfBuilder.timeoutAction')}</label>
                  <select
                    value={node.acceptance?.timeout_action ?? 'remind'}
                    onChange={e => {
                      const v = e.target.value as 'remind' | 'auto_pass' | 'auto_fail'
                      onUpdate(node.id, { acceptance: { ...(node.acceptance ?? { notify: [], allow_reject_to: [], timeout_secs: null, timeout_action: 'remind' }), timeout_action: v } })
                    }}
                    className="field-input text-[11px]"
                  >
                    <option value="remind">{t('wfBuilder.timeoutRemind')}</option>
                    <option value="auto_pass">{t('wfBuilder.timeoutAutoPass')}</option>
                    <option value="auto_fail">{t('wfBuilder.timeoutAutoFail')}</option>
                  </select>
                </div>
              </div>
            </div>
          ) : isFanOut ? (
            <div className="space-y-1.5">
              <div>
                <label className="mb-0.5 block text-[11px] font-medium text-gray-500">{t('wfBuilder.childNode')}</label>
                <input
                  value={node.fan_out?.child_node_id ?? ''}
                  onChange={e => {
                    const fo = node.fan_out ?? { split: { static: { items: [] } }, child_node_id: '', converge: 'and' as const, on_child_fail: 'fail_parent' as const }
                    onUpdate(node.id, { fan_out: { ...fo, child_node_id: e.target.value } })
                  }}
                  placeholder="node-id of child template"
                  className="field-input text-[11px] font-mono" />
              </div>
              <div className="flex gap-2">
                <div className="flex-1">
                  <label className="mb-0.5 block text-[11px] font-medium text-gray-500">{t('wfBuilder.converge')}</label>
                  <select
                    value={node.fan_out?.converge ?? 'and'}
                    onChange={e => {
                      const v = e.target.value as 'and' | 'or'
                      const fo = node.fan_out ?? { split: { static: { items: [] } }, child_node_id: '', converge: 'and' as const, on_child_fail: 'fail_parent' as const }
                      onUpdate(node.id, { fan_out: { ...fo, converge: v } })
                    }}
                    className="field-input text-[11px]"
                  >
                    <option value="and">AND ({t('wfBuilder.allMustPass')})</option>
                    <option value="or">OR ({t('wfBuilder.anyPass')})</option>
                  </select>
                </div>
                <div className="flex-1">
                  <label className="mb-0.5 block text-[11px] font-medium text-gray-500">{t('wfBuilder.onChildFail')}</label>
                  <select
                    value={node.fan_out?.on_child_fail ?? 'fail_parent'}
                    onChange={e => {
                      const v = e.target.value as 'fail_parent' | 'continue' | 'cancel_siblings'
                      const fo = node.fan_out ?? { split: { static: { items: [] } }, child_node_id: '', converge: 'and' as const, on_child_fail: 'fail_parent' as const }
                      onUpdate(node.id, { fan_out: { ...fo, on_child_fail: v } })
                    }}
                    className="field-input text-[11px]"
                  >
                    <option value="fail_parent">{t('wfBuilder.failParent')}</option>
                    <option value="continue">{t('wfBuilder.continue')}</option>
                    <option value="cancel_siblings">{t('wfBuilder.cancelSiblings')}</option>
                  </select>
                </div>
              </div>
            </div>
          ) : (
            <div>
              <label className="mb-0.5 block text-[11px] font-medium text-gray-500">{t('mcpAgent.promptLabel')}</label>
              <textarea value={node.prompt}
                onChange={e => onUpdate(node.id, { prompt: e.target.value })}
                rows={2} placeholder={t('mcpAgent.promptPlaceholder')}
                className="field-input resize-none text-xs" />
            </div>
          )}
          <div className="flex items-center justify-between pt-1">
            <div className="flex gap-1">
              <button onClick={() => onMove(node.id, -1)} disabled={index === 0}
                className="text-gray-300 hover:text-gray-600 disabled:opacity-30" title={t('mcpAgent.moveUp')}>
                <ArrowUp className="h-3 w-3" />
              </button>
              <button onClick={() => onMove(node.id, 1)} disabled={index === total - 1}
                className="text-gray-300 hover:text-gray-600 disabled:opacity-30" title={t('mcpAgent.moveDown')}>
                <ArrowDown className="h-3 w-3" />
              </button>
            </div>
            <button onClick={() => onRemove(node.id)} className="text-gray-300 hover:text-red-500" title={t('mcpAgent.removeNode')}>
              <Trash2 className="h-3 w-3" />
            </button>
          </div>
        </div>
      )}
    </div>
  )
}

// ── JSON arguments editor (textarea with validation) ─────────────────────────

function JsonArgsEditor({ value, onChange }: {
  value: Record<string, unknown>
  onChange: (v: Record<string, unknown>) => void
}) {
  const [text, setText] = useState(() => {
    const keys = Object.keys(value)
    return keys.length ? JSON.stringify(value, null, 2) : ''
  })
  const [err, setErr] = useState('')

  function commit(raw: string) {
    setText(raw)
    if (!raw.trim()) { setErr(''); onChange({}); return }
    try {
      const parsed = JSON.parse(raw)
      if (parsed && typeof parsed === 'object' && !Array.isArray(parsed)) {
        setErr(''); onChange(parsed as Record<string, unknown>)
      } else {
        setErr('Must be a JSON object')
      }
    } catch {
      setErr('Invalid JSON')
    }
  }

  return (
    <div>
      <textarea value={text} onChange={e => commit(e.target.value)}
        rows={3} placeholder={'{\n  "query": "{{input}}"\n}'}
        className="field-input resize-none font-mono text-xs" />
      {err && <p className="mt-0.5 text-[11px] text-red-500">{err}</p>}
    </div>
  )
}

// ── Dispatch strategy editor (agent_task node) ────────────────────────────────

function DispatchEditor({ dispatch, onChange }: {
  dispatch: DispatchConfig | null | undefined
  onChange: (d: DispatchConfig | null) => void
}) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)

  const cfg: DispatchConfig = dispatch ?? {
    strategy: 'fixed',
    candidates: [],
    required_capabilities: [],
    timeout_secs: 10,
  }

  function update(patch: Partial<DispatchConfig>) {
    onChange({ ...cfg, ...patch })
  }

  function updateCandidate(i: number, patch: Partial<AgentCandidate>) {
    const next = [...cfg.candidates]
    next[i] = { ...next[i], ...patch }
    update({ candidates: next })
  }

  function addCandidate() {
    update({
      candidates: [...cfg.candidates, { id: '', url: '', capabilities: [], priority: 100 }],
    })
  }

  function removeCandidate(i: number) {
    update({ candidates: cfg.candidates.filter((_, idx) => idx !== i) })
  }

  const strategyLabel = cfg.strategy === 'fixed' ? t('wfBuilder.dispatchFixed')
    : cfg.strategy === 'failover' ? t('wfBuilder.dispatchFailover')
    : cfg.strategy === 'capability_match' ? t('wfBuilder.dispatchCapability')
    : t('wfBuilder.dispatchRandom')

  return (
    <div className="mt-2 rounded-lg border border-gray-200 dark:border-gray-700 overflow-hidden">
      <button
        onClick={() => setExpanded(e => !e)}
        className="flex w-full items-center justify-between px-2 py-1 text-[11px] font-medium text-gray-600 dark:text-gray-300 bg-gray-50 dark:bg-gray-800/50 hover:bg-gray-100 dark:hover:bg-gray-800"
      >
        <span className="flex items-center gap-1">
          <Shuffle className="h-3 w-3 text-blue-500" />
          {t('wfBuilder.dispatchTitle')}: {strategyLabel}
        </span>
        {expanded ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />}
      </button>

      {expanded && (
        <div className="px-2 py-2 space-y-2">
          {/* Strategy selector */}
          <div className="flex gap-2">
            <div className="flex-1">
              <label className="mb-0.5 block text-[10px] font-medium text-gray-400">{t('wfBuilder.dispatchStrategy')}</label>
              <select
                value={cfg.strategy}
                onChange={e => update({ strategy: e.target.value as DispatchConfig['strategy'] })}
                className="field-input text-[11px]"
              >
                <option value="fixed">{t('wfBuilder.dispatchFixed')}</option>
                <option value="failover">{t('wfBuilder.dispatchFailover')}</option>
                <option value="capability_match">{t('wfBuilder.dispatchCapability')}</option>
                <option value="random">{t('wfBuilder.dispatchRandom')}</option>
              </select>
            </div>
            <div className="w-20">
              <label className="mb-0.5 block text-[10px] font-medium text-gray-400">{t('wfBuilder.dispatchTimeout')}</label>
              <input
                type="number"
                value={cfg.timeout_secs ?? 10}
                onChange={e => update({ timeout_secs: e.target.value ? parseInt(e.target.value) : 10 })}
                className="field-input text-[11px]"
              />
            </div>
          </div>

          {/* Required capabilities (only for capability_match) */}
          {cfg.strategy === 'capability_match' && (
            <div>
              <label className="mb-0.5 block text-[10px] font-medium text-gray-400">{t('wfBuilder.requiredCapabilities')}</label>
              <input
                value={(cfg.required_capabilities ?? []).join(', ')}
                onChange={e => {
                  const arr = e.target.value.split(',').map(s => s.trim()).filter(Boolean)
                  update({ required_capabilities: arr })
                }}
                placeholder="python, web-scraping"
                className="field-input text-[11px] font-mono"
              />
            </div>
          )}

          {/* Candidates list (hidden for fixed strategy) */}
          {cfg.strategy !== 'fixed' && (
            <div className="space-y-1.5">
              <div className="flex items-center justify-between">
                <label className="text-[10px] font-medium text-gray-400">{t('wfBuilder.candidates')}</label>
                <button
                  onClick={addCandidate}
                  className="flex items-center gap-0.5 rounded px-1.5 py-0.5 text-[10px] font-medium text-blue-500 hover:bg-blue-50 dark:hover:bg-blue-900/20"
                >
                  <Plus className="h-2.5 w-2.5" /> {t('wfBuilder.addCandidate')}
                </button>
              </div>
              {cfg.candidates.length === 0 && (
                <p className="text-[10px] text-gray-400 italic">{t('wfBuilder.noCandidates')}</p>
              )}
              {cfg.candidates.map((c, i) => (
                <div key={i} className="rounded border border-gray-200 dark:border-gray-700 p-1.5 space-y-1">
                  <div className="flex gap-1">
                    <input
                      value={c.id}
                      onChange={e => updateCandidate(i, { id: e.target.value })}
                      placeholder="agent-id"
                      className="field-input flex-1 text-[10px] font-mono"
                    />
                    <input
                      type="number"
                      value={c.priority ?? 100}
                      onChange={e => updateCandidate(i, { priority: e.target.value ? parseInt(e.target.value) : 100 })}
                      placeholder="100"
                      title={t('wfBuilder.priority')}
                      className="field-input w-14 text-[10px]"
                    />
                    <button
                      onClick={() => removeCandidate(i)}
                      className="text-gray-300 hover:text-red-500 px-1"
                    >
                      <Trash2 className="h-3 w-3" />
                    </button>
                  </div>
                  <input
                    value={c.url}
                    onChange={e => updateCandidate(i, { url: e.target.value })}
                    placeholder="http://localhost:8502/task"
                    className="field-input text-[10px] font-mono"
                  />
                  {cfg.strategy === 'capability_match' && (
                    <input
                      value={(c.capabilities ?? []).join(', ')}
                      onChange={e => {
                        const arr = e.target.value.split(',').map(s => s.trim()).filter(Boolean)
                        updateCandidate(i, { capabilities: arr })
                      }}
                      placeholder="python, data-analysis"
                      className="field-input text-[10px] font-mono"
                    />
                  )}
                </div>
              ))}
            </div>
          )}
        </div>
      )}
    </div>
  )
}
