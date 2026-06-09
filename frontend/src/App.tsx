import { useEffect, useRef, useState, useCallback } from 'react'
import { useAgentStore } from './store/agentStore'
import { AgentList } from './components/AgentList'
import { AgentDetail } from './components/AgentDetail'
import { AgentForm } from './components/AgentForm'
import { TerminalPanel } from './components/TerminalPanel'
import { PortManager } from './pages/PortManager'
import { McpAgent } from './pages/McpAgent'
import { Dashboard } from './pages/Dashboard'
import { ManagerAgent, type ManagerSessionState } from './pages/ManagerAgent'
import { ProxyManager } from './pages/ProxyManager'
import { useTheme } from './theme'
import type { AgentState } from './types/agent'
import {
  Plus, RefreshCw, Bot, X, Globe, Network, Cpu,
  Maximize2, Minimize2, TerminalSquare, Sun, Moon,
  Crown, LayoutDashboard, Shield,
} from 'lucide-react'
import logoUrl from '/logo.png'

type NavPage = 'agents' | 'mcp-agent' | 'ports' | 'dashboard' | 'manager' | 'proxy'
type TabKind = 'ui' | 'terminal'

interface OpenTab {
  agentId: string
  label: string
  kind: TabKind
  port?: number
  token?: string
  command?: string
  args?: string[]
  cwd?: string
  env?: Record<string, string>
}

const MIN_SIDEBAR = 200
const MAX_SIDEBAR = 480
const DEFAULT_SIDEBAR = 288
const MIN_PANEL_H = 120
const MAX_PANEL_H = 800
const DEFAULT_PANEL_H = 380

export default function App() {
  const {
    agents, selectedId, logs, loading,
    fetchAgents, selectAgent, reorderAgents,
    startAgent, stopAgent, saveAgent, deleteAgent,
  } = useAgentStore()

  const { theme, toggle: toggleTheme } = useTheme()

  const [page, setPage] = useState<NavPage>('agents')
  const [showForm, setShowForm] = useState(false)
  const [editingAgent, setEditingAgent] = useState<AgentState | null>(null)
  const [sidebarWidth, setSidebarWidth] = useState(DEFAULT_SIDEBAR)
  const [openTabs, setOpenTabs] = useState<OpenTab[]>([])
  const [activeTabKey, setActiveTabKey] = useState<string | null>(null)
  const [panelHeight, setPanelHeight] = useState(DEFAULT_PANEL_H)
  const [panelFullscreen, setPanelFullscreen] = useState(false)
  const [managerSession, setManagerSession] = useState<ManagerSessionState>({ messages: [], selectedProvider: '' })

  const colDragging = useRef(false)
  const colStartX = useRef(0)
  const colStartW = useRef(DEFAULT_SIDEBAR)
  const rowDragging = useRef(false)
  const rowStartY = useRef(0)
  const rowStartH = useRef(DEFAULT_PANEL_H)

  const selectedAgent = agents.find(a => a.config.id === selectedId) ?? null
  const agentLogs = selectedId ? (logs[selectedId] ?? []) : []
  const activeTab = openTabs.find(t => `${t.agentId}:${t.kind}` === activeTabKey)
    ?? openTabs.find(t => t.agentId === selectedId)
    ?? null
  const showPanel = activeTab !== null
  const showSplit = showPanel && !panelFullscreen

  useEffect(() => {
    fetchAgents()
    const id = setInterval(fetchAgents, 3000)
    return () => clearInterval(id)
  }, [])

  useEffect(() => {
    if (!selectedId) return
    const id = setInterval(() => useAgentStore.getState().fetchLogs(selectedId), 2000)
    return () => clearInterval(id)
  }, [selectedId])

  const onColMouseDown = useCallback((e: React.MouseEvent) => {
    colDragging.current = true
    colStartX.current = e.clientX
    colStartW.current = sidebarWidth
    document.body.style.cursor = 'col-resize'
    document.body.style.userSelect = 'none'
  }, [sidebarWidth])

  const onRowMouseDown = useCallback((e: React.MouseEvent) => {
    e.preventDefault()
    rowDragging.current = true
    rowStartY.current = e.clientY
    rowStartH.current = panelHeight
    document.body.style.cursor = 'row-resize'
    document.body.style.userSelect = 'none'
  }, [panelHeight])

  useEffect(() => {
    const onMove = (e: MouseEvent) => {
      if (colDragging.current) {
        const d = e.clientX - colStartX.current
        setSidebarWidth(Math.min(MAX_SIDEBAR, Math.max(MIN_SIDEBAR, colStartW.current + d)))
      }
      if (rowDragging.current) {
        const d = e.clientY - rowStartY.current
        setPanelHeight(Math.min(MAX_PANEL_H, Math.max(MIN_PANEL_H, rowStartH.current + d)))
      }
    }
    const onUp = () => {
      colDragging.current = false
      rowDragging.current = false
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
    const id = agent.config.id
    const existing = openTabs.find(t => t.agentId === id && t.kind === 'ui')
    if (!existing) {
      setOpenTabs(tabs => [
        ...tabs.filter(t => !(t.agentId === id && t.kind === 'ui')),
        { agentId: id, label: agent.config.name, kind: 'ui', port: agent.config.port!, token: agent.config.ui_token },
      ])
    }
    selectAgent(id)
    setActiveTabKey(`${id}:ui`)
  }

  function openAgentTerminal(agent: AgentState) {
    const id = agent.config.id
    const existing = openTabs.find(t => t.agentId === id && t.kind === 'terminal')
    if (!existing) {
      setOpenTabs(tabs => [
        ...tabs.filter(t => !(t.agentId === id && t.kind === 'terminal')),
        {
          agentId: id,
          label: agent.config.name,
          kind: 'terminal',
          command: agent.config.command,
          args: agent.config.args,
          cwd: agent.config.working_dir,
          env: agent.config.env,
        },
      ])
    }
    selectAgent(id)
    setActiveTabKey(`${id}:terminal`)
  }

  function closeTab(tab: OpenTab, e: React.MouseEvent) {
    e.stopPropagation()
    const key = `${tab.agentId}:${tab.kind}`
    setOpenTabs(tabs => tabs.filter(t => !(t.agentId === tab.agentId && t.kind === tab.kind)))
    if (activeTabKey === key) setActiveTabKey(null)
    setPanelFullscreen(false)
  }

  function openNew() { setEditingAgent(null); setShowForm(true) }
  function openEdit(agent: AgentState) { setEditingAgent(agent); setShowForm(true) }

  const uiIsOpen = selectedAgent
    ? openTabs.some(t => t.agentId === selectedAgent.config.id && t.kind === 'ui')
    : false
  const termIsOpen = selectedAgent
    ? openTabs.some(t => t.agentId === selectedAgent.config.id && t.kind === 'terminal')
    : false

  return (
    <div className="flex h-screen overflow-hidden bg-gray-100 text-gray-800 dark:bg-gray-950 dark:text-gray-100">

      {/* ── Sidebar ─────────────────────────────────────── */}
      <aside
        style={{ width: sidebarWidth }}
        className="flex shrink-0 flex-col border-r border-gray-200 bg-white dark:border-gray-800 dark:bg-gray-900"
      >
        {/* App title */}
        <div className="flex items-center justify-between border-b border-gray-200 px-4 py-3 dark:border-gray-800">
          <div className="flex items-center gap-2.5">
            <img src={logoUrl} alt="智管-Agent Manager" className="h-7 w-7 rounded-lg" />
            <div className="flex flex-col leading-tight">
              <span className="text-sm font-semibold text-gray-900 dark:text-gray-100">智管-Agent Manager</span>
              <span className="text-[10px] text-gray-400 dark:text-gray-500">v0.2.1</span>
            </div>
          </div>
          <button
            onClick={toggleTheme}
            className="rounded-md p-1.5 text-gray-400 transition-colors hover:bg-gray-100 hover:text-gray-600 dark:hover:bg-gray-800 dark:hover:text-gray-300"
            title={theme === 'dark' ? 'Switch to light mode' : 'Switch to dark mode'}
          >
            {theme === 'dark' ? <Sun className="h-4 w-4" /> : <Moon className="h-4 w-4" />}
          </button>
        </div>

        {/* Nav */}
        <div className="flex flex-col gap-0.5 p-2 border-b border-gray-200 dark:border-gray-800">
          {([
            { id: 'dashboard', icon: <LayoutDashboard className="h-4 w-4" />, label: 'Dashboard' },
            { id: 'manager',   icon: <Crown className="h-4 w-4" />,           label: 'Manager' },
            { id: 'agents',    icon: <Bot className="h-4 w-4" />,             label: 'Agents' },
            { id: 'mcp-agent', icon: <Cpu className="h-4 w-4" />,             label: 'MCP Agent' },
            { id: 'ports',     icon: <Network className="h-4 w-4" />,         label: 'Port Manager' },
            { id: 'proxy',     icon: <Shield className="h-4 w-4" />,          label: '代理发布' },
          ] as const).map(nav => (
            <button
              key={nav.id}
              onClick={() => setPage(nav.id)}
              className={`flex items-center gap-2.5 rounded-lg px-3 py-2 text-sm font-medium transition-colors text-left ${
                page === nav.id
                  ? 'bg-blue-50 text-blue-600 dark:bg-blue-600/20 dark:text-blue-400'
                  : 'text-gray-500 hover:bg-gray-100 hover:text-gray-800 dark:text-gray-400 dark:hover:bg-gray-800 dark:hover:text-gray-200'
              }`}
            >
              {nav.icon}{nav.label}
            </button>
          ))}
        </div>

        {/* Agent list */}
        {page === 'agents' && <>
          <div className="flex items-center justify-between px-4 py-2">
            <p className="text-xs text-gray-400">{agents.length} configured</p>
            <div className="flex gap-1">
              <button onClick={openNew} className="rounded p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600 dark:hover:bg-gray-800 dark:hover:text-gray-300" title="Add">
                <Plus className="h-3.5 w-3.5" />
              </button>
              <button onClick={fetchAgents} disabled={loading} className="rounded p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600 dark:hover:bg-gray-800 dark:hover:text-gray-300 disabled:opacity-40">
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
          <div className="border-t border-gray-200 p-3 dark:border-gray-800">
            <button onClick={openNew} className="flex w-full items-center justify-center gap-2 rounded-xl bg-blue-600 px-3 py-2 text-sm font-medium text-white transition-colors hover:bg-blue-500">
              <Plus className="h-4 w-4" /> New Agent
            </button>
          </div>
        </>}

        {page !== 'agents' && <div className="flex-1" />}
      </aside>

      {/* ── Col-resize handle ──────────────────────────── */}
      <div
        onMouseDown={onColMouseDown}
        className="drag-col"
      />

      {/* ── Main panel ─────────────────────────────────── */}
      <main className="flex flex-1 flex-col overflow-hidden bg-gray-50 dark:bg-gray-950">

        {page === 'dashboard' && (
          <Dashboard
            agents={agents}
            onSelectAgent={(id) => { selectAgent(id); setPage('agents'); setPanelFullscreen(false) }}
            onOpenAgentUI={(agent) => { openAgentUI(agent); setPage('agents'); setPanelFullscreen(false) }}
            onStartAgent={startAgent}
            onStopAgent={stopAgent}
            onNavigateToAgents={() => { setPage('agents'); setPanelFullscreen(false) }}
            onOpenManagerAgent={() => setPage('manager')}
          />
        )}

        {/* Manager is always mounted to preserve session state, just hidden when inactive */}
        <div className={`flex flex-1 flex-col overflow-hidden ${page === 'manager' ? '' : 'hidden'}`}>
          <ManagerAgent
            agents={agents}
            session={managerSession}
            onSessionChange={setManagerSession}
            onOpenAgentUI={openAgentUI}
            onOpenAgentTerminal={openAgentTerminal}
            onStartAgent={startAgent}
            onStopAgent={stopAgent}
            onNavigate={(p) => { setPage(p as NavPage); if (p === 'agents') setPanelFullscreen(false) }}
          />
        </div>

        {page === 'mcp-agent' && <McpAgent />}
        {page === 'ports' && <PortManager />}
        {page === 'proxy' && <ProxyManager agents={agents} />}

        {page === 'agents' && (
          <div className="flex flex-1 flex-col overflow-hidden">

            {/* ── Tab bar ── */}
            {openTabs.length > 0 && (
              <div className="flex items-center gap-0.5 border-b border-gray-200 bg-white px-2 pt-1.5 shrink-0 dark:border-gray-800 dark:bg-gray-900">
                {openTabs.map(tab => {
                  const isActive = selectedId === tab.agentId
                  return (
                    <button
                      key={`${tab.agentId}-${tab.kind}`}
                      onClick={() => { selectAgent(tab.agentId); setActiveTabKey(`${tab.agentId}:${tab.kind}`); }}
                      className={`group flex items-center gap-1.5 rounded-t-md border border-b-0 px-3 py-1.5 text-xs font-medium transition-colors ${
                        isActive && activeTab?.kind === tab.kind
                          ? tab.kind === 'terminal'
                            ? 'border-gray-200 bg-gray-50 text-green-600 dark:border-gray-700 dark:bg-gray-950 dark:text-green-400'
                            : 'border-gray-200 bg-gray-50 text-blue-600 dark:border-gray-700 dark:bg-gray-950 dark:text-blue-400'
                          : 'border-transparent text-gray-400 hover:bg-gray-100 hover:text-gray-600 dark:text-gray-500 dark:hover:bg-gray-800 dark:hover:text-gray-300'
                      }`}
                    >
                      {tab.kind === 'terminal'
                        ? <TerminalSquare className="h-3 w-3 shrink-0" />
                        : <Globe className="h-3 w-3 shrink-0" />
                      }
                      <span className="max-w-[100px] truncate">{tab.label}</span>
                      <span className="ml-0.5 text-[10px] text-gray-400 dark:text-gray-600">
                        {tab.kind === 'terminal' ? 'term' : `${tab.port}`}
                      </span>
                      <span
                        onClick={e => closeTab(tab, e)}
                        className="ml-1 rounded p-0.5 opacity-0 hover:bg-gray-200 hover:text-red-500 group-hover:opacity-100 dark:hover:bg-gray-700 dark:hover:text-red-400"
                      >
                        <X className="h-3 w-3" />
                      </span>
                    </button>
                  )
                })}
              </div>
            )}

            {/* ── Active panel (UI iframe or Terminal) ── */}
            {showPanel && activeTab && (
              <div
                style={panelFullscreen ? undefined : { height: panelHeight }}
                className={`shrink-0 flex flex-col ${panelFullscreen ? 'flex-1' : ''}`}
              >
                {/* Panel toolbar */}
                <div className="flex items-center gap-2 border-b border-gray-200 bg-white px-3 py-1 shrink-0 dark:border-gray-800 dark:bg-gray-900">
                  {activeTab.kind === 'terminal'
                    ? <TerminalSquare className="h-3.5 w-3.5 shrink-0 text-green-500" />
                    : <Globe className="h-3.5 w-3.5 shrink-0 text-blue-500 dark:text-blue-400" />
                  }
                  <span className="flex-1 font-mono text-xs text-gray-500 dark:text-gray-400">
                    {activeTab.kind === 'terminal'
                      ? `${activeTab.command} ${(activeTab.args ?? []).join(' ')}`
                      : `http://localhost:${activeTab.port}`
                    }
                  </span>
                  {activeTab.kind === 'ui' && activeTab.token && (
                    <span className="rounded px-1.5 py-0.5 text-[10px] font-medium bg-amber-100 text-amber-600 shrink-0 dark:bg-amber-900/40 dark:text-amber-400">
                      🔑 auth
                    </span>
                  )}
                  <button
                    onClick={() => setPanelFullscreen(f => !f)}
                    className="rounded p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600 dark:hover:bg-gray-700 dark:hover:text-gray-300"
                    title={panelFullscreen ? '还原' : '全屏'}
                  >
                    {panelFullscreen ? <Minimize2 className="h-3.5 w-3.5" /> : <Maximize2 className="h-3.5 w-3.5" />}
                  </button>
                  <button
                    onClick={e => closeTab(activeTab, e)}
                    className="rounded p-1 text-gray-400 hover:bg-gray-100 hover:text-red-500 dark:hover:bg-gray-700 dark:hover:text-red-400"
                    title="Close"
                  >
                    <X className="h-3.5 w-3.5" />
                  </button>
                </div>

                {/* Panel content */}
                <div className="flex-1 overflow-hidden">
                  {activeTab.kind === 'terminal' ? (
                    <TerminalPanel
                      id={activeTab.agentId}
                      command={activeTab.command!}
                      args={activeTab.args ?? []}
                      cwd={activeTab.cwd ?? ''}
                      env={activeTab.env ?? {}}
                    />
                  ) : (
                    <IframePanel tab={activeTab} />
                  )}
                </div>
              </div>
            )}

            {/* ── Row-resize handle ── */}
            {showSplit && (
              <div
                onMouseDown={onRowMouseDown}
                className="drag-row"
              />
            )}

            {/* ── Agent detail ── */}
            {!panelFullscreen && (
              <div className="flex-1 overflow-hidden">
                {selectedAgent ? (
                  <AgentDetail
                    agent={selectedAgent}
                    logs={agentLogs}
                    onStart={startAgent}
                    onStop={stopAgent}
                    onOpenUI={openAgentUI}
                    onOpenTerminal={openAgentTerminal}
                    uiIsOpen={uiIsOpen}
                    termIsOpen={termIsOpen}
                  />
                ) : (
                  <div className="flex h-full flex-col items-center justify-center gap-3">
                    <div className="flex h-16 w-16 items-center justify-center rounded-2xl bg-gray-100 dark:bg-gray-800">
                      <img src={logoUrl} alt="" className="h-10 w-10 opacity-40" />
                    </div>
                    <p className="text-sm text-gray-400 dark:text-gray-500">Select an agent to view details</p>
                    <button onClick={openNew} className="text-sm text-blue-500 hover:text-blue-400 dark:text-blue-400 dark:hover:text-blue-300">
                      + Create your first agent
                    </button>
                  </div>
                )}
              </div>
            )}

          </div>
        )}
      </main>

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

// ── IframePanel ────────────────────────────────────────────
function IframePanel({ tab }: { tab: OpenTab }) {
  const [key, setKey] = useState(0)
  const iframeRef = useRef<HTMLIFrameElement>(null)
  const base = `http://localhost:${tab.port}`

  function handleLoad() {
    if (!tab.token) return
    const iframe = iframeRef.current
    if (!iframe) return
    try {
      const doc = iframe.contentDocument
      if (!doc) return
      const wsUrl = `ws://127.0.0.1:${tab.port}`
      const token = tab.token
      const script = doc.createElement('script')
      script.textContent = `
        (function autoConnect() {
          var attempts = 0;
          var iv = setInterval(function() {
            attempts++;
            var inputs = Array.from(document.querySelectorAll('input'));
            var wsInput = inputs.find(function(el) {
              return el.type === 'text' || el.type === 'url' || el.placeholder.toLowerCase().includes('ws');
            });
            var tokenInput = inputs.find(function(el) {
              return el.type === 'password' || el.placeholder.toLowerCase().includes('token') || el.placeholder.includes('令牌');
            });
            var connectBtn = Array.from(document.querySelectorAll('button')).find(function(b) {
              return b.textContent.trim().includes('连接') || b.textContent.trim().toLowerCase().includes('connect');
            });
            if (wsInput && tokenInput && connectBtn) {
              clearInterval(iv);
              var setter = Object.getOwnPropertyDescriptor(window.HTMLInputElement.prototype, 'value').set;
              setter.call(wsInput, ${JSON.stringify(wsUrl)});
              wsInput.dispatchEvent(new Event('input', { bubbles: true }));
              setter.call(tokenInput, ${JSON.stringify(token)});
              tokenInput.dispatchEvent(new Event('input', { bubbles: true }));
              setTimeout(function() { connectBtn.click(); }, 150);
            }
            if (attempts > 40) clearInterval(iv);
          }, 100);
        })();
      `
      doc.head.appendChild(script)
    } catch { /* cross-origin, user fills manually */ }
  }

  return (
    <div className="flex h-full flex-col">
      <div className="flex items-center gap-2 border-b border-gray-200 bg-white px-3 py-1 shrink-0 dark:border-gray-800 dark:bg-gray-900">
        <button onClick={() => setKey(k => k + 1)} className="rounded p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600 dark:hover:bg-gray-700 dark:hover:text-gray-300" title="Reload">
          <RefreshCw className="h-3.5 w-3.5" />
        </button>
      </div>
      <iframe
        ref={iframeRef}
        key={key}
        src={base}
        title={tab.label}
        className="flex-1 w-full border-0 bg-white"
        sandbox="allow-scripts allow-same-origin allow-forms allow-popups"
        onLoad={handleLoad}
      />
    </div>
  )
}
