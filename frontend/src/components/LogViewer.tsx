import { useEffect, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
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

export function LogViewer({ logs, agentName }: Props) {
  const { t } = useTranslation()
  const containerRef = useRef<HTMLDivElement>(null)
  const bottomRef = useRef<HTMLDivElement>(null)
  const [following, setFollowing] = useState(true)

  useEffect(() => {
    if (!following) return
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' })
  }, [logs, following])

  function handleScroll() {
    const el = containerRef.current
    if (!el) return
    const atBottom = el.scrollHeight - el.scrollTop - el.clientHeight < 40
    if (atBottom !== following) {
      setFollowing(atBottom)
    }
  }

  return (
    <div className="flex flex-col h-full bg-gray-50 dark:bg-gray-950">
      <div className="flex items-center justify-between px-4 py-2 border-b border-gray-200 shrink-0 dark:border-gray-800">
        <span className="text-sm font-medium text-gray-600 dark:text-gray-300">{t('agentDetail.logsTitle')} — {agentName}</span>
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
      <div
        ref={containerRef}
        onScroll={handleScroll}
        className="flex-1 overflow-y-auto font-mono text-xs p-4 space-y-0.5"
      >
        {logs.length === 0 ? (
          <span className="text-gray-400 dark:text-gray-600">{t('agentDetail.noLogs')}</span>
        ) : (
          logs.map((entry, i) => (
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
