import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { invoke } from '@tauri-apps/api/core'
import { useWorkflowStore, runStatusColor, formatTime, formatDuration, type RunSummary, type RunTrigger } from '../store/workflowStore'
import { WorkflowRunView } from './WorkflowRunView'
import { RefreshCw, ArrowLeft, CheckCircle, Clock, AlertOctagon, RotateCcw } from 'lucide-react'

// ── Metrics 四卡片（阶段三 3f） ──────────────────────────────────────────────

interface WorkflowMetrics {
  success_rate: number
  total_runs: number
  success_runs: number
  avg_duration_ms: number
  top_failed_nodes: { node_id: string; fail_count: number; failure_kind: string }[]
  rework_rate: number
  acceptance_total: number
  acceptance_rejected: number
}

function MetricsCards() {
  const { t } = useTranslation()
  const [metrics, setMetrics] = useState<WorkflowMetrics | null>(null)

  useEffect(() => {
    invoke<WorkflowMetrics>('get_workflow_metrics').then(setMetrics).catch(() => {})
  }, [])

  if (!metrics || metrics.total_runs === 0) return null

  const cards = [
    {
      icon: CheckCircle,
      label: t('workflow.metrics.successRate'),
      value: `${metrics.success_rate.toFixed(1)}%`,
      sub: `${metrics.success_runs}/${metrics.total_runs}`,
      color: metrics.success_rate >= 80 ? 'text-green-600 dark:text-green-400' : metrics.success_rate >= 50 ? 'text-amber-600 dark:text-amber-400' : 'text-red-600 dark:text-red-400',
      bg: 'bg-green-50 dark:bg-green-900/20',
    },
    {
      icon: Clock,
      label: t('workflow.metrics.avgDuration'),
      value: formatAvgDuration(metrics.avg_duration_ms),
      sub: t('workflow.metrics.successRunsOnly'),
      color: 'text-blue-600 dark:text-blue-400',
      bg: 'bg-blue-50 dark:bg-blue-900/20',
    },
    {
      icon: AlertOctagon,
      label: t('workflow.metrics.topFailed'),
      value: metrics.top_failed_nodes.length > 0
        ? metrics.top_failed_nodes[0].node_id
        : t('workflow.metrics.none'),
      sub: metrics.top_failed_nodes.length > 0
        ? `${metrics.top_failed_nodes[0].fail_count}× ${t(`workflow.failureKind.${metrics.top_failed_nodes[0].failure_kind}`, metrics.top_failed_nodes[0].failure_kind)}`
        : '',
      color: 'text-red-600 dark:text-red-400',
      bg: 'bg-red-50 dark:bg-red-900/20',
    },
    {
      icon: RotateCcw,
      label: t('workflow.metrics.reworkRate'),
      value: `${metrics.rework_rate.toFixed(1)}%`,
      sub: `${metrics.acceptance_rejected}/${metrics.acceptance_total}`,
      color: metrics.rework_rate < 30 ? 'text-green-600 dark:text-green-400' : 'text-amber-600 dark:text-amber-400',
      bg: 'bg-amber-50 dark:bg-amber-900/20',
    },
  ]

  return (
    <div className="grid grid-cols-2 md:grid-cols-4 gap-2 px-4 py-2 border-b border-gray-200 dark:border-gray-800 shrink-0">
      {cards.map((card, i) => {
        const Icon = card.icon
        return (
          <div key={i} className={`rounded-lg p-2.5 ${card.bg}`}>
            <div className="flex items-center gap-1.5 mb-1">
              <Icon className={`h-3.5 w-3.5 ${card.color}`} />
              <span className="text-[10px] text-gray-500 dark:text-gray-400">{card.label}</span>
            </div>
            <div className={`text-lg font-semibold ${card.color} truncate`}>{card.value}</div>
            {card.sub && <div className="text-[10px] text-gray-400 dark:text-gray-500 truncate">{card.sub}</div>}
          </div>
        )
      })}
    </div>
  )
}

function formatAvgDuration(ms: number): string {
  if (ms === 0) return '-'
  if (ms < 1000) return `${ms}ms`
  if (ms < 60000) return `${(ms / 1000).toFixed(1)}s`
  return `${(ms / 60000).toFixed(1)}m`
}

export function RunsHistory() {
  const { t } = useTranslation()
  const { runs, loadingRuns, fetchRuns } = useWorkflowStore()
  const [selectedRunId, setSelectedRunId] = useState<string | null>(null)

  useEffect(() => {
    fetchRuns()
  }, [fetchRuns])

  // 选中某个 Run 后显示详情视图
  if (selectedRunId) {
    return (
      <div className="flex flex-col h-full">
        <button
          onClick={() => setSelectedRunId(null)}
          className="flex items-center gap-1.5 text-sm text-gray-500 hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-200 px-4 py-2 border-b border-gray-200 dark:border-gray-800"
        >
          <ArrowLeft className="h-3.5 w-3.5" />
          {t('workflow.run.backToList')}
        </button>
        <div className="flex-1 overflow-hidden">
          <WorkflowRunView runId={selectedRunId} onClose={() => setSelectedRunId(null)} />
        </div>
      </div>
    )
  }

  return (
    <div className="flex flex-col h-full overflow-hidden">
      {/* Header */}
      <div className="flex items-center justify-between gap-2 border-b border-gray-200 dark:border-gray-800 px-4 py-3 shrink-0">
        <h2 className="text-sm font-semibold text-gray-800 dark:text-gray-200">
          {t('workflow.run.title')}
        </h2>
        <button
          onClick={() => fetchRuns()}
          disabled={loadingRuns}
          className="flex items-center gap-1.5 text-xs text-gray-500 hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-200 disabled:opacity-50"
        >
          <RefreshCw className={`h-3.5 w-3.5 ${loadingRuns ? 'animate-spin' : ''}`} />
          {t('common.refresh')}
        </button>
      </div>

      {/* 阶段三 3f：Metrics 四卡片 */}
      <MetricsCards />

      {/* Table */}
      <div className="flex-1 overflow-auto">
        {runs.length === 0 ? (
          <div className="flex items-center justify-center h-full text-gray-400 text-sm">
            {t('workflow.run.empty')}
          </div>
        ) : (
          <table className="w-full text-sm">
            <thead className="sticky top-0 bg-gray-50 dark:bg-gray-900 border-b border-gray-200 dark:border-gray-800">
              <tr className="text-left text-xs text-gray-500 dark:text-gray-400">
                <th className="px-4 py-2 font-medium">Run ID</th>
                <th className="px-4 py-2 font-medium">{t('workflow.run.template')}</th>
                <th className="px-4 py-2 font-medium">{t('workflow.run.statusLabel')}</th>
                <th className="px-4 py-2 font-medium">{t('workflow.run.source')}</th>
                <th className="px-4 py-2 font-medium">{t('workflow.run.started')}</th>
                <th className="px-4 py-2 font-medium">{t('workflow.run.duration')}</th>
                <th className="px-4 py-2 font-medium">{t('workflow.run.steps')}</th>
              </tr>
            </thead>
            <tbody>
              {runs.map((r: RunSummary) => (
                <tr
                  key={r.runId}
                  onClick={() => setSelectedRunId(r.runId)}
                  className="border-b border-gray-100 dark:border-gray-800 hover:bg-gray-50 dark:hover:bg-gray-800/50 cursor-pointer transition-colors"
                >
                  <td className="px-4 py-2 font-mono text-xs text-gray-600 dark:text-gray-400">
                    {r.runId.slice(0, 8)}
                  </td>
                  <td className="px-4 py-2 text-gray-700 dark:text-gray-300 truncate max-w-[200px]">
                    {r.templateKey ?? r.templateId}
                  </td>
                  <td className="px-4 py-2">
                    <span className={`text-xs px-2 py-0.5 rounded ${runStatusColor(r.status)}`}>
                      {t(`workflow.run.status.${r.status}`, r.status)}
                    </span>
                  </td>
                  <td className="px-4 py-2">
                    <span className={`text-xs px-2 py-0.5 rounded ${triggerColor(r.trigger)}`}>
                      {triggerLabel(r.trigger, t)}
                    </span>
                  </td>
                  <td className="px-4 py-2 text-xs text-gray-500 dark:text-gray-400">
                    {formatTime(r.createdAt)}
                  </td>
                  <td className="px-4 py-2 text-xs text-gray-500 dark:text-gray-400">
                    {r.finishedAt ? formatDuration(r.createdAt, r.finishedAt) : '-'}
                  </td>
                  <td className="px-4 py-2 text-xs text-gray-500 dark:text-gray-400">
                    {r.stepCount}
                  </td>
                </tr>
              ))}
            </tbody>
          </table>
        )}
      </div>
    </div>
  )
}

/** 触发来源标签 */
function triggerLabel(trigger: RunTrigger | undefined, t: (k: string) => string): string {
  if (!trigger) return '-'
  switch (trigger.trigger) {
    case 'manual': return t('workflow.run.sourceManual')
    case 'hook': return t('workflow.run.sourceHook')
    case 'rework': return t('workflow.run.sourceRework')
    case 'schedule': return t('workflow.run.sourceSchedule')
    default: return '-'
  }
}

/** 触发来源对应的 CSS 类 */
function triggerColor(trigger: RunTrigger | undefined): string {
  if (!trigger) return 'bg-gray-100 text-gray-500'
  switch (trigger.trigger) {
    case 'manual': return 'bg-gray-100 text-gray-600'
    case 'hook': return 'bg-purple-100 text-purple-700'
    case 'rework': return 'bg-amber-100 text-amber-700'
    case 'schedule': return 'bg-blue-100 text-blue-700'
    default: return 'bg-gray-100 text-gray-500'
  }
}
