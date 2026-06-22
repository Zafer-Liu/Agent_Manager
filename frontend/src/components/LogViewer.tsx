import { useEffect, useRef, useState, useMemo } from 'react'
import { useTranslation } from 'react-i18next'
import { Search, X } from 'lucide-react'
import type { LogEntry } from '../types/agent'

interface Props {
  logs: LogEntry[]
  agentName: string
}

const levelColors: Record<string, string> = {
  info: 'text-blue-600 dark:text-blue-400',
  warn: 'text-yellow-600 dark:text-yellow-400',
  error: 'text-red-600 dark:text-red-400',
  debug: 'text-gray-400 dark:text-gray-500',
}

const ALL_LEVELS = ['info', 'warn', 'error', 'debug']

export function LogViewer({ logs, agentName }: Props) {
  const { t } = useTranslation()
  const containerRef = useRef<HTMLDivElement>(null)
  const bottomRef = useRef<HTMLDivElement>(null)
  const [following, setFollowing] = useState(true)
  const [search, setSearch] = useState('')
  const [activeLevels, setActiveLevels] = useState<Set<string>>(new Set(ALL_LEVELS))

  const filtered = useMemo(() => {
    const q = search.toLowerCase()
    return logs.filter(entry => {
      if (!activeLevels.has(entry.level)) return false
      if (!q) return true
      return entry.message.toLowerCase().includes(q)
    })
  }, [logs, search, activeLevels])

  useEffect(() => {
    if (!following) return
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' })
  }, [filtered, following])

  function handleScroll() {
    const el = containerRef.current
    if (!el) return
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40
    if (atBottom !== following) {
      setFollowing(atBottom)
    }
  }

  function toggleLevel(level: string) {
    setActiveLevels(prev => {
      const next = new Set(prev)
      if (next.has(level)) next.delete(level)
      else next.add(level)
      return next
    })
  }

  const matchCount = search ? filtered.length : 0

  return (
    <div className="flex flex-col h-full bg-gray-50 dark:bg-gray-950">
      {/* Header with search and filters */}
      <div className="px-4 py-2 border-b border-gray-200 shrink-0 dark:border-gray-800 space-y-2">
        <div className="flex items-center justify-between">
          <span className="text-sm font-medium text-gray-600 dark:text-gray-300">{t('agentDetail.logsTitle')} — {agentName}</span>
          <div className="flex items-center gap-2">
            {search && (
              <span className="text-xs text-gray-400">{matchCount} matches</span>
            )}
            {!following && (
              <button
                onClick={() => {
                  setFollowing(true)
                  bottomRef.current?.scrollIntoView({ behavior: 'smooth' })
                }}
                className="text-xs text-blue-600 hover:text-blue-500 transition-colors dark:text-blue-400 dark:hover:text-blue-300"
              >
                {t('agentDetail.jumpToLatest')}
              </button>
            )}
          </div>
        </div>

        {/* Search bar */}
        <div className="relative">
          <Search className="absolute left-2.5 top-1/2 h-3.5 w-3.5 -translate-y-1/2 text-gray-400" />
          <input
            value={search}
            onChange={e => setSearch(e.target.value)}
            placeholder={t('agentDetail.searchLogs')}
            className="w-full rounded-lg border border-gray-200 bg-white pl-8 pr-7 py-1.5 text-xs text-gray-700 dark:border-gray-700 dark:bg-gray-900 dark:text-gray-300 outline-none focus:border-blue-400"
          />
          {search && (
            <button
              onClick={() => setSearch('')}
              className="absolute right-2 top-1/2 -translate-y-1/2 text-gray-400 hover:text-gray-600"
            >
              <X className="h-3.5 w-3.5" />
            </button>
          )}
        </div>

        {/* Level filters */}
        <div className="flex items-center gap-1.5">
          {ALL_LEVELS.map(level => (
            <button
              key={level}
              onClick={() => toggleLevel(level)}
              className={`rounded px-2 py-0.5 text-[10px] font-medium uppercase transition-colors ${
                activeLevels.has(level)
                  ? `${levelColors[level]} bg-gray-100 dark:bg-gray-800`
                  : 'text-gray-300 dark:text-gray-700 bg-gray-50 dark:bg-gray-900'
              }`}
            >
              {level}
            </button>
          ))}
        </div>
      </div>

      {/* Log entries */}
      <div
        ref={containerRef}
        onScroll={handleScroll}
        className="flex-1 overflow-y-auto font-mono text-xs p-4 space-y-0.5"
      >
        {filtered.length === 0 ? (
          <span className="text-gray-400 dark:text-gray-600">
            {logs.length === 0 ? t('agentDetail.noLogs') : t('agentDetail.noMatchingLogs')}
          </span>
        ) : (
          filtered.map((entry, i) => (
            <div key={i} className="flex gap-3 leading-5">
              <span className="text-gray-400 shrink-0 dark:text-gray-600">
                {new Date(entry.timestamp).toLocaleTimeString()}
              </span>
              <span className={`uppercase shrink-0 w-10 ${levelColors[entry.level] ?? 'text-gray-500'}`}>
                {entry.level}
              </span>
              <span className="text-gray-700 break-all dark:text-gray-300">{entry.message}</span>
            </div>
          ))
        )}
        <div ref={bottomRef} />
      </div>
    </div>
  )
}
