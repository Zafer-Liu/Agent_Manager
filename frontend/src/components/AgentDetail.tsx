import { useState } from 'react'
import type { AgentState } from '../types/agent'
import type { LogEntry } from '../types/agent'
import { LogViewer } from './LogViewer'
import { Play, Square, Globe, LayoutList, Info } from 'lucide-react'

interface Props {
  agent: AgentState
  logs: LogEntry[]
  onStart: (id: string) => void
  onStop: (id: string) => void
  onOpenUI: (agent: AgentState) => void
}

type Tab = 'overview' | 'logs'

export function AgentDetail({ agent, logs, onStart, onStop, onOpenUI }: Props) {
  const [tab, setTab] = useState<Tab>('overview')
  const { config, status, pid, started_at, port_open } = agent

  const tabs: { id: Tab; label: string; icon: React.ReactNode }[] = [
    { id: 'overview', label: 'Overview', icon: <Info className="h-3.5 w-3.5" /> },
    { id: 'logs', label: 'Logs', icon: <LayoutList className="h-3.5 w-3.5" /> },
  ]

  return (
    <div className="flex h-full flex-col bg-white dark:bg-gray-900">
      {/* Top bar */}
      <div className="border-b border-gray-200 px-6 py-4 dark:border-gray-700">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-3">
            <div className="flex h-10 w-10 items-center justify-center rounded-xl bg-gray-100 text-lg dark:bg-gray-800">
              {config.name.charAt(0).toUpperCase()}
            </div>
            <div>
              <h2 className="text-base font-semibold text-gray-900 dark:text-gray-100">{config.name}</h2>
              {config.description && (
                <p className="text-xs text-gray-500 dark:text-gray-400">{config.description}</p>
              )}
            </div>
          </div>

          <div className="flex items-center gap-2">
            {config.port && (
              <span className={`rounded-lg px-2 py-1 text-xs font-mono ${
                port_open
                  ? 'bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400'
                  : 'bg-gray-100 text-gray-500 dark:bg-gray-800 dark:text-gray-500'
              }`}>
                :{config.port} {port_open ? '● open' : '○ closed'}
              </span>
            )}
            {config.port && port_open && (
              <button
                onClick={() => onOpenUI(agent)}
                className="flex items-center gap-1.5 rounded-lg bg-blue-50 px-3 py-1.5 text-sm font-medium text-blue-600 transition-colors hover:bg-blue-100 dark:bg-blue-900/20 dark:text-blue-400"
              >
                <Globe className="h-3.5 w-3.5" /> Open UI
              </button>
            )}
            {status === 'running' ? (
              <button
                onClick={() => onStop(config.id)}
                className="flex items-center gap-1.5 rounded-lg bg-red-50 px-3 py-1.5 text-sm font-medium text-red-600 transition-colors hover:bg-red-100 dark:bg-red-900/20 dark:text-red-400"
              >
                <Square className="h-3.5 w-3.5" /> Stop
              </button>
            ) : (
              <button
                onClick={() => onStart(config.id)}
                className="flex items-center gap-1.5 rounded-lg bg-green-50 px-3 py-1.5 text-sm font-medium text-green-600 transition-colors hover:bg-green-100 dark:bg-green-900/20 dark:text-green-400"
              >
                <Play className="h-3.5 w-3.5" /> Start
              </button>
            )}
          </div>
        </div>

        {/* Tabs */}
        <div className="mt-4 flex gap-1">
          {tabs.map(t => (
            <button
              key={t.id}
              onClick={() => setTab(t.id)}
              className={`flex items-center gap-1.5 rounded-lg px-3 py-1.5 text-sm font-medium transition-colors ${
                tab === t.id
                  ? 'bg-blue-50 text-blue-600 dark:bg-blue-900/20 dark:text-blue-400'
                  : 'text-gray-500 hover:bg-gray-100 hover:text-gray-700 dark:text-gray-400 dark:hover:bg-gray-800'
              }`}
            >
              {t.icon}
              {t.label}
            </button>
          ))}
        </div>
      </div>

      {/* Tab content */}
      <div className="flex-1 overflow-hidden">
        {tab === 'overview' && (
          <div className="h-full overflow-y-auto p-6 space-y-4">
            <div className="grid grid-cols-3 gap-3">
              <InfoCard label="Status"><StatusBadge status={status} /></InfoCard>
              <InfoCard label="PID">
                <span className="font-mono text-sm text-gray-900 dark:text-gray-100">{pid ?? '—'}</span>
              </InfoCard>
              <InfoCard label="Started">
                <span className="text-sm text-gray-900 dark:text-gray-100">
                  {started_at ? new Date(started_at).toLocaleTimeString() : '—'}
                </span>
              </InfoCard>
            </div>

            <div className="rounded-xl border border-gray-200 dark:border-gray-700 p-4">
              <p className="mb-2 text-xs font-medium uppercase tracking-wide text-gray-500 dark:text-gray-400">Command</p>
              <code className="block rounded-lg bg-gray-50 dark:bg-gray-800 px-3 py-2 text-sm font-mono text-gray-800 dark:text-gray-200">
                {config.command} {config.args.join(' ')}
              </code>
              {config.working_dir && (
                <p className="mt-2 text-xs text-gray-500 dark:text-gray-400">
                  <span className="font-medium">cwd:</span> {config.working_dir}
                </p>
              )}
            </div>

            {Object.keys(config.env).length > 0 && (
              <div className="rounded-xl border border-gray-200 dark:border-gray-700 p-4">
                <p className="mb-2 text-xs font-medium uppercase tracking-wide text-gray-500 dark:text-gray-400">Environment</p>
                <div className="space-y-1">
                  {Object.entries(config.env).map(([k, v]) => (
                    <div key={k} className="flex gap-2 font-mono text-xs">
                      <span className="text-blue-600 dark:text-blue-400">{k}</span>
                      <span className="text-gray-400">=</span>
                      <span className="truncate text-gray-700 dark:text-gray-300">{v}</span>
                    </div>
                  ))}
                </div>
              </div>
            )}

            {config.port && (
              <div className="rounded-xl border border-gray-200 dark:border-gray-700 p-4">
                <p className="mb-2 text-xs font-medium uppercase tracking-wide text-gray-500 dark:text-gray-400">Port</p>
                <div className="flex items-center gap-3">
                  <span className="font-mono text-sm text-gray-900 dark:text-gray-100">:{config.port}</span>
                  <span className={`text-xs ${port_open ? 'text-green-500' : 'text-gray-400'}`}>
                    {port_open ? '● listening' : '○ not listening'}
                  </span>
                  {port_open && (
                    <button
                      onClick={() => onOpenUI(agent)}
                      className="flex items-center gap-1 text-xs text-blue-500 hover:text-blue-400"
                    >
                      <Globe className="h-3 w-3" /> Open UI tab
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
    <div className="rounded-xl border border-gray-200 dark:border-gray-700 px-4 py-3">
      <p className="mb-1 text-xs font-medium uppercase tracking-wide text-gray-500 dark:text-gray-400">{label}</p>
      {children}
    </div>
  )
}

function StatusBadge({ status }: { status: AgentState['status'] }) {
  const styles: Record<string, string> = {
    running: 'bg-green-100 text-green-700 dark:bg-green-900/30 dark:text-green-400',
    stopped: 'bg-gray-100 text-gray-600 dark:bg-gray-800 dark:text-gray-400',
    error: 'bg-red-100 text-red-600 dark:bg-red-900/30 dark:text-red-400',
    starting: 'bg-yellow-100 text-yellow-700 dark:bg-yellow-900/30 dark:text-yellow-400',
  }
  return (
    <span className={`inline-flex items-center gap-1 rounded-md px-2 py-0.5 text-xs font-medium ${styles[status] ?? styles.stopped}`}>
      <span className="h-1.5 w-1.5 rounded-full bg-current" />
      {status}
    </span>
  )
}
