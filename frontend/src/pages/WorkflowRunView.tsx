import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { useWorkflowStore, type StepInstance, type RunRecord, type RunTrigger, type Verdict, type FailureTrace, formatDuration, formatTime, statusColor, runStatusColor, verdictKind } from '../store/workflowStore'
import { ChevronDown, ChevronRight, AlertTriangle, Terminal } from 'lucide-react'

// ── 触发来源标签 ──────────────────────────────────────────────────────────────

function triggerBadge(trigger: RunTrigger | undefined, t: (k: string) => string) {
  if (!trigger) return null
  const map: Record<string, { label: string; cls: string }> = {
    manual: { label: t('workflow.run.sourceManual'), cls: 'bg-gray-100 text-gray-600 dark:bg-gray-800 dark:text-gray-400' },
    hook: { label: t('workflow.run.sourceHook'), cls: 'bg-purple-100 text-purple-700 dark:bg-purple-900/30 dark:text-purple-300' },
    rework: { label: t('workflow.run.sourceRework'), cls: 'bg-amber-100 text-amber-700 dark:bg-amber-900/30 dark:text-amber-300' },
    schedule: { label: t('workflow.run.sourceSchedule'), cls: 'bg-blue-100 text-blue-700 dark:bg-blue-900/30 dark:text-blue-300' },
  }
  const info = map[trigger.trigger]
  if (!info) return null
  return <span className={`text-xs px-2 py-0.5 rounded ${info.cls}`}>{info.label}</span>
}

// ── Acceptance 面板（阶段 2d）─────────────────────────────────────────────────

function AcceptancePanel({ runId, allowRejectTo }: { runId: string; allowRejectTo: string[] }) {
  const { t } = useTranslation()
  const { approveRun, rejectRun } = useWorkflowStore()
  const [reason, setReason] = useState('')
  const [rejectTo, setRejectTo] = useState(allowRejectTo[0] ?? '')
  const [submitting, setSubmitting] = useState(false)

  const handleApprove = async () => {
    setSubmitting(true)
    await approveRun(runId)
    setSubmitting(false)
  }

  const handleReject = async () => {
    if (!rejectTo) return
    setSubmitting(true)
    await rejectRun(runId, rejectTo, reason)
    setSubmitting(false)
  }

  return (
    <div className="border-2 border-blue-400 dark:border-blue-600 rounded-lg p-4 bg-blue-50 dark:bg-blue-900/20 mx-3 mb-3">
      <h3 className="text-sm font-semibold text-blue-700 dark:text-blue-300">
        {t('workflow.acceptance.title')}
      </h3>

      <div className="flex flex-wrap items-center gap-2 mt-3">
        <button
          onClick={handleApprove}
          disabled={submitting}
          className="bg-green-600 hover:bg-green-700 disabled:opacity-50 text-white text-sm px-4 py-1.5 rounded transition-colors"
        >
          {t('workflow.acceptance.approve')}
        </button>

        <span className="text-xs text-gray-500 dark:text-gray-400">
          {t('workflow.acceptance.rejectTo')}:
        </span>
        <select
          value={rejectTo}
          onChange={(e) => setRejectTo(e.target.value)}
          className="text-sm border border-gray-300 dark:border-gray-600 rounded px-2 py-1 bg-white dark:bg-gray-800 text-gray-700 dark:text-gray-300"
        >
          {allowRejectTo.map((nodeId) => (
            <option key={nodeId} value={nodeId}>{nodeId}</option>
          ))}
        </select>

        <button
          onClick={handleReject}
          disabled={submitting || !rejectTo}
          className="bg-red-600 hover:bg-red-700 disabled:opacity-50 text-white text-sm px-4 py-1.5 rounded transition-colors"
        >
          {t('workflow.acceptance.reject')}
        </button>
      </div>

      <textarea
        value={reason}
        onChange={(e) => setReason(e.target.value)}
        placeholder={t('workflow.acceptance.reasonPlaceholder')}
        className="w-full mt-2 text-sm border border-gray-300 dark:border-gray-600 rounded p-2 bg-white dark:bg-gray-800 text-gray-700 dark:text-gray-300 resize-none"
        rows={2}
      />
    </div>
  )
}

// ── Verdict 徽章 ─────────────────────────────────────────────────────────────

function VerdictBadge({ verdict }: { verdict?: Verdict }) {
  const { t } = useTranslation()
  if (!verdict) return null
  const kind = verdictKind(verdict)
  const color =
    kind === 'pass'
      ? 'bg-green-100 text-green-700 border-green-300'
      : kind === 'fail'
        ? 'bg-red-100 text-red-700 border-red-300'
        : 'bg-amber-100 text-amber-700 border-amber-300'
  const label = t(`workflow.verdict.${kind}`, kind)
  const reason =
    kind === 'fail'
      ? (verdict as { kind: 'fail'; reason: string }).reason
      : kind === 'blocked'
        ? (verdict as { kind: 'blocked'; reason: string }).reason
        : ''
  return (
    <span
      className={`inline-flex items-center rounded border px-1.5 py-0.5 text-xs font-medium ${color}`}
      title={reason || undefined}
    >
      {label}
      {reason ? `: ${reason.slice(0, 60)}${reason.length > 60 ? '…' : ''}` : ''}
    </span>
  )
}

// ── FailureTrace 折叠面板（阶段三 3e） ───────────────────────────────────────

function failureKindColor(kind: string): string {
  switch (kind) {
    case 'timeout': return 'text-orange-600 dark:text-orange-400 bg-orange-50 dark:bg-orange-900/20'
    case 'network': return 'text-blue-600 dark:text-blue-400 bg-blue-50 dark:bg-blue-900/20'
    case 'rpc_error': return 'text-red-600 dark:text-red-400 bg-red-50 dark:bg-red-900/20'
    case 'schema_violation': return 'text-purple-600 dark:text-purple-400 bg-purple-50 dark:bg-purple-900/20'
    case 'agent_blocked': return 'text-amber-600 dark:text-amber-400 bg-amber-50 dark:bg-amber-900/20'
    case 'process_exited': return 'text-red-700 dark:text-red-300 bg-red-50 dark:bg-red-900/20'
    default: return 'text-gray-600 dark:text-gray-400 bg-gray-50 dark:bg-gray-800/50'
  }
}

function FailureTracePanel({ trace }: { trace: FailureTrace }) {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)
  const [showStderr, setShowStderr] = useState(false)

  const kindLabel = t(`workflow.failureKind.${trace.failureKind}`, trace.failureKind)

  return (
    <div className="mt-2 border border-red-200 dark:border-red-800 rounded-md overflow-hidden">
      {/* 头部：点击折叠/展开 */}
      <button
        onClick={() => setExpanded(!expanded)}
        className="w-full flex items-center gap-2 px-2 py-1.5 bg-red-50 dark:bg-red-900/20 hover:bg-red-100 dark:hover:bg-red-900/30 transition-colors"
      >
        {expanded ? <ChevronDown className="h-3.5 w-3.5 text-gray-500" /> : <ChevronRight className="h-3.5 w-3.5 text-gray-500" />}
        <AlertTriangle className="h-3.5 w-3.5 text-red-500 shrink-0" />
        <span className={`text-xs font-medium px-1.5 py-0.5 rounded ${failureKindColor(trace.failureKind)}`}>
          {kindLabel}
        </span>
        <span className="text-xs text-gray-600 dark:text-gray-400 truncate">
          {trace.reason.slice(0, 80)}{trace.reason.length > 80 ? '…' : ''}
        </span>
      </button>

      {expanded && (
        <div className="px-2 py-2 space-y-1.5 bg-white dark:bg-gray-900">
          {/* 基本信息 */}
          <div className="text-xs text-gray-500 dark:text-gray-400 space-y-0.5">
            <div><span className="font-medium">{t('workflow.failureTrace.nodeId')}:</span> {trace.nodeId}</div>
            {trace.agentId && <div><span className="font-medium">{t('workflow.failureTrace.agentId')}:</span> {trace.agentId}</div>}
            {trace.tool && <div><span className="font-medium">{t('workflow.failureTrace.tool')}:</span> {trace.tool}</div>}
            <div><span className="font-medium">{t('workflow.failureTrace.finalStatus')}:</span> {trace.finalStatus}</div>
          </div>

          {/* 完整 reason */}
          <div className="text-xs text-red-600 dark:text-red-400 bg-red-50 dark:bg-red-900/10 rounded p-1.5">
            {trace.reason}
          </div>

          {/* retry history */}
          {trace.retryHistory.length > 0 && (
            <div className="text-xs">
              <div className="font-medium text-gray-500 dark:text-gray-400 mb-0.5">
                {t('workflow.failureTrace.retryHistory')}
              </div>
              {trace.retryHistory.map((r, i) => (
                <div key={i} className="text-gray-500 dark:text-gray-400 pl-2">
                  #{r.attempt}: {r.reason} ({formatTime(r.at)})
                </div>
              ))}
            </div>
          )}

          {/* stderr excerpt */}
          {trace.stderrExcerpt && (
            <div>
              <button
                onClick={() => setShowStderr(!showStderr)}
                className="flex items-center gap-1 text-xs text-gray-500 hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-200"
              >
                <Terminal className="h-3.5 w-3.5" />
                {t('workflow.failureTrace.stderrExcerpt')}
                {showStderr ? <ChevronDown className="h-3 w-3" /> : <ChevronRight className="h-3 w-3" />}
              </button>
              {showStderr && (
                <pre className="mt-1 text-xs text-gray-400 dark:text-gray-500 bg-gray-900 dark:bg-black/40 rounded p-2 max-h-32 overflow-auto whitespace-pre-wrap break-all font-mono">
                  {trace.stderrExcerpt}
                </pre>
              )}
            </div>
          )}
        </div>
      )}
    </div>
  )
}

// ── Step 卡片 ────────────────────────────────────────────────────────────────

function StepCard({ step }: { step: StepInstance }) {
  const { t } = useTranslation()
  const statusLabel = t(`workflow.nodeState.${step.status}`, step.status)
  const kindLabel = step.kind
    ? t(`mcpAgent.nodeKind${step.kind.charAt(0).toUpperCase()}${step.kind.slice(1)}`, step.kind)
    : ''
  return (
    <div className="border border-gray-200 dark:border-gray-700 rounded-lg p-3 bg-white dark:bg-gray-900">
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-2 min-w-0">
          <span className="font-medium text-sm text-gray-800 dark:text-gray-200 truncate">
            {step.nodeId}
          </span>
          {kindLabel && (
            <span className="text-[10px] px-1.5 py-0.5 rounded bg-gray-100 text-gray-500 dark:bg-gray-800 dark:text-gray-400 shrink-0">
              {kindLabel}
            </span>
          )}
          {step.attempt > 1 && (
            <span className="text-xs text-amber-600 dark:text-amber-400">
              {t('workflow.run.attempt', { n: step.attempt })}
            </span>
          )}
        </div>
        <span className={`text-xs px-2 py-0.5 rounded shrink-0 ${statusColor(step.status)}`}>
          {statusLabel}
        </span>
      </div>

      {/* Verdict 徽章 */}
      {step.submission?.verdict && (
        <div className="mt-1.5">
          <VerdictBadge verdict={step.submission.verdict} />
        </div>
      )}

      {/* 输出预览 */}
      {step.output && (
        <div className="mt-2 text-xs text-gray-600 dark:text-gray-400 max-h-24 overflow-auto">
          <pre className="whitespace-pre-wrap break-all">{step.output.slice(0, 500)}{step.output.length > 500 ? '…' : ''}</pre>
        </div>
      )}

      {/* 错误信息 */}
      {step.error && (
        <div className="mt-2 text-xs text-red-600 dark:text-red-400 bg-red-50 dark:bg-red-900/20 rounded p-1.5">
          {step.error}
        </div>
      )}

      {/* 阶段三 3e：FailureTrace 折叠面板 */}
      {step.failureTrace && (
        <FailureTracePanel trace={step.failureTrace} />
      )}

      {/* 耗时 */}
      <div className="mt-1.5 text-xs text-gray-400 dark:text-gray-500">
        {formatTime(step.startedAt)} → {formatTime(step.finishedAt)}
        {step.startedAt && step.finishedAt && (
          <span className="ml-2">· {formatDuration(step.startedAt, step.finishedAt)}</span>
        )}
      </div>
    </div>
  )
}

// ── Fan-out 子任务卡片（紧凑版） ──────────────────────────────────────────────

function FanOutChildCard({ step, index }: { step: StepInstance; index: number }) {
  const { t } = useTranslation()
  const statusLabel = t(`workflow.nodeState.${step.status}`, step.status)
  return (
    <div className="border border-gray-200 dark:border-gray-700 rounded-md p-2 bg-gray-50 dark:bg-gray-800/50">
      <div className="flex items-center justify-between gap-2">
        <div className="flex items-center gap-1.5 min-w-0">
          <span className="text-xs font-mono text-gray-400 shrink-0">#{index + 1}</span>
          <span className={`text-xs px-1.5 py-0.5 rounded shrink-0 ${statusColor(step.status)}`}>
            {statusLabel}
          </span>
        </div>
        <div className="text-xs text-gray-400 dark:text-gray-500 shrink-0">
          {step.startedAt && step.finishedAt && formatDuration(step.startedAt, step.finishedAt)}
        </div>
      </div>
      {step.output && (
        <div className="mt-1 text-xs text-gray-500 dark:text-gray-400 max-h-16 overflow-auto">
          <pre className="whitespace-pre-wrap break-all">{step.output.slice(0, 200)}{step.output.length > 200 ? '…' : ''}</pre>
        </div>
      )}
      {step.error && (
        <div className="mt-1 text-xs text-red-600 dark:text-red-400">
          {step.error}
        </div>
      )}
    </div>
  )
}

// ── Step 分组（父 + fan_out 子任务折叠） ──────────────────────────────────────

function StepGroup({ parentStep, childSteps }: { parentStep: StepInstance; childSteps: StepInstance[] }) {
  const [expanded, setExpanded] = useState(true)
  const { t } = useTranslation()
  const childCount = childSteps.length
  const successCount = childSteps.filter(s => s.status === 'success').length
  const failCount = childSteps.filter(s => s.status === 'failed').length

  return (
    <div className="space-y-1">
      <StepCard step={parentStep} />
      {childCount > 0 && (
        <div className="ml-4">
          <button
            onClick={() => setExpanded(!expanded)}
            className="flex items-center gap-1.5 text-xs text-gray-500 hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-200 py-1"
          >
            {expanded ? <ChevronDown className="h-3.5 w-3.5" /> : <ChevronRight className="h-3.5 w-3.5" />}
            <span>
              {t('workflow.run.fanOutChildren', { total: childCount, success: successCount, fail: failCount })}
            </span>
          </button>
          {expanded && (
            <div className="space-y-1 mt-1">
              {childSteps.map((child) => {
                const idx = child.nodeId.includes('#') ? parseInt(child.nodeId.split('#').pop() || '0', 10) : 0
                return <FanOutChildCard key={child.stepId} step={child} index={idx} />
              })}
            </div>
          )}
        </div>
      )}
    </div>
  )
}

// ── Run 详情视图 ─────────────────────────────────────────────────────────────

export function WorkflowRunView({ runId, onClose }: { runId: string; onClose?: () => void }) {
  const { t } = useTranslation()
  const { activeRun, fetchRun, pendingAcceptance } = useWorkflowStore()
  const [blockedToast, setBlockedToast] = useState<{ nodeId: string; reason: string } | null>(null)

  useEffect(() => {
    fetchRun(runId)
  }, [runId, fetchRun])

  // 阶段三 3e：监听 sweeper step-failed 事件
  useEffect(() => {
    let unlisten: (() => void) | null = null
    let unlistenStep: (() => void) | null = null
    let unlistenFinished: (() => void) | null = null

    async function setup() {
      const { listen } = await import('@tauri-apps/api/event')
      unlisten = await listen<{ run_id: string; node_id: string; reason: string }>('workflow-step-failed', (e) => {
        if (e.payload.run_id === runId) {
          setBlockedToast({ nodeId: e.payload.node_id, reason: e.payload.reason })
          // 刷新 run 数据以显示更新后的 step 状态
          fetchRun(runId)
          // 5 秒后自动清除 toast
          setTimeout(() => setBlockedToast(null), 5000)
        }
      })
      // 阶段三 3e：监听 step 完成事件实时刷新
      unlistenStep = await listen<{ run_id: string }>('workflow-step', (e) => {
        if (e.payload.run_id === runId) {
          fetchRun(runId)
        }
      })
      // 阶段三 3e：监听 run 结束事件
      unlistenFinished = await listen<{ run_id: string }>('workflow-run-finished', (e) => {
        if (e.payload.run_id === runId) {
          fetchRun(runId)
        }
      })
    }
    setup()

    return () => {
      unlisten?.()
      unlistenStep?.()
      unlistenFinished?.()
    }
  }, [runId, fetchRun])

  if (!activeRun || activeRun.runId !== runId) {
    return (
      <div className="flex items-center justify-center p-8 text-gray-400">
        {t('workflow.run.loading')}
      </div>
    )
  }

  const run: RunRecord = activeRun
  const duration = run.finishedAt
    ? formatDuration(run.createdAt, run.finishedAt)
    : t('workflow.run.inProgress')

  return (
    <div className="flex flex-col h-full overflow-hidden">
      {/* 阶段三 3e：Sweeper 超时/blocked toast */}
      {blockedToast && (
        <div className="px-4 py-2 bg-red-50 dark:bg-red-900/20 border-b border-red-200 dark:border-red-800 flex items-center gap-2 shrink-0">
          <AlertTriangle className="h-4 w-4 text-red-500 shrink-0" />
          <span className="text-xs text-red-700 dark:text-red-300">
            {t('workflow.run.stepBlocked', { nodeId: blockedToast.nodeId, reason: blockedToast.reason })}
          </span>
          <button
            onClick={() => setBlockedToast(null)}
            className="ml-auto text-red-400 hover:text-red-600 text-xs"
          >
            ✕
          </button>
        </div>
      )}

      {/* Header */}
      <div className="flex items-center justify-between gap-2 border-b border-gray-200 dark:border-gray-800 px-4 py-3 shrink-0">
        <div className="flex items-center gap-3 min-w-0">
          <span className={`text-xs px-2 py-0.5 rounded ${runStatusColor(run.status)}`}>
            {t(`workflow.run.status.${run.status}`, run.status)}
          </span>
          <span className="text-sm text-gray-500 dark:text-gray-400 truncate">
            {run.templateKey ?? run.templateId}
          </span>
          {triggerBadge(run.trigger, t)}
          <span className="text-xs text-gray-400 font-mono">{run.runId.slice(0, 8)}</span>
        </div>
        <div className="flex items-center gap-3 shrink-0">
          <span className="text-xs text-gray-400">{duration}</span>
          <span className="text-xs text-gray-400">
            {run.steps.length} {t('workflow.run.steps')}
          </span>
          {onClose && (
            <button
              onClick={onClose}
              className="text-gray-400 hover:text-gray-600 dark:hover:text-gray-200"
            >
              ✕
            </button>
          )}
        </div>
      </div>

      {/* 阶段二：验收面板（当 Run 等待验收时显示） */}
      {run.status === 'waiting_acceptance' && pendingAcceptance && pendingAcceptance.runId === runId && (
        <AcceptancePanel
          runId={runId}
          allowRejectTo={pendingAcceptance.allowRejectTo}
        />
      )}

      {/* Steps */}
      <div className="flex-1 overflow-auto p-3 space-y-2">
        {run.steps.length === 0 ? (
          <div className="text-center text-gray-400 py-8">{t('workflow.run.empty')}</div>
        ) : (
          (() => {
            // 将 fan_out 子步骤（nodeId 含 '#'）分组到父步骤下
            const parents = run.steps.filter(s => !s.nodeId.includes('#'))
            const children = run.steps.filter(s => s.nodeId.includes('#'))
            return parents.map((parent) => {
              const parentId = parent.nodeId
              const parentChildren = children.filter(c => c.nodeId.startsWith(`${parentId}#`))
              if (parentChildren.length > 0) {
                return (
                  <StepGroup
                    key={parent.stepId}
                    parentStep={parent}
                    childSteps={parentChildren}
                  />
                )
              }
              return <StepCard key={parent.stepId} step={parent} />
            })
          })()
        )}
      </div>
    </div>
  )
}
