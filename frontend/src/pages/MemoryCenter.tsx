import { useEffect, useLayoutEffect, useMemo, useRef, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { listen } from '@tauri-apps/api/event'
import { open } from '@tauri-apps/plugin-dialog'
import {
  Brain, Search, Trash2, Pencil, MoonStar, CheckCircle2, XCircle, Loader2, X,
  PowerOff, Webhook, History, RefreshCw, ChevronDown, ChevronUp, FileText, Star, PlugZap, Database, FolderOpen,
  ChevronRight, Plus,
} from 'lucide-react'
import { useMemoryStore } from '../store/memoryStore'
import type { ConsolidationResult, MemoryImportResult, MemoryItem, MemoryLayerDocument } from '../types/memory'
import { normalizeL2Document } from '../lib/thinking'
import { MemoryMarkdown } from '../components/MemoryMarkdown'
import { sourceLabel } from '../components/ConversationDialog'

function localTime(value: string) {
  // Rust serializes UTC timestamps with nanosecond precision. Normalise to
  // JavaScript's millisecond precision before parsing so WebView variants do
  // not fall back to exposing the raw RFC3339 value.
  const normalized = value.trim().replace(/(\.\d{3})\d+(?=(Z|[+-]\d{2}:\d{2})$)/, '$1')
  const date = new Date(normalized)
  return Number.isNaN(date.getTime()) ? value : new Intl.DateTimeFormat('zh-CN', {
    year: 'numeric', month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', second: '2-digit', hour12: false,
  }).format(date)
}

function looksLikeModelAnalysis(content: string) {
  const opening = content.trim().slice(0, 180).toLowerCase()
  return /^(the user wants|let me (analyze|review|think)|i (need|will|should) |analysis:)/.test(opening)
}

function isInvalidL3Draft(content: string) {
  const normalized = content.trim().toLowerCase()
  return looksLikeModelAnalysis(content)
    || normalized.includes('pending human approval')
    || normalized.includes('[内容已截断]')
    || normalized.includes('[truncated]')
}

function LayerWindow({ document }: { document: MemoryLayerDocument }) {
  if (!document.window_start || !document.window_end) return null
  return <span className="text-gray-400">整理范围 · {localTime(document.window_start)} 至 {localTime(document.window_end)}</span>
}

/** 文档内容限高：不超过限高时完整展示、不渲染按钮；超出时折叠到限高并显示
    「查看完整内容」，由用户手动展开。与 Tailwind `max-h-80` 保持同源。 */
const DOCUMENT_COLLAPSED_MAX_HEIGHT_PX = 320

function CollapsibleDocumentContent({ content, contentId, accent }: { content: string; contentId: string; accent: 'sky' | 'violet' }) {
  const ref = useRef<HTMLDivElement | null>(null)
  const [open, setOpen] = useState(false)
  const [overflows, setOverflows] = useState(false)
  useLayoutEffect(() => {
    const el = ref.current
    if (!el) return
    // scrollHeight 是内容自然高度（不受 max-h 裁切影响）；展开/收起两种
    // 状态下判定结果一致，按钮只随内容本身变化。
    const measure = () => setOverflows(el.scrollHeight > DOCUMENT_COLLAPSED_MAX_HEIGHT_PX + 2)
    measure()
    const observer = new ResizeObserver(measure)
    observer.observe(el)
    return () => observer.disconnect()
  }, [content])
  const buttonClass = accent === 'sky'
    ? 'border-sky-300 text-sky-700 hover:bg-sky-100 dark:border-sky-800 dark:text-sky-200 dark:hover:bg-sky-950/50'
    : 'border-violet-300 text-violet-700 hover:bg-violet-100 dark:border-violet-700 dark:text-violet-200 dark:hover:bg-violet-950/50'
  return (<>
    <div ref={ref} id={contentId} className={`mt-2 min-w-0 max-w-full overscroll-contain pr-1 ${open ? '' : 'max-h-80 overflow-hidden'}`}>
      <MemoryMarkdown content={content} unwrapDocumentFence />
    </div>
    {overflows && (
      <button type="button" onClick={() => setOpen((value) => !value)} aria-expanded={open} aria-controls={contentId} className={`mt-2 inline-flex items-center gap-1 rounded-md border px-2.5 py-1 text-xs font-medium transition ${buttonClass}`}>
        {open ? '收起完整内容' : '查看完整内容'}{open ? <ChevronUp size={14} /> : <ChevronDown size={14} />}
      </button>
    )}
  </>)
}

export function MemoryCenter({ onOpenUsage, onOpenPending, onOpenOrganized, onOpenInjection }: { onOpenUsage?: () => void; onOpenPending?: () => void; onOpenOrganized?: () => void; onOpenInjection?: () => void }) {
  const { t } = useTranslation()
  const {
    engineOnline, memories, importance, loading, lastSearchResults, lastSearchQuery, memoryCacheReady, localMemoryStats,
    memoryMcp, ingestStatus, telemetrySummary, telemetryLiveStatus, telemetryEvents, l2Documents, l3Documents, checkIngest, checkTelemetry, checkMemoryMcp, loadMemoryLayers, consolidateShortTermMemory, draftLongTermProfile, publishLongTermProfile, updatePublishedMemoryDocument, deleteLongTermProfileDraft, setIngestEnabled, organizeConversations, importMemoryFolder,
    checkEngine, stopEngine, search, listMemories, resetL1ForReextraction, updateMemory, deleteMemory, addUserMemory, dreaming, restoreConsolidation, refreshImportance, setMemoryPinned,
  } = useMemoryStore()

  const [query, setQuery] = useState('')
  const [topK, setTopK] = useState(10)
  const [notice, setNotice] = useState<{ kind: 'ok' | 'err'; text: string } | null>(null)
  const [editing, setEditing] = useState<MemoryItem | null>(null)
  const [editContent, setEditContent] = useState('')
  const [deleteTarget, setDeleteTarget] = useState<MemoryItem | null>(null)
  const [expandedReceiptId, setExpandedReceiptId] = useState<number | null>(null)
  const [refreshing, setRefreshing] = useState(false)
  const [organizingConversations, setOrganizingConversations] = useState(false)
  const [importingMemories, setImportingMemories] = useState(false)
  const [memoryImportResult, setMemoryImportResult] = useState<MemoryImportResult | null>(null)
  const [consolidating, setConsolidating] = useState(false)
  const [consolidationResult, setConsolidationResult] = useState<{ kind: 'ok' | 'err'; text: string; result?: ConsolidationResult } | null>(null)
  const [restoringConsolidation, setRestoringConsolidation] = useState(false)
  const [refreshingImportance, setRefreshingImportance] = useState(false)
  const [ingestOpen, setIngestOpen] = useState(false)
  const [recordsOpen, setRecordsOpen] = useState(false)
  const [consolidationOpen, setConsolidationOpen] = useState(false)
  const [consolidationProgress, setConsolidationProgress] = useState<string | null>(null)
  const [consolidatingL2, setConsolidatingL2] = useState(false)
  const [draftingL3, setDraftingL3] = useState(false)
  const [l3DraftError, setL3DraftError] = useState<string | null>(null)
  const [publishingL3, setPublishingL3] = useState<string | null>(null)
  const [deletingL3, setDeletingL3] = useState<string | null>(null)
  const [l3DeleteTarget, setL3DeleteTarget] = useState<MemoryLayerDocument | null>(null)
  const [resettingL1, setResettingL1] = useState(false)
  const [resetL1ConfirmOpen, setResetL1ConfirmOpen] = useState(false)
  // 用户自定义添加：折叠表单，提交后写入 user_defined 长期记忆。
  const [userMemoryOpen, setUserMemoryOpen] = useState(false)
  const [userMemoryContent, setUserMemoryContent] = useState('')
  const [userMemoryType, setUserMemoryType] = useState('fact')
  const [addingUserMemory, setAddingUserMemory] = useState(false)
  // 草案编辑：只存在组件状态里，不单独持久化；发布时随发布落盘，
  // 离开页面即丢弃。
  const [editingL3Id, setEditingL3Id] = useState<string | null>(null)
  const [editingL3Content, setEditingL3Content] = useState('')
  // L2 工作记忆编辑：L2 生成即发布，保存即覆盖当前发布版。
  const [editingL2, setEditingL2] = useState(false)
  const [editingL2Content, setEditingL2Content] = useState('')
  const [savingL2, setSavingL2] = useState(false)
  // L2 卡片的自定义添加展开态（与 L3 卡片共用同一份表单内容与提交状态）。
  const [l2UserMemoryOpen, setL2UserMemoryOpen] = useState(false)
  // The page is unmounted when navigating away.  Keep the loading decision in
  // the shared store so revisiting it in this app session paints immediately.
  const [booting, setBooting] = useState(() => !memoryCacheReady)
  const visibleL3Documents = l3Documents.filter((document) => !isInvalidL3Draft(document.content))
  const invalidL3Drafts = l3Documents.filter((document) => document.state === 'draft' && isInvalidL3Draft(document.content))

  useEffect(() => {
    let mounted = true
    // Render a bounded loading state while the local ledger opens. Semantic
    // and MCP checks remain deferred so the first paint never waits on CLIs.
    void Promise.allSettled([listMemories(), checkIngest(), checkTelemetry(), loadMemoryLayers()]).finally(() => {
      if (mounted) setBooting(false)
    })
    void checkEngine().then((online) => { if (online) void listMemories(true) })
    // Token reconciliation runs once as an application-start background job.
    // This page only reads the durable SQLite result so navigation is instant.
    // MCP 连接状态只用于顶部「记忆注入」卡片计数；连接管理在独立面板完成。
    void checkMemoryMcp()
    return () => { mounted = false }
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [])

  useEffect(() => {
    let unlisten: (() => void) | undefined
    void listen<{ detail?: string }>('memory-consolidation-progress', (event) => {
      setConsolidationProgress(event.payload.detail ?? '正在裁决语义候选')
    }).then((dispose) => { unlisten = dispose }).catch(() => {})
    return () => unlisten?.()
  }, [])

  useEffect(() => {
    const timer = window.setInterval(() => {
      void checkIngest()
      void checkTelemetry()
    }, 10_000)
    return () => window.clearInterval(timer)
  }, [checkIngest, checkTelemetry])

  function flash(kind: 'ok' | 'err', text: string) {
    setNotice({ kind, text })
    setTimeout(() => setNotice(null), 3500)
  }

  async function handleStop() {
    await stopEngine()
    flash('ok', t('memory.engineStopped'))
  }

  if (booting) {
    return <div className="flex h-full min-h-0 items-center justify-center bg-gray-50 px-6 dark:bg-gray-950" role="status" aria-live="polite">
      <div className="w-full max-w-md rounded-2xl border border-gray-200 bg-white p-6 shadow-sm dark:border-gray-800 dark:bg-gray-900">
        <div className="flex items-center gap-3 text-gray-800 dark:text-gray-100"><span className="rounded-xl bg-violet-500/10 p-2.5 text-violet-600 dark:text-violet-400"><Loader2 className="animate-spin motion-reduce:animate-none" size={21} /></span><div><p className="text-sm font-semibold">正在打开记忆中心</p><p className="mt-0.5 text-xs text-gray-500 dark:text-gray-400">读取本地记忆账本与会话状态…</p></div></div>
        <div className="mt-5 space-y-2.5" aria-hidden="true"><div className="h-3 w-4/5 animate-pulse rounded bg-gray-100 dark:bg-gray-800" /><div className="h-3 w-full animate-pulse rounded bg-gray-100 dark:bg-gray-800" /><div className="h-3 w-3/5 animate-pulse rounded bg-gray-100 dark:bg-gray-800" /></div>
      </div>
    </div>
  }

  async function handleSearch() {
    if (!query.trim()) return
    try {
      await search(query.trim(), topK)
    } catch (e) {
      flash('err', `${t('common.failed')}: ${String(e)}`)
    }
  }

  async function handleRefresh() {
    if (refreshing) return
    setRefreshing(true)
    const results = await Promise.allSettled([
      checkEngine(),
      listMemories(true),
      checkIngest(),
      checkTelemetry({ backfill: true, refreshUsage: true, limit: 50 }),
    ])
    setRefreshing(false)
    const failure = results.find((result) => result.status === 'rejected')
    flash(failure ? 'err' : 'ok', failure ? t('memory.refreshFailed') : t('memory.refreshDone'))
  }

  async function handleL2Consolidation() {
    if (consolidatingL2) return
    setConsolidatingL2(true)
    try {
      const result = await consolidateShortTermMemory()
      flash('ok', result.message)
    } catch (error) { flash('err', `${t('common.failed')}: ${String(error)}`) } finally { setConsolidatingL2(false) }
  }

  // L2 编辑保存：直接覆盖当前发布版，下一次注入与 L3 草案立刻生效。
  async function handleL2Save(document: MemoryLayerDocument) {
    if (savingL2 || !editingL2Content.trim()) return
    setSavingL2(true)
    try {
      await updatePublishedMemoryDocument(document.id, editingL2Content)
      setEditingL2(false)
      setEditingL2Content('')
      flash('ok', '工作记忆已更新，下次注入与 L3 草案将使用新内容')
    } catch (error) { flash('err', `${t('common.failed')}: ${String(error)}`) } finally { setSavingL2(false) }
  }

  // 已发布 L3 Profile 的手动编辑：保存即覆盖当前发布版，立刻用于注入，
  // 并作为下一次「创建草案」的权威基线输入。
  async function handleL3SavePublished(document: MemoryLayerDocument) {
    if (publishingL3 || !editingL3Content.trim()) return
    if (isInvalidL3Draft(editingL3Content)) {
      flash('err', '编辑后的内容不符合 L3 发布格式，无法保存；请调整后再试。')
      return
    }
    setPublishingL3(document.id)
    try {
      await updatePublishedMemoryDocument(document.id, editingL3Content)
      setEditingL3Id(null)
      setEditingL3Content('')
      flash('ok', '长期 Profile 已更新，新的 Agent 初始化会使用该内容')
    } catch (error) { flash('err', `${t('common.failed')}: ${String(error)}`) } finally { setPublishingL3(null) }
  }

  async function handleL3Draft() {
    if (draftingL3) return
    setL3DraftError(null)
    setDraftingL3(true)
    try {
      const result = await draftLongTermProfile()
      flash('ok', result.message)
    } catch (error) {
      const message = String(error)
      setL3DraftError(message)
      flash('err', `${t('common.failed')}: ${message}`)
    } finally { setDraftingL3(false) }
  }

  async function handleL3Publish(document: MemoryLayerDocument) {
    const edited = editingL3Id === document.id ? editingL3Content : null
    const effectiveContent = edited ?? document.content
    if (isInvalidL3Draft(effectiveContent)) {
      flash('err', '该历史草案不符合发布标准，无法发布；请重新创建。')
      return
    }
    if (publishingL3) return
    setPublishingL3(document.id)
    try {
      // 编辑不单独保存：仅在发布这一刻把编辑后的正文交给后端覆盖发布。
      await publishLongTermProfile(document.id, edited?.trim() || undefined)
      setEditingL3Id(null)
      setEditingL3Content('')
      flash('ok', '长期 Profile 已发布；新的 Agent 初始化会使用该版本')
    } catch (error) { flash('err', `${t('common.failed')}: ${String(error)}`) } finally { setPublishingL3(null) }
  }

  async function confirmL3DraftDelete() {
    if (!l3DeleteTarget || deletingL3) return
    const document = l3DeleteTarget
    setDeletingL3(document.id)
    try {
      await deleteLongTermProfileDraft(document.id)
      setL3DeleteTarget(null)
      flash('ok', '草案已删除')
    } catch (error) { flash('err', `${t('common.failed')}: ${String(error)}`) } finally { setDeletingL3(null) }
  }

  function requestResetL1() {
    if (!resettingL1) setResetL1ConfirmOpen(true)
  }

  async function confirmResetL1() {
    if (resettingL1) return
    setResettingL1(true)
    try {
      const result = await resetL1ForReextraction()
      setResetL1ConfirmOpen(false)
      flash('ok', `已清空 ${result.cleared_l1} 条会话记忆和记忆派生文档 ${result.cleared_derived_documents} 份；${result.requeued_conversations} 个完整会话已待重新整理`)
    } catch (error) { flash('err', `${t('common.failed')}: ${String(error)}`) } finally { setResettingL1(false) }
  }

  async function handleOrganizeConversations() {
    setOrganizingConversations(true)
    try {
      const result = await organizeConversations()
      flash(result.failed ? 'err' : 'ok', `整理完成：成功 ${result.succeeded}，失败 ${result.failed}${result.failure_reasons[0] ? `；${result.failure_reasons[0]}` : ''}`)
    } catch (error) { flash('err', `${t('common.failed')}: ${String(error)}`) } finally { setOrganizingConversations(false) }
  }

  async function handleImportMemoryFolder() {
    if (importingMemories) return
    const selected = await open({ directory: true, multiple: false, title: '选择包含 Agent 记忆或会话导出的文件夹' })
    if (!selected || Array.isArray(selected)) return
    setImportingMemories(true)
    setMemoryImportResult(null)
    try {
      const result = await importMemoryFolder(selected)
      setMemoryImportResult(result)
      flash('ok', result.message)
    } catch (error) {
      flash('err', `${t('common.failed')}: ${String(error)}`)
    } finally { setImportingMemories(false) }
  }

  function openEdit(item: MemoryItem) {
    setEditing(item)
    setEditContent(item.memory)
  }

  async function handleSaveEdit() {
    if (!editing) return
    try {
      await updateMemory(editing.id, editContent)
      flash('ok', t('memory.updated'))
      setEditing(null)
    } catch (e) {
      flash('err', `${t('common.failed')}: ${String(e)}`)
    }
  }

  async function handleDelete(item: MemoryItem) {
    setDeleteTarget(item)
  }

  const userMemoryTypeOptions = [
    { value: 'fact', label: t('memory.userDefinedTypeFact') },
    { value: 'decision', label: t('memory.userDefinedTypeDecision') },
    { value: 'constraint', label: t('memory.userDefinedTypeConstraint') },
    { value: 'preference_candidate', label: t('memory.userDefinedTypePreference') },
    { value: 'open_item', label: t('memory.userDefinedTypeOpenItem') },
  ]

  async function handleAddUserMemory(scope: 'l2' | 'l3') {
    const content = userMemoryContent.trim()
    if (!content || addingUserMemory) return
    setAddingUserMemory(true)
    try {
      await addUserMemory(content, userMemoryType, scope)
      setUserMemoryContent('')
      setUserMemoryOpen(false)
      flash('ok', t('memory.userDefinedAdded'))
    } catch (e) {
      flash('err', `${t('common.failed')}: ${String(e)}`)
    } finally { setAddingUserMemory(false) }
  }

  async function confirmDelete() {
    if (!deleteTarget) return
    try {
      await deleteMemory(deleteTarget.id)
      flash('ok', t('memory.deleted'))
    } catch (e) {
      flash('err', `${t('common.failed')}: ${String(e)}`)
    } finally {
      setDeleteTarget(null)
    }
  }

  async function handleDreaming() {
    if (consolidating) return
    setConsolidating(true)
    setConsolidationResult(null)
    setConsolidationProgress('正在准备本地语义候选')
    try {
      const result = await dreaming((processed, total) => {
        setConsolidationProgress(`正在本地生成语义候选 ${processed}/${total}`)
      })
      await listMemories()
      const text = result.message || t('memory.dreamDone')
      setConsolidationResult({ kind: 'ok', text, result })
      flash('ok', t('memory.dreamDone'))
    } catch (e) {
      const text = `${t('common.failed')}: ${String(e)}`
      setConsolidationResult({ kind: 'err', text })
      flash('err', text)
    } finally {
      setConsolidating(false)
      setConsolidationProgress(null)
    }
  }

  async function handleRefreshImportance() {
    if (refreshingImportance) return
    setRefreshingImportance(true)
    try {
      const result = await refreshImportance()
      flash('ok', result.message)
    } catch (error) {
      flash('err', `${t('common.failed')}: ${String(error)}`)
    } finally {
      setRefreshingImportance(false)
    }
  }

  async function handlePinMemory(memoryId: string, pinned: boolean) {
    try {
      await setMemoryPinned(memoryId, pinned)
      await handleRefreshImportance()
    } catch (error) {
      flash('err', `${t('common.failed')}: ${String(error)}`)
    }
  }

  async function handleRestoreConsolidation() {
    const snapshotId = consolidationResult?.result?.snapshot_id
    if (!snapshotId || restoringConsolidation) return
    setRestoringConsolidation(true)
    try {
      const result = await restoreConsolidation(snapshotId)
      await listMemories()
      setConsolidationResult({ kind: 'ok', text: t('memory.dreamRestoreDone'), result })
      flash('ok', t('memory.dreamRestoreDone'))
    } catch (error) {
      const text = `${t('common.failed')}: ${String(error)}`
      setConsolidationResult({ kind: 'err', text })
      flash('err', text)
    } finally {
      setRestoringConsolidation(false)
    }
  }

  // Native L1 extraction stores conversation summaries as `conversation`.
  // They are factual, searchable records—not user preferences.  Only the
  // profile-oriented kinds belong to the preference total.
  const isPreference = (memory: MemoryItem) => ['preference', 'profile', 'constraint'].includes((memory.memory_type ?? '').toLowerCase())
  const factCount = memories.filter((memory) => !isPreference(memory)).length
  const prefCount = memories.filter(isPreference).length
  const engineState = engineOnline === null ? 'unknown' : engineOnline ? 'online' : 'offline'
  const mcpStatusList = Object.values(memoryMcp)
  const mcpStatusLoaded = mcpStatusList.some(Boolean)
  const connectedMcpCount = mcpStatusList.filter((status) => status?.installed).length

  // 实时运行状态：优先展示活跃会话所属的 Agent / 来源；无法识别时回退为纯会话数。
  const activeSourceNames = (telemetryLiveStatus?.active_sources ?? []).map(sourceLabel)
  const liveRunText = telemetryLiveStatus?.active_sessions
    ? activeSourceNames.length === 0
      ? `正在处理 ${telemetryLiveStatus.active_sessions} 个会话`
      : telemetryLiveStatus.active_sessions === 1 && activeSourceNames.length === 1
        ? `正在处理 ${activeSourceNames[0]} 的对话`
        : `正在处理 ${telemetryLiveStatus.active_sessions} 个会话：${activeSourceNames.join('、')}`
    : `监听就绪 · 已捕获 ${telemetryLiveStatus?.captured_sessions ?? 0} 个会话`

  return (
    <div className="flex flex-col gap-4 overflow-y-auto p-4">
      {/* 本地记忆与智能检索始终可用；旧语义侧车仅作为可选高级能力。 */}
      <div className="flex flex-wrap items-center gap-3">
        <div className="p-2 rounded-lg bg-violet-500/10 text-violet-600 dark:text-violet-400">
          <Brain size={20} />
        </div>
        <div>
          <h1 className="text-lg font-semibold text-gray-800 dark:text-gray-100">{t('memory.title')}</h1>
          <p className="text-xs text-gray-500 dark:text-gray-400">{t('memory.subtitle')}</p>
        </div>
        <div className="ml-auto flex flex-wrap items-center justify-end gap-1.5">
          {notice && (
            <span className={`text-xs px-2 py-1 rounded ${notice.kind === 'ok' ? 'bg-green-500/10 text-green-600 dark:text-green-400' : 'bg-red-500/10 text-red-600 dark:text-red-400'}`}>
              {notice.text}
            </span>
          )}
          <span className={`inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-xs ${telemetryLiveStatus?.active_sessions ? 'bg-sky-500/10 text-sky-700 dark:text-sky-300' : 'bg-gray-100 text-gray-600 dark:bg-gray-800 dark:text-gray-300'}`} title="实时运行状态"><span className={`h-1.5 w-1.5 rounded-full ${telemetryLiveStatus?.active_sessions ? 'animate-pulse bg-sky-500 motion-reduce:animate-none' : 'bg-gray-400'}`} />实时运行：{liveRunText}</span>
          <span className="inline-flex items-center gap-1.5 rounded-full border border-emerald-500/35 px-2.5 py-1 text-xs text-emerald-700 dark:text-emerald-300" title="记忆库状态"><Database size={13} />记忆库：本地可用</span>
          <span className={`inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-xs ${telemetryLiveStatus?.pending_memory_sessions ? 'bg-violet-500/10 text-violet-700 dark:text-violet-300' : telemetryLiveStatus?.retrying_memory_sessions || telemetryLiveStatus?.failed_memory_sessions || telemetryLiveStatus?.failed_transcript_scans ? 'bg-amber-500/10 text-amber-700 dark:text-amber-300' : 'bg-gray-100 text-gray-600 dark:text-gray-300'}`} title="记忆同步状态：仅在你点击整理会话后才会调用记忆模型"><Brain size={13} />记忆同步：{telemetryLiveStatus?.pending_memory_sessions ? `待整理 ${telemetryLiveStatus.pending_memory_sessions}` : telemetryLiveStatus?.failed_memory_sessions ? `失败 ${telemetryLiveStatus.failed_memory_sessions}` : telemetryLiveStatus?.retrying_memory_sessions ? `待重试 ${telemetryLiveStatus.retrying_memory_sessions}` : telemetryLiveStatus?.failed_transcript_scans ? `文件失败 ${telemetryLiveStatus.failed_transcript_scans}` : '已同步'}</span>
          <span className={`inline-flex items-center gap-1.5 rounded-full px-2.5 py-1 text-xs ${connectedMcpCount > 0 ? 'bg-violet-500/10 text-violet-700 dark:text-violet-300' : 'bg-gray-100 text-gray-600 dark:bg-gray-800 dark:text-gray-300'}`} title="记忆注入状态"><PlugZap size={13} />记忆注入：{mcpStatusLoaded ? (connectedMcpCount ? `已连接 ${connectedMcpCount} 个 Agent` : '未连接 Agent') : '检测中'}</span>
          {engineState === 'online' && (
              <button
                onClick={handleStop}
                className="inline-flex items-center gap-1 px-2.5 py-1 rounded-md border border-gray-300 dark:border-gray-600 text-xs text-gray-600 dark:text-gray-300 hover:bg-gray-50 dark:hover:bg-gray-700"
              >
                <PowerOff size={12} />{t('memory.engineStop')}
              </button>
            )}
        </div>
      </div>

      {telemetrySummary && (
        <section className="grid grid-cols-2 sm:grid-cols-4 gap-2">
          <button type="button" onClick={onOpenUsage} className="group rounded-lg border border-gray-200 bg-white px-3 py-2 text-left transition hover:border-blue-300 hover:bg-blue-50/40 focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-500 dark:border-gray-700 dark:bg-gray-800 dark:hover:border-blue-500/60 dark:hover:bg-blue-500/10">
            <div className="flex items-center justify-between gap-2"><p className="text-xs text-gray-400">Token 用量</p><ChevronRight size={14} className="text-gray-300 transition group-hover:translate-x-0.5 group-hover:text-blue-500 dark:text-gray-600 dark:group-hover:text-blue-300" /></div><p className="text-sm font-semibold text-gray-800 dark:text-gray-100">{telemetrySummary.total_tokens.toLocaleString()}</p><p className="mt-0.5 text-[11px] text-gray-400">输入 {telemetrySummary.input_tokens.toLocaleString()} · 输出 {telemetrySummary.output_tokens.toLocaleString()}</p>
          </button>
          <button type="button" onClick={onOpenOrganized} className="group rounded-lg border border-gray-200 bg-white px-3 py-2 text-left transition hover:border-emerald-300 hover:bg-emerald-50/40 focus:outline-none focus-visible:ring-2 focus-visible:ring-emerald-500 dark:border-gray-700 dark:bg-gray-800 dark:hover:border-emerald-500/60 dark:hover:bg-emerald-500/10">
            <div className="flex items-center justify-between gap-2"><p className="text-xs text-gray-400">已整理对话</p><ChevronRight size={14} className="text-gray-300 transition group-hover:translate-x-0.5 group-hover:text-emerald-500 dark:text-gray-600 dark:group-hover:text-emerald-300" /></div><p className="text-sm font-semibold text-gray-800 dark:text-gray-100">{telemetryLiveStatus?.organized_memory_conversations ?? 0}</p><p className="mt-0.5 text-[11px] text-gray-400">完整对话 {telemetryLiveStatus?.completed_conversations ?? 0}</p>
          </button>
          <button type="button" onClick={onOpenPending} className="group rounded-lg border border-gray-200 bg-white px-3 py-2 text-left transition hover:border-violet-300 hover:bg-violet-50/40 focus:outline-none focus-visible:ring-2 focus-visible:ring-violet-500 dark:border-gray-700 dark:bg-gray-800 dark:hover:border-violet-500/60 dark:hover:bg-violet-500/10">
            <div className="flex items-center justify-between gap-2"><p className="text-xs text-gray-400">待提取记忆</p><ChevronRight size={14} className="text-gray-300 transition group-hover:translate-x-0.5 group-hover:text-violet-500 dark:text-gray-600 dark:group-hover:text-violet-300" /></div><p className="text-sm font-semibold text-gray-800 dark:text-gray-100">{(telemetryLiveStatus?.pending_memory_sessions ?? 0) + (telemetryLiveStatus?.retrying_memory_sessions ?? 0)}</p><p className="mt-0.5 text-[11px] text-gray-400">{telemetryLiveStatus?.retrying_memory_sessions ? `待重试 ${telemetryLiveStatus.retrying_memory_sessions}` : '点击查看待整理会话'}</p>
          </button>
          <button type="button" onClick={onOpenInjection} className="group rounded-lg border border-gray-200 bg-white px-3 py-2 text-left transition hover:border-violet-300 hover:bg-violet-50/40 focus:outline-none focus-visible:ring-2 focus-visible:ring-violet-500 dark:border-gray-700 dark:bg-gray-800 dark:hover:border-violet-500/60 dark:hover:bg-violet-500/10">
            <div className="flex items-center justify-between gap-2"><p className="text-xs text-gray-400">记忆注入</p><ChevronRight size={14} className="text-gray-300 transition group-hover:translate-x-0.5 group-hover:text-violet-500 dark:text-gray-600 dark:group-hover:text-violet-300" /></div><p className="text-sm font-semibold text-gray-800 dark:text-gray-100">{connectedMcpCount} 个 Agent</p><p className="mt-0.5 text-[11px] text-gray-400">{mcpStatusLoaded ? (connectedMcpCount ? '共享记忆已连接' : '尚未连接 Agent') : '正在检测连接'}</p>
          </button>
        </section>
      )}
      {!telemetrySummary && (
        <section className="grid grid-cols-2 sm:grid-cols-4 gap-2" aria-live="polite">
          {['Token 用量', '已整理对话', '待提取记忆', '记忆注入'].map((label) => {
            const handler = label === 'Token 用量' ? onOpenUsage : label === '待提取记忆' ? onOpenPending : label === '已整理对话' ? onOpenOrganized : onOpenInjection
            const tone = label === 'Token 用量' ? 'hover:border-blue-300 hover:bg-blue-50/40 focus-visible:ring-blue-500 dark:hover:border-blue-500/60 dark:hover:bg-blue-500/10'
              : label === '已整理对话' ? 'hover:border-emerald-300 hover:bg-emerald-50/40 focus-visible:ring-emerald-500 dark:hover:border-emerald-500/60 dark:hover:bg-emerald-500/10'
              : 'hover:border-violet-300 hover:bg-violet-50/40 focus-visible:ring-violet-500 dark:hover:border-violet-500/60 dark:hover:bg-violet-500/10'
            const arrowTone = label === 'Token 用量' ? 'group-hover:text-blue-500 dark:group-hover:text-blue-300'
              : label === '已整理对话' ? 'group-hover:text-emerald-500 dark:group-hover:text-emerald-300'
              : 'group-hover:text-violet-500 dark:group-hover:text-violet-300'
            return <button key={label} type="button" onClick={handler} className={`group rounded-lg border border-gray-200 bg-white px-3 py-2 text-left transition focus:outline-none focus-visible:ring-2 dark:border-gray-700 dark:bg-gray-800 ${tone}`}><div className="flex items-center justify-between gap-2"><p className="text-xs text-gray-400">{label}</p><ChevronRight size={14} className={`text-gray-300 transition group-hover:translate-x-0.5 dark:text-gray-600 ${arrowTone}`} /></div><p className="mt-1 inline-flex items-center gap-1.5 text-xs text-gray-500 dark:text-gray-400"><Loader2 className="animate-spin motion-reduce:animate-none" size={13} />正在读取本地账本…</p></button>
          })}
        </section>
      )}

      {/* 自动沉淀：Hook 安装、会话启动注入与数据目录已统一到「记忆注入」面板。 */}
      <section className="order-10 rounded-xl border border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-800 p-4 space-y-3">
        <div className="flex items-center gap-2">
          <Webhook size={15} className="text-violet-600 dark:text-violet-400" />
          <div className="min-w-0 flex-1"><h2 className="text-sm font-semibold text-gray-700 dark:text-gray-200">{t('memory.ingestTitle')}</h2><p className="mt-0.5 text-xs text-gray-400">{t('memory.ingestHint')}Hook 连接、会话启动注入与数据目录请前往<button type="button" onClick={onOpenInjection} className="mx-0.5 inline-flex items-center gap-0.5 font-medium text-violet-600 hover:underline dark:text-violet-300">记忆注入面板<ChevronRight size={12} /></button>管理。</p></div>
          <div className="flex shrink-0 items-center gap-2">
            {ingestStatus?.enabled
              ? <span className="inline-flex items-center gap-1 text-xs text-green-600 dark:text-green-400"><CheckCircle2 size={12} />{t('memory.ingestOn')}</span>
              : <span className="inline-flex items-center gap-1 text-xs text-gray-400"><XCircle size={12} />{t('memory.ingestOff')}</span>}
            <button
              onClick={() => setIngestEnabled(!ingestStatus?.enabled)}
              className={`relative w-9 h-5 rounded-full transition-colors ${ingestStatus?.enabled ? 'bg-violet-600' : 'bg-gray-300 dark:bg-gray-600'}`}
              title={ingestStatus?.enabled ? t('memory.ingestDisable') : t('memory.ingestEnable')}
            >
              <span className={`absolute top-0.5 w-4 h-4 rounded-full bg-white transition-all ${ingestStatus?.enabled ? 'left-4.5' : 'left-0.5'}`} style={{ left: ingestStatus?.enabled ? '18px' : '2px' }} />
            </button>
            <button type="button" onClick={() => setIngestOpen((open) => !open)} aria-expanded={ingestOpen} className="inline-flex items-center rounded-md p-1 text-gray-500 hover:bg-gray-100 dark:text-gray-400 dark:hover:bg-gray-700" title={ingestOpen ? '收起' : '展开'}>{ingestOpen ? <ChevronUp size={15} /> : <ChevronDown size={15} />}</button>
          </div>
        </div>

        {ingestOpen && <div className="space-y-3 border-t border-gray-100 pt-3 dark:border-gray-700">
        <div className={`flex items-center gap-1.5 text-xs ${ingestStatus?.model_ready ? 'text-green-700 dark:text-green-400' : 'text-amber-800 dark:text-amber-300'}`}>
          {ingestStatus?.model_ready ? <CheckCircle2 size={13} /> : <XCircle size={13} />}
          {ingestStatus?.model_ready
            ? t('memory.modelReady', { provider: ingestStatus.model_provider_id })
            : t('memory.modelMissing')}
        </div>

        <div className="flex flex-wrap items-center gap-2 rounded-lg border border-gray-200 bg-gray-50 px-3 py-2.5 dark:border-gray-700 dark:bg-gray-900/40">
          <FileText size={14} className="text-violet-600 dark:text-violet-400" />
          <div className="min-w-0 flex-1"><p className="text-xs font-medium text-gray-700 dark:text-gray-200">{t('memory.organizeConversationsTitle')}</p><p className="mt-0.5 text-xs text-gray-500 dark:text-gray-400">{t('memory.organizeConversationsHint')}</p></div>
          <button onClick={() => { void handleOrganizeConversations() }} disabled={organizingConversations || !ingestStatus?.model_ready} className="inline-flex items-center gap-1.5 rounded-md bg-violet-600 px-2.5 py-1.5 text-xs font-medium text-white hover:bg-violet-500 disabled:cursor-not-allowed disabled:opacity-50">{organizingConversations ? <Loader2 size={13} className="animate-spin" /> : <RefreshCw size={13} />}{organizingConversations ? t('memory.organizeConversationsWorking') : t('memory.organizeConversationsAction')}</button>
        </div>

        <div className="flex flex-wrap items-center gap-2 rounded-lg border border-gray-200 bg-gray-50 px-3 py-2.5 dark:border-gray-700 dark:bg-gray-900/40">
          <FolderOpen size={14} className="text-violet-600 dark:text-violet-400" />
          <div className="min-w-0 flex-1"><p className="text-xs font-medium text-gray-700 dark:text-gray-200">导入其他 Agent 记忆</p><p className="mt-0.5 text-xs text-gray-500 dark:text-gray-400">选择导出的会话或记忆文件夹；调用记忆模型整理后写入下方的可检索记忆。</p></div>
          <button onClick={() => { void handleImportMemoryFolder() }} disabled={importingMemories || !ingestStatus?.model_ready} className="inline-flex items-center gap-1.5 rounded-md border border-violet-200 bg-white px-2.5 py-1.5 text-xs font-medium text-violet-700 hover:bg-violet-50 disabled:cursor-not-allowed disabled:opacity-50 dark:border-violet-500/30 dark:bg-gray-800 dark:text-violet-300 dark:hover:bg-violet-500/10">{importingMemories ? <Loader2 size={13} className="animate-spin" /> : <FolderOpen size={13} />}{importingMemories ? '正在识别并提取' : '选择文件夹导入'}</button>
          {memoryImportResult && <p className="basis-full text-[11px] text-green-700 dark:text-green-300" role="status">已扫描 {memoryImportResult.scanned_files} 个文件 · 识别 {memoryImportResult.recognized_files} 个文本文件 · 写入 {memoryImportResult.imported_memories} 条记忆{memoryImportResult.skipped_files > 0 ? ` · 跳过 ${memoryImportResult.skipped_files} 个` : ''}</p>}
        </div>

        {ingestStatus && ingestStatus.buffered_sessions > 0 && (
          <p className="rounded-md bg-amber-500/10 px-2.5 py-2 text-xs text-amber-800 dark:text-amber-200">
            {t(ingestStatus.model_ready ? 'memory.ingestBuffering' : 'memory.ingestModelUnavailable', { count: ingestStatus.buffered_sessions })}
          </p>
        )}

        {ingestStatus && ingestStatus.recent.length > 0 && (
          <div className="min-w-0">
            <p className="text-xs text-gray-500 dark:text-gray-400">{t('memory.ingestRecent')}</p>
            <div className="mt-1 max-h-44 space-y-1 overflow-y-auto rounded-md border border-gray-200 bg-gray-50 p-2 pr-1.5 dark:border-gray-700 dark:bg-gray-900/40">
              {ingestStatus.recent.map((log, i) => (
                <div key={`${log.at}-${log.state}-${log.detail}-${i}`} className="flex min-w-0 items-center gap-2 text-xs text-gray-600 dark:text-gray-300">
                  <span className="shrink-0 text-gray-400 font-mono">{log.at}</span>
                  <span className={`shrink-0 px-1.5 rounded ${log.state === 'retrying' ? 'bg-amber-500/10 text-amber-700 dark:text-amber-300' : log.state === 'working' ? 'bg-sky-500/10 text-sky-700 dark:text-sky-300' : 'bg-violet-500/10 text-violet-600 dark:text-violet-400'}`}>
                    {log.state === 'retrying' ? t('memory.recordRetrying') : log.state === 'working' ? t('memory.organizeConversationsWorking') : t('memory.recordStored')}
                  </span>
                  <span className="min-w-0 flex-1 truncate" title={log.detail}>{log.detail}</span>
                </div>
              ))}
            </div>
          </div>
        )}
        </div>}

        {/* Hook 接收记录：自动沉淀的原始回执，作为本区块子项。 */}
        <div className="border-t border-gray-100 pt-3 dark:border-gray-700">
          <div className="flex flex-wrap items-center gap-2">
            <History size={14} className="text-violet-600 dark:text-violet-400" />
            <h3 className="text-xs font-semibold text-gray-700 dark:text-gray-200">{t('memory.recordsTitle')}</h3>
            <span className="text-xs text-gray-500 dark:text-gray-400">{t('memory.recordsHint')}</span>
            <button type="button" onClick={() => setRecordsOpen((open) => !open)} aria-expanded={recordsOpen} className="ml-auto inline-flex items-center gap-1 rounded-md px-2 py-1 text-xs font-medium text-gray-600 hover:bg-gray-100 dark:text-gray-300 dark:hover:bg-gray-700">
              {recordsOpen ? '收起' : '查看记录'}{recordsOpen ? <ChevronUp size={14} /> : <ChevronDown size={14} />}
            </button>
            <button onClick={() => { void handleRefresh() }} disabled={refreshing} className="inline-flex items-center gap-1 rounded-md border border-gray-300 px-2.5 py-1 text-xs text-gray-600 hover:bg-gray-50 disabled:cursor-wait disabled:opacity-60 dark:border-gray-600 dark:text-gray-300 dark:hover:bg-gray-700">
              <RefreshCw size={13} className={refreshing ? 'animate-spin' : ''} />{refreshing ? t('memory.refreshing') : t('common.refresh')}
            </button>
          </div>
          {recordsOpen && <>
          {telemetryEvents.length === 0 ? (
            <div className="py-6 text-center">
              <p className="text-sm text-gray-600 dark:text-gray-300">{t('memory.recordsEmpty')}</p>
              <p className="mt-1 text-xs text-gray-500 dark:text-gray-400">{t('memory.recordsEmptyHint')}</p>
            </div>
          ) : (
            <div className="mt-3 max-h-72 divide-y divide-gray-100 overflow-y-auto rounded-lg border border-gray-200 dark:divide-gray-700 dark:border-gray-700">
              {telemetryEvents.map((event) => {
                const tokenTotal = event.input_tokens + event.output_tokens + event.cached_tokens
                const hasConversation = Boolean(event.conversation_text)
                const expanded = expandedReceiptId === event.id
                const conversationState = event.conversation_state === 'full'
                  ? t('memory.recordConversationFull', { count: event.conversation_message_count })
                  : event.conversation_state === 'partial'
                    ? t('memory.recordConversationPartial')
                    : t('memory.recordConversationUnavailable')
                return <div key={event.id} className="px-3 py-2.5 text-xs">
                  <div className="grid grid-cols-[minmax(0,1fr)_auto] gap-x-3 gap-y-1">
                    <div className="min-w-0"><div className="flex flex-wrap items-center gap-1.5"><span className="rounded bg-violet-500/10 px-1.5 py-0.5 font-medium text-violet-700 dark:text-violet-300">{event.source}</span><span className="text-gray-700 dark:text-gray-200">{event.event_type}</span><span className="font-mono text-gray-400">{event.session_id}</span></div><p className="mt-1 truncate text-gray-500 dark:text-gray-400">{localTime(event.occurred_at)}</p></div>
                    <span className={event.token_source === 'reported' ? 'self-center font-mono text-gray-700 dark:text-gray-200' : 'self-center text-gray-400'}>{event.token_source === 'reported' ? `${tokenTotal.toLocaleString()} tokens` : t('memory.tokenUnavailable')}</span>
                  </div>
                  {event.event_type === 'Stop' && <div className={`mt-2 rounded-md px-2.5 py-2 ${event.conversation_state === 'full' ? 'bg-emerald-500/10 text-emerald-800 dark:text-emerald-200' : 'bg-gray-100 text-gray-600 dark:bg-gray-700/60 dark:text-gray-300'}`}>
                    <div className="flex items-center gap-1.5"><FileText size={13} aria-hidden="true" /><span className="font-medium">{conversationState}</span>{hasConversation && <button type="button" onClick={() => setExpandedReceiptId(expanded ? null : event.id)} aria-expanded={expanded} className="ml-auto inline-flex items-center gap-1 rounded px-1 py-0.5 font-medium hover:bg-black/5 focus:outline-none focus:ring-1 focus:ring-violet-500 dark:hover:bg-white/10">{expanded ? t('memory.recordConversationCollapse') : t('memory.recordConversationExpand')}{expanded ? <ChevronUp size={13} /> : <ChevronDown size={13} />}</button>}</div>
                    {hasConversation && !expanded && <p className="mt-1.5 max-h-10 overflow-hidden whitespace-pre-wrap text-gray-700 dark:text-gray-200">{event.conversation_text}</p>}
                    {hasConversation && expanded && <pre className="mt-2 max-h-72 overflow-auto whitespace-pre-wrap rounded border border-black/10 bg-white/70 p-2.5 font-sans text-xs leading-5 text-gray-800 dark:border-white/10 dark:bg-gray-900/40 dark:text-gray-100">{event.conversation_text}</pre>}
                  </div>}
                </div>
              })}
            </div>
          )}
          </>}
        </div>
      </section>

      {/* L2 / L3 层级记忆：明确与 L1 清洗分开，只有已发布 L3 才会注入 Agent。 */}
      <section className="order-30 grid gap-3 xl:grid-cols-2">
        <article className="flex flex-col rounded-xl border border-sky-200 bg-sky-50/40 p-4 dark:border-sky-900/70 dark:bg-sky-950/20">
          <div className="flex items-start gap-2">
            <div className="mt-0.5 rounded-md bg-sky-500/10 p-1.5 text-sky-600 dark:text-sky-300"><FileText size={14} /></div>
            <div className="min-w-0 flex-1 min-h-[62px]"><h2 className="text-sm font-semibold text-gray-800 dark:text-gray-100">近 30 天工作记忆</h2><p className="mt-0.5 text-xs leading-5 text-gray-500 dark:text-gray-400 line-clamp-2" title="手动从近期会话记忆分批压缩。保留来源 id，不会把全部记忆注入上下文。">手动从近期会话记忆分批压缩。保留来源 id，不会把全部记忆注入上下文。</p></div>
            <button onClick={() => { void handleL2Consolidation() }} disabled={consolidatingL2} className="inline-flex shrink-0 items-center gap-1.5 rounded-md bg-sky-600 px-2.5 py-1.5 text-xs font-medium text-white hover:bg-sky-500 disabled:opacity-50">{consolidatingL2 ? <Loader2 size={13} className="animate-spin" /> : <RefreshCw size={13} />}{consolidatingL2 ? '正在分批巩固' : '生成'}</button>
          </div>
          {/* 用户自定义添加：与右侧 L3 卡片同位置（操作按钮底下），共用同一份
              表单内容与提交状态；写入的条目会在 L2 巩固时强制并入证据。 */}
          <div className="mt-3 rounded-md border border-sky-200 bg-white/70 px-3 py-2.5 dark:border-sky-900/70 dark:bg-gray-900/30">
            <button
              type="button"
              onClick={() => setL2UserMemoryOpen((open) => !open)}
              aria-expanded={l2UserMemoryOpen}
              className="inline-flex items-center gap-1.5 text-sm font-medium text-sky-700 dark:text-sky-300"
              title={t('memory.userDefinedPlaceholder')}
            >
              <Plus size={15} />
              {t('memory.userDefinedAdd')}
              {l2UserMemoryOpen ? <ChevronUp size={14} /> : <ChevronDown size={14} />}
            </button>
            {l2UserMemoryOpen && (
              <div className="mt-2 space-y-2">
                <textarea
                  value={userMemoryContent}
                  onChange={(e) => setUserMemoryContent(e.target.value)}
                  placeholder={t('memory.userDefinedPlaceholder')}
                  rows={3}
                  className="w-full rounded-md border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-700 px-2.5 py-1.5 text-sm text-gray-800 dark:text-gray-100 focus:outline-none focus:ring-1 focus:ring-sky-500"
                />
                <div className="flex items-center gap-2">
                  <select
                    value={userMemoryType}
                    onChange={(e) => setUserMemoryType(e.target.value)}
                    className="rounded-md border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-700 px-2 py-1.5 text-sm text-gray-800 dark:text-gray-100 focus:outline-none"
                  >
                    {userMemoryTypeOptions.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
                  </select>
                  <button
                    type="button"
                    onClick={() => { void handleAddUserMemory('l2') }}
                    disabled={addingUserMemory || !userMemoryContent.trim()}
                    className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md bg-sky-600 hover:bg-sky-500 disabled:opacity-50 text-white text-sm font-medium"
                  >
                    {addingUserMemory ? <Loader2 size={15} className="animate-spin" /> : <Plus size={15} />}
                    {t('memory.userDefinedAdd')}
                  </button>
                </div>
              </div>
            )}
          </div>
          {l2Documents.find((document) => document.state === 'published') ? (() => {
            const document = l2Documents.find((item) => item.state === 'published')!
            const content = normalizeL2Document(document.content)
            const repairedLegacyContent = content !== document.content.trim()
            const hasAnalysisPreamble = looksLikeModelAnalysis(content)
            return <div className="mt-3 flex min-h-0 flex-1 flex-col border-t border-sky-200/70 pt-3 dark:border-sky-900/70">
              <UserMemoryList accent="sky" scope="l2" />
              {/* 已发布区块与右侧 L3 文档行同构：同一套底纹卡片 +
                  绿色状态标签 + 灰色元信息行，两卡格式统一。 */}
              <div className="min-w-0 flex-1 rounded-md bg-white/70 px-2.5 py-2 dark:bg-gray-900/30">
                <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs">
                  <span className="text-green-600 dark:text-green-300">已发布</span>
                  <span className="min-w-0 text-gray-500 dark:text-gray-400">{document.source_count} 条来源 · 约 {document.token_estimate} tokens · {localTime(document.created_at)}</span>
                  <LayerWindow document={document} />
                  <button type="button" onClick={() => { setEditingL2(true); setEditingL2Content(content) }} disabled={savingL2 || consolidatingL2} className="ml-auto inline-flex shrink-0 items-center gap-1 rounded border border-gray-300 px-2 py-0.5 font-medium text-gray-700 hover:bg-gray-100 disabled:opacity-50 dark:border-gray-600 dark:text-gray-200 dark:hover:bg-gray-700"><Pencil size={12} />编辑</button>
                </div>
              {editingL2 ? (
                <div className="mt-2 space-y-1.5">
                  <textarea
                    value={editingL2Content}
                    onChange={(e) => setEditingL2Content(e.target.value)}
                    rows={12}
                    className="w-full rounded-md border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-700 px-2.5 py-2 font-mono text-xs leading-5 text-gray-800 dark:text-gray-100 focus:outline-none focus:ring-1 focus:ring-sky-500"
                  />
                  <div className="flex items-center gap-2">
                    <button type="button" onClick={() => { void handleL2Save(document) }} disabled={savingL2 || !editingL2Content.trim()} className="inline-flex items-center gap-1 rounded border border-sky-300 px-2 py-0.5 text-xs font-medium text-sky-700 hover:bg-sky-100 disabled:opacity-50 dark:border-sky-700 dark:text-sky-200">{savingL2 ? <Loader2 size={12} className="animate-spin" /> : null}{savingL2 ? '保存中' : '保存'}</button>
                    <button type="button" onClick={() => { setEditingL2(false); setEditingL2Content('') }} disabled={savingL2} className="rounded border border-gray-300 px-2 py-0.5 text-xs font-medium text-gray-700 hover:bg-gray-100 disabled:opacity-50 dark:border-gray-600 dark:text-gray-200 dark:hover:bg-gray-700">取消</button>
                    <p className="text-xs text-gray-400 dark:text-gray-500">保存即覆盖当前发布版，立刻用于注入与 L3 草案。</p>
                  </div>
                </div>
              ) : (<>
                {repairedLegacyContent && <p className="mt-2 rounded-md border border-sky-200 bg-sky-100/60 px-2.5 py-2 text-xs leading-5 text-sky-800 dark:border-sky-900/70 dark:bg-sky-950/30 dark:text-sky-200">已自动隐藏旧文档中的模型执行分析，仅显示可用的最终记忆正文。</p>}
                {hasAnalysisPreamble && <p className="mt-2 rounded-md border border-amber-200 bg-amber-50 px-2.5 py-2 text-xs leading-5 text-amber-800 dark:border-amber-900/70 dark:bg-amber-950/30 dark:text-amber-200">这份文档含有模型的执行分析开头，并非理想的最终记忆格式。请重新生成。</p>}
                <CollapsibleDocumentContent content={content} contentId="l2-published-content" accent="sky" />
              </>)}
              </div>
            </div>
          })() : (<><p className="mt-3 text-xs text-gray-500 dark:text-gray-400">尚未生成。先完成会话整理后手动生成。</p><UserMemoryList accent="sky" scope="l2" /></>)}
        </article>
        <article className="flex flex-col rounded-xl border border-violet-200 bg-violet-50/40 p-4 dark:border-violet-900/70 dark:bg-violet-950/20">
          <div className="flex items-start gap-2">
            <div className="mt-0.5 rounded-md bg-violet-500/10 p-1.5 text-violet-600 dark:text-violet-300"><Brain size={14} /></div>
            <div className="min-w-0 flex-1 min-h-[62px]"><h2 className="text-sm font-semibold text-gray-800 dark:text-gray-100">长期 Profile</h2><p className="mt-0.5 text-xs leading-5 text-gray-500 dark:text-gray-400 line-clamp-2" title="基于已发布的工作记忆与筛选后的长期会话记忆创建草案。人工发布前不会进入 MCP 初始化上下文；手动添加的自定义记忆则始终随整理一起注入。">基于已发布的工作记忆与筛选后的长期会话记忆创建草案。人工发布前不会进入 MCP 初始化上下文；手动添加的自定义记忆则始终随整理一起注入。</p></div>
            <button onClick={() => { void handleL3Draft() }} disabled={draftingL3 || !l2Documents.some((document) => document.state === 'published')} className="inline-flex shrink-0 items-center gap-1.5 rounded-md bg-violet-600 px-2.5 py-1.5 text-xs font-medium text-white hover:bg-violet-500 disabled:opacity-50">{draftingL3 ? <Loader2 size={13} className="animate-spin" /> : <Pencil size={13} />}{draftingL3 ? '正在起草' : '创建草案'}</button>
          </div>
          {/* 用户自定义添加：放在「创建草案」底下。写入 user_defined 长期
              记忆，自动整理不会删改，并随 L2/L3 整理与注入一起下发给 Agent。 */}
          <div className="mt-3 rounded-md border border-violet-200 bg-white/70 px-3 py-2.5 dark:border-violet-900/70 dark:bg-gray-900/30">
            <button
              type="button"
              onClick={() => setUserMemoryOpen((open) => !open)}
              aria-expanded={userMemoryOpen}
              className="inline-flex items-center gap-1.5 text-sm font-medium text-violet-700 dark:text-violet-300"
              title={t('memory.userDefinedPlaceholder')}
            >
              <Plus size={15} />
              {t('memory.userDefinedAdd')}
              {userMemoryOpen ? <ChevronUp size={14} /> : <ChevronDown size={14} />}
            </button>
            {userMemoryOpen && (
              <div className="mt-2 space-y-2">
                <textarea
                  value={userMemoryContent}
                  onChange={(e) => setUserMemoryContent(e.target.value)}
                  placeholder={t('memory.userDefinedPlaceholder')}
                  rows={3}
                  className="w-full rounded-md border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-700 px-2.5 py-1.5 text-sm text-gray-800 dark:text-gray-100 focus:outline-none focus:ring-1 focus:ring-violet-500"
                />
                <div className="flex items-center gap-2">
                  <select
                    value={userMemoryType}
                    onChange={(e) => setUserMemoryType(e.target.value)}
                    className="rounded-md border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-700 px-2 py-1.5 text-sm text-gray-800 dark:text-gray-100 focus:outline-none"
                  >
                    {userMemoryTypeOptions.map((option) => <option key={option.value} value={option.value}>{option.label}</option>)}
                  </select>
                  <button
                    type="button"
                    onClick={() => { void handleAddUserMemory('l3') }}
                    disabled={addingUserMemory || !userMemoryContent.trim()}
                    className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md bg-violet-600 hover:bg-violet-500 disabled:opacity-50 text-white text-sm font-medium"
                  >
                    {addingUserMemory ? <Loader2 size={15} className="animate-spin" /> : <Plus size={15} />}
                    {t('memory.userDefinedAdd')}
                  </button>
                </div>
              </div>
            )}
          </div>
          {l3DraftError && <div role="alert" className="mt-3 rounded-md border border-red-200 bg-red-50 px-2.5 py-2 text-xs leading-5 text-red-800 dark:border-red-900/70 dark:bg-red-950/30 dark:text-red-200"><span className="font-medium">创建草案失败：</span>{l3DraftError}</div>}
          {visibleL3Documents.length || invalidL3Drafts.length ? <div className="mt-3 min-h-0 flex-1 space-y-2 overflow-y-auto overscroll-contain border-t border-violet-200/70 pt-3 dark:border-violet-900/70"><UserMemoryList accent="violet" scope="l3" />{visibleL3Documents.slice(0, 2).map((document) => {
            return <div key={document.id} className="min-w-0 rounded-md bg-white/70 px-2.5 py-2 dark:bg-gray-900/30">
              <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-xs">
                <span className={document.state === 'published' ? 'text-green-600 dark:text-green-300' : document.state === 'archived' ? 'text-gray-500 dark:text-gray-400' : 'text-amber-700 dark:text-amber-300'}>{document.state === 'published' ? '已发布' : document.state === 'archived' ? '历史归档版' : '草案待确认'}</span>
                <span className="min-w-0 text-gray-500 dark:text-gray-400">{document.source_count} 份来源 · 约 {document.token_estimate} tokens</span>
                {document.state === 'draft' && <div className="ml-auto flex shrink-0 items-center gap-1"><button type="button" onClick={() => { setEditingL3Id(document.id); setEditingL3Content(document.content) }} disabled={publishingL3 === document.id || deletingL3 === document.id} className="inline-flex items-center gap-1 rounded border border-gray-300 px-2 py-0.5 font-medium text-gray-700 hover:bg-gray-100 disabled:opacity-50 dark:border-gray-600 dark:text-gray-200 dark:hover:bg-gray-700"><Pencil size={12} />编辑</button><button onClick={() => { void handleL3Publish(document) }} disabled={publishingL3 === document.id || deletingL3 === document.id} className="rounded border border-violet-300 px-2 py-0.5 font-medium text-violet-700 hover:bg-violet-100 disabled:opacity-50 dark:border-violet-700 dark:text-violet-200">{publishingL3 === document.id ? '发布中' : '确认发布'}</button><button type="button" onClick={() => setL3DeleteTarget(document)} disabled={publishingL3 === document.id || deletingL3 === document.id} className="inline-flex items-center gap-1 rounded border border-red-200 px-2 py-0.5 font-medium text-red-700 hover:bg-red-50 disabled:opacity-50 dark:border-red-900/70 dark:text-red-300 dark:hover:bg-red-950/30"><Trash2 size={12} />删除草案</button></div>}
                {document.state === 'published' && <div className="ml-auto flex shrink-0 items-center gap-1"><button type="button" onClick={() => { setEditingL3Id(document.id); setEditingL3Content(document.content) }} disabled={publishingL3 === document.id} className="inline-flex items-center gap-1 rounded border border-gray-300 px-2 py-0.5 font-medium text-gray-700 hover:bg-gray-100 disabled:opacity-50 dark:border-gray-600 dark:text-gray-200 dark:hover:bg-gray-700"><Pencil size={12} />编辑</button></div>}
                {document.state === 'archived' && <div className="ml-auto flex shrink-0 items-center gap-1"><button type="button" onClick={() => setL3DeleteTarget(document)} disabled={deletingL3 === document.id} className="inline-flex items-center gap-1 rounded border border-red-200 px-2 py-0.5 font-medium text-red-700 hover:bg-red-50 disabled:opacity-50 dark:border-red-900/70 dark:text-red-300 dark:hover:bg-red-950/30"><Trash2 size={12} />删除归档版</button></div>}
              </div>
              {editingL3Id === document.id ? (
                <div className="mt-2 space-y-1.5">
                  <textarea
                    value={editingL3Content}
                    onChange={(e) => setEditingL3Content(e.target.value)}
                    rows={12}
                    className="w-full rounded-md border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-700 px-2.5 py-2 font-mono text-xs leading-5 text-gray-800 dark:text-gray-100 focus:outline-none focus:ring-1 focus:ring-violet-500"
                  />
                  <div className="flex items-center gap-2">
                    {document.state === 'published' ? (
                      <button type="button" onClick={() => { void handleL3SavePublished(document) }} disabled={publishingL3 === document.id || !editingL3Content.trim()} className="inline-flex items-center gap-1 rounded border border-violet-300 px-2 py-0.5 text-xs font-medium text-violet-700 hover:bg-violet-100 disabled:opacity-50 dark:border-violet-700 dark:text-violet-200">{publishingL3 === document.id ? <Loader2 size={12} className="animate-spin" /> : null}{publishingL3 === document.id ? '保存中' : '保存'}</button>
                    ) : null}
                    <button type="button" onClick={() => { setEditingL3Id(null); setEditingL3Content('') }} disabled={publishingL3 === document.id} className="rounded border border-gray-300 px-2 py-0.5 text-xs font-medium text-gray-700 hover:bg-gray-100 disabled:opacity-50 dark:border-gray-600 dark:text-gray-200 dark:hover:bg-gray-700">{document.state === 'published' ? '取消' : '取消编辑'}</button>
                    <p className="text-xs text-gray-400 dark:text-gray-500">{document.state === 'published' ? '保存即覆盖当前发布版，立刻用于注入，并作为下一次创建草案的重要输入。' : '编辑不单独保存：确认发布即按编辑后内容发布，离开页面则丢弃。'}</p>
                  </div>
                </div>
              ) : (
                /* 草案内容限高展示：不超过限高完整显示；超出时折叠并显示
                   「查看完整内容」，由用户手动展开。 */
                <CollapsibleDocumentContent content={document.content} contentId={`l3-document-${document.id}`} accent="violet" />
              )}
            </div>
          })}{invalidL3Drafts.map((document) => <div key={document.id} className="rounded-md border border-amber-200 bg-amber-50/80 px-2.5 py-2 text-xs dark:border-amber-900/70 dark:bg-amber-950/25"><div className="flex flex-wrap items-center gap-x-2 gap-y-1"><span className="font-medium text-amber-800 dark:text-amber-200">历史无效草案</span><span className="text-amber-700/80 dark:text-amber-300/80">{document.source_count} 份 L2/L1 来源 · 不可发布</span><button type="button" onClick={() => setL3DeleteTarget(document)} disabled={deletingL3 === document.id} className="ml-auto inline-flex shrink-0 items-center gap-1 rounded border border-red-200 bg-white/70 px-2 py-0.5 font-medium text-red-700 hover:bg-red-50 disabled:opacity-50 dark:border-red-900/70 dark:bg-gray-900/40 dark:text-red-300 dark:hover:bg-red-950/30"><Trash2 size={12} />{deletingL3 === document.id ? '删除中' : '删除草案'}</button></div><p className="mt-1.5 leading-5 text-amber-800 dark:text-amber-200">内容未展示，因其不符合 L3 发布格式。可直接删除后重新创建草案。</p></div>)}</div> : (<><p className="mt-3 text-xs text-gray-500 dark:text-gray-400">尚无可发布的 L3 草案。请点击“创建草案”重新生成。</p><UserMemoryList accent="violet" scope="l3" /></>)}
        </article>
      </section>

      {/* 可检索记忆（L1） */}
      <section className="order-20 rounded-xl border border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-800 p-4 space-y-3">
        <div className="flex items-center gap-2">
          <h2 className="text-sm font-semibold text-gray-700 dark:text-gray-200">{t('memory.storageTitle')}</h2>
          <div className="ml-auto flex items-center gap-2 text-xs text-gray-500 dark:text-gray-400">
            <span>{t('memory.total')}: <b className="text-gray-700 dark:text-gray-200">{localMemoryStats?.total ?? memories.length}</b></span>
            <span className="text-violet-600 dark:text-violet-400">{t('memory.fact')}: {localMemoryStats?.facts ?? factCount}</span>
            <span className="text-amber-600 dark:text-amber-400">{t('memory.pref')}: {localMemoryStats?.preferences ?? prefCount}</span>
            <span className="rounded bg-gray-100 px-1.5 py-0.5 text-gray-600 dark:bg-gray-700 dark:text-gray-200">{t('memory.durabilitySession')}: {localMemoryStats?.session_only ?? memories.filter((memory) => memory.durability === 'session').length}</span>
            <span className="rounded bg-sky-500/10 px-1.5 py-0.5 text-sky-700 dark:text-sky-300">{t('memory.durabilityShortTerm')}: {localMemoryStats?.short_term ?? memories.filter((memory) => memory.durability === 'short_term').length}</span>
            <span className="rounded bg-emerald-500/10 px-1.5 py-0.5 text-emerald-700 dark:text-emerald-300">{t('memory.durabilityLongTerm')}: {localMemoryStats?.long_term ?? memories.filter((memory) => memory.durability === 'long_term').length}</span>
            <button
              onClick={() => { void handleRefreshImportance() }}
              disabled={refreshingImportance || memories.length === 0}
              className="inline-flex items-center gap-1 rounded-md border border-gray-300 px-2 py-1 font-medium text-gray-700 hover:bg-gray-50 disabled:cursor-wait disabled:opacity-60 dark:border-gray-600 dark:text-gray-200 dark:hover:bg-gray-700"
              title={t('memory.importanceRefreshHint')}
            >
              <Star size={13} className={refreshingImportance ? 'animate-pulse fill-current text-amber-500' : ''} />
              {refreshingImportance ? t('memory.importanceRefreshing') : t('memory.importanceRefresh')}
            </button>
            <button
              onClick={requestResetL1}
              disabled={resettingL1}
              className="inline-flex items-center gap-1 rounded-md border border-red-200 px-2 py-1 font-medium text-red-700 hover:bg-red-50 disabled:cursor-wait disabled:opacity-60 dark:border-red-900/70 dark:text-red-300 dark:hover:bg-red-950/30"
              title="清空派生记忆并按当前提取规则重新整理原始会话"
            >
              {resettingL1 ? <Loader2 size={13} className="animate-spin" /> : <Trash2 size={13} />}
              {resettingL1 ? '正在重置' : '重新整理'}
            </button>
            <button
              onClick={() => { void handleRefresh() }}
              disabled={refreshing}
              className="rounded-md p-1 hover:bg-gray-100 disabled:cursor-wait disabled:opacity-60 dark:hover:bg-gray-700"
              title={refreshing ? t('memory.refreshing') : t('common.refresh')}
              aria-label={refreshing ? t('memory.refreshing') : t('common.refresh')}
            >
              <RefreshCw size={14} className={refreshing ? 'animate-spin' : ''} />
            </button>
          </div>
        </div>

        {/* 语义检索：检索对象就是本区块的可检索记忆（含 L0 对话回退）。 */}
        <div className="flex flex-wrap gap-2">
          <input
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => e.key === 'Enter' && handleSearch()}
            placeholder={t('memory.searchPlaceholder')}
            className="flex-1 rounded-md border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-700 px-2.5 py-1.5 text-sm text-gray-800 dark:text-gray-100 focus:outline-none focus:ring-1 focus:ring-violet-500"
          />
          <select
            value={topK}
            onChange={(e) => setTopK(Number(e.target.value))}
            className="rounded-md border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-700 px-2 py-1.5 text-sm text-gray-800 dark:text-gray-100 focus:outline-none"
          >
            {[5, 10, 20].map((n) => <option key={n} value={n}>{n}</option>)}
          </select>
          <button
            onClick={handleSearch}
            disabled={loading || !query.trim()}
            className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md bg-violet-600 hover:bg-violet-500 disabled:opacity-50 text-white text-sm font-medium"
          >
            {loading ? <Loader2 size={15} className="animate-spin" /> : <Search size={15} />}
            {t('memory.search')}
          </button>
        </div>
        {lastSearchResults.length > 0 && (
          <div className="space-y-1.5">
            <p className="text-xs text-gray-500 dark:text-gray-400">
              {t('memory.resultsFor')}「{lastSearchQuery}」
            </p>
            {lastSearchResults.map((m, i) => (
              <div key={m.id} className="rounded-md bg-gray-50 dark:bg-gray-700/50 border border-gray-200 dark:border-gray-600 px-3 py-2">
                <div className="flex flex-wrap items-center gap-2 mb-1">
                  <span className="text-xs font-mono text-gray-400">#{i + 1}</span>
                  {m.score != null && (
                    <span className="text-xs font-mono text-amber-600 dark:text-amber-400">score {Number(m.score).toFixed(3)}</span>
                  )}
                  {m.memory_type && <span className="text-xs px-1.5 rounded bg-violet-500/10 text-violet-600 dark:text-violet-400">{m.memory_type}</span>}
                  {m.durability && <span className={`text-xs px-1.5 rounded ${m.durability === 'long_term' ? 'bg-emerald-500/10 text-emerald-700 dark:text-emerald-300' : m.durability === 'short_term' ? 'bg-sky-500/10 text-sky-700 dark:text-sky-300' : 'bg-gray-200 text-gray-600 dark:bg-gray-600 dark:text-gray-200'}`}>{t(`memory.durability${m.durability === 'long_term' ? 'LongTerm' : m.durability === 'short_term' ? 'ShortTerm' : 'Session'}`)}</span>}
                  {m.retrieval_source === 'conversation' && <span className="text-xs px-1.5 rounded bg-emerald-500/10 text-emerald-700 dark:text-emerald-300">{t('memory.searchSourceConversation')}</span>}
                  {m.retrieval_source === 'memory' && <span className="text-xs px-1.5 rounded bg-sky-500/10 text-sky-700 dark:text-sky-300">{t('memory.searchSourceMemory')}</span>}
                  {(m.last_update_at || m.occurred_at) && <span className="text-xs text-gray-400">{localTime(m.last_update_at || m.occurred_at || '')}</span>}
                </div>
                <p className="text-sm text-gray-700 dark:text-gray-200 whitespace-pre-wrap break-all">{m.memory}</p>
              </div>
            ))}
          </div>
        )}
        {lastSearchResults.length === 0 && lastSearchQuery && !loading && (
          <p className="text-sm text-gray-400 text-center py-4">{t('memory.noResults')}</p>
        )}

        {memories.length === 0 ? (
          <p className="text-sm text-gray-400 dark:text-gray-500 text-center py-6">{t('memory.empty')}</p>
        ) : (
          <div className="space-y-1.5 max-h-[420px] overflow-y-auto pr-1">
            {memories.map((m) => (
              <div key={m.id} className="rounded-md bg-gray-50 dark:bg-gray-700/50 border border-gray-200 dark:border-gray-600 px-3 py-2">
                {(() => {
                  const ranking = importance[m.id]
                  const level = ranking?.score == null ? null : ranking.score >= 70 ? 'high' : ranking.score >= 40 ? 'medium' : 'low'
                  return <>
                <div className="flex items-start gap-2">
                  <MemoryMarkdown content={m.memory} className="flex-1 break-words" />
                  <div className="flex shrink-0 gap-1">
                    <button onClick={() => { void handlePinMemory(m.id, !ranking?.pinned) }} className={`p-1 rounded hover:bg-amber-100 dark:hover:bg-amber-900/40 ${ranking?.pinned ? 'text-amber-500' : 'text-gray-500 dark:text-gray-400'}`} title={ranking?.pinned ? t('memory.importanceUnpin') : t('memory.importancePin')}>
                      <Star size={13} className={ranking?.pinned ? 'fill-current' : ''} />
                    </button>
                    <button onClick={() => openEdit(m)} className="p-1 rounded hover:bg-gray-200 dark:hover:bg-gray-600 text-gray-500 dark:text-gray-400" title={t('common.edit')}>
                      <Pencil size={13} />
                    </button>
                    <button onClick={() => handleDelete(m)} className="p-1 rounded hover:bg-red-100 dark:hover:bg-red-900/40 text-gray-500 dark:text-gray-400 hover:text-red-500" title={t('common.delete')}>
                      <Trash2 size={13} />
                    </button>
                  </div>
                </div>
                <div className="mt-0.5 flex flex-wrap items-center gap-2">
                  {m.user_defined && <span className="text-xs px-1.5 rounded bg-violet-600/15 font-medium text-violet-700 dark:text-violet-300" title={t('memory.userDefinedPlaceholder')}>{t('memory.userDefinedBadge')}</span>}
                  {m.memory_type && <span className="text-xs px-1.5 rounded bg-violet-500/10 text-violet-600 dark:text-violet-400">{m.memory_type}</span>}
                  {m.durability && <span className={`text-xs px-1.5 rounded ${m.durability === 'long_term' ? 'bg-emerald-500/10 text-emerald-700 dark:text-emerald-300' : m.durability === 'short_term' ? 'bg-sky-500/10 text-sky-700 dark:text-sky-300' : 'bg-gray-200 text-gray-600 dark:bg-gray-600 dark:text-gray-200'}`}>{t(`memory.durability${m.durability === 'long_term' ? 'LongTerm' : m.durability === 'short_term' ? 'ShortTerm' : 'Session'}`)}</span>}
                  {level && <span className={`text-xs px-1.5 rounded ${level === 'high' ? 'bg-amber-500/15 text-amber-700 dark:text-amber-300' : level === 'medium' ? 'bg-sky-500/10 text-sky-700 dark:text-sky-300' : 'bg-gray-200 text-gray-600 dark:bg-gray-600 dark:text-gray-200'}`}>{t(`memory.importance${level[0].toUpperCase()}${level.slice(1)}`, { score: ranking.score })}</span>}
                  {ranking && <span className="text-xs text-gray-400">{t('memory.importanceEvidence', { sessions: ranking.supporting_sessions, agents: ranking.supporting_agents, recalls: ranking.recall_count })}</span>}
                  {(m.event_time || m.last_update_at) && <span className="text-xs text-gray-400" title={m.event_time ? '会话发生时间' : '记忆整理时间'}>{localTime(m.event_time || m.last_update_at || '')}</span>}
                </div>
                  </>
                })()}
              </div>
            ))}
          </div>
        )}

        {/* L1 清洗与去重：作为可检索记忆的子项；不是 L2/L3 的层级巩固。 */}
        <div className="border-t border-gray-100 pt-3 dark:border-gray-700">
          <div className="flex flex-wrap items-center gap-4">
            <div className="flex-1">
            <h3 className="text-xs font-semibold text-gray-700 dark:text-gray-200">记忆清洗与去重</h3>
            <p className="text-xs text-gray-500 dark:text-gray-400 mt-0.5">仅审阅可检索记忆中的语义重复项并安全合并；不会生成新的工作记忆或修改长期 Profile。执行前会创建可恢复检查点。</p>
            </div>
            <button type="button" onClick={() => setConsolidationOpen((open) => !open)} aria-expanded={consolidationOpen} className="inline-flex items-center gap-1 rounded-md border border-gray-300 px-2.5 py-1.5 text-xs font-medium text-gray-600 hover:bg-gray-50 dark:border-gray-600 dark:text-gray-300 dark:hover:bg-gray-700">{consolidationOpen ? '收起' : '查看'}{consolidationOpen ? <ChevronUp size={14} /> : <ChevronDown size={14} />}</button>
            {consolidationOpen &&
            <button
            onClick={handleDreaming}
            disabled={consolidating}
            className="inline-flex items-center gap-1.5 px-3 py-1.5 rounded-md bg-violet-600 hover:bg-violet-500 disabled:opacity-50 text-white text-sm font-medium"
            >
            {consolidating ? <Loader2 size={15} className="animate-spin" /> : <MoonStar size={15} />}
            {consolidating ? '正在清洗' : '开始清洗'}
            </button>
            }
          </div>
          {consolidationOpen && <>{consolidating && <p className="mt-3 rounded-md bg-violet-500/10 px-2.5 py-2 text-xs text-violet-800 dark:text-violet-200">{consolidationProgress ?? t('memory.dreamInProgress')}<span className="ml-1.5 inline-block animate-pulse">…</span></p>}
          {consolidationResult && <div className={`mt-3 rounded-md px-2.5 py-2.5 text-xs ${consolidationResult.kind === 'ok' ? 'bg-green-500/10 text-green-800 dark:text-green-200' : 'bg-red-500/10 text-red-800 dark:text-red-200'}`}><p>{consolidationResult.text}</p>{consolidationResult.result && <div className="mt-1.5 flex flex-wrap gap-x-3 gap-y-1 text-green-700 dark:text-green-300"><span>{t('memory.dreamSummaryScanned', { count: consolidationResult.result.scopes })}</span><span>{t('memory.dreamSummaryClusters', { count: consolidationResult.result.clusters })}</span><span>{t('memory.dreamSummaryActions', { count: consolidationResult.result.actions })}</span><span>{t('memory.dreamSummaryCount', { before: consolidationResult.result.before_count, after: consolidationResult.result.after_count })}</span></div>}{consolidationResult.kind === 'ok' && consolidationResult.result?.snapshot_id && <button onClick={() => { void handleRestoreConsolidation() }} disabled={restoringConsolidation} className="mt-2 inline-flex items-center gap-1 rounded border border-green-700/30 px-2 py-1 font-medium text-green-800 hover:bg-green-500/10 disabled:opacity-50 dark:text-green-200">{restoringConsolidation ? <Loader2 size={12} className="animate-spin" /> : <History size={12} />}{restoringConsolidation ? t('memory.dreamRestoring') : t('memory.dreamRestore')}</button>}</div>}</>}
        </div>
      </section>

      {/* 编辑弹窗 */}
      {editing && (
        <div className="fixed inset-0 z-50 bg-black/40 flex items-center justify-center p-4" onClick={() => setEditing(null)}>
          <div className="w-full max-w-md rounded-xl bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 p-4 space-y-3" onClick={(e) => e.stopPropagation()}>
            <div className="flex items-center justify-between">
              <h3 className="text-sm font-semibold text-gray-800 dark:text-gray-100">{t('memory.editTitle')}</h3>
              <button onClick={() => setEditing(null)} className="p-1 rounded hover:bg-gray-100 dark:hover:bg-gray-700 text-gray-500">
                <X size={15} />
              </button>
            </div>
            <textarea
              value={editContent}
              onChange={(e) => setEditContent(e.target.value)}
              rows={4}
              className="w-full rounded-md border border-gray-300 dark:border-gray-600 bg-white dark:bg-gray-700 px-2.5 py-2 text-sm text-gray-800 dark:text-gray-100 focus:outline-none focus:ring-1 focus:ring-violet-500"
            />
            <div className="flex justify-end gap-2">
              <button onClick={() => setEditing(null)} className="px-3 py-1.5 rounded-md border border-gray-300 dark:border-gray-600 text-sm text-gray-600 dark:text-gray-300 hover:bg-gray-50 dark:hover:bg-gray-700">
                {t('common.cancel')}
              </button>
              <button onClick={handleSaveEdit} className="px-3 py-1.5 rounded-md bg-violet-600 hover:bg-violet-500 text-white text-sm font-medium">
                {t('common.save')}
              </button>
            </div>
          </div>
        </div>
      )}

      {/* L3 草案删除确认：仅允许删除未发布的草案。 */}
      {l3DeleteTarget && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4" onClick={() => !deletingL3 && setL3DeleteTarget(null)}>
          <div className="w-full max-w-sm space-y-3 rounded-xl border border-gray-200 bg-white p-4 dark:border-gray-700 dark:bg-gray-800" onClick={(event) => event.stopPropagation()}>
            <div className="flex items-center gap-2">
              <Trash2 size={18} className="text-red-500" />
              <h3 className="text-sm font-semibold text-gray-800 dark:text-gray-100">{l3DeleteTarget.state === 'archived' ? '删除历史归档版？' : '删除草案？'}</h3>
            </div>
            <p className="text-sm leading-6 text-gray-600 dark:text-gray-300">此操作会删除该未发布{ l3DeleteTarget.state === 'archived' ? '归档版' : '草案'}及其来源关联，已发布的 Profile 不会受影响。</p>
            <p className="text-xs text-gray-500 dark:text-gray-400">{l3DeleteTarget.source_count} 份来源 · 约 {l3DeleteTarget.token_estimate} tokens</p>
            <div className="flex justify-end gap-2">
              <button onClick={() => setL3DeleteTarget(null)} disabled={Boolean(deletingL3)} className="rounded-md border border-gray-300 px-3 py-1.5 text-sm text-gray-600 hover:bg-gray-50 disabled:opacity-50 dark:border-gray-600 dark:text-gray-300 dark:hover:bg-gray-700">{t('common.cancel')}</button>
              <button onClick={() => { void confirmL3DraftDelete() }} disabled={Boolean(deletingL3)} className="inline-flex items-center gap-1 rounded-md bg-red-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-red-500 disabled:opacity-50">{deletingL3 ? <Loader2 size={14} className="animate-spin" /> : <Trash2 size={14} />}{deletingL3 ? '正在删除' : '删除草案'}</button>
            </div>
          </div>
        </div>
      )}

      {/* L1 重新提取确认：保留 L0 与使用量，只重建派生层。 */}
      {resetL1ConfirmOpen && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 p-4" onClick={() => !resettingL1 && setResetL1ConfirmOpen(false)}>
          <div className="w-full max-w-md space-y-3 rounded-xl border border-gray-200 bg-white p-4 dark:border-gray-700 dark:bg-gray-800" onClick={(event) => event.stopPropagation()}>
            <div className="flex items-center gap-2"><RefreshCw size={18} className="text-violet-600 dark:text-violet-300" /><h3 className="text-sm font-semibold text-gray-800 dark:text-gray-100">重新提取会话记忆？</h3></div>
            <p className="text-sm leading-6 text-gray-600 dark:text-gray-300">将清空现有会话记忆及其派生文档，再把完整原始会话放回整理队列。原始对话、用量、Skill 与你手动添加的自定义记忆不会删除。</p>
            <p className="text-xs leading-5 text-violet-700 dark:text-violet-300">重新提取后，每条记忆会显示“仅本会话 / 短期 / 长期”标签；长期 Profile 仅使用长期记忆。</p>
            <div className="flex justify-end gap-2"><button onClick={() => setResetL1ConfirmOpen(false)} disabled={resettingL1} className="rounded-md border border-gray-300 px-3 py-1.5 text-sm text-gray-600 hover:bg-gray-50 disabled:opacity-50 dark:border-gray-600 dark:text-gray-300 dark:hover:bg-gray-700">{t('common.cancel')}</button><button onClick={() => { void confirmResetL1() }} disabled={resettingL1} className="inline-flex items-center gap-1 rounded-md bg-violet-600 px-3 py-1.5 text-sm font-medium text-white hover:bg-violet-500 disabled:opacity-50">{resettingL1 ? <Loader2 size={14} className="animate-spin" /> : <RefreshCw size={14} />}{resettingL1 ? '正在重置' : '开始重跑'}</button></div>
          </div>
        </div>
      )}

      {/* 删除确认弹窗 */}
      {deleteTarget && (
        <div className="fixed inset-0 z-50 bg-black/40 flex items-center justify-center p-4" onClick={() => setDeleteTarget(null)}>
          <div className="w-full max-w-sm rounded-xl bg-white dark:bg-gray-800 border border-gray-200 dark:border-gray-700 p-4 space-y-3" onClick={(e) => e.stopPropagation()}>
            <div className="flex items-center gap-2">
              <Trash2 size={18} className="text-red-500" />
              <h3 className="text-sm font-semibold text-gray-800 dark:text-gray-100">{t('memory.deleteConfirm')}</h3>
            </div>
            <p className="text-sm text-gray-600 dark:text-gray-300 line-clamp-3 break-all">{deleteTarget.memory}</p>
            <div className="flex justify-end gap-2">
              <button onClick={() => setDeleteTarget(null)} className="px-3 py-1.5 rounded-md border border-gray-300 dark:border-gray-600 text-sm text-gray-600 dark:text-gray-300 hover:bg-gray-50 dark:hover:bg-gray-700">
                {t('common.cancel')}
              </button>
              <button onClick={confirmDelete} className="px-3 py-1.5 rounded-md bg-red-600 hover:bg-red-500 text-white text-sm font-medium">
                {t('common.delete')}
              </button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}

// 用户自定义记忆列表：L2 工作记忆与 L3 长期 Profile 卡片各自只显示
// 归属本层的 user_defined 条目（id 前缀 local-user-l2: / local-user-l3:，
// 旧版 local-user: 归入 l3），支持内联编辑与删除；复用 store 的
// updateMemory/deleteMemory，保存/删除后自动刷新列表。
function UserMemoryList({ accent, scope }: { accent: 'sky' | 'violet'; scope: 'l2' | 'l3' }) {
  const { memories, updateMemory, deleteMemory } = useMemoryStore()
  const customMemories = useMemo(() => memories.filter((memory) => {
    if (!memory.user_defined) return false
    return scope === 'l2'
      ? memory.id.startsWith('local-user-l2:')
      : !memory.id.startsWith('local-user-l2:')
  }), [memories, scope])
  const [editingId, setEditingId] = useState<string | null>(null)
  const [editContent, setEditContent] = useState('')
  const [savingId, setSavingId] = useState<string | null>(null)
  const [deletingId, setDeletingId] = useState<string | null>(null)
  const [error, setError] = useState<string | null>(null)

  if (!customMemories.length) return null

  const accentText = accent === 'sky' ? 'text-sky-700 dark:text-sky-300' : 'text-violet-700 dark:text-violet-300'
  const accentSave = accent === 'sky'
    ? 'border-sky-300 text-sky-700 hover:bg-sky-100 dark:border-sky-700 dark:text-sky-200 dark:hover:bg-sky-950/40'
    : 'border-violet-300 text-violet-700 hover:bg-violet-100 dark:border-violet-700 dark:text-violet-200 dark:hover:bg-violet-950/40'

  const handleSave = async (id: string) => {
    const content = editContent.trim()
    if (!content) return
    setSavingId(id)
    setError(null)
    try {
      await updateMemory(id, content)
      setEditingId(null)
      setEditContent('')
    } catch (err) {
      setError(String(err))
    } finally {
      setSavingId(null)
    }
  }

  const handleDelete = async (id: string) => {
    if (!window.confirm('确定删除这条自定义记忆吗？删除后无法恢复。')) return
    setDeletingId(id)
    setError(null)
    try {
      await deleteMemory(id)
    } catch (err) {
      setError(String(err))
    } finally {
      setDeletingId(null)
    }
  }

  return (
    <div className="mt-3 space-y-1.5">
      <p className={`text-xs font-medium ${accentText}`}>自定义记忆 · {customMemories.length} 条</p>
      {error && <p className="rounded border border-red-200 bg-red-50 px-2 py-1 text-xs text-red-700 dark:border-red-900/70 dark:bg-red-950/30 dark:text-red-300">{error}</p>}
      {customMemories.map((memory) => (
        <div key={memory.id} className="rounded-md border border-gray-200 bg-white/70 px-2.5 py-2 dark:border-gray-700 dark:bg-gray-900/30">
          {editingId === memory.id ? (
            <div className="space-y-1.5">
              <textarea
                value={editContent}
                onChange={(event) => setEditContent(event.target.value)}
                rows={3}
                className="w-full rounded-md border border-gray-300 bg-white px-2.5 py-1.5 text-sm text-gray-800 focus:outline-none focus:ring-1 focus:ring-violet-500 dark:border-gray-600 dark:bg-gray-700 dark:text-gray-100"
              />
              <div className="flex items-center gap-2">
                <button type="button" onClick={() => { void handleSave(memory.id) }} disabled={savingId === memory.id || !editContent.trim()} className={`inline-flex items-center gap-1 rounded border px-2 py-0.5 text-xs font-medium disabled:opacity-50 ${accentSave}`}>
                  {savingId === memory.id ? <Loader2 size={12} className="animate-spin" /> : null}
                  {savingId === memory.id ? '保存中' : '保存'}
                </button>
                <button type="button" onClick={() => { setEditingId(null); setEditContent('') }} disabled={savingId === memory.id} className="rounded border border-gray-300 px-2 py-0.5 text-xs font-medium text-gray-700 hover:bg-gray-100 disabled:opacity-50 dark:border-gray-600 dark:text-gray-200 dark:hover:bg-gray-700">取消</button>
              </div>
            </div>
          ) : (
            <div className="flex items-start gap-2">
              <span className="mt-0.5 shrink-0 rounded bg-gray-100 px-1.5 py-0.5 font-mono text-[10px] text-gray-600 dark:bg-gray-700 dark:text-gray-300">{memory.memory_type ?? 'fact'}</span>
              <p className="min-w-0 flex-1 whitespace-pre-wrap break-words text-sm text-gray-800 dark:text-gray-100">{memory.memory}</p>
              <div className="flex shrink-0 items-center gap-1">
                <button type="button" onClick={() => { setEditingId(memory.id); setEditContent(memory.memory) }} disabled={deletingId === memory.id} className="inline-flex items-center gap-1 rounded border border-gray-300 px-1.5 py-0.5 text-xs font-medium text-gray-700 hover:bg-gray-100 disabled:opacity-50 dark:border-gray-600 dark:text-gray-200 dark:hover:bg-gray-700"><Pencil size={11} />编辑</button>
                <button type="button" onClick={() => { void handleDelete(memory.id) }} disabled={deletingId === memory.id} className="inline-flex items-center gap-1 rounded border border-red-200 px-1.5 py-0.5 text-xs font-medium text-red-700 hover:bg-red-50 disabled:opacity-50 dark:border-red-900/70 dark:text-red-300 dark:hover:bg-red-950/30">{deletingId === memory.id ? <Loader2 size={11} className="animate-spin" /> : <Trash2 size={11} />}删除</button>
              </div>
            </div>
          )}
        </div>
      ))}
    </div>
  )
}
