import { useCallback, useEffect, useMemo, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import {
  DndContext, useDraggable, useDroppable, DragOverlay,
  PointerSensor, useSensor, useSensors, type DragStartEvent, type DragEndEvent,
} from '@dnd-kit/core'
import {
  Plus, Trash2, Save, Loader2, Wrench, Sparkles,
  ArrowUp, ArrowDown, GripVertical, AlertCircle, ChevronDown, ChevronRight, X, Bot, Play,
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

interface WorkflowNode {
  id: string
  kind: 'tool' | 'llm' | 'mcp_agent'
  label: string
  server: string
  tool: string
  arguments: Record<string, unknown>
  prompt: string
}

interface Workflow {
  id: string
  name: string
  description: string
  nodes: WorkflowNode[]
  created_at: string
  updated_at: string
}

function uid() {
  return `n_${Date.now()}_${Math.random().toString(36).slice(2, 7)}`
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
  }

  function startEdit(w: Workflow) {
    setEditing(JSON.parse(JSON.stringify(w)))
    setError('')
  }

  function addToolNode(tool: McpToolInfo) {
    setEditing(w => w ? {
      ...w,
      nodes: [...w.nodes, {
        id: uid(), kind: 'tool', label: `${tool.server}__${tool.name}`,
        server: tool.server, tool: tool.name, arguments: {}, prompt: '',
      }],
    } : w)
  }

  function addLlmNode() {
    setEditing(w => w ? {
      ...w,
      nodes: [...w.nodes, {
        id: uid(), kind: 'llm', label: t('mcpAgent.llmNode'),
        server: '', tool: '', arguments: {}, prompt: '',
      }],
    } : w)
  }

  function addMcpAgentNode(server: McpServer) {
    setEditing(w => w ? {
      ...w,
      nodes: [...w.nodes, {
        id: uid(), kind: 'mcp_agent', label: server.name,
        server: server.name, tool: '', arguments: {}, prompt: '',
      }],
    } : w)
  }

  function updateNode(id: string, patch: Partial<WorkflowNode>) {
    setEditing(w => w ? { ...w, nodes: w.nodes.map(n => n.id === id ? { ...n, ...patch } : n) } : w)
  }
  function removeNode(id: string) {
    setEditing(w => w ? { ...w, nodes: w.nodes.filter(n => n.id !== id) } : w)
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
    if (e.over?.id === 'workflow-canvas') {
      const tool = e.active.data.current?.tool as McpToolInfo | undefined
      if (tool && editing) addToolNode(tool)
    }
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

              <div className="flex flex-wrap items-center gap-2">
                <button onClick={addLlmNode}
                  className="flex items-center gap-1.5 rounded-lg border border-gray-200 px-2.5 py-1.5 text-xs text-gray-600 hover:bg-gray-50 dark:border-gray-700 dark:text-gray-300 dark:hover:bg-gray-800">
                  <Sparkles className="h-3.5 w-3.5 text-purple-500" /> {t('mcpAgent.addLlmNode')}
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

              {/* Canvas (droppable pipeline) */}
              <Canvas nodes={editing.nodes}
                onUpdate={updateNode} onRemove={removeNode} onMove={moveNode} />

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

// ── Canvas (droppable pipeline) ──────────────────────────────────────────────

function Canvas({ nodes, onUpdate, onRemove, onMove }: {
  nodes: WorkflowNode[]
  onUpdate: (id: string, patch: Partial<WorkflowNode>) => void
  onRemove: (id: string) => void
  onMove: (id: string, dir: -1 | 1) => void
}) {
  const { t } = useTranslation()
  const { setNodeRef, isOver } = useDroppable({ id: 'workflow-canvas' })

  return (
    <div ref={setNodeRef}
      className={`min-h-40 rounded-xl border-2 border-dashed p-3 transition-colors ${
        isOver ? 'border-purple-400 bg-purple-50/50 dark:bg-purple-900/10' : 'border-gray-200 dark:border-gray-700'
      }`}>
      {nodes.length === 0 ? (
        <div className="flex h-32 items-center justify-center text-center text-xs text-gray-400">
          {t('mcpAgent.canvasEmpty')}
        </div>
      ) : (
        <div className="space-y-2">
          {nodes.map((n, i) => (
            <div key={n.id}>
              <NodeCard node={n} index={i} total={nodes.length}
                onUpdate={onUpdate} onRemove={onRemove} onMove={onMove} />
              {i < nodes.length - 1 && (
                <div className="flex justify-center py-0.5">
                  <ArrowDown className="h-3.5 w-3.5 text-gray-300" />
                </div>
              )}
            </div>
          ))}
        </div>
      )}
    </div>
  )
}

function NodeCard({ node, index, total, onUpdate, onRemove, onMove }: {
  node: WorkflowNode
  index: number
  total: number
  onUpdate: (id: string, patch: Partial<WorkflowNode>) => void
  onRemove: (id: string) => void
  onMove: (id: string, dir: -1 | 1) => void
}) {
  const { t } = useTranslation()
  const [open, setOpen] = useState(true)
  const isTool = node.kind === 'tool'
  const isMcpAgent = node.kind === 'mcp_agent'

  const kindLabel = isTool
    ? t('mcpAgent.nodeKindTool')
    : isMcpAgent
      ? t('mcpAgent.nodeKindMcpAgent')
      : t('mcpAgent.nodeKindLlm')

  const nodeIcon = isTool
    ? <Wrench className="h-3.5 w-3.5 text-orange-500" />
    : isMcpAgent
      ? <Bot className="h-3.5 w-3.5 text-purple-500" />
      : <Sparkles className="h-3.5 w-3.5 text-purple-500" />

  const nodeTitle = isTool
    ? `${node.server}__${node.tool}`
    : isMcpAgent
      ? `🔌 ${node.server}`
      : t('mcpAgent.llmNode')

  return (
    <div className="rounded-xl border border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-900">
      <div className="flex items-center gap-2 px-3 py-2">
        <GripVertical className="h-3.5 w-3.5 text-gray-300" />
        <span className="flex h-5 w-5 items-center justify-center rounded-full bg-purple-100 text-[11px] font-semibold text-purple-700 dark:bg-purple-900/40 dark:text-purple-300">{index + 1}</span>
        {nodeIcon}
        <span className="flex-1 truncate text-xs font-medium text-gray-800 dark:text-gray-200 font-mono">
          {nodeTitle}
        </span>
        <span className="rounded px-1.5 py-0.5 text-[10px] font-semibold uppercase bg-gray-100 text-gray-500 dark:bg-gray-700 dark:text-gray-400">
          {kindLabel}
        </span>
        <button onClick={() => onMove(node.id, -1)} disabled={index === 0}
          className="text-gray-300 hover:text-gray-600 disabled:opacity-30" title={t('mcpAgent.moveUp')}>
          <ArrowUp className="h-3.5 w-3.5" />
        </button>
        <button onClick={() => onMove(node.id, 1)} disabled={index === total - 1}
          className="text-gray-300 hover:text-gray-600 disabled:opacity-30" title={t('mcpAgent.moveDown')}>
          <ArrowDown className="h-3.5 w-3.5" />
        </button>
        <button onClick={() => setOpen(o => !o)} className="text-gray-400 hover:text-gray-600">
          {open ? <ChevronDown className="h-3.5 w-3.5" /> : <ChevronRight className="h-3.5 w-3.5" />}
        </button>
        <button onClick={() => onRemove(node.id)} className="text-gray-300 hover:text-red-500" title={t('mcpAgent.removeNode')}>
          <Trash2 className="h-3.5 w-3.5" />
        </button>
      </div>

      {open && (
        <div className="border-t border-gray-100 dark:border-gray-800 px-3 py-2.5 space-y-2">
          {isTool ? (
            <div>
              <label className="mb-1 block text-xs font-medium text-gray-500">{t('mcpAgent.argumentsLabel')}</label>
              <JsonArgsEditor
                value={node.arguments}
                onChange={args => onUpdate(node.id, { arguments: args })}
              />
              <p className="mt-1 text-[11px] text-gray-400">{t('mcpAgent.inputPlaceholderHint')}</p>
            </div>
          ) : isMcpAgent ? (
            <div>
              <label className="mb-1 block text-xs font-medium text-gray-500">{t('mcpAgent.mcpAgentPromptLabel')}</label>
              <textarea value={node.prompt}
                onChange={e => onUpdate(node.id, { prompt: e.target.value })}
                rows={3} placeholder={t('mcpAgent.mcpAgentPromptPlaceholder')}
                className="field-input resize-none text-xs" />
              <p className="mt-1 text-[11px] text-gray-400">{t('mcpAgent.inputPlaceholderHint')}</p>
            </div>
          ) : (
            <div>
              <label className="mb-1 block text-xs font-medium text-gray-500">{t('mcpAgent.promptLabel')}</label>
              <textarea value={node.prompt}
                onChange={e => onUpdate(node.id, { prompt: e.target.value })}
                rows={3} placeholder={t('mcpAgent.promptPlaceholder')}
                className="field-input resize-none text-xs" />
            </div>
          )}
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
