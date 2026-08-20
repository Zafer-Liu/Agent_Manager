import { useEffect, useMemo, useState } from 'react'
import {
  ArrowLeft, BarChart3, CalendarDays, ChevronDown, Database, FileClock,
  Gauge, RefreshCw, Sparkles, Zap,
} from 'lucide-react'
import { useMemoryStore } from '../store/memoryStore'
import type { TelemetryUsageAnalytics, TelemetryUsageBucket, TelemetryUsageRecord } from '../types/memory'

type RangeKey = 'today' | '7d' | '30d' | 'all'
type TabKey = 'trend' | 'records'

const RANGES: { id: RangeKey; label: string }[] = [
  { id: 'today', label: '当天' },
  { id: '7d', label: '近 7 天' },
  { id: '30d', label: '近 30 天' },
  { id: 'all', label: '全部历史' },
]

function amount(value: number) { return new Intl.NumberFormat('zh-CN').format(value) }

function compactAmount(value: number) {
  if (value >= 100_000_000) return `${(value / 100_000_000).toFixed(2)} 亿`
  if (value >= 10_000) return `${(value / 10_000).toFixed(value >= 1_000_000 ? 1 : 0)} 万`
  return amount(value)
}

function sourceLabel(source: string) {
  return ({ codex: 'Codex', claude: 'Claude Code', qoder: 'Qoder', workbuddy: 'WorkBuddy', minimax: 'MiniMax Code', kimi: 'Kimi', gemini: 'Gemini CLI', opencode: 'OpenCode', openclaw: 'OpenClaw', pi: 'Pi', grokbuild: 'Grok Build' } as Record<string, string>)[source] ?? source
}

function parseTime(value: string) {
  return new Date(value.trim().replace(/(\.\d{3})\d+(?=(Z|[+-]\d{2}:\d{2})$)/, '$1'))
}

function displayTime(value: string) {
  const date = parseTime(value)
  return Number.isNaN(date.getTime()) ? value : new Intl.DateTimeFormat('zh-CN', { month: '2-digit', day: '2-digit', hour: '2-digit', minute: '2-digit', hour12: false }).format(date)
}

function tokenTotal(event: TelemetryUsageRecord) { return event.input_tokens + event.output_tokens }

function boundsFor(range: RangeKey) {
  if (range === 'all') return { bucket: 'day' as const }
  const now = new Date()
  const tomorrow = new Date(now.getFullYear(), now.getMonth(), now.getDate() + 1)
  const start = new Date(now.getFullYear(), now.getMonth(), now.getDate() - (range === 'today' ? 0 : range === '7d' ? 6 : 29))
  return { startAt: start.toISOString(), endAt: tomorrow.toISOString(), bucket: range === 'today' ? 'hour' as const : 'day' as const }
}

function bucketLabel(bucket: TelemetryUsageBucket, range: RangeKey) {
  if (range === 'today') return bucket.label.slice(11, 16)
  return range === 'all' && bucket.label.length >= 10 ? bucket.label.slice(0, 7).replace('-', '/') : bucket.label.slice(5).replace('-', '/')
}

function UsageTrend({ buckets, range }: { buckets: TelemetryUsageBucket[]; range: RangeKey }) {
  const [activeIndex, setActiveIndex] = useState<number | null>(null)
  const points = buckets
  const primaryMax = Math.max(1, ...points.flatMap(point => [point.input_tokens, point.cached_tokens]))
  const outputMax = Math.max(1, ...points.map(point => point.output_tokens))
  const primaryTicks = [primaryMax, Math.round(primaryMax * 0.75), Math.round(primaryMax * 0.5), Math.round(primaryMax * 0.25), 0]
  const outputTicks = [outputMax, Math.round(outputMax * 0.75), Math.round(outputMax * 0.5), Math.round(outputMax * 0.25), 0]
  const pointX = (index: number) => points.length <= 1 ? 50 : (index / (points.length - 1)) * 100
  const pointY = (value: number, scale: number = primaryMax) => 92 - (value / scale) * 82
  const coordinates = (field: 'input_tokens' | 'output_tokens' | 'cached_tokens') => points.map((point, index) => {
    return `${pointX(index)},${pointY(point[field], field === 'output_tokens' ? outputMax : primaryMax)}`
  }).join(' ')
  const fill = points.length ? `0,94 ${coordinates('input_tokens')} 100,94` : ''
  const labelIndexes = [...new Set([0, Math.round((points.length - 1) * .25), Math.round((points.length - 1) * .5), Math.round((points.length - 1) * .75), Math.max(0, points.length - 1)])]
  const activePoint = activeIndex === null ? null : points[activeIndex]

  function selectNearestPoint(clientX: number, target: SVGSVGElement) {
    const bounds = target.getBoundingClientRect()
    const ratio = Math.max(0, Math.min(1, (clientX - bounds.left) / Math.max(1, bounds.width)))
    setActiveIndex(Math.round(ratio * Math.max(0, points.length - 1)))
  }

  if (!points.length) return <div className="mt-5 flex h-64 items-center justify-center rounded-xl border border-dashed border-slate-200 bg-slate-50/70 text-sm text-slate-500 dark:border-slate-800 dark:bg-slate-950/40 dark:text-slate-400">所选日期范围内暂无可绘制的 Token 记录</div>

  return <div className="mt-5 grid grid-cols-[3.8rem_minmax(0,1fr)_3.8rem] gap-2" aria-label="Token 使用趋势图">
    <div className="flex h-56 flex-col justify-between pb-7 pt-1 text-right font-mono text-[11px] tabular-nums text-slate-500 dark:text-slate-400" aria-label="Token 坐标轴">
      {primaryTicks.map((tick, index) => <span key={`${tick}-${index}`}>{compactAmount(tick)}</span>)}
    </div>
    <div className="min-w-0">
      <div className="relative h-56 overflow-hidden rounded-xl border border-slate-100 bg-slate-50/70 px-2 pt-2 dark:border-slate-800 dark:bg-slate-950/40">
        <div className="pointer-events-none absolute inset-x-2 top-2 bottom-7 grid grid-rows-4 border-b border-dashed border-slate-200/80 dark:border-slate-800">
          {[0, 1, 2, 3].map(row => <div key={row} className="border-t border-dashed border-slate-200/80 dark:border-slate-800" />)}
        </div>
        <svg viewBox="0 0 100 100" preserveAspectRatio="none" className="relative z-10 h-[calc(100%-1.25rem)] w-full cursor-crosshair overflow-visible outline-none focus:outline-none" role="group" tabIndex={0} aria-label="输入、输出和缓存 Token 趋势；可悬停查看详情" onMouseMove={event => selectNearestPoint(event.clientX, event.currentTarget)} onMouseLeave={() => setActiveIndex(null)} onFocus={() => setActiveIndex(current => current ?? 0)} onKeyDown={event => {
          if (event.key !== 'ArrowLeft' && event.key !== 'ArrowRight') return
          event.preventDefault()
          setActiveIndex(current => Math.max(0, Math.min(points.length - 1, (current ?? 0) + (event.key === 'ArrowRight' ? 1 : -1))))
        }}>
          <defs><linearGradient id="usage-input-fill" x1="0" x2="0" y1="0" y2="1"><stop offset="0%" stopColor="#3b82f6" stopOpacity="0.24" /><stop offset="100%" stopColor="#3b82f6" stopOpacity="0" /></linearGradient></defs>
          <polygon points={fill} fill="url(#usage-input-fill)" />
          <polyline points={coordinates('input_tokens')} fill="none" stroke="#3b82f6" strokeWidth="1.5" vectorEffect="non-scaling-stroke" />
          <polyline points={coordinates('output_tokens')} fill="none" stroke="#8b5cf6" strokeWidth="1.5" vectorEffect="non-scaling-stroke" />
          <polyline points={coordinates('cached_tokens')} fill="none" stroke="#10b981" strokeWidth="1.25" vectorEffect="non-scaling-stroke" />
          {activePoint && activeIndex !== null && <>
            <line x1={pointX(activeIndex)} x2={pointX(activeIndex)} y1="4" y2="92" stroke="#94a3b8" strokeDasharray="2 3" strokeWidth="0.7" vectorEffect="non-scaling-stroke" />
          </>}
          <rect x="0" y="0" width="100" height="100" fill="transparent" />
        </svg>
        {activePoint && activeIndex !== null && <div className="pointer-events-none absolute inset-x-2 top-2 z-20 h-[calc(100%-1.25rem)]" aria-hidden="true">
          <TrendMarker x={pointX(activeIndex)} y={pointY(activePoint.input_tokens)} tone="bg-blue-500" />
          <TrendMarker x={pointX(activeIndex)} y={pointY(activePoint.output_tokens, outputMax)} tone="bg-violet-500" />
          <TrendMarker x={pointX(activeIndex)} y={pointY(activePoint.cached_tokens)} tone="bg-emerald-500" />
        </div>}
        {activePoint && activeIndex !== null && <div className={`pointer-events-none absolute top-3 z-20 w-48 rounded-lg border border-slate-200/90 bg-white/95 px-3 py-2.5 shadow-lg shadow-slate-900/10 backdrop-blur-sm dark:border-slate-700 dark:bg-slate-900/95 ${pointX(activeIndex) > 72 ? '-translate-x-full -ml-2' : 'ml-2'}`} style={{ left: `${pointX(activeIndex)}%` }} role="status" aria-live="polite">
          <p className="mb-2 font-mono text-xs font-semibold tabular-nums text-slate-700 dark:text-slate-200">{range === 'today' ? activePoint.label : activePoint.label.replaceAll('-', '/')}</p>
          <TooltipMetric label="输入" value={activePoint.input_tokens} tone="bg-blue-500" />
          <TooltipMetric label="输出" value={activePoint.output_tokens} tone="bg-violet-500" />
          <TooltipMetric label="缓存读取" value={activePoint.cached_tokens} tone="bg-emerald-500" />
        </div>}
        <div className="absolute inset-x-3 bottom-1.5 flex justify-between font-mono text-[11px] tabular-nums text-slate-500 dark:text-slate-400" aria-label="日期坐标轴">
          {labelIndexes.map(index => <span key={`${points[index].label}-${index}`}>{bucketLabel(points[index], range)}</span>)}
        </div>
      </div>
    </div>
    <div className="flex h-56 flex-col justify-between pb-7 pt-1 font-mono text-[11px] tabular-nums text-violet-500/85 dark:text-violet-300/85" aria-label="输出 Token 副坐标轴">
      {outputTicks.map((tick, index) => <span key={`${tick}-${index}`}>{compactAmount(tick)}</span>)}
    </div>
  </div>
}

export function UsageAnalytics({ onBack }: { onBack: () => void }) {
  const { loadUsageAnalytics, checkTelemetry } = useMemoryStore()
  const [tab, setTab] = useState<TabKey>('trend')
  const [range, setRange] = useState<RangeKey>('today')
  const [source, setSource] = useState('all')
  const [analytics, setAnalytics] = useState<TelemetryUsageAnalytics | null>(null)
  const [loading, setLoading] = useState(true)
  const [refreshing, setRefreshing] = useState(false)
  const filters = useMemo(() => boundsFor(range), [range])

  useEffect(() => { void checkTelemetry({ limit: 20 }) }, [checkTelemetry])
  useEffect(() => {
    let active = true
    setLoading(true)
    void loadUsageAnalytics({ ...filters, source }).then(result => { if (active) setAnalytics(result) }).catch(() => { if (active) setAnalytics(null) }).finally(() => { if (active) setLoading(false) })
    return () => { active = false }
  }, [filters, source, loadUsageAnalytics])

  const totals = { input: analytics?.input_tokens ?? 0, output: analytics?.output_tokens ?? 0, cache: analytics?.cached_tokens ?? 0 }
  const currentTotal = totals.input + totals.output
  const cacheRatio = totals.input > 0 ? (totals.cache / totals.input) * 100 : 0
  const selectedRange = RANGES.find(item => item.id === range)!

  async function refresh() {
    if (refreshing) return
    setRefreshing(true)
    try {
      await checkTelemetry({ limit: 20, refreshUsage: true })
      setLoading(true)
      setAnalytics(await loadUsageAnalytics({ ...filters, source }))
    } finally { setRefreshing(false); setLoading(false) }
  }

  return <main className="h-full overflow-y-auto bg-[#fbfcfe] text-slate-900 dark:bg-slate-950 dark:text-slate-100">
    <div className="mx-auto max-w-[1500px] px-5 py-6 lg:px-9 lg:py-8">
      <header className="flex flex-wrap items-start justify-between gap-4">
        <div className="flex items-start gap-3"><button type="button" onClick={onBack} className="mt-0.5 inline-flex h-9 w-9 items-center justify-center rounded-lg text-slate-500 transition hover:bg-slate-100 hover:text-slate-900 focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-500 dark:text-slate-400 dark:hover:bg-slate-800 dark:hover:text-white" aria-label="返回记忆中心"><ArrowLeft size={19} /></button><div><h1 className="text-2xl font-bold tracking-[-0.025em]">使用统计</h1><p className="mt-1 text-sm text-slate-500 dark:text-slate-400">按本地 SQLite 全量账本聚合 Token 使用与缓存，不受明细列表上限影响。</p></div></div>
        <div className="flex flex-wrap items-center gap-2">
          <label className="sr-only" htmlFor="usage-source">来源</label><div className="relative"><select id="usage-source" value={source} onChange={event => setSource(event.target.value)} className="h-10 appearance-none rounded-lg border border-slate-200 bg-white py-0 pl-3 pr-9 text-sm font-medium text-slate-700 outline-none transition hover:border-slate-300 focus:border-blue-500 focus:ring-2 focus:ring-blue-500/20 dark:border-slate-700 dark:bg-slate-900 dark:text-slate-200"><option value="all">全部来源</option>{(analytics?.sources ?? []).map(item => <option key={item} value={item}>{sourceLabel(item)}</option>)}</select><ChevronDown size={16} className="pointer-events-none absolute right-3 top-3 text-slate-400" /></div>
          <div className="relative"><select value={range} onChange={event => setRange(event.target.value as RangeKey)} className="h-10 appearance-none rounded-lg border border-slate-200 bg-white py-0 pl-9 pr-9 text-sm font-medium text-slate-700 outline-none transition hover:border-slate-300 focus:border-blue-500 focus:ring-2 focus:ring-blue-500/20 dark:border-slate-700 dark:bg-slate-900 dark:text-slate-200"><option value="today">当天</option><option value="7d">近 7 天</option><option value="30d">近 30 天</option><option value="all">全部历史</option></select><CalendarDays size={16} className="pointer-events-none absolute left-3 top-3 text-slate-400" /><ChevronDown size={16} className="pointer-events-none absolute right-3 top-3 text-slate-400" /></div>
          <button type="button" onClick={() => { void refresh() }} disabled={refreshing} className="inline-flex h-10 items-center gap-2 rounded-lg border border-slate-200 bg-white px-3 text-sm font-medium text-slate-700 transition hover:border-slate-300 hover:bg-slate-50 disabled:cursor-wait disabled:opacity-60 focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-500 dark:border-slate-700 dark:bg-slate-900 dark:text-slate-200 dark:hover:bg-slate-800"><RefreshCw size={16} className={refreshing ? 'animate-spin motion-reduce:animate-none' : ''} />重新扫描</button>
        </div>
      </header>

      <section className="mt-7 rounded-2xl border border-slate-200 bg-white p-5 shadow-[0_1px_2px_rgba(15,23,42,0.03)] dark:border-slate-800 dark:bg-slate-900 sm:p-7" aria-label="Token 总览">
        <div className="flex flex-wrap items-start justify-between gap-5"><div className="flex items-center gap-4"><span className="grid h-12 w-12 place-items-center rounded-2xl bg-blue-50 text-blue-500 dark:bg-blue-500/15 dark:text-blue-300"><Zap size={25} /></span><div><p className="text-sm font-medium text-slate-500 dark:text-slate-400">所选范围 Tokens</p><p className="mt-0.5 font-mono text-3xl font-semibold tracking-[-0.04em] tabular-nums sm:text-4xl">{loading ? '—' : amount(currentTotal)}</p><p className="mt-1 text-xs text-slate-400">{loading ? '正在查询本地账本…' : `约 ${compactAmount(currentTotal)} Tokens`}</p></div></div><div className="grid min-w-[14rem] grid-cols-2 divide-x divide-slate-100 rounded-xl border border-slate-100 dark:divide-slate-800 dark:border-slate-800"><div className="px-4 py-3"><p className="text-xs font-medium text-slate-500">账本记录</p><p className="mt-1 font-mono text-xl font-semibold tabular-nums">{loading ? '—' : amount(analytics?.record_count ?? 0)}</p></div><div className="px-4 py-3"><p className="text-xs font-medium text-slate-500">展示明细</p><p className="mt-1 text-sm font-semibold text-slate-600 dark:text-slate-300">{loading ? '—' : analytics?.truncated_records ? '最新 500 条' : '全部记录'}</p></div></div></div>
        <div className="mt-6 grid gap-3 sm:grid-cols-2 xl:grid-cols-4"><Metric icon={<BarChart3 size={16} />} label="输入" value={amount(totals.input)} tone="blue" /><Metric icon={<Sparkles size={16} />} label="输出" value={amount(totals.output)} tone="violet" /><Metric icon={<Database size={16} />} label="缓存读取" value={amount(totals.cache)} tone="emerald" /><div className="rounded-xl border border-slate-100 px-4 py-3 dark:border-slate-800"><div className="flex items-center justify-between text-sm text-slate-500"><span>缓存命中率</span><span className="font-semibold text-emerald-600 dark:text-emerald-400">{cacheRatio.toFixed(1)}%</span></div><div className="mt-3 h-1.5 overflow-hidden rounded-full bg-slate-100 dark:bg-slate-800"><div className="h-full rounded-full bg-emerald-500 transition-[width] duration-200 motion-reduce:transition-none" style={{ width: `${Math.min(100, cacheRatio)}%` }} /></div></div></div>
      </section>

      <section className="mt-7 rounded-2xl border border-slate-200 bg-white p-5 shadow-[0_1px_2px_rgba(15,23,42,0.03)] dark:border-slate-800 dark:bg-slate-900 sm:p-7"><div className="flex flex-wrap items-center justify-between gap-3"><div className="flex rounded-lg bg-slate-100 p-1 dark:bg-slate-800" role="tablist" aria-label="使用统计视图"><Tab active={tab === 'trend'} icon={<Gauge size={15} />} onClick={() => setTab('trend')}>使用趋势</Tab><Tab active={tab === 'records'} icon={<FileClock size={15} />} onClick={() => setTab('records')}>用量明细</Tab></div><p className="text-sm text-slate-500 dark:text-slate-400">{selectedRange.label} · {source === 'all' ? '全部来源' : sourceLabel(source)}</p></div>
        {tab === 'trend' ? <><div className="mt-6 flex items-center justify-between"><h2 className="text-lg font-semibold tracking-[-0.015em]">使用趋势</h2><div className="hidden items-center gap-3 text-xs text-slate-500 sm:flex"><Legend color="bg-blue-500" label="输入" /><Legend color="bg-violet-500" label="输出" /><Legend color="bg-emerald-500" label="缓存" /></div></div>{loading ? <div className="mt-5 h-64 animate-pulse rounded-xl bg-slate-100 dark:bg-slate-800" /> : <UsageTrend buckets={analytics?.buckets ?? []} range={range} />}</> : <UsageRecords events={analytics?.records ?? []} truncated={Boolean(analytics?.truncated_records)} />}
      </section>
    </div>
  </main>
}

function Metric({ icon, label, value, tone }: { icon: React.ReactNode; label: string; value: string; tone: 'blue' | 'violet' | 'emerald' }) { const tones = { blue: 'text-blue-500', violet: 'text-violet-500', emerald: 'text-emerald-500' }; return <div className="rounded-xl border border-slate-100 px-4 py-3 dark:border-slate-800"><div className="flex items-center gap-2 text-sm text-slate-500"><span className={tones[tone]}>{icon}</span>{label}</div><p className="mt-2 font-mono text-xl font-semibold tracking-[-0.02em] tabular-nums">{value}</p></div> }
function Tab({ active, icon, children, onClick }: { active: boolean; icon: React.ReactNode; children: React.ReactNode; onClick: () => void }) { return <button type="button" role="tab" aria-selected={active} onClick={onClick} className={`inline-flex items-center gap-2 rounded-md px-3 py-2 text-sm font-medium transition focus:outline-none focus-visible:ring-2 focus-visible:ring-blue-500 ${active ? 'bg-white text-blue-600 shadow-sm dark:bg-slate-700 dark:text-blue-300' : 'text-slate-500 hover:text-slate-800 dark:text-slate-400 dark:hover:text-slate-100'}`}>{icon}{children}</button> }
function Legend({ color, label }: { color: string; label: string }) { return <span className="inline-flex items-center gap-1.5"><i className={`h-2 w-2 rounded-full ${color}`} />{label}</span> }
function TooltipMetric({ label, value, tone }: { label: string; value: number; tone: string }) { return <p className="flex items-center justify-between gap-3 font-mono text-xs leading-6 tabular-nums text-slate-600 dark:text-slate-300"><span className="inline-flex items-center gap-1.5"><i className={`h-2 w-2 rounded-full ${tone}`} />{label}</span><span>{amount(value)}</span></p> }
function TrendMarker({ x, y, tone }: { x: number; y: number; tone: string }) { return <i className={`absolute h-2.5 w-2.5 -translate-x-1/2 -translate-y-1/2 rounded-full border-2 border-white shadow-sm dark:border-slate-900 ${tone}`} style={{ left: `${x}%`, top: `${y}%` }} /> }

function UsageRecords({ events, truncated }: { events: TelemetryUsageRecord[]; truncated: boolean }) {
  if (!events.length) return <div className="flex min-h-64 flex-col items-center justify-center text-center"><span className="grid h-11 w-11 place-items-center rounded-xl bg-slate-100 text-slate-400 dark:bg-slate-800"><FileClock size={20} /></span><h2 className="mt-3 font-medium">此筛选范围内暂无用量记录</h2><p className="mt-1 max-w-md text-sm text-slate-500">Agent 完成并被本地账本捕获后，Token 用量会显示在这里。</p></div>
  return <><div className="mt-4 flex items-center justify-between text-xs text-slate-500 dark:text-slate-400"><span>按时间倒序</span>{truncated && <span>明细较多，仅显示最新 500 条；上方汇总和趋势仍为全量。</span>}</div><div className="mt-2 overflow-x-auto rounded-xl border border-slate-100 dark:border-slate-800"><table className="min-w-[850px] w-full text-left text-sm"><thead className="border-b border-slate-100 bg-slate-50/80 text-xs font-medium text-slate-500 dark:border-slate-800 dark:bg-slate-950/40"><tr><th className="px-5 py-3.5">时间</th><th className="px-5 py-3.5">来源</th><th className="px-5 py-3.5">模型 / 记录</th><th className="px-5 py-3.5 text-right">输入</th><th className="px-5 py-3.5 text-right">输出</th><th className="px-5 py-3.5 text-right">缓存</th><th className="px-5 py-3.5 text-right">合计</th><th className="px-5 py-3.5">数据来源</th></tr></thead><tbody className="divide-y divide-slate-100 dark:divide-slate-800">{events.map(event => <tr key={event.record_id} className="transition hover:bg-slate-50/70 dark:hover:bg-slate-800/40"><td className="whitespace-nowrap px-5 py-4 font-mono text-xs text-slate-500">{displayTime(event.occurred_at)}</td><td className="px-5 py-4 font-medium">{sourceLabel(event.source)}</td><td className="px-5 py-4 text-slate-600 dark:text-slate-300"><span className="rounded bg-slate-100 px-2 py-1 text-xs dark:bg-slate-800">{event.model ?? (event.record_kind === 'session_total' ? '会话总量' : '未标注模型')}</span></td><td className="px-5 py-4 text-right font-mono tabular-nums">{amount(event.input_tokens)}</td><td className="px-5 py-4 text-right font-mono tabular-nums">{amount(event.output_tokens)}</td><td className="px-5 py-4 text-right font-mono tabular-nums text-emerald-600 dark:text-emerald-400">{amount(event.cached_tokens)}</td><td className="px-5 py-4 text-right font-mono font-semibold tabular-nums">{amount(tokenTotal(event))}</td><td className="px-5 py-4"><span className={event.origin.startsWith('native') || event.origin.startsWith('adapter') ? 'text-emerald-600 dark:text-emerald-400' : 'text-amber-600 dark:text-amber-400'}>{event.origin.startsWith('native') ? '原生日志' : event.origin.startsWith('adapter') ? 'Agent 上报' : '估算'}</span></td></tr>)}</tbody></table></div></>
}
