import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { invoke } from '@tauri-apps/api/core'
import { useAgentStore } from './store/agentStore'
import { LanguageSwitcher } from './components/LanguageSwitcher'
import { AgentList } from './components/AgentList'
import { AgentDetail } from './components/AgentDetail'
import { AgentForm } from './components/AgentForm'
import { TerminalPanel } from './components/TerminalPanel'
import { UpdateChecker } from './components/UpdateChecker'
import { PortManager } from './pages/PortManager'
import { McpAgent } from './pages/McpAgent'
import { Dashboard } from './pages/Dashboard'
import { ManagerAgent, type ManagerSessionState } from './pages/ManagerAgent'
import { ProxyManager } from './pages/ProxyManager'
import { Settings } from './pages/Settings'
import { useTheme } from './theme'
import type { AgentState } from './types/agent'
import { useResizable } from './hooks/useResizable'
import { NativeWebviewPanel, type OpenTab } from './components/NativeWebviewPanel'
import {
  Plus, RefreshCw, Bot, X, Globe, Network, Cpu,
  Maximize2, Minimize2, TerminalSquare, Sun, Moon,
  Crown, LayoutDashboard, Shield, Eraser, Settings2,
} from 'lucide-react'
import logoUrl from '/logo.png'

type NavPage = 'agents' | 'mcp-agent' | 'ports' | 'dashboard' | 'manager' | 'proxy' | 'settings'

export default function App() {
  const {
    agents, selectedId, logs, loading,
    fetchAgents, selectAgent, reorderAgents,
    startAgent, stopAgent, saveAgent, deleteAgent,
  } = useAgentStore()

  const { theme, toggle: toggleTheme } = useTheme()
  const { t } = useTranslation()

  const [page, setPage] = useState<NavPage>('agents')
  const [showForm, setShowForm] = useState(false)
  const [editingAgent, setEditingAgent] = useState<AgentState | null>(null)
  const [openTabs, setOpenTabs] = useState<OpenTab[]>([])
  const [activeTabKey, setActiveTabKey] = useState<string | null>(null)
  const [terminalClearVersions, setTerminalClearVersions] = useState<Record<string, number>>({})
  const [panelFullscreen, setPanelFullscreen] = useState(false)
  const [managerSession, setManagerSession] = useState<ManagerSessionState>({ messages: [], selectedProvider: '' })

  const { width: sidebarWidth, height: panelHeight, onColMouseDown, onRowMouseDown } = useResizable({
    minW: 200, maxW: 480, defaultW: 288,
    minH: 120, maxH: 800, defaultH: 380,
  })

  // Silent update check: runs once per session; throttled to every 7 days in the component
  const [hasUpdate, setHasUpdate] = useState(false)
  useEffect(() => {
    const LAST_CHECK_KEY = 'updater_last_check'
    const CHECK_INTERVAL_MS = 7 * 24 * 60 * 60 * 1000
    const last = localStorage.getItem(LAST_CHECK_KEY)
    if (last && Date.now() - Number(last) < CHECK_INTERVAL_MS) return
    invoke<{ has_update: boolean; latest: string }>('check_for_update')
      .then(r => {
        if (r.has_update) setHasUpdate(true)
        localStorage.setItem(LAST_CHECK_KEY, String(Date.now()))
      })
      .catch(() => {})
  }, [])

  const selectedAgent = agents.find(a => a.config.id === selectedId) ?? null
  const agentLogs = selectedId ? (logs[selectedId] ?? []) : []
  const activeTab = openTabs.find(t => `${t.agentId}:${t.kind}` === activeTabKey)
    ?? openTabs.find(t => t.agentId === selectedId)
    ?? openTabs[0]
    ?? null
  const resolvedActiveTabKey = activeTab ? `${activeTab.agentId}:${activeTab.kind}` : null
  const showPanel = activeTab !== null
  const showSplit = showPanel && !panelFullscreen

  useEffect(() => {
    fetchAgents()
    const id = setInterval(fetchAgents, 5000)
    return () => clearInterval(id)
  }, [fetchAgents])

  useEffect(() => {
    if (!selectedId) return
    const id = setInterval(() => useAgentStore.getState().fetchLogs(selectedId), 3000)
    return () => clearInterval(id)
  }, [selectedId])

  function openAgentUI(agent: AgentState) {
    if (!agent.config.port) return
    const id = agent.config.id
    const uiTab: OpenTab = {
      agentId: id,
      label: agent.config.name,
      kind: 'ui',
      port: agent.config.port,
      token: agent.config.ui_token,
    }
    setOpenTabs(tabs => {
      const exists = tabs.some(tab => tab.agentId === id && tab.kind === 'ui')
      return exists
        ? tabs.map(tab => tab.agentId === id && tab.kind === 'ui' ? uiTab : tab)
        : [...tabs, uiTab]
    })
    selectAgent(id)
    setActiveTabKey(`${id}:ui`)
    setPage('agents')
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
    if (tab.kind === 'ui') {
      invoke('close_agent_ui_webview', { agentId: tab.agentId }).catch(() => {})
    }
    const key = `${tab.agentId}:${tab.kind}`
    setOpenTabs(tabs => tabs.filter(t => !(t.agentId === tab.agentId && t.kind === tab.kind)))
    if (activeTabKey === key) setActiveTabKey(null)
    setPanelFullscreen(false)
  }

  function clearActiveTerminal() {
    if (!activeTab || activeTab.kind !== 'terminal') return
    setTerminalClearVersions(versions => ({
      ...versions,
      [activeTab.agentId]: (versions[activeTab.agentId] ?? 0) + 1,
    }))
  }

  function togglePanelFullscreen() {
    if (!activeTab) return
    if (activeTab.kind === 'ui') {
      invoke('fullscreen_agent_ui_webview', {
        agentId: activeTab.agentId,
        title: activeTab.label,
      }).catch(error => console.error('Failed to fullscreen agent UI', error))
      return
    }
    setPanelFullscreen(fullscreen => !fullscreen)
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
            <img src={logoUrl} alt={t('app.title')} className="h-7 w-7 rounded-lg" />
            <div className="flex flex-col leading-tight">
              <span className="text-sm font-semibold text-gray-900 dark:text-gray-100">{t('app.title')}</span>
              <div className="flex items-center gap-1">
                <span className="text-[10px] text-gray-400 dark:text-gray-500">v0.2.3</span>
                {hasUpdate && (
                  <button
                    onClick={() => setPage('settings')}
                    className="rounded-full bg-blue-100 px-1.5 py-0 text-[9px] font-semibold text-blue-700 hover:bg-blue-200 dark:bg-blue-900/50 dark:text-blue-300"
                    title={t('updater.newVersionAvailable', { version: '' })}
                  >
                    UPDATE
                  </button>
                )}
              </div>
            </div>
          </div>
          <div className="flex items-center gap-0.5">
            <LanguageSwitcher />
            <button
              onClick={toggleTheme}
              className="rounded-md p-1.5 text-gray-400 transition-colors hover:bg-gray-100 hover:text-gray-600 dark:hover:bg-gray-800 dark:hover:text-gray-300"
              title={theme === 'dark' ? t('app.switchToLight') : t('app.switchToDark')}
            >
              {theme === 'dark' ? <Sun className="h-4 w-4" /> : <Moon className="h-4 w-4" />}
            </button>
          </div>
        </div>

        {/* Nav */}
        <div className="flex flex-col gap-0.5 p-2 border-b border-gray-200 dark:border-gray-800">
          {([
            { id: 'dashboard', icon: <LayoutDashboard className="h-4 w-4" />, label: t('nav.dashboard') },
            { id: 'manager',   icon: <Crown className="h-4 w-4" />,           label: t('nav.manager') },
            { id: 'agents',    icon: <Bot className="h-4 w-4" />,             label: t('nav.agents') },
            { id: 'mcp-agent', icon: <Cpu className="h-4 w-4" />,             label: t('nav.mcpAgent') },
            { id: 'ports',     icon: <Network className="h-4 w-4" />,         label: t('nav.ports') },
            { id: 'proxy',     icon: <Shield className="h-4 w-4" />,          label: t('nav.proxy') },
            { id: 'settings',  icon: <Settings2 className="h-4 w-4" />,       label: t('nav.settings') },
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
            <p className="text-xs text-gray-400">{t('app.configured', { count: agents.length })}</p>
            <div className="flex gap-1">
              <button onClick={openNew} className="rounded p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600 dark:hover:bg-gray-800 dark:hover:text-gray-300" title={t('common.add')}>
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
              <Plus className="h-4 w-4" /> {t('app.newAgent')}
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
        {page === 'ports' && <PortManager agents={agents} />}
        {page === 'proxy' && <ProxyManager agents={agents} />}
        {page === 'settings' && <Settings />}

        {/* Keep the workspace mounted so terminals and iframe state survive navigation. */}
        <div className={`flex flex-1 flex-col overflow-hidden ${page === 'agents' ? '' : 'hidden'}`}>

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
                        {tab.kind === 'terminal' ? t('app.term') : `${tab.port}`}
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

            {/* ── Active terminal or embedded browser panel ── */}
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
                      : `http://127.0.0.1:${activeTab.port}`
                    }
                  </span>
                  {activeTab.kind === 'terminal' && (
                    <button
                      onClick={clearActiveTerminal}
                      className="rounded p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600 dark:hover:bg-gray-700 dark:hover:text-gray-300"
                      title={t('common.clear')}
                    >
                      <Eraser className="h-3.5 w-3.5" />
                    </button>
                  )}
                  <button
                    onClick={togglePanelFullscreen}
                    className="rounded p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600 dark:hover:bg-gray-700 dark:hover:text-gray-300"
                    title={activeTab.kind === 'terminal' && panelFullscreen ? t('common.restore') : t('common.fullscreen')}
                  >
                    {activeTab.kind === 'terminal' && panelFullscreen
                      ? <Minimize2 className="h-3.5 w-3.5" />
                      : <Maximize2 className="h-3.5 w-3.5" />
                    }
                  </button>
                  <button
                    onClick={e => closeTab(activeTab, e)}
                    className="rounded p-1 text-gray-400 hover:bg-gray-100 hover:text-red-500 dark:hover:bg-gray-700 dark:hover:text-red-400"
                    title={t('common.close')}
                  >
                    <X className="h-3.5 w-3.5" />
                  </button>
                </div>

                {/* Panel content */}
                <div className="flex-1 overflow-hidden">
                  {openTabs.map(tab => {
                    const tabKey = `${tab.agentId}:${tab.kind}`
                    const isActive = tabKey === resolvedActiveTabKey
                    return (
                      <div
                        key={tabKey}
                        className={`h-full w-full ${isActive ? '' : 'hidden'}`}
                      >
                        {tab.kind === 'terminal' ? (
                          <TerminalPanel
                            id={tab.agentId}
                            command={tab.command!}
                            args={tab.args ?? []}
                            cwd={tab.cwd ?? ''}
                            env={tab.env ?? {}}
                            active={isActive && page === 'agents'}
                            clearVersion={terminalClearVersions[tab.agentId] ?? 0}
                          />
                        ) : (
                          <NativeWebviewPanel
                            tab={tab}
                            active={isActive && page === 'agents' && !showForm}
                          />
                        )}
                      </div>
                    )
                  })}
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
                    <p className="text-sm text-gray-400 dark:text-gray-500">{t('app.selectAgentHint')}</p>
                    <button onClick={openNew} className="text-sm text-blue-500 hover:text-blue-400 dark:text-blue-400 dark:hover:text-blue-300">
                      {t('app.createFirstAgent')}
                    </button>
                  </div>
                )}
              </div>
            )}

        </div>
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
