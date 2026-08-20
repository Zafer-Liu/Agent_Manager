import { useEffect, useState } from 'react'
import { open } from '@tauri-apps/plugin-dialog'
import {
  ArrowLeft, ArrowDownToLine, ArrowUpFromLine, CheckCircle2, FolderOpen, Loader2,
  PlugZap, RefreshCw, Webhook, X, XCircle, Zap,
} from 'lucide-react'
import { useMemoryStore } from '../store/memoryStore'
import type { HookStatus, MemoryMcpTarget } from '../types/memory'
import { displayFullTime } from '../components/ConversationDialog'

const MCP_ADAPTERS: { type: MemoryMcpTarget; label: string }[] = [
  { type: 'codex_cli', label: 'Codex CLI' },
  { type: 'claude_cli', label: 'Claude Code CLI' },
  { type: 'codex_desktop', label: 'Codex Desktop' },
  { type: 'claude_desktop', label: 'Claude Desktop' },
  { type: 'qoder', label: 'Qoder' },
  { type: 'workbuddy', label: 'WorkBuddy' },
  { type: 'minimax', label: 'MiniMax Code' },
  { type: 'kimi', label: 'Kimi' },
]

type HookAgentType = 'claude' | 'qoder' | 'codex' | 'workbuddy'

/** 详情回放：JSON 内容格式化展示，纯文本（如注入上下文）原样返回。 */
function formatLogDetail(detail: string) {
  try {
    return JSON.stringify(JSON.parse(detail), null, 2)
  } catch {
    return detail
  }
}

/** 支持命令式 Hook 的 Agent；SessionStart 注入只对这些 Agent 可用。 */
const HOOK_AGENTS: { id: HookAgentType; label: string }[] = [
  { id: 'codex', label: 'Codex' },
  { id: 'claude', label: 'Claude Code' },
  { id: 'qoder', label: 'Qoder' },
  { id: 'workbuddy', label: 'WorkBuddy' },
]

export function MemoryInjection({ onBack }: { onBack: () => void }) {
  const {
    hookStatus, qoderHookStatus, codexHookStatus, workbuddyHookStatus,
    ingestStatus, agentSources, memoryMcp, mcpAccessLogs,
    checkIngest, installHook, uninstallHook, setIngestEnabled,
    checkMemoryMcp, installMemoryMcp, uninstallMemoryMcp, checkMcpAccessLogs,
    loadAgentSources, setAgentSourceOverride,
  } = useMemoryStore()
  const [mcpAction, setMcpAction] = useState<MemoryMcpTarget | null>(null)
  const [hookAction, setHookAction] = useState<string | null>(null)
  const [refreshing, setRefreshing] = useState(false)
  const [expandedLogId, setExpandedLogId] = useState<number | null>(null)
  const [notice, setNotice] = useState<{ kind: 'ok' | 'err'; text: string } | null>(null)

  useEffect(() => {
    void checkIngest()
    void checkMemoryMcp()
    void checkMcpAccessLogs()
    void loadAgentSources()
  }, [checkIngest, checkMemoryMcp, checkMcpAccessLogs, loadAgentSources])

  useEffect(() => {
    const timer = window.setInterval(() => { void checkMcpAccessLogs() }, 10_000)
    return () => window.clearInterval(timer)
  }, [checkMcpAccessLogs])

  function flash(kind: 'ok' | 'err', text: string) {
    setNotice({ kind, text })
    setTimeout(() => setNotice(null), 3500)
  }

  async function handleRefresh() {
    if (refreshing) return
    setRefreshing(true)
    try {
      await Promise.all([checkIngest(), checkMemoryMcp(), checkMcpAccessLogs(), loadAgentSources()])
    } finally {
      setRefreshing(false)
    }
  }

  // ── 会话采集（Hook / 转录扫描） ──────────────────────────────────────────

  const hookStatusByAgent: Record<string, HookStatus | null> = {
    codex: codexHookStatus,
    claude: hookStatus,
    qoder: qoderHookStatus,
    workbuddy: workbuddyHookStatus,
  }

  async function handleHookInstall(agentType: HookAgentType, label: string) {
    if (hookAction) return
    setHookAction(agentType)
    try {
      await installHook(agentType)
      flash('ok', `${label} 会话采集 Hook 已安装`)
    } catch (error) {
      flash('err', `安装失败: ${String(error)}`)
    } finally {
      setHookAction(null)
    }
  }

  async function handleHookUninstall(agentType: HookAgentType, label: string) {
    if (hookAction) return
    setHookAction(agentType)
    try {
      await uninstallHook(agentType)
      flash('ok', `${label} 会话采集 Hook 已移除`)
    } catch (error) {
      flash('err', `移除失败: ${String(error)}`)
    } finally {
      setHookAction(null)
    }
  }

  async function handleChangeSourceDir(id: string) {
    const selected = await open({ directory: true, multiple: false, title: '选择该 Agent 的会话数据目录' })
    if (!selected || Array.isArray(selected)) return
    try {
      await setAgentSourceOverride(id, [selected], null)
      flash('ok', '已更新数据目录，下次刷新时按新目录扫描')
    } catch (error) {
      flash('err', `更新失败: ${String(error)}`)
    }
  }

  async function handleResetSourceDir(id: string) {
    try {
      await setAgentSourceOverride(id, null, null)
      flash('ok', '已恢复自动探测的数据目录')
    } catch (error) {
      flash('err', `恢复失败: ${String(error)}`)
    }
  }

  // ── 记忆注入（共享记忆 MCP） ─────────────────────────────────────────────

  function adapterLabel(agentType: MemoryMcpTarget) {
    return MCP_ADAPTERS.find((adapter) => adapter.type === agentType)?.label ?? agentType
  }

  async function handleMcpInstall(agentType: MemoryMcpTarget) {
    if (mcpAction) return
    setMcpAction(agentType)
    try {
      await installMemoryMcp(agentType)
      flash('ok', `${adapterLabel(agentType)} 已连接共享记忆`)
    } catch (error) {
      flash('err', `配置失败: ${String(error)}`)
    } finally {
      setMcpAction(null)
    }
  }

  async function handleMcpUninstall(agentType: MemoryMcpTarget) {
    if (mcpAction) return
    setMcpAction(agentType)
    try {
      await uninstallMemoryMcp(agentType)
      flash('ok', `${adapterLabel(agentType)} 已断开共享记忆`)
    } catch (error) {
      flash('err', `移除失败: ${String(error)}`)
    } finally {
      setMcpAction(null)
    }
  }

  const hookReadyCount = agentSources.filter((source) =>
    source.supports_hooks ? hookStatusByAgent[source.id]?.installed : true,
  ).length
  const mcpStatusList = Object.values(memoryMcp)
  const mcpStatusLoaded = mcpStatusList.some(Boolean)
  const connectedMcpCount = mcpStatusList.filter((status) => status?.installed).length
  // SessionStart 注入与采集共用同一份 Hook 配置；早期版本只装了 3 个采集事件，
  // 因此采集已就绪不代表注入已启用，两者必须分开表达。
  const COLLECT_EVENTS = ['UserPromptSubmit', 'PostToolUse', 'Stop']
  const collectEventCount = (hook: HookStatus | null) => hook?.events.filter((event) => COLLECT_EVENTS.includes(event)).length ?? 0
  const injectEnabled = (hook: HookStatus | null) => Boolean(hook?.events.includes('SessionStart'))
  const injectReadyCount = HOOK_AGENTS.filter((agent) => injectEnabled(hookStatusByAgent[agent.id])).length

  return <main className="h-full overflow-y-auto bg-[#fbfcfe] text-slate-900 dark:bg-slate-950 dark:text-slate-100">
    <div className="mx-auto max-w-[1200px] px-5 py-6 lg:px-9 lg:py-8">
      <header className="flex flex-wrap items-start justify-between gap-4">
        <div className="flex items-start gap-3">
          <button type="button" onClick={onBack} className="mt-0.5 inline-flex h-9 w-9 items-center justify-center rounded-lg text-slate-500 transition hover:bg-slate-100 hover:text-slate-900 focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-500 dark:text-slate-400 dark:hover:bg-slate-800 dark:hover:text-white" aria-label="返回记忆中心"><ArrowLeft size={19} /></button>
          <div>
            <h1 className="text-2xl font-bold tracking-[-0.025em]">记忆注入</h1>
            <p className="mt-1 text-sm text-slate-500 dark:text-slate-400">双向连接：Hook 与转录扫描把 Agent 会话沉淀为记忆；SessionStart 注入与共享记忆 MCP 把记忆与 Skill 注入回 Agent。</p>
          </div>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          {notice && <span className={`rounded-lg px-3 py-2 text-xs ${notice.kind === 'ok' ? 'bg-emerald-500/10 text-emerald-700 dark:text-emerald-300' : 'bg-red-500/10 text-red-700 dark:text-red-300'}`} role="status">{notice.text}</span>}
          <button type="button" onClick={() => { void handleRefresh() }} disabled={refreshing} className="inline-flex h-10 items-center gap-2 rounded-lg border border-slate-200 bg-white px-3 text-sm font-medium text-slate-700 transition hover:border-slate-300 hover:bg-slate-50 disabled:cursor-wait disabled:opacity-60 focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-500 dark:border-slate-700 dark:bg-slate-900 dark:text-slate-200 dark:hover:bg-slate-800"><RefreshCw size={16} className={refreshing ? 'animate-spin motion-reduce:animate-none' : ''} />刷新状态</button>
        </div>
      </header>

      <section className="mt-7 grid gap-3 sm:grid-cols-2" aria-label="双向连接概览">
        <div className="rounded-xl border border-slate-200 bg-white px-4 py-3 dark:border-slate-800 dark:bg-slate-900">
          <div className="flex items-center gap-2 text-sm text-slate-500 dark:text-slate-400"><ArrowDownToLine size={15} className="text-sky-500" />会话采集 · Agent → 记忆</div>
          <p className="mt-2 font-mono text-2xl font-semibold tabular-nums">{agentSources.length === 0 ? '—' : `${hookReadyCount} / ${agentSources.length}`}</p>
          <p className="mt-1 text-xs text-slate-400">Hook 或本地转录扫描已就绪的 Agent</p>
        </div>
        <div className="rounded-xl border border-slate-200 bg-white px-4 py-3 dark:border-slate-800 dark:bg-slate-900">
          <div className="flex items-center gap-2 text-sm text-slate-500 dark:text-slate-400"><ArrowUpFromLine size={15} className="text-violet-500" />记忆注入 · 记忆 → Agent</div>
          <p className="mt-2 font-mono text-2xl font-semibold tabular-nums">{mcpStatusLoaded ? `${connectedMcpCount} / ${MCP_ADAPTERS.length}` : '—'}</p>
          <p className="mt-1 text-xs text-slate-400">共享记忆 MCP 已连接 · 会话启动注入 {injectReadyCount} / {HOOK_AGENTS.length}</p>
        </div>
      </section>

      <section className="mt-6 rounded-2xl border border-slate-200 bg-white p-5 shadow-[0_1px_2px_rgba(15,23,42,0.03)] dark:border-slate-800 dark:bg-slate-900 sm:p-6" aria-label="会话采集">
        <div className="flex flex-wrap items-center gap-3">
          <span className="grid h-9 w-9 shrink-0 place-items-center rounded-xl bg-sky-500/10 text-sky-600 dark:text-sky-300"><Webhook size={18} /></span>
          <div className="min-w-0 flex-1">
            <h2 className="text-sm font-semibold text-slate-700 dark:text-slate-200">会话采集 · Agent → 记忆</h2>
            <p className="mt-0.5 text-xs text-slate-500 dark:text-slate-400">会话完成后进入「待提取记忆」，由记忆模型提炼为可检索记忆；仅整理动作会调用模型。</p>
          </div>
          <div className="flex shrink-0 items-center gap-2">
            {ingestStatus?.enabled
              ? <span className="inline-flex items-center gap-1 text-xs text-emerald-600 dark:text-emerald-400"><CheckCircle2 size={12} />自动沉淀已开启</span>
              : <span className="inline-flex items-center gap-1 text-xs text-slate-400"><XCircle size={12} />自动沉淀已关闭</span>}
            <button
              type="button"
              onClick={() => setIngestEnabled(!ingestStatus?.enabled)}
              className={`relative h-5 w-9 rounded-full transition-colors ${ingestStatus?.enabled ? 'bg-sky-600' : 'bg-slate-300 dark:bg-slate-600'}`}
              title={ingestStatus?.enabled ? '关闭自动沉淀' : '开启自动沉淀'}
              aria-pressed={Boolean(ingestStatus?.enabled)}
            >
              <span className="absolute top-0.5 h-4 w-4 rounded-full bg-white transition-all" style={{ left: ingestStatus?.enabled ? '18px' : '2px' }} />
            </button>
          </div>
        </div>

        <div className={`mt-3 flex items-center gap-1.5 text-xs ${ingestStatus?.model_ready ? 'text-emerald-700 dark:text-emerald-400' : 'text-amber-800 dark:text-amber-300'}`}>
          {ingestStatus?.model_ready ? <CheckCircle2 size={13} /> : <XCircle size={13} />}
          {ingestStatus?.model_ready ? `记忆模型就绪（${ingestStatus.model_provider_id ?? '已配置'}）` : '记忆模型未配置：会话会继续保留在待提取列表，配置后即可整理'}
        </div>

        <div className="mt-3 space-y-1.5">
          {agentSources.map((source) => {
            const hook = hookStatusByAgent[source.id]
            const supportsHook = source.supports_hooks
            const roots = source.transcript_roots
            const primary = roots[0]
            const missing = roots.every((root) => !root.exists)
            const working = hookAction === source.id
            return <div key={source.id} className="rounded-lg border border-slate-200 bg-slate-50/70 px-3 py-2.5 dark:border-slate-700 dark:bg-slate-900/35">
              <div className="flex flex-wrap items-center gap-2">
                <span className="text-sm font-medium text-slate-700 dark:text-slate-200">{source.label}</span>
                {supportsHook ? (
                  hook?.installed
                    ? <span className="inline-flex items-center gap-1 rounded-full bg-emerald-500/10 px-2 py-1 text-xs text-emerald-600 dark:text-emerald-400" title={`采集事件：${hook.events.filter((event) => COLLECT_EVENTS.includes(event)).join(' / ') || '无'}`}><CheckCircle2 size={12} />采集 Hook 已安装（{collectEventCount(hook)} 事件）</span>
                    : <span className="inline-flex items-center gap-1 rounded-full bg-slate-100 px-2 py-1 text-xs text-slate-500 dark:bg-slate-700 dark:text-slate-400"><XCircle size={12} />Hook 未安装</span>
                ) : (
                  <span className="inline-flex items-center gap-1 rounded-full bg-sky-500/10 px-2 py-1 text-xs text-sky-700 dark:text-sky-300" title="该 Agent 没有 Hook 入口；启动或点击刷新时扫描本地转录并回填"><CheckCircle2 size={12} />本地转录扫描</span>
                )}
                {supportsHook && (
                  hook?.installed
                    ? <button onClick={() => { void handleHookUninstall(source.id as HookAgentType, source.label) }} disabled={working} className="ml-auto inline-flex shrink-0 items-center gap-1 rounded-md border border-slate-300 px-2.5 py-1 text-xs text-slate-600 hover:bg-slate-50 disabled:opacity-50 dark:border-slate-600 dark:text-slate-300 dark:hover:bg-slate-700">{working ? <Loader2 size={13} className="animate-spin motion-reduce:animate-none" /> : <X size={13} />}卸载 Hook</button>
                    : <button onClick={() => { void handleHookInstall(source.id as HookAgentType, source.label) }} disabled={working} className="ml-auto inline-flex shrink-0 items-center gap-1 rounded-md bg-sky-600 px-2.5 py-1 text-xs font-medium text-white transition-colors hover:bg-sky-500 disabled:opacity-50">{working ? <Loader2 size={13} className="animate-spin motion-reduce:animate-none" /> : <Zap size={13} />}安装 Hook</button>
                )}
              </div>
              <div className="mt-1.5 flex flex-wrap items-center gap-2 text-xs">
                <span className="shrink-0 text-slate-400">数据目录</span>
                <span className={`min-w-0 flex-1 truncate font-mono ${missing ? 'text-amber-600 dark:text-amber-400' : 'text-slate-500 dark:text-slate-400'}`} title={roots.map((root) => root.path).join('\n')}>
                  {primary ? primary.path : '未配置'}{roots.length > 1 ? ` 等 ${roots.length} 个目录` : ''}{missing ? '（未找到）' : ''}
                </span>
                {primary?.is_override && <button onClick={() => { void handleResetSourceDir(source.id) }} className="shrink-0 rounded border border-slate-300 px-2 py-0.5 text-[11px] text-slate-600 hover:bg-slate-100 dark:border-slate-600 dark:text-slate-300 dark:hover:bg-slate-700">恢复默认</button>}
                <button onClick={() => { void handleChangeSourceDir(source.id) }} className="inline-flex shrink-0 items-center gap-1 rounded border border-sky-200 bg-white px-2 py-0.5 text-[11px] font-medium text-sky-700 hover:bg-sky-50 dark:border-sky-500/30 dark:bg-slate-800 dark:text-sky-300 dark:hover:bg-sky-500/10"><FolderOpen size={11} />更改</button>
              </div>
            </div>
          })}
        </div>
        <p className="mt-2 text-[11px] leading-4 text-slate-500 dark:text-slate-400">Hook 写入 Agent 的用户级配置：UserPromptSubmit / PostToolUse / Stop 负责采集，SessionStart 负责下方的会话启动注入，安装时一并写入、卸载时一并移除。MiniMax Code 与 Kimi 没有 Hook 入口，由本地转录扫描代替。Token 统计与会话沉淀都从数据目录读取，换设备后可改为实际目录。</p>

        {ingestStatus && ingestStatus.buffered_sessions > 0 && (
          <p className="mt-3 rounded-md bg-amber-500/10 px-2.5 py-2 text-xs text-amber-800 dark:text-amber-200">
            {ingestStatus.model_ready ? `有 ${ingestStatus.buffered_sessions} 个会话正在等待写入` : `记忆模型不可用，${ingestStatus.buffered_sessions} 个会话暂存中`}
          </p>
        )}

        {ingestStatus && ingestStatus.recent.length > 0 && (
          <div className="mt-3 min-w-0">
            <p className="text-xs text-slate-500 dark:text-slate-400">对话整理记录</p>
            <div className="scrollbar-slim mt-1 max-h-44 space-y-1 overflow-y-auto rounded-md border border-slate-200 bg-slate-50 p-2 pr-1.5 dark:border-slate-700 dark:bg-slate-900/40">
              {ingestStatus.recent.map((log, i) => (
                <div key={`${log.at}-${log.state}-${log.detail}-${i}`} className="flex min-w-0 items-start gap-2 text-xs text-slate-600 dark:text-slate-300">
                  <span className="shrink-0 font-mono text-slate-400">{log.at}</span>
                  <span className={`shrink-0 rounded px-1.5 ${log.state === 'retrying' ? 'bg-amber-500/10 text-amber-700 dark:text-amber-300' : log.state === 'working' ? 'bg-sky-500/10 text-sky-700 dark:text-sky-300' : log.state === 'failed' ? 'bg-red-500/10 text-red-700 dark:text-red-300' : 'bg-violet-500/10 text-violet-600 dark:text-violet-400'}`}>
                    {log.state === 'retrying' ? '待重试' : log.state === 'working' ? '整理中' : log.state === 'failed' ? '失败' : '已沉淀'}
                  </span>
                  <span className="min-w-0 flex-1 break-words" title={log.detail}>{log.detail}</span>
                </div>
              ))}
            </div>
          </div>
        )}
      </section>

      <section className="mt-6 rounded-2xl border border-slate-200 bg-white p-5 shadow-[0_1px_2px_rgba(15,23,42,0.03)] dark:border-slate-800 dark:bg-slate-900 sm:p-6" aria-label="记忆注入">
        <div className="flex flex-wrap items-center gap-3">
          <span className="grid h-9 w-9 shrink-0 place-items-center rounded-xl bg-violet-500/10 text-violet-600 dark:text-violet-300"><PlugZap size={18} /></span>
          <div className="min-w-0 flex-1">
            <h2 className="text-sm font-semibold text-slate-700 dark:text-slate-200">记忆注入 · 记忆 → Agent</h2>
            <p className="mt-0.5 text-xs text-slate-500 dark:text-slate-400">两条互补通道：会话启动注入在每次会话开始时自动附带记忆上下文；共享记忆 MCP 供 Agent 在会话中主动检索。</p>
          </div>
        </div>

        <div className="mt-4 rounded-xl border border-violet-200/70 bg-violet-50/40 p-3.5 dark:border-violet-500/20 dark:bg-violet-500/5">
          <div className="flex flex-wrap items-center gap-2">
            <Zap size={14} className="text-violet-600 dark:text-violet-300" />
            <h3 className="text-xs font-semibold text-slate-700 dark:text-slate-200">会话启动注入（SessionStart Hook）</h3>
            <span className="text-[11px] text-slate-500 dark:text-slate-400">自动 · 无需工具调用 · 不挂 MCP 也生效</span>
          </div>
          <p className="mt-1.5 text-[11px] leading-4 text-slate-500 dark:text-slate-400">会话启动时自动拉取本机 <code className="rounded bg-violet-100 px-1 py-0.5 font-mono text-violet-800 dark:bg-violet-500/15 dark:text-violet-200">/memory/context</code>，把 L3 长期画像（≤10000 token）与 L2 短期摘要（≤10000 token）作为 additionalContext 写入模型上下文；应用未运行时静默跳过，不影响会话。</p>
          <div className="mt-2.5 space-y-1.5">
            {HOOK_AGENTS.map((agent) => {
              const hook = hookStatusByAgent[agent.id]
              const enabled = injectEnabled(hook)
              const working = hookAction === agent.id
              return <div key={agent.id} className="flex flex-wrap items-center gap-2 rounded-lg border border-slate-200 bg-white px-3 py-2.5 dark:border-slate-700 dark:bg-slate-900/35">
                <span className="text-sm font-medium text-slate-700 dark:text-slate-200">{agent.label}</span>
                {enabled
                  ? <span className="inline-flex items-center gap-1 rounded-full bg-emerald-500/10 px-2 py-1 text-xs text-emerald-600 dark:text-emerald-400"><CheckCircle2 size={12} />启动注入已启用</span>
                  : <span className="inline-flex items-center gap-1 rounded-full bg-slate-100 px-2 py-1 text-xs text-slate-500 dark:bg-slate-700 dark:text-slate-400"><XCircle size={12} />{hook?.installed ? '未启用（采集 Hook 为旧版，缺少 SessionStart）' : '未启用'}</span>}
                {!enabled && <button onClick={() => { void handleHookInstall(agent.id, agent.label) }} disabled={working} title="幂等补装：追加 SessionStart 注入，不影响已有配置" className="ml-auto inline-flex shrink-0 items-center gap-1 rounded-md bg-violet-600 px-2.5 py-1 text-xs font-medium text-white transition-colors hover:bg-violet-500 disabled:opacity-50">{working ? <Loader2 size={13} className="animate-spin motion-reduce:animate-none" /> : <Zap size={13} />}启用注入</button>}
              </div>
            })}
          </div>
          <p className="mt-2 text-[11px] leading-4 text-slate-500 dark:text-slate-400">MiniMax Code 与 Kimi 没有 Hook 入口，无法使用启动注入。注入与采集共用同一份 Hook 配置，卸载请在上方「会话采集」区操作。</p>
        </div>

        <div className="mt-4 rounded-xl border border-violet-200/70 bg-violet-50/40 p-3.5 dark:border-violet-500/20 dark:bg-violet-500/5">
          <div className="flex flex-wrap items-center gap-2">
            <PlugZap size={14} className="text-violet-600 dark:text-violet-300" />
            <h3 className="text-xs font-semibold text-slate-700 dark:text-slate-200">共享记忆 MCP（主动检索）</h3>
            <span className="text-[11px] text-slate-500 dark:text-slate-400">会话中按需调用 · 可读取共享 Skill</span>
          </div>
          <p className="mt-1.5 text-[11px] leading-4 text-slate-500 dark:text-slate-400">接入后 Agent 可使用 <code className="rounded bg-violet-100 px-1 py-0.5 font-mono text-violet-800 dark:bg-violet-500/15 dark:text-violet-200">recall_memory</code> 检索上下文，并读取已发布的共享 Skill；配置保存在当前用户目录，重启 Agent 后生效。</p>
          <div className="mt-2.5 space-y-1.5">
            {MCP_ADAPTERS.map((adapter) => {
              const status = memoryMcp[adapter.type]
              const working = mcpAction === adapter.type
              return <div key={adapter.type} className="flex flex-wrap items-center gap-2 rounded-lg border border-slate-200 bg-white px-3 py-2.5 dark:border-slate-700 dark:bg-slate-900/35">
                <span className="text-sm font-medium text-slate-700 dark:text-slate-200">{adapter.label}</span>
                {status?.installed
                  ? <span className="inline-flex items-center gap-1 rounded-full bg-emerald-500/10 px-2 py-1 text-xs text-emerald-600 dark:text-emerald-400" title={status.detail}><CheckCircle2 size={12} />已连接共享记忆与 Skill</span>
                  : <span className="inline-flex items-center gap-1 rounded-full bg-slate-100 px-2 py-1 text-xs text-slate-500 dark:bg-slate-700 dark:text-slate-400" title={status?.detail}><XCircle size={12} />尚未连接</span>}
                {status?.installed
                  ? <button onClick={() => { void handleMcpUninstall(adapter.type) }} disabled={working} className="ml-auto inline-flex shrink-0 items-center gap-1 rounded-md border border-slate-300 px-2.5 py-1 text-xs text-slate-600 hover:bg-slate-50 disabled:opacity-50 dark:border-slate-600 dark:text-slate-300 dark:hover:bg-slate-700">{working ? <Loader2 size={13} className="animate-spin motion-reduce:animate-none" /> : <X size={13} />}断开</button>
                  : <button onClick={() => { void handleMcpInstall(adapter.type) }} disabled={working} className="ml-auto inline-flex shrink-0 items-center gap-1 rounded-md bg-violet-600 px-2.5 py-1 text-xs font-medium text-white transition-colors hover:bg-violet-500 disabled:opacity-50">{working ? <Loader2 size={13} className="animate-spin motion-reduce:animate-none" /> : <PlugZap size={13} />}一键连接</button>}
              </div>
            })}
          </div>
        </div>
      </section>

      <section className="mt-6 rounded-2xl border border-slate-200 bg-white p-5 shadow-[0_1px_2px_rgba(15,23,42,0.03)] dark:border-slate-800 dark:bg-slate-900 sm:p-6" aria-label="记忆注入摘要">
        <h2 className="text-sm font-semibold text-slate-700 dark:text-slate-200">记忆注入摘要</h2>
        <p className="mt-1 text-xs text-slate-500 dark:text-slate-400">同时记录 SessionStart 启动注入与共享记忆 MCP 检索，完整保存查询内容、记忆正文与 Skill 正文；点击任意记录可展开回放。每 10 秒自动刷新。</p>
        <div className="mt-3 space-y-1.5">
          {mcpAccessLogs.length === 0
            ? <p className="rounded-md bg-slate-50 px-2.5 py-2 text-xs text-slate-500 dark:bg-slate-900/40 dark:text-slate-400">尚未收到记忆注入调用。</p>
            : mcpAccessLogs.map((log) => {
              const expanded = expandedLogId === log.id
              return <div key={log.id} className="rounded-md bg-slate-50 dark:bg-slate-900/40">
                <div role="button" tabIndex={0} title={log.detail ? '点击展开完整内容' : undefined}
                  onClick={() => { if (log.detail) setExpandedLogId(expanded ? null : log.id) }}
                  onKeyDown={(event) => { if (log.detail && (event.key === 'Enter' || event.key === ' ')) { event.preventDefault(); setExpandedLogId(expanded ? null : log.id) } }}
                  className={`flex flex-wrap items-center gap-x-2 gap-y-1 px-2.5 py-2 text-xs ${log.detail ? 'cursor-pointer transition hover:bg-slate-100 dark:hover:bg-slate-800/60' : ''}`}>
                  <span className="font-mono text-slate-400">{displayFullTime(log.occurred_at)}</span>
                  <span className="font-medium text-slate-700 dark:text-slate-200">{log.client_name}</span>
                  {log.tool_name === 'session_start_inject' ? <span className="inline-flex items-center gap-1 rounded bg-violet-500/10 px-1.5 py-0.5 text-violet-700 dark:text-violet-300"><Zap size={11} />启动注入</span> : <code className="rounded bg-sky-500/10 px-1 py-0.5 text-sky-700 dark:text-sky-300">{log.tool_name}</code>}
                  <span className={`ml-auto ${log.success ? 'text-emerald-700 dark:text-emerald-300' : 'text-amber-700 dark:text-amber-300'}`}>{log.summary}</span>
                  {log.detail && <span className="shrink-0 text-[11px] text-slate-400">{expanded ? '收起' : '详情'}</span>}
                </div>
                {expanded && log.detail && <pre className="mx-2.5 mb-2 max-h-72 overflow-auto whitespace-pre-wrap break-words rounded border border-slate-200 bg-white p-2.5 font-mono text-[11px] leading-5 text-slate-700 dark:border-slate-700 dark:bg-slate-950/40 dark:text-slate-200">{formatLogDetail(log.detail)}</pre>}
              </div>
            })}
        </div>
      </section>
    </div>
  </main>
}
