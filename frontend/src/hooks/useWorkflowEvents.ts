import { useEffect } from 'react'
import { listen } from '@tauri-apps/api/event'
import { useWorkflowStore, type RunSummary, type RunStatus, type AcceptanceRequest } from '../store/workflowStore'

/**
 * 全局工作流事件监听器（阶段 1e + 2d）。
 *
 * 监听后端 5 个事件并更新 workflowStore：
 * - workflow-run-started → 新建 Run 摘要
 * - workflow-step → 更新 Step（含 run_id 路由）
 * - workflow-run-finished → 更新 Run 状态
 * - workflow-acceptance-requested → 阶段二：设置 pendingAcceptance
 *
 * 旧 workflow-done / workflow-step 事件 McpAgent.tsx 仍自己监听处理，
 * 这里只是额外把数据同步到 workflowStore 供 RunsHistory 页面使用。
 *
 * 在 App.tsx 顶层调用一次即可。
 */
export function useWorkflowEvents() {
  useEffect(() => {
    const unlisteners: Array<() => void> = []
    const store = useWorkflowStore.getState

    // workflow-run-started：新建 Run
    listen<{
      run_id: string
      template_id: string
      created_at: number
    }>('workflow-run-started', (e) => {
      const p = e.payload
      const summary: RunSummary = {
        runId: p.run_id,
        templateId: p.template_id,
        status: 'running',
        createdAt: p.created_at,
        stepCount: 0,
      }
      store().upsertRunSummary(summary)
    }).then((unlisten) => unlisteners.push(unlisten))

    // workflow-step：单步完成（后端 1d 起带 run_id 字段）
    listen<{
      node_id: string
      label: string
      kind: string
      output: string
      error: string | null
      run_id?: string
      submission?: unknown
      status?: string
      attempt?: number
    }>('workflow-step', (e) => {
      const p = e.payload
      if (!p.run_id) return // 旧格式事件无 run_id，跳过

      // 更新 Run 摘要的 stepCount
      const existing = store().runs.find((r) => r.runId === p.run_id)
      if (existing) {
        store().upsertRunSummary({
          ...existing,
          stepCount: existing.stepCount + 1,
        })
      }
    }).then((unlisten) => unlisteners.push(unlisten))

    // workflow-run-finished：Run 结束
    listen<{
      run_id: string
      status: string
      success: boolean
      finished_at: number
    }>('workflow-run-finished', (e) => {
      const p = e.payload
      const existing = store().runs.find((r) => r.runId === p.run_id)
      if (existing) {
        const status = (p.status || (p.success ? 'success' : 'failed')) as RunStatus
        store().upsertRunSummary({
          ...existing,
          status,
          finishedAt: p.finished_at,
        })
      }
    }).then((unlisten) => unlisteners.push(unlisten))

    // 阶段二：workflow-acceptance-requested → 设置 pendingAcceptance
    listen<{
      run_id: string
      node_id: string
      label: string
      allow_reject_to: string[]
      executed_node_ids: string[]
    }>('workflow-acceptance-requested', (e) => {
      const p = e.payload
      const req: AcceptanceRequest = {
        runId: p.run_id,
        nodeId: p.node_id,
        label: p.label,
        allowRejectTo: p.allow_reject_to.length > 0 ? p.allow_reject_to : p.executed_node_ids,
        executedNodeIds: p.executed_node_ids,
      }
      store().setPendingAcceptance(req)
    }).then((unlisten) => unlisteners.push(unlisten))

    return () => {
      unlisteners.forEach((fn) => fn())
    }
  }, [])
}
