import { useState } from 'react'
import type { AgentState, LogEntry } from '../types/agent'
import { LogViewer } from './LogViewer'
import { Play, Square, Globe, LayoutList, Info, TerminalSquare } from 'lucide-react'

interface Props {
  agent: AgentState
  logs: LogEntry[]
  onStart: (id: string) => void
  onStop: (id: string) => void
  onOpenUI: (agent: AgentState) => void
  onOpenTerminal: (agent: AgentState) => void
  uiIsOpen?: boolean
  termIsOpen?: boolean
}

type Tab = 'overview' | 'logs'

export function AgentDetail({
  agent, logs, onStart, onStop, onOpenUI, onOpenTerminal,
  uiIsOpen = false, termIsOpen = false,
}: Props) {
  const [tab, setTab] = useState<Tab>('overview')
  const { config, status, pid, started_at, port_open } = agent

  return (
    <div className="flex h-full flex-col bg-white dark:bg-gray-900">

      {/* ── Top bar ── */}
      <div className="border-b border-gray-200 px-5 py-3.5 dark:border-gray-800">
        <div className="flex items-center justify-between">
          {/* Agent identity */}
          <div className="flex items-center gap-3 min-w-0">
            <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-xl bg-gray-100 text-base font-semibold text-gray-600 dark:bg-gray-800 dark:text-gray-200">
              {config.name.charAt(0).toUpperCase()}
            </div>
            <div className="min-w-0">
              <h2 className="text-sm font-semibold text-gray-900 truncate dark:text-gray-100">{config.name}</h2>
              {config.description && (
                <p className="text-xs text-gray-400 truncate dark:text-gray-500">{config.description}</p>
              )}
            </div>
          </div>

          {/* Actions */}
          <div className="flex items-center gap-1.5 shrink-0 ml-3">
            {config.port && (
              <span className={`rounded-md px-2 py-1 text-xs font-mono shrink-0 ${
                port_open
                  ? 'bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400'
                  : 'bg-gray-100 text-gray-400 dark:bg-gray-800 dark:text-gray-500'
              }`}>
                :{config.port} {port_open ? '●' : '○'}
              </span>
            )}

            {/* Open Terminal button */}
            <button
              onClick={() => onOpenTerminal(agent)}
              className={`flex items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-xs font-medium transition-colors ${
                termIsOpen
                  ? 'bg-green-100 text-green-700 hover:bg-green-200 dark:bg-green-900/30 dark:text-green-400 dark:hover:bg-green-900/50'
                  : 'bg-gray-100 text-gray-600 hover:bg-gray-200 dark:bg-gray-800 dark:text-gray-300 dark:hover:bg-gray-700'
              }`}
              title="Open interactive terminal"
            >
              <TerminalSquare className="h-3.5 w-3.5" />
              Terminal
            </button>

            {/* Open UI button */}
            {config.port && port_open && (
              <button
                onClick={() => onOpenUI(agent)}
                className={`flex items-center gap-1.5 rounded-lg px-2.5 py-1.5 text-xs font-medium transition-colors ${
                  uiIsOpen
                    ? 'bg-blue-100 text-blue-700 hover:bg-blue-200 dark:bg-blue-900/30 dark:text-blue-400 dark:hover:bg-blue-900/50'
                    : 'bg-gray-100 text-gray-600 hover:bg-gray-200 dark:bg-gray-800 dark:text-gray-300 dark:hover:bg-gray-700'
                }`}
                title="Open web UI"
              >
                <Globe className="h-3.5 w-3.5" />
                {uiIsOpen ? 'UI Open' : 'Open UI'}
              </button>
            )}

            {/* Start / Stop */}
            {config.port && (status === 'running' ? (
              <button
                onClick={() => onStop(config.id)}
                className="flex items-center gap-1.5 rounded-lg bg-red-100 px-2.5 py-1.5 text-xs font-medium text-red-600 transition-colors hover:bg-red-200 dark:bg-red-900/30 dark:text-red-400 dark:hover:bg-red-900/50"
              >
                <Square className="h-3.5 w-3.5" /> Stop
              </button>
            ) : (
              <button
                onClick={() => onStart(config.id)}
                className="flex items-center gap-1.5 rounded-lg bg-green-100 px-2.5 py-1.5 text-xs font-medium text-green-700 transition-colors hover:bg-green-200 dark:bg-green-900/30 dark:text-green-400 dark:hover:bg-green-900/50"
              >
                <Play className="h-3.5 w-3.5" /> Start
              </button>
            ))}
          </div>
        </div>

        {/* Sub-tabs */}
        <div className="mt-3 flex gap-0.5">
          {([
            { id: 'overview' as Tab, label: 'Overview', icon: <Info className="h-3.5 w-3.5" /> },
            { id: 'logs' as Tab, label: 'Logs', icon: <LayoutList className="h-3.5 w-3.5" /> },
          ]).map(t => (
            <button
              key={t.id}
              onClick={() => setTab(t.id)}
              className={`flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-xs font-medium transition-colors ${
                tab === t.id
                  ? 'bg-gray-100 text-gray-800 dark:bg-gray-800 dark:text-gray-100'
                  : 'text-gray-400 hover:bg-gray-100 hover:text-gray-600 dark:text-gray-500 dark:hover:bg-gray-800 dark:hover:text-gray-300'
              }`}
            >
              {t.icon}{t.label}
            </button>
          ))}
        </div>
      </div>

      {/* ── Tab content ── */}
      <div className="flex-1 overflow-hidden">
        {tab === 'overview' && (
          <div className="h-full overflow-y-auto p-5 space-y-3">
            <div className="grid grid-cols-3 gap-2">
              <InfoCard label="Status"><StatusBadge status={status} /></InfoCard>
              <InfoCard label="PID">
                <span className="font-mono text-sm text-gray-700 dark:text-gray-200">{pid ?? '—'}</span>
              </InfoCard>
              <InfoCard label="Started">
                <span className="text-sm text-gray-700 dark:text-gray-200">
                  {started_at ? new Date(started_at).toLocaleTimeString() : '—'}
                </span>
              </InfoCard>
            </div>

            <div className="rounded-xl border border-gray-200 p-4 dark:border-gray-800">
              <p className="mb-2 text-xs font-medium uppercase tracking-wide text-gray-400 dark:text-gray-500">Command</p>
              <code className="block rounded-lg bg-gray-100 px-3 py-2 text-sm font-mono text-gray-800 break-all dark:bg-gray-800 dark:text-gray-200">
                {config.command} {config.args.join(' ')}
              </code>
              {config.working_dir && (
                <p className="mt-2 text-xs text-gray-400 dark:text-gray-500">
                  <span className="font-medium text-gray-500 dark:text-gray-400">cwd:</span> {config.working_dir}
                </p>
              )}
            </div>

            {Object.keys(config.env).length > 0 && (
              <div className="rounded-xl border border-gray-200 p-4 dark:border-gray-800">
                <p className="mb-2 text-xs font-medium uppercase tracking-wide text-gray-400 dark:text-gray-500">Environment</p>
                <div className="space-y-1">
                  {Object.entries(config.env).map(([k, v]) => (
                    <div key={k} className="flex gap-2 font-mono text-xs">
                      <span className="text-blue-600 dark:text-blue-400">{k}</span>
                      <span className="text-gray-400">=</span>
                      <span className="truncate text-gray-500 dark:text-gray-400">{v}</span>
                    </div>
                  ))}
                </div>
              </div>
            )}

            {config.port && (
              <div className="rounded-xl border border-gray-200 p-4 dark:border-gray-800">
                <p className="mb-2 text-xs font-medium uppercase tracking-wide text-gray-400 dark:text-gray-500">Port</p>
                <div className="flex items-center gap-3">
                  <span className="font-mono text-sm text-gray-700 dark:text-gray-200">:{config.port}</span>
                  <span className={`text-xs ${port_open ? 'text-green-600 dark:text-green-400' : 'text-gray-400 dark:text-gray-600'}`}>
                    {port_open ? '● listening' : '○ not listening'}
                  </span>
                  {port_open && (
                    <button
                      onClick={() => onOpenUI(agent)}
                      className="flex items-center gap-1 text-xs text-blue-600 hover:text-blue-500 dark:text-blue-400 dark:hover:text-blue-300"
                    >
                      <Globe className="h-3 w-3" />
                      {uiIsOpen ? 'UI already open ↑' : 'Open UI tab'}
                    </button>
                  )}
                </div>
              </div>
            )}
          </div>
        )}

        {tab === 'logs' && (
          <LogViewer logs={logs} agentName={config.name} />
        )}
      </div>
    </div>
  )
}

function InfoCard({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div className="rounded-xl border border-gray-200 px-4 py-3 dark:border-gray-800">
      <p className="mb-1 text-xs font-medium uppercase tracking-wide text-gray-400 dark:text-gray-600">{label}</p>
      {children}
    </div>
  )
}

function StatusBadge({ status }: { status: AgentState['status'] }) {
  const styles: Record<string, string> = {
    running:  'bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400',
    stopped:  'bg-gray-100 text-gray-500 dark:bg-gray-800 dark:text-gray-400',
    error:    'bg-red-100 text-red-600 dark:bg-red-900/30 dark:text-red-400',
    starting: 'bg-yellow-100 text-yellow-700 dark:bg-yellow-900/30 dark:text-yellow-400',
  }
  return (
    <span className={`inline-flex items-center gap-1 rounded-md px-2 py-0.5 text-xs font-medium ${styles[status] ?? styles.stopped}`}>
      <span className="h-1.5 w-1.5 rounded-full bg-current" />
      {status}
    </span>
  )
}
