import { useCallback, useEffect, useMemo, useState } from 'react'
import {
  ArrowLeft, Brain, CheckCircle2, Clock3, Loader2, MessagesSquare, RefreshCw, RotateCcw, XCircle,
} from 'lucide-react'
import { useMemoryStore } from '../store/memoryStore'
import type { MemoryConversationDetail, PendingMemorySession } from '../types/memory'
import { ConversationDialog, STATE_META, displayTime, sourceLabel } from '../components/ConversationDialog'

type StateFilter = 'all' | 'pending' | 'retrying' | 'failed'

export function PendingMemories({ onBack }: { onBack: () => void }) {
  const { loadPendingSessions, loadConversationDetail, organizeSession, organizeConversations, ingestStatus, checkIngest, checkTelemetry } = useMemoryStore()
  const [sessions, setSessions] = useState<PendingMemorySession[] | null>(null)
  const [filter, setFilter] = useState<StateFilter>('all')
  const [refreshing, setRefreshing] = useState(false)
  const [organizingAll, setOrganizingAll] = useState(false)
  const [organizingKey, setOrganizingKey] = useState<string | null>(null)
  const [notice, setNotice] = useState<{ kind: 'ok' | 'err'; text: string } | null>(null)
  const [detailKey, setDetailKey] = useState<string | null>(null)
  const [detail, setDetail] = useState<MemoryConversationDetail | null>(null)
  const [detailError, setDetailError] = useState<string | null>(null)

  const reload = useCallback(async () => {
    try {
      setSessions(await loadPendingSessions())
    } catch {
      setSessions([])
    }
  }, [loadPendingSessions])

  useEffect(() => {
    void reload()
    void checkIngest()
    void checkTelemetry({ limit: 20 })
  }, [reload, checkIngest, checkTelemetry])

  const counts = useMemo(() => {
    const list = sessions ?? []
    return {
      pending: list.filter((item) => item.l1_state === 'pending').length,
      retrying: list.filter((item) => item.l1_state === 'retrying').length,
      failed: list.filter((item) => item.l1_state === 'failed').length,
    }
  }, [sessions])

  const visible = useMemo(
    () => (sessions ?? []).filter((item) => filter === 'all' || item.l1_state === filter),
    [sessions, filter],
  )

  const modelReady = Boolean(ingestStatus?.model_ready)

  async function handleRefresh() {
    if (refreshing) return
    setRefreshing(true)
    try {
      await checkTelemetry({ limit: 20 })
      await reload()
    } finally {
      setRefreshing(false)
    }
  }

  async function handleOrganizeAll() {
    if (organizingAll) return
    setOrganizingAll(true)
    setNotice(null)
    try {
      const result = await organizeConversations()
      setNotice(
        result.failed > 0
          ? { kind: 'err', text: `已整理 ${result.succeeded} 个会话，${result.failed} 个失败` }
          : { kind: 'ok', text: `已整理 ${result.succeeded} 个会话` },
      )
      await reload()
    } catch (error) {
      setNotice({ kind: 'err', text: `整理失败：${String(error)}` })
    } finally {
      setOrganizingAll(false)
    }
  }

  async function handleOrganizeOne(eventKey: string) {
    if (organizingKey) return
    setOrganizingKey(eventKey)
    setNotice(null)
    try {
      const result = await organizeSession(eventKey)
      setNotice(
        result.failed > 0
          ? { kind: 'err', text: result.failure_reasons[0] ?? '该会话整理失败' }
          : { kind: 'ok', text: '已提取该会话的记忆' },
      )
      await reload()
    } catch (error) {
      setNotice({ kind: 'err', text: `整理失败：${String(error)}` })
    } finally {
      setOrganizingKey(null)
    }
  }

  async function handleOpenDetail(eventKey: string) {
    setDetailKey(eventKey)
    setDetail(null)
    setDetailError(null)
    try {
      setDetail(await loadConversationDetail(eventKey))
    } catch (error) {
      setDetailError(String(error))
    }
  }

  function handleCloseDetail() {
    setDetailKey(null)
    setDetail(null)
    setDetailError(null)
  }

  return <main className="h-full overflow-y-auto bg-[#fbfcfe] text-slate-900 dark:bg-slate-950 dark:text-slate-100">
    <div className="mx-auto max-w-[1200px] px-5 py-6 lg:px-9 lg:py-8">
      <header className="flex flex-wrap items-start justify-between gap-4">
        <div className="flex items-start gap-3">
          <button type="button" onClick={onBack} className="mt-0.5 inline-flex h-9 w-9 items-center justify-center rounded-lg text-slate-500 transition hover:bg-slate-100 hover:text-slate-900 focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-500 dark:text-slate-400 dark:hover:bg-slate-800 dark:hover:text-white" aria-label="返回记忆中心"><ArrowLeft size={19} /></button>
          <div>
            <h1 className="text-2xl font-bold tracking-[-0.025em]">待提取记忆</h1>
            <p className="mt-1 text-sm text-slate-500 dark:text-slate-400">已完成但尚未提炼为记忆的会话；只有点击整理才会调用记忆模型。</p>
          </div>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          {notice && <span className={`rounded-lg px-3 py-2 text-xs ${notice.kind === 'ok' ? 'bg-emerald-500/10 text-emerald-700 dark:text-emerald-300' : 'bg-red-500/10 text-red-700 dark:text-red-300'}`} role="status">{notice.text}</span>}
          <button type="button" onClick={() => { void handleRefresh() }} disabled={refreshing} className="inline-flex h-10 items-center gap-2 rounded-lg border border-slate-200 bg-white px-3 text-sm font-medium text-slate-700 transition hover:border-slate-300 hover:bg-slate-50 disabled:cursor-wait disabled:opacity-60 focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-500 dark:border-slate-700 dark:bg-slate-900 dark:text-slate-200 dark:hover:bg-slate-800"><RefreshCw size={16} className={refreshing ? 'animate-spin motion-reduce:animate-none' : ''} />刷新</button>
          <button type="button" onClick={() => { void handleOrganizeAll() }} disabled={organizingAll || !modelReady || (sessions ?? []).length === 0} title={modelReady ? '整理全部待提取会话' : '记忆模型未配置，无法整理'} className="inline-flex h-10 items-center gap-2 rounded-lg bg-violet-600 px-4 text-sm font-medium text-white transition hover:bg-violet-500 disabled:cursor-not-allowed disabled:opacity-50 focus:outline-none focus-visible:ring-2 focus-visible:ring-violet-500">{organizingAll ? <Loader2 size={16} className="animate-spin motion-reduce:animate-none" /> : <Brain size={16} />}{organizingAll ? '正在整理' : '整理全部'}</button>
        </div>
      </header>

      <section className="mt-7 grid gap-3 sm:grid-cols-3" aria-label="待提取概览">
        {(['pending', 'retrying', 'failed'] as const).map((state) => {
          const meta = STATE_META[state]
          const active = filter === state
          return <button key={state} type="button" onClick={() => setFilter(active ? 'all' : state)} aria-pressed={active} className={`rounded-xl border px-4 py-3 text-left transition focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-500 ${active ? 'border-violet-400 bg-violet-50/60 dark:border-violet-500/60 dark:bg-violet-500/10' : 'border-slate-200 bg-white hover:border-slate-300 dark:border-slate-800 dark:bg-slate-900 dark:hover:border-slate-700'}`}>
            <div className="flex items-center gap-2 text-sm text-slate-500 dark:text-slate-400">
              {state === 'pending' ? <Clock3 size={15} className="text-violet-500" /> : state === 'retrying' ? <RotateCcw size={15} className="text-amber-500" /> : <XCircle size={15} className="text-red-500" />}
              {meta.label}
            </div>
            <p className="mt-2 font-mono text-2xl font-semibold tabular-nums">{sessions === null ? '—' : counts[state]}</p>
          </button>
        })}
      </section>

      {!modelReady && ingestStatus && (
        <p className="mt-4 rounded-lg bg-amber-500/10 px-3 py-2.5 text-xs text-amber-800 dark:text-amber-200">记忆模型未配置或不可用：会话会继续保留在这里，配置模型后即可整理。</p>
      )}

      <section className="mt-6 rounded-2xl border border-slate-200 bg-white p-5 shadow-[0_1px_2px_rgba(15,23,42,0.03)] dark:border-slate-800 dark:bg-slate-900 sm:p-6" aria-label="待提取会话列表">
        {sessions === null ? (
          <div className="flex min-h-48 items-center justify-center gap-2 text-sm text-slate-400"><Loader2 size={16} className="animate-spin motion-reduce:animate-none" />正在读取本地账本…</div>
        ) : visible.length === 0 ? (
          <div className="flex min-h-48 flex-col items-center justify-center text-center">
            <span className="grid h-11 w-11 place-items-center rounded-xl bg-emerald-500/10 text-emerald-500"><CheckCircle2 size={20} /></span>
            <h2 className="mt-3 font-medium">{filter === 'all' ? '没有待提取的会话' : `没有「${STATE_META[filter].label}」状态的会话`}</h2>
            <p className="mt-1 max-w-md text-sm text-slate-500 dark:text-slate-400">Agent 完成对话并被本地账本捕获后，会出现在这里等待整理。</p>
          </div>
        ) : (
          <div className="divide-y divide-slate-100 dark:divide-slate-800">
            {visible.map((item) => {
              const meta = STATE_META[item.l1_state] ?? STATE_META.pending
              const working = organizingKey === item.event_key
              return <div key={item.event_key} role="button" tabIndex={0} title="点击查看完整对话"
                onClick={() => { void handleOpenDetail(item.event_key) }}
                onKeyDown={(event) => { if (event.key === 'Enter' || event.key === ' ') { event.preventDefault(); void handleOpenDetail(item.event_key) } }}
                className="group -mx-2 flex cursor-pointer flex-wrap items-center gap-x-3 gap-y-2 rounded-lg px-2 py-3.5 transition hover:bg-slate-50 focus:outline-none focus-visible:ring-2 focus-visible:ring-violet-500 dark:hover:bg-slate-800/50">
                <span className={`inline-flex shrink-0 items-center rounded px-1.5 py-0.5 text-xs ${meta.badge}`}>{meta.label}</span>
                <span className="shrink-0 text-sm font-medium">{sourceLabel(item.source)}</span>
                <span className="min-w-0 flex-1 truncate font-mono text-xs text-slate-400" title={item.session_id}>{item.session_id}</span>
                <span className="shrink-0 font-mono text-xs text-slate-400">{displayTime(item.occurred_at)}</span>
                <span className="shrink-0 text-xs text-slate-400">{item.message_count} 条消息</span>
                <span className="inline-flex shrink-0 items-center gap-1 text-xs text-slate-300 transition group-hover:text-violet-500 dark:text-slate-600 dark:group-hover:text-violet-300"><MessagesSquare size={13} />查看对话</span>
                <button type="button" onClick={(event) => { event.stopPropagation(); void handleOrganizeOne(item.event_key) }} disabled={working || organizingAll || !modelReady} title={modelReady ? '仅整理该会话' : '记忆模型未配置，无法整理'} className="inline-flex shrink-0 items-center gap-1 rounded-md border border-violet-200 bg-white px-2.5 py-1 text-xs font-medium text-violet-700 transition hover:bg-violet-50 disabled:cursor-not-allowed disabled:opacity-50 dark:border-violet-500/30 dark:bg-slate-900 dark:text-violet-300 dark:hover:bg-violet-500/10">{working ? <Loader2 size={12} className="animate-spin motion-reduce:animate-none" /> : <Brain size={12} />}整理</button>
                <p className="basis-full truncate text-xs text-slate-500 dark:text-slate-400" title={item.excerpt}>{item.excerpt.replace(/\s+/g, ' ').trim() || '（无对话摘要）'}</p>
                {item.error && <p className="basis-full truncate text-xs text-red-600 dark:text-red-400" title={item.error}>上次失败：{item.error}</p>}
              </div>
            })}
          </div>
        )}
      </section>

      {detailKey && <ConversationDialog
        detail={detail}
        error={detailError}
        onClose={handleCloseDetail}
      />}
    </div>
  </main>
}

