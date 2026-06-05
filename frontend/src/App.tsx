import { useEffect, useRef, useState, useCallback } from 'react'
import { useAgentStore } from './store/agentStore'
import { AgentList } from './components/AgentList'
import { AgentDetail } from './components/AgentDetail'
import { AgentForm } from './components/AgentForm'
import { McpManager } from './pages/McpManager'
import { PortManager } from './pages/PortManager'
import type { AgentState } from './types/agent'
import { Plus, RefreshCw, Bot, X, Globe, Plug, Network } from 'lucide-react'

type NavPage = 'agents' | 'mcp' | 'ports'

interface OpenTab {
  agentId: string
  label: string
  port: number
}

const MIN_SIDEBAR = 200
const MAX_SIDEBAR = 480
const DEFAULT_SIDEBAR = 288

export default function App() {
  const {
    agents, selectedId, logs, loading,
    fetchAgents, selectAgent, reorderAgents,
    startAgent, stopAgent, saveAgent, deleteAgent,
  } = useAgentStore()

  const [page, setPage] = useState<NavPage>('agents')
  const [showForm, setShowForm] = useState(false)
  const [editingAgent, setEditingAgent] = useState<AgentState | null>(null)
  const [sidebarWidth, setSidebarWidth] = useState(DEFAULT_SIDEBAR)
  const [openTabs, setOpenTabs] = useState<OpenTab[]>([])
  const [activeTabId, setActiveTabId] = useState<string | null>(null) // agentId of active UI tab, null = detail view

  const dragging = useRef(false)
  const startX = useRef(0)
  const startWidth = useRef(DEFAULT_SIDEBAR)

  const selectedAgent = agents.find(a => a.config.id === selectedId) ?? null
  const agentLogs = selectedId ? (logs[selectedId] ?? []) : []

  useEffect(() => {
    fetchAgents()
    const interval = setInterval(fetchAgents, 3000)
    return () => clearInterval(interval)
  }, [])

  useEffect(() => {
    if (!selectedId) return
    const interval = setInterval(() => {
      useAgentStore.getState().fetchLogs(selectedId)
    }, 2000)
    return () => clearInterval(interval)
  }, [selectedId])

  // Sidebar resize handlers
  const onMouseDown = useCallback((e: React.MouseEvent) => {
    dragging.current = true
    startX.current = e.clientX
    startWidth.current = sidebarWidth
    document.body.style.cursor = 'col-resize'
    document.body.style.userSelect = 'none'
  }, [sidebarWidth])

  useEffect(() => {
    const onMove = (e: MouseEvent) => {
      if (!dragging.current) return
      const delta = e.clientX - startX.current
      setSidebarWidth(Math.min(MAX_SIDEBAR, Math.max(MIN_SIDEBAR, startWidth.current + delta)))
    }
    const onUp = () => {
      dragging.current = false
      document.body.style.cursor = ''
      document.body.style.userSelect = ''
    }
    window.addEventListener('mousemove', onMove)
    window.addEventListener('mouseup', onUp)
    return () => {
      window.removeEventListener('mousemove', onMove)
      window.removeEventListener('mouseup', onUp)
    }
  }, [])

  function openAgentUI(agent: AgentState) {
    if (!agent.config.port) return
    const existing = openTabs.find(t => t.agentId === agent.config.id)
    if (existing) {
      setActiveTabId(agent.config.id)
      return
    }
    setOpenTabs(tabs => [...tabs, {
      agentId: agent.config.id,
      label: agent.config.name,
      port: agent.config.port!,
    }])
    setActiveTabId(agent.config.id)
  }

  function closeTab(agentId: string, e: React.MouseEvent) {
    e.stopPropagation()
    setOpenTabs(tabs => tabs.filter(t => t.agentId !== agentId))
    if (activeTabId === agentId) setActiveTabId(null)
  }

  function openNew() { setEditingAgent(null); setShowForm(true) }
  function openEdit(agent: AgentState) { setEditingAgent(agent); setShowForm(true) }

  return (
    <div className="flex h-screen flex-col bg-gray-50 dark:bg-gray-950 overflow-hidden">

      {/* ── Browser-style tab bar ── */}
      {openTabs.length > 0 && (
        <div className="flex items-center gap-1 border-b border-gray-200 bg-gray-100 px-2 pt-1.5 dark:border-gray-700 dark:bg-gray-900 shrink-0">
          {/* "Detail" pseudo-tab */}
          <button
            onClick={() => setActiveTabId(null)}
            className={`flex items-center gap-1.5 rounded-t-lg border border-b-0 px-3 py-1.5 text-xs font-medium transition-colors ${
              activeTabId === null
                ? 'border-gray-200 bg-white text-gray-900 dark:border-gray-700 dark:bg-gray-950 dark:text-gray-100'
                : 'border-transparent text-gray-500 hover:bg-gray-200 dark:text-gray-400 dark:hover:bg-gray-800'
            }`}
          >
            <Bot className="h-3 w-3" />
            Agent
          </button>

          {openTabs.map(tab => (
            <button
              key={tab.agentId}
              onClick={() => setActiveTabId(tab.agentId)}
              className={`group flex items-center gap-1.5 rounded-t-lg border border-b-0 px-3 py-1.5 text-xs font-medium transition-colors ${
                activeTabId === tab.agentId
                  ? 'border-gray-200 bg-white text-gray-900 dark:border-gray-700 dark:bg-gray-950 dark:text-gray-100'
                  : 'border-transparent text-gray-500 hover:bg-gray-200 dark:text-gray-400 dark:hover:bg-gray-800'
              }`}
            >
              <Globe className="h-3 w-3 shrink-0" />
              <span className="max-w-[120px] truncate">{tab.label}</span>
              <span
                onClick={e => closeTab(tab.agentId, e)}
                className="ml-0.5 rounded p-0.5 opacity-0 hover:bg-gray-300 group-hover:opacity-100 dark:hover:bg-gray-600"
              >
                <X className="h-3 w-3" />
              </span>
            </button>
          ))}
        </div>
      )}

      {/* ── Main area ── */}
      <div className="flex flex-1 overflow-hidden">

        {/* Sidebar */}
        <aside
          style={{ width: sidebarWidth }}
          className="flex shrink-0 flex-col border-r border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-900"
        >
          {/* App title */}
          <div className="flex items-center gap-2 border-b border-gray-200 px-4 py-3 dark:border-gray-700">
            <Bot className="h-5 w-5 text-blue-500" />
            <span className="text-sm font-semibold text-gray-900 dark:text-gray-100">Agent Manager</span>
          </div>

          {/* Nav */}
          <div className="flex flex-col gap-0.5 p-2 border-b border-gray-200 dark:border-gray-700">
            {([
              { id: 'agents', icon: <Bot className="h-4 w-4" />, label: 'Agents' },
              { id: 'mcp',    icon: <Plug className="h-4 w-4" />, label: 'MCP Servers' },
              { id: 'ports',  icon: <Network className="h-4 w-4" />, label: 'Port Manager' },
            ] as const).map(nav => (
              <button
                key={nav.id}
                onClick={() => setPage(nav.id)}
                className={`flex items-center gap-2.5 rounded-lg px-3 py-2 text-sm font-medium transition-colors text-left ${
                  page === nav.id
                    ? 'bg-blue-50 text-blue-600 dark:bg-blue-900/20 dark:text-blue-400'
                    : 'text-gray-600 hover:bg-gray-100 dark:text-gray-400 dark:hover:bg-gray-800'
                }`}
              >
                {nav.icon}
                {nav.label}
              </button>
            ))}
          </div>

          {/* Agents list (only when on agents page) */}
          {page === 'agents' && <>
            <div className="flex items-center justify-between px-4 py-2">
              <p className="text-xs text-gray-500 dark:text-gray-400">{agents.length} configured</p>
              <div className="flex gap-1">
                <button onClick={openNew} className="rounded p-1 text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-800" title="Add">
                  <Plus className="h-3.5 w-3.5" />
                </button>
                <button onClick={fetchAgents} disabled={loading} className="rounded p-1 text-gray-400 hover:bg-gray-100 disabled:opacity-40 dark:hover:bg-gray-800">
                  <RefreshCw className={`h-3.5 w-3.5 ${loading ? 'animate-spin' : ''}`} />
                </button>
              </div>
            </div>
            <div className="flex-1 overflow-y-auto">
              <AgentList
                agents={agents}
                selectedId={selectedId}
                onSelect={selectAgent}
                onStart={startAgent}
                onStop={stopAgent}
                onDelete={deleteAgent}
                onConfigure={openEdit}
                onReorder={reorderAgents}
              />
            </div>
            <div className="border-t border-gray-200 p-3 dark:border-gray-700">
              <button onClick={openNew} className="flex w-full items-center justify-center gap-2 rounded-xl bg-blue-600 px-3 py-2 text-sm font-medium text-white transition-colors hover:bg-blue-500">
                <Plus className="h-4 w-4" /> New Agent
              </button>
            </div>
          </>}

          {/* Spacer for non-agent pages */}
          {page !== 'agents' && <div className="flex-1" />}
        </aside>

        {/* Drag divider */}
        <div
          onMouseDown={onMouseDown}
          className="w-1 shrink-0 cursor-col-resize bg-transparent hover:bg-blue-400/40 active:bg-blue-400/60 transition-colors"
        />

        {/* Main panel */}
        <main className="flex-1 overflow-hidden">
          {/* MCP / Port pages */}
          {page === 'mcp' && <McpManager />}
          {page === 'ports' && <PortManager />}

          {/* Agent page */}
          {page === 'agents' && <>
            {/* UI tab iframe panels */}
            {openTabs.map(tab => (
              <div key={tab.agentId} className={`flex h-full flex-col ${activeTabId === tab.agentId ? 'block' : 'hidden'}`}>
                <IframePanel tab={tab} onClose={e => closeTab(tab.agentId, e)} />
              </div>
            ))}

            {/* Agent detail panel */}
            <div className={activeTabId === null ? 'h-full' : 'hidden'}>
              {selectedAgent ? (
                <AgentDetail
                  agent={selectedAgent}
                  logs={agentLogs}
                  onStart={startAgent}
                  onStop={stopAgent}
                  onOpenUI={openAgentUI}
                />
              ) : (
                <div className="flex h-full flex-col items-center justify-center gap-3 text-gray-400">
                  <Bot className="h-12 w-12 opacity-30" />
                  <p className="text-base text-gray-500 dark:text-gray-400">Select an agent to view details</p>
                  <button onClick={openNew} className="text-sm text-blue-500 hover:text-blue-400">
                    + Create your first agent
                  </button>
                </div>
              )}
            </div>
          </>}
        </main>
      </div>

      {showForm && (
        <AgentForm
          initial={editingAgent?.config}
          onSave={saveAgent}
          onClose={() => setShowForm(false)}
        />
      )}
    </div>
  )
}

function IframePanel({ tab, onClose }: { tab: OpenTab; onClose: (e: React.MouseEvent) => void }) {
  const [key, setKey] = useState(0)
  const url = `http://localhost:${tab.port}`
  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center gap-2 border-b border-gray-200 bg-gray-50 px-4 py-1.5 dark:border-gray-700 dark:bg-gray-800">
        <Globe className="h-3.5 w-3.5 shrink-0 text-gray-400" />
        <span className="flex-1 font-mono text-xs text-gray-600 dark:text-gray-400">{url}</span>
        <button onClick={() => setKey(k => k + 1)} className="rounded p-1 text-gray-400 hover:bg-gray-200 dark:hover:bg-gray-700" title="Reload">
          <RefreshCw className="h-3.5 w-3.5" />
        </button>
        <a href={url} target="_blank" rel="noreferrer" className="rounded p-1 text-gray-400 hover:bg-gray-200 dark:hover:bg-gray-700" title="Open in browser">
          <Globe className="h-3.5 w-3.5" />
        </a>
        <button onClick={onClose} className="rounded p-1 text-gray-400 hover:bg-red-50 hover:text-red-500 dark:hover:bg-red-900/20" title="Close tab">
          <X className="h-3.5 w-3.5" />
        </button>
      </div>
      <iframe
        key={key}
        src={url}
        title={tab.label}
        className="flex-1 w-full border-0 bg-white"
        sandbox="allow-scripts allow-same-origin allow-forms allow-popups"
      />
    </div>
  )
}
