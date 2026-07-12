import { create } from 'zustand'
import { invoke } from '@tauri-apps/api/core'

// ── Types (mirror backend workflow.rs + workflow_store.rs) ───────────────────

export type Verdict =
  | 'pass'
  | { kind: 'fail'; reason: string; rootCause?: string }
  | { kind: 'blocked'; reason: string; notify?: string }

export type NodeStatus =
  | 'pending' | 'running' | 'success' | 'failed' | 'blocked' | 'skipped'

export interface Submission {
  artifact: unknown
  verdict: Verdict
  confidence?: number
  note?: string
}

export type FailureKind =
  | 'timeout' | 'network' | 'rpc_error' | 'schema_violation'
  | 'agent_blocked' | 'process_exited' | 'unknown'

export interface FailureTrace {
  runId: string
  stepId: string
  nodeId: string
  agentId?: string
  tool?: string
  failureKind: FailureKind
  reason: string
  stderrExcerpt?: string
  retryHistory: { attempt: number; at: number; reason: string }[]
  finalStatus: string
}

export interface StepInstance {
  stepId: string
  runId: string
  nodeId: string
  kind?: string
  status: NodeStatus
  submission?: Submission
  output?: string
  error?: string
  startedAt?: number
  finishedAt?: number
  attempt: number
  failureTrace?: FailureTrace
}

export type RunStatus =
  | 'running' | 'success' | 'failed' | 'blocked' | 'waiting_acceptance' | 'closed'

export type RunTrigger =
  | { trigger: 'manual'; user: string }
  | { trigger: 'hook'; source: string; external_id: string }
  | { trigger: 'schedule'; cron: string }
  | { trigger: 'rework'; parent_run_id: string }

export interface RunSummary {
  runId: string
  templateId: string
  templateKey?: string
  status: RunStatus
  createdAt: number
  finishedAt?: number
  stepCount: number
  trigger?: RunTrigger
}

export interface RunRecord extends RunSummary {
  steps: StepInstance[]
}

// ── snake_case → camelCase 映射（后端 serde 默认输出 snake_case） ──────────

/** 后端原始 RunSummary 格式 */
export interface RawRunSummary {
  run_id: string
  template_id: string
  template_key?: string
  status: RunStatus
  created_at: number
  finished_at?: number
  step_count: number
  trigger?: RunTrigger
}

/** 后端原始 StepInstance 格式 */
interface RawStepInstance {
  step_id: string
  run_id: string
  node_id: string
  kind?: string
  status: NodeStatus
  submission?: Submission
  output?: string
  error?: string
  started_at?: number
  finished_at?: number
  attempt?: number
  failure_trace?: RawFailureTrace
}

/** 后端原始 FailureTrace 格式 */
interface RawFailureTrace {
  run_id: string
  step_id: string
  node_id: string
  agent_id?: string
  tool?: string
  failure_kind: string
  reason: string
  stderr_excerpt?: string
  retry_history: { attempt: number; at: number; reason: string }[]
  final_status: string
}

function mapFailureTrace(t: RawFailureTrace): FailureTrace {
  return {
    runId: t.run_id,
    stepId: t.step_id,
    nodeId: t.node_id,
    agentId: t.agent_id,
    tool: t.tool,
    failureKind: t.failure_kind as FailureKind,
    reason: t.reason,
    stderrExcerpt: t.stderr_excerpt,
    retryHistory: t.retry_history ?? [],
    finalStatus: t.final_status,
  }
}

function mapStep(s: RawStepInstance): StepInstance {
  return {
    stepId: s.step_id,
    runId: s.run_id,
    nodeId: s.node_id,
    kind: s.kind,
    status: s.status,
    submission: s.submission,
    output: s.output,
    error: s.error,
    startedAt: s.started_at,
    finishedAt: s.finished_at,
    attempt: s.attempt ?? 1,
    failureTrace: s.failure_trace ? mapFailureTrace(s.failure_trace) : undefined,
  }
}

/** 后端原始 RunRecord 格式 */
interface RawRunRecord extends RawRunSummary {
  steps: RawStepInstance[]
  rework_context?: unknown
}

export function mapSummary(r: RawRunSummary): RunSummary {
  return {
    runId: r.run_id,
    templateId: r.template_id,
    templateKey: r.template_key,
    status: r.status,
    createdAt: r.created_at,
    finishedAt: r.finished_at,
    stepCount: r.step_count,
    trigger: r.trigger,
  }
}

function mapRunRecord(r: RawRunRecord): RunRecord {
  return {
    ...mapSummary(r),
    steps: (r.steps ?? []).map(mapStep),
  }
}

// ── Persistence (只存摘要列表到 localStorage，steps 按需 invoke 拉取) ────────

const LS_RUNS = 'workflow-runs-cache'

function loadCachedRuns(): RunSummary[] {
  try {
    const raw = localStorage.getItem(LS_RUNS)
    if (!raw) return []
    const parsed = JSON.parse(raw)
    return Array.isArray(parsed) ? parsed.slice(0, 50) : []
  } catch {
    return []
  }
}

function saveCachedRuns(runs: RunSummary[]) {
  try {
    localStorage.setItem(LS_RUNS, JSON.stringify(runs.slice(0, 50)))
  } catch {
    // ignore quota errors
  }
}

// ── Store ────────────────────────────────────────────────────────────────────

interface WorkflowStore {
  /** 已知的 Run 摘要列表（内存 + localStorage 缓存） */
  runs: RunSummary[]
  /** 当前查看的 Run 详情（含 steps） */
  activeRun: RunRecord | null
  /** 加载状态 */
  loadingRuns: boolean
  /** 阶段二：待验收的 Run 信息（来自 workflow-acceptance-requested 事件） */
  pendingAcceptance: AcceptanceRequest | null

  /** 从后端拉取 Run 列表 */
  fetchRuns: () => Promise<void>
  /** 从后端拉取完整 Run（含 steps） */
  fetchRun: (runId: string) => Promise<void>
  /** 内存中 upsert 一个 Run 摘要（来自事件） */
  upsertRunSummary: (summary: RunSummary) => void
  /** 内存中 upsert 一个 Step（来自事件） */
  upsertStep: (step: StepInstance) => void
  /** 设置当前查看的 Run */
  setActiveRun: (run: RunRecord | null) => void
  /** 阶段二：设置待验收请求 */
  setPendingAcceptance: (req: AcceptanceRequest | null) => void
  /** 阶段二：通过验收 */
  approveRun: (runId: string) => Promise<void>
  /** 阶段二：驳回验收（定向 Rework） */
  rejectRun: (runId: string, rejectToNode: string, reason: string) => Promise<string | null>
  /** 清空 */
  clear: () => void
}

/** 阶段二：验收请求（来自 workflow-acceptance-requested 事件） */
export interface AcceptanceRequest {
  runId: string
  nodeId: string
  label: string
  allowRejectTo: string[]
  executedNodeIds: string[]
}

export const useWorkflowStore = create<WorkflowStore>((set, get) => ({
  runs: loadCachedRuns(),
  activeRun: null,
  loadingRuns: false,
  pendingAcceptance: null,

  fetchRuns: async () => {
    set({ loadingRuns: true })
    try {
      const rawList = await invoke<RawRunSummary[]>('list_workflow_runs')
      const list = rawList.map(mapSummary)
      // 合并：后端列表 + 内存中事件追加的（后端可能还没落盘的）
      const existing = get().runs
      const merged = [...list]
      for (const e of existing) {
        if (!merged.some((m) => m.runId === e.runId)) {
          merged.unshift(e)
        }
      }
      saveCachedRuns(merged.slice(0, 50))
      set({ runs: merged, loadingRuns: false })
    } catch {
      set({ loadingRuns: false })
    }
  },

  fetchRun: async (runId: string) => {
    try {
      const raw = await invoke<RawRunRecord>('get_workflow_run', { runId })
      set({ activeRun: mapRunRecord(raw) })
    } catch {
      // ignore
    }
  },

  upsertRunSummary: (summary) =>
    set((s) => {
      const idx = s.runs.findIndex((r) => r.runId === summary.runId)
      const next =
        idx >= 0
          ? s.runs.map((r) => (r.runId === summary.runId ? { ...r, ...summary } : r))
          : [summary, ...s.runs].slice(0, 200)
      saveCachedRuns(next.slice(0, 50))
      return { runs: next }
    }),

  upsertStep: (step) =>
    set((s) => {
      if (!s.activeRun || s.activeRun.runId !== step.runId) return s
      const steps = s.activeRun.steps
      const i = steps.findIndex((x) => x.stepId === step.stepId)
      const nextSteps =
        i >= 0
          ? steps.map((x) => (x.stepId === step.stepId ? { ...x, ...step } : x))
          : [...steps, step]
      return { activeRun: { ...s.activeRun, steps: nextSteps } }
    }),

  setActiveRun: (run) => set({ activeRun: run }),

  setPendingAcceptance: (req) => set({ pendingAcceptance: req }),

  approveRun: async (runId) => {
    try {
      const raw = await invoke<RawRunRecord>('approve_run', { runId })
      const updated = mapRunRecord(raw)
      set({ activeRun: updated, pendingAcceptance: null })
      // 更新列表摘要
      get().upsertRunSummary({
        runId: updated.runId,
        templateId: updated.templateId,
        templateKey: updated.templateKey,
        status: updated.status,
        createdAt: updated.createdAt,
        finishedAt: updated.finishedAt,
        stepCount: updated.steps.length,
      })
    } catch (e) {
      console.error('approve_run failed:', e)
    }
  },

  rejectRun: async (runId, rejectToNode, reason) => {
    try {
      const newRunId = await invoke<string>('reject_run', { runId, rejectToNode, reason })
      set({ pendingAcceptance: null })
      return newRunId
    } catch (e) {
      console.error('reject_run failed:', e)
      return null
    }
  },

  clear: () => {
    saveCachedRuns([])
    set({ runs: [], activeRun: null })
  },
}))

// ── Helpers ──────────────────────────────────────────────────────────────────

/** 从 Verdict 提取种类字符串 */
export function verdictKind(v: Verdict | undefined | null): string {
  if (!v) return 'unknown'
  if (v === 'pass') return 'pass'
  return v.kind
}

/** 节点状态对应的 CSS 类（用于 StepCard 颜色） */
export function statusColor(status: NodeStatus | undefined): string {
  if (!status) return 'bg-gray-100 text-gray-500'
  switch (status) {
    case 'pending':
      return 'bg-gray-100 text-gray-500'
    case 'running':
      return 'bg-blue-100 text-blue-700 animate-pulse'
    case 'success':
      return 'bg-green-100 text-green-700'
    case 'failed':
      return 'bg-red-100 text-red-700'
    case 'blocked':
      return 'bg-amber-100 text-amber-700'
    case 'skipped':
      return 'bg-gray-50 text-gray-400'
    default:
      return 'bg-gray-100 text-gray-500'
  }
}

/** Run 状态对应的 CSS 类 */
export function runStatusColor(status: RunStatus | undefined): string {
  if (!status) return 'bg-gray-100 text-gray-500'
  switch (status) {
    case 'running':
      return 'bg-blue-100 text-blue-700'
    case 'success':
      return 'bg-green-100 text-green-700'
    case 'failed':
      return 'bg-red-100 text-red-700'
    case 'blocked':
      return 'bg-amber-100 text-amber-700'
    case 'waiting_acceptance':
      return 'bg-purple-100 text-purple-700'
    case 'closed':
      return 'bg-gray-100 text-gray-600'
    default:
      return 'bg-gray-100 text-gray-500'
  }
}

/** 格式化耗时（ms → "1.2s"） */
export function formatDuration(startMs?: number, endMs?: number): string {
  if (!startMs || !endMs) return '-'
  const secs = (endMs - startMs) / 1000
  if (secs < 60) return `${secs.toFixed(1)}s`
  return `${Math.floor(secs / 60)}m ${(secs % 60).toFixed(0)}s`
}

/** 格式化时间戳 */
export function formatTime(ms?: number): string {
  if (!ms) return '-'
  return new Date(ms).toLocaleTimeString()
}
