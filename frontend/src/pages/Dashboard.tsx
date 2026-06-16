import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import type { AgentState } from '../types/agent'
import { Play, Square, Crown, Moon, Zap, AlertTriangle, Loader2, MessageSquare } from 'lucide-react'

// ── Types ────────────────────────────────────────────────────────────────────

interface DashboardProps {
  agents: AgentState[]
  onSelectAgent: (id: string) => void
  onOpenAgentUI: (agent: AgentState) => void
  onStartAgent: (id: string) => Promise<void>
  onStopAgent: (id: string) => Promise<void>
  onNavigateToAgents: () => void
  onOpenManagerAgent: () => void
}

// ── Constants ────────────────────────────────────────────────────────────────

const COLS = 4
const ROWS = 3
const TOTAL_SEATS = COLS * ROWS

// ── Dashboard ────────────────────────────────────────────────────────────────

export function Dashboard({
  agents,
  onSelectAgent,
  onOpenAgentUI,
  onStartAgent,
  onStopAgent,
  onNavigateToAgents,
  onOpenManagerAgent,
}: DashboardProps) {
  const { t } = useTranslation()
  const [hoveredSeat, setHoveredSeat] = useState<number | null>(null)
  const [actionLoading, setActionLoading] = useState<string | null>(null)

  async function handleStart(id: string, e: React.MouseEvent) {
    e.stopPropagation()
    setActionLoading(id)
    try { await onStartAgent(id) } finally { setActionLoading(null) }
  }

  async function handleStop(id: string, e: React.MouseEvent) {
    e.stopPropagation()
    setActionLoading(id)
    try { await onStopAgent(id) } finally { setActionLoading(null) }
  }

  function handleSeatClick(agent: AgentState) {
    onSelectAgent(agent.config.id)
    if (agent.config.port && agent.status === 'running') {
      onOpenAgentUI(agent)
    } else {
      onNavigateToAgents()
    }
  }

  // Fill seats with agents, pad with null for empty seats
  const seats: (AgentState | null)[] = [
    ...agents.slice(0, TOTAL_SEATS),
    ...Array(Math.max(0, TOTAL_SEATS - agents.length)).fill(null),
  ]

  const runningCount = agents.filter(a => a.status === 'running').length
  const errorCount = agents.filter(a => a.status === 'error').length

  return (
    <div className="flex h-full flex-col bg-gray-50 dark:bg-gray-950 overflow-hidden">

      {/* Header */}
      <div className="shrink-0 border-b border-gray-200 dark:border-gray-800 bg-white dark:bg-gray-900 px-6 py-3">
        <div className="flex items-center justify-between">
          <div className="flex items-center gap-2">
            <Crown className="h-5 w-5 text-amber-500" />
            <span className="text-base font-semibold text-gray-900 dark:text-gray-100">{t('dashboard.title')}</span>
          </div>
          <div className="flex items-center gap-4 text-xs text-gray-500 dark:text-gray-400">
            <span className="flex items-center gap-1.5">
              <span className="h-2 w-2 rounded-full bg-green-400" />
              {t('dashboard.running', { count: runningCount })}
            </span>
            {errorCount > 0 && (
              <span className="flex items-center gap-1.5 text-red-500">
                <span className="h-2 w-2 rounded-full bg-red-400" />
                {t('dashboard.errors', { count: errorCount })}
              </span>
            )}
            <span>{t('dashboard.seats', { count: agents.length, total: TOTAL_SEATS })}</span>
          </div>
        </div>
      </div>

      <div className="flex-1 overflow-y-auto p-6">

        {/* ── Podium: Manager Agent ── */}
        <div className="mb-8 flex justify-center">
          <button
            onClick={onOpenManagerAgent}
            className="group relative flex flex-col items-center gap-2 rounded-2xl border-2 border-amber-300 bg-gradient-to-b from-amber-50 to-orange-50 px-10 py-5 shadow-md transition-all hover:shadow-lg hover:scale-105 dark:border-amber-600 dark:from-amber-900/20 dark:to-orange-900/20"
          >
            {/* Glow ring */}
            <div className="absolute inset-0 rounded-2xl bg-amber-400/10 opacity-0 group-hover:opacity-100 transition-opacity" />

            {/* Crown icon */}
            <div className="relative flex h-14 w-14 items-center justify-center rounded-2xl bg-gradient-to-br from-amber-400 to-orange-500 shadow-lg">
              <Crown className="h-7 w-7 text-white" />
              {/* Pulse ring */}
              <div className="absolute inset-0 rounded-2xl border-2 border-amber-400 animate-ping opacity-30" />
            </div>

            <div className="text-center">
              <p className="text-sm font-bold text-amber-700 dark:text-amber-400">{t('dashboard.managerName')}</p>
              <p className="text-xs text-amber-600/70 dark:text-amber-500/70">{t('dashboard.managerHint')}</p>
            </div>

            {/* Chat bubble hint */}
            <div className="absolute -right-2 -top-2 flex h-6 w-6 items-center justify-center rounded-full bg-amber-500 text-white shadow-md opacity-0 group-hover:opacity-100 transition-opacity">
              <MessageSquare className="h-3 w-3" />
            </div>
          </button>
        </div>

        {/* ── Divider: "讲台" label ── */}
        <div className="mb-6 flex items-center gap-3">
          <div className="flex-1 border-t border-dashed border-gray-300 dark:border-gray-700" />
          <span className="text-xs font-medium text-gray-400 dark:text-gray-600 uppercase tracking-widest">{t('dashboard.studentSeats')}</span>
          <div className="flex-1 border-t border-dashed border-gray-300 dark:border-gray-700" />
        </div>

        {/* ── Classroom grid ── */}
        {agents.length === 0 ? (
          <div className="flex flex-col items-center justify-center py-16 gap-3 text-gray-400">
            <div className="text-5xl">🏫</div>
            <p className="text-sm">{t('dashboard.noAgents')}</p>
            <button
              onClick={onNavigateToAgents}
              className="text-sm text-blue-500 hover:text-blue-400"
            >
              {t('dashboard.goAddAgents')}
            </button>
          </div>
        ) : (
          <div
            className="grid gap-4 mx-auto"
            style={{
              gridTemplateColumns: `repeat(${COLS}, minmax(0, 1fr))`,
              maxWidth: COLS * 200,
            }}
          >
            {seats.map((agent, i) => (
              <SeatCard
                key={agent ? agent.config.id : `empty-${i}`}
                agent={agent}
                seatIndex={i}
                isHovered={hoveredSeat === i}
                isLoading={agent ? actionLoading === agent.config.id : false}
                onHover={() => setHoveredSeat(i)}
                onLeave={() => setHoveredSeat(null)}
                onClick={() => agent && handleSeatClick(agent)}
                onStart={(e) => agent && handleStart(agent.config.id, e)}
                onStop={(e) => agent && handleStop(agent.config.id, e)}
              />
            ))}
          </div>
        )}

        {/* Overflow agents hint */}
        {agents.length > TOTAL_SEATS && (
          <div className="mt-4 text-center text-xs text-gray-400">
            {t('dashboard.overflowHint', { count: agents.length - TOTAL_SEATS })}
          </div>
        )}
      </div>
    </div>
  )
}

// ── SeatCard ─────────────────────────────────────────────────────────────────

interface SeatCardProps {
  agent: AgentState | null
  seatIndex: number
  isHovered: boolean
  isLoading: boolean
  onHover: () => void
  onLeave: () => void
  onClick: () => void
  onStart: (e: React.MouseEvent) => void
  onStop: (e: React.MouseEvent) => void
}

function SeatCard({ agent, isHovered, isLoading, onHover, onLeave, onClick, onStart, onStop }: SeatCardProps) {
  const { t } = useTranslation()
  if (!agent) {
    return (
      <div className="flex aspect-[3/4] flex-col items-center justify-center rounded-2xl border-2 border-dashed border-gray-200 dark:border-gray-800 opacity-40">
        <div className="h-10 w-10 rounded-xl bg-gray-100 dark:bg-gray-800" />
        <div className="mt-2 h-2 w-12 rounded bg-gray-100 dark:bg-gray-800" />
      </div>
    )
  }

  const status = agent.status

  return (
    <div
      className="relative flex aspect-[3/4] cursor-pointer flex-col items-center justify-between rounded-2xl border-2 p-3 transition-all duration-200 select-none"
      style={cardStyle(status, isHovered)}
      onMouseEnter={onHover}
      onMouseLeave={onLeave}
      onClick={onClick}
    >
      {/* Status badge */}
      <div className="absolute right-2 top-2">
        <StatusDot status={status} />
      </div>

      {/* Avatar */}
      <div className="mt-2 flex flex-col items-center gap-2">
        <AgentAvatar agent={agent} status={status} />
        <StatusLabel status={status} />
      </div>

      {/* Name + description */}
      <div className="w-full text-center">
        <p className="truncate text-xs font-semibold text-gray-800 dark:text-gray-100">
          {agent.config.name}
        </p>
        {agent.config.description && (
          <p className="mt-0.5 truncate text-[10px] text-gray-400 dark:text-gray-500">
            {agent.config.description}
          </p>
        )}
        {agent.config.port && (
          <p className="mt-0.5 text-[10px] font-mono text-gray-400 dark:text-gray-600">
            :{agent.config.port}
          </p>
        )}
      </div>

      {/* Hover controls */}
      {isHovered && (
        <div className="absolute inset-0 flex items-center justify-center rounded-2xl bg-black/30 backdrop-blur-[2px]">
          {isLoading ? (
            <Loader2 className="h-6 w-6 animate-spin text-white" />
          ) : (
            <div className="flex gap-2">
              {status === 'running' || status === 'starting' ? (
                <ControlBtn icon={<Square className="h-4 w-4" />} label={t('dashboard.ctrlStop')} danger onClick={onStop} />
              ) : (
                <ControlBtn icon={<Play className="h-4 w-4" />} label={t('dashboard.ctrlStart')} onClick={onStart} />
              )}
              <ControlBtn icon={<MessageSquare className="h-4 w-4" />} label={t('dashboard.ctrlDetail')} onClick={(e) => { e.stopPropagation(); onClick() }} />
            </div>
          )}
        </div>
      )}
    </div>
  )
}

// ── AgentAvatar ───────────────────────────────────────────────────────────────

function AgentAvatar({ agent, status }: { agent: AgentState; status: string }) {
  const initials = agent.config.name
    .split(/[\s_-]+/)
    .slice(0, 2)
    .map(w => w[0]?.toUpperCase() ?? '')
    .join('')

  const bgColor = avatarColor(agent.config.id)

  return (
    <div className="relative">
      {/* Main circle */}
      <div
        className={`flex h-12 w-12 items-center justify-center rounded-2xl text-white font-bold text-sm shadow-md transition-transform ${
          status === 'running' ? 'scale-100' : status === 'stopped' ? 'scale-90 opacity-60' : 'scale-100'
        }`}
        style={{ background: bgColor }}
      >
        {initials || '?'}
      </div>

      {/* Status animations */}
      {status === 'running' && (
        <div
          className="absolute inset-0 rounded-2xl border-2 animate-ping"
          style={{ borderColor: '#4ade80', opacity: 0.4 }}
        />
      )}
      {status === 'stopped' && (
        <div className="absolute -bottom-1 left-1/2 -translate-x-1/2 text-[10px] animate-pulse">
          💤
        </div>
      )}
      {status === 'error' && (
        <div className="absolute -top-1 -right-1 flex h-5 w-5 items-center justify-center rounded-full bg-red-500 text-white shadow">
          <AlertTriangle className="h-2.5 w-2.5" />
        </div>
      )}
      {status === 'starting' && (
        <div className="absolute inset-0 flex items-center justify-center rounded-2xl bg-yellow-400/20">
          <Loader2 className="h-4 w-4 animate-spin text-yellow-500" />
        </div>
      )}
    </div>
  )
}

// ── StatusLabel ───────────────────────────────────────────────────────────────

function StatusLabel({ status }: { status: string }) {
  const { t } = useTranslation()
  const map: Record<string, { icon: React.ReactNode; labelKey: string; cls: string }> = {
    running:  { icon: <Zap className="h-2.5 w-2.5" />,           labelKey: 'dashboard.labelWorking', cls: 'text-green-600 dark:text-green-400 bg-green-50 dark:bg-green-900/30' },
    stopped:  { icon: <Moon className="h-2.5 w-2.5" />,          labelKey: 'dashboard.labelResting', cls: 'text-gray-400 dark:text-gray-500 bg-gray-100 dark:bg-gray-800' },
    error:    { icon: <AlertTriangle className="h-2.5 w-2.5" />, labelKey: 'dashboard.labelError',   cls: 'text-red-500 dark:text-red-400 bg-red-50 dark:bg-red-900/30' },
    starting: { icon: <Loader2 className="h-2.5 w-2.5 animate-spin" />, labelKey: 'dashboard.labelStarting', cls: 'text-yellow-600 dark:text-yellow-400 bg-yellow-50 dark:bg-yellow-900/30' },
  }
  const s = map[status] ?? map['stopped']
  return (
    <span className={`flex items-center gap-1 rounded-full px-2 py-0.5 text-[10px] font-medium ${s.cls}`}>
      {s.icon}{t(s.labelKey)}
    </span>
  )
}

// ── StatusDot ─────────────────────────────────────────────────────────────────

function StatusDot({ status }: { status: string }) {
  const cls: Record<string, string> = {
    running:  'bg-green-400',
    stopped:  'bg-gray-300 dark:bg-gray-600',
    error:    'bg-red-400',
    starting: 'bg-yellow-400 animate-pulse',
  }
  return <span className={`block h-2 w-2 rounded-full ${cls[status] ?? cls['stopped']}`} />
}

// ── ControlBtn ────────────────────────────────────────────────────────────────

function ControlBtn({ icon, label, onClick, danger }: {
  icon: React.ReactNode; label: string
  onClick: (e: React.MouseEvent) => void; danger?: boolean
}) {
  return (
    <button
      onClick={onClick}
      title={label}
      className={`flex flex-col items-center gap-1 rounded-xl px-3 py-2 text-white text-[10px] font-medium backdrop-blur-sm transition-all hover:scale-105 ${
        danger
          ? 'bg-red-500/80 hover:bg-red-500'
          : 'bg-blue-500/80 hover:bg-blue-500'
      }`}
    >
      {icon}
      {label}
    </button>
  )
}

// ── Helpers ───────────────────────────────────────────────────────────────────

function cardStyle(status: string, hovered: boolean): React.CSSProperties {
  const base: React.CSSProperties = {
    transition: 'all 0.2s ease',
    transform: hovered ? 'translateY(-4px) scale(1.02)' : 'none',
  }
  if (status === 'running') return { ...base, borderColor: '#4ade80', backgroundColor: hovered ? 'rgba(74,222,128,0.08)' : 'rgba(74,222,128,0.04)' }
  if (status === 'error')   return { ...base, borderColor: '#f87171', backgroundColor: hovered ? 'rgba(248,113,113,0.08)' : 'rgba(248,113,113,0.04)' }
  if (status === 'starting') return { ...base, borderColor: '#fbbf24', backgroundColor: hovered ? 'rgba(251,191,36,0.08)' : 'rgba(251,191,36,0.04)' }
  // stopped
  return { ...base, borderColor: '#e5e7eb', backgroundColor: 'white' }
}

const AVATAR_COLORS = [
  'linear-gradient(135deg,#667eea,#764ba2)',
  'linear-gradient(135deg,#f093fb,#f5576c)',
  'linear-gradient(135deg,#4facfe,#00f2fe)',
  'linear-gradient(135deg,#43e97b,#38f9d7)',
  'linear-gradient(135deg,#fa709a,#fee140)',
  'linear-gradient(135deg,#a18cd1,#fbc2eb)',
  'linear-gradient(135deg,#ffecd2,#fcb69f)',
  'linear-gradient(135deg,#a1c4fd,#c2e9fb)',
]

function avatarColor(id: string): string {
  let hash = 0
  for (let i = 0; i < id.length; i++) hash = (hash * 31 + id.charCodeAt(i)) & 0xffffffff
  return AVATAR_COLORS[Math.abs(hash) % AVATAR_COLORS.length]
}
