import { useEffect, useRef } from 'react'
import type { LogEntry } from '../types/agent'

interface Props {
  logs: LogEntry[]
  agentName: string
}

const levelColors: Record<string, string> = {
  info: 'text-blue-400',
  warn: 'text-yellow-400',
  error: 'text-red-400',
  debug: 'text-gray-500',
}

export function LogViewer({ logs, agentName }: Props) {
  const bottomRef = useRef<HTMLDivElement>(null)

  useEffect(() => {
    bottomRef.current?.scrollIntoView({ behavior: 'smooth' })
  }, [logs])

  return (
    <div className="flex flex-col h-full">
      <div className="px-4 py-2 border-b border-gray-800 text-sm font-medium text-gray-300">
        Logs — {agentName}
      </div>
      <div className="flex-1 overflow-y-auto font-mono text-xs p-4 space-y-0.5">
        {logs.length === 0 ? (
          <span className="text-gray-600">No logs yet.</span>
        ) : (
          logs.map((entry, i) => (
            <div key={i} className="flex gap-3 leading-5">
              <span className="text-gray-600 shrink-0">
                {new Date(entry.timestamp).toLocaleTimeString()}
              </span>
              <span className={`uppercase shrink-0 w-10 ${levelColors[entry.level] ?? 'text-gray-400'}`}>
                {entry.level}
              </span>
              <span className="text-gray-300 break-all">{entry.message}</span>
            </div>
          ))
        )}
        <div ref={bottomRef} />
      </div>
    </div>
  )
}
