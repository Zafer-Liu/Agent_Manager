import { useEffect, useMemo, useState } from 'react'
import { useTranslation } from 'react-i18next'
import {
  BookOpenText, Check, ChevronRight, FileCode2, FolderSync, RefreshCw,
  Search, Sparkles, Copy, CheckCheck, Hash, Globe, ArrowRightLeft,
  PlusCircle, PencilLine, ShieldCheck, AlertTriangle, Layers, RotateCcw,
  Loader2,
} from 'lucide-react'
import { useMemoryStore } from '../store/memoryStore'
import type { SkillDocument, SkillItem, SkillSyncPreview } from '../types/memory'

type SourceFilter = 'all' | 'codex' | 'claude' | 'qoder' | 'workbuddy'

const SOURCES: { id: Exclude<SourceFilter, 'all'>; label: string; color: string }[] = [
  { id: 'codex',    label: 'Codex',     color: 'bg-emerald-500' },
  { id: 'claude',   label: 'Claude Code', color: 'bg-amber-500' },
  { id: 'qoder',    label: 'Qoder',      color: 'bg-sky-500' },
  { id: 'workbuddy', label: 'WorkBuddy', color: 'bg-rose-500' },
]

const sourceColor = (source: string) => SOURCES.find(s => s.id === source)?.color ?? 'bg-gray-400'

export function SkillLibrary() {
  const { t } = useTranslation()
  const { skills, skillCacheReady, loadSkills, scanSkills, readSkill, previewSkillSync, applySkillSync, setSkillStatus, setSkillAssignment, rollbackSkillLatest } = useMemoryStore()
  const [filter, setFilter] = useState<SourceFilter>('all')
  const [query, setQuery] = useState('')
  const [selected, setSelected] = useState<SkillDocument | null>(null)
  const [target, setTarget] = useState('codex')
  const [preview, setPreview] = useState<SkillSyncPreview | null>(null)
  const [loading, setLoading] = useState(false)
  const [notice, setNotice] = useState<string | null>(null)
  const [copied, setCopied] = useState(false)
  const [showMobileSync, setShowMobileSync] = useState(false)
  // Shared catalog cache survives page navigation; only its first creation
  // deserves a blocking loading state.
  const [booting, setBooting] = useState(() => !skillCacheReady)

  useEffect(() => {
    let mounted = true
    void loadSkills().catch(() => setNotice('无法读取共享 Skill 库')).finally(() => {
      if (mounted) setBooting(false)
    })
    return () => { mounted = false }
  }, [loadSkills])

  // ── Derived data ──

  const visibleSkills = useMemo(() => skills.filter((skill) => {
    const sourceMatches = filter === 'all' || skill.source === filter
    const q = query.trim().toLowerCase()
    return sourceMatches && (!q || `${skill.name} ${skill.description} ${skill.source}`.toLowerCase().includes(q))
  }), [skills, filter, query])

  const countsBySource = useMemo(() => {
    const map: Record<string, number> = {}
    for (const s of skills) map[s.source] = (map[s.source] || 0) + 1
    return map
  }, [skills])

  // ── Actions ──

  async function selectSkill(skill: SkillItem) {
    setLoading(true)
    try {
      setSelected(await readSkill(skill.source, skill.name))
    } catch (error) {
      setNotice(`读取 Skill 失败：${String(error)}`)
    } finally {
      setLoading(false)
    }
  }

  async function importSkills() {
    setLoading(true)
    try {
      const imported = await scanSkills()
      setNotice(`已扫描并导入 ${imported.length} 个 Skill`)
      if (selected && !imported.some((skill) => skill.source === selected.item.source && skill.name === selected.item.name)) setSelected(null)
    } catch (error) {
      setNotice(`扫描失败：${String(error)}`)
    } finally {
      setLoading(false)
    }
  }

  async function buildPreview() {
    setLoading(true)
    try {
      setPreview(await previewSkillSync(target))
    } catch (error) {
      setNotice(`无法生成同步预览：${String(error)}`)
    } finally {
      setLoading(false)
    }
  }

  async function applySync(overwrite: boolean) {
    setLoading(true)
    try {
      const result = await applySkillSync(target, overwrite)
      setPreview(result)
      setNotice(overwrite ? '同步并覆盖已确认的差异' : '仅同步新增 Skill')
    } catch (error) {
      setNotice(`同步失败：${String(error)}`)
    } finally {
      setLoading(false)
    }
  }

  async function copyHash(hash: string) {
    await navigator.clipboard.writeText(hash)
    setCopied(true)
    setTimeout(() => setCopied(false), 2000)
  }

  async function reloadSelected(source: string, name: string) {
    setSelected(await readSkill(source, name))
  }

  async function updateSkillStatus(status: 'draft' | 'published') {
    if (!selected) return
    setLoading(true)
    try {
      await setSkillStatus(selected.item.source, selected.item.name, status)
      await reloadSelected(selected.item.source, selected.item.name)
      setNotice(status === 'published' ? 'Skill 已发布，可装备到 Agent' : 'Skill 已转为草稿，不会参与同步')
    } catch (error) {
      setNotice(`更新状态失败：${String(error)}`)
    } finally {
      setLoading(false)
    }
  }

  async function updateAssignment(target: string, equipped: boolean) {
    if (!selected) return
    setLoading(true)
    try {
      await setSkillAssignment(selected.item.source, selected.item.name, target, equipped)
      await reloadSelected(selected.item.source, selected.item.name)
      setNotice(equipped ? `已装备到 ${target}` : `已从 ${target} 卸下`)
    } catch (error) {
      setNotice(`更新装备失败：${String(error)}`)
    } finally {
      setLoading(false)
    }
  }

  async function rollbackLatest() {
    if (!selected) return
    setLoading(true)
    try {
      await rollbackSkillLatest(selected.item.source, selected.item.name)
      await reloadSelected(selected.item.source, selected.item.name)
      setNotice('已恢复到上一份版本快照；请重新发布后同步')
    } catch (error) {
      setNotice(`回滚不可用：${String(error)}`)
    } finally {
      setLoading(false)
    }
  }

  // ── Render ──

  if (booting) {
    return <div className="flex h-full min-h-0 items-center justify-center bg-gray-50 px-6 dark:bg-gray-950" role="status" aria-live="polite">
      <div className="w-full max-w-md rounded-2xl border border-gray-200 bg-white p-6 shadow-sm dark:border-gray-800 dark:bg-gray-900">
        <div className="flex items-center gap-3 text-gray-800 dark:text-gray-100"><span className="rounded-xl bg-violet-500/10 p-2.5 text-violet-600 dark:text-violet-400"><Loader2 className="animate-spin motion-reduce:animate-none" size={21} /></span><div><p className="text-sm font-semibold">正在打开 Skill 库</p><p className="mt-0.5 text-xs text-gray-500 dark:text-gray-400">读取共享 Skill 索引与同步状态…</p></div></div>
        <div className="mt-5 grid grid-cols-2 gap-2.5" aria-hidden="true"><div className="h-16 animate-pulse rounded-lg bg-gray-100 dark:bg-gray-800" /><div className="h-16 animate-pulse rounded-lg bg-gray-100 dark:bg-gray-800" /><div className="col-span-2 h-10 animate-pulse rounded-lg bg-gray-100 dark:bg-gray-800" /></div>
      </div>
    </div>
  }

  return (
    <div className="flex h-full min-h-0 flex-col bg-gray-50 dark:bg-gray-950">
      {/* ── Header ── */}
      <header className="flex flex-wrap items-start gap-4 border-b border-gray-200 bg-white px-6 py-5 dark:border-gray-800 dark:bg-gray-900">
        <div className="flex items-center gap-3 min-w-0">
          <div className="rounded-xl bg-violet-500/10 p-2.5 text-violet-600 dark:text-violet-400">
            <BookOpenText size={22} />
          </div>
          <div className="min-w-0">
            <h1 className="text-base font-semibold text-gray-900 dark:text-gray-100">{t('skills.title')}</h1>
            <p className="text-xs text-gray-500 dark:text-gray-400 mt-0.5">{t('skills.subtitle')}</p>
          </div>
        </div>

        {/* source stat dots */}
        <div className="hidden sm:flex items-center gap-3 ml-6 text-xs text-gray-500 dark:text-gray-400">
          {SOURCES.map(s => (
            <div key={s.id} className="flex items-center gap-1.5">
              <span className={`inline-block h-2 w-2 rounded-full ${s.color}`} />
              <span>{s.label}</span>
              <span className="font-mono font-medium text-gray-700 dark:text-gray-300">{countsBySource[s.id] ?? 0}</span>
            </div>
          ))}
        </div>

        <div className="flex items-center gap-2 ml-auto">
          {notice && (
            <span role="status" className="max-w-52 truncate text-xs text-gray-500 dark:text-gray-400">{notice}</span>
          )}
          <button
            onClick={importSkills}
            disabled={loading}
            className="inline-flex items-center gap-2 rounded-lg bg-violet-600 px-4 py-2 text-xs font-medium text-white transition-all hover:bg-violet-500 active:scale-[0.97] disabled:opacity-50"
          >
            <RefreshCw size={14} className={loading ? 'animate-spin' : ''} />
            {t('skills.scan')}
          </button>

          {/* mobile sync trigger */}
          <button
            onClick={() => setShowMobileSync(v => !v)}
            className="inline-flex items-center gap-1.5 rounded-lg border border-gray-300 px-3 py-2 text-xs font-medium text-gray-700 hover:bg-gray-50 dark:border-gray-600 dark:text-gray-200 dark:hover:bg-gray-800 lg:hidden"
          >
            <ArrowRightLeft size={14} />
            {t('skills.syncTitle')}
          </button>
        </div>
      </header>

      {/* ── Mobile sync panel (collapsible) ── */}
      {showMobileSync && (
        <div className="border-b border-gray-200 bg-white px-4 py-4 dark:border-gray-800 dark:bg-gray-900 lg:hidden">
          <SyncPanel
            t={t}
            target={target}
            setTarget={(v) => { setTarget(v); setPreview(null) }}
            preview={preview}
            loading={loading}
            skillsCount={skills.length}
            onPreview={buildPreview}
            onSyncNew={() => applySync(false)}
            onSyncOverwrite={() => applySync(true)}
          />
        </div>
      )}

      {/* ── Main grid ── */}
      <div className="grid min-h-0 flex-1 grid-cols-[minmax(220px,0.8fr)_minmax(0,1.6fr)] lg:grid-cols-[240px_minmax(0,1fr)_260px]">
        {/* ── Left: skill list ── */}
        <aside className="flex min-h-0 flex-col border-r border-gray-200 bg-white dark:border-gray-800 dark:bg-gray-900">
          {/* search + filters */}
          <div className="space-y-3 border-b border-gray-200 p-3 dark:border-gray-800">
            <label className="relative block">
              <Search size={14} className="pointer-events-none absolute left-2.5 top-2.5 text-gray-400" />
              <input
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder={t('skills.search')}
                className="w-full rounded-lg border border-gray-300 bg-white py-1.5 pl-8 pr-3 text-xs text-gray-800 outline-none transition focus:border-violet-400 focus:ring-2 focus:ring-violet-400/20 dark:border-gray-600 dark:bg-gray-800 dark:text-gray-100 dark:focus:border-violet-500"
              />
            </label>
            <div className="flex flex-wrap gap-1.5">
              <FilterPill active={filter === 'all'} onClick={() => setFilter('all')}>
                {t('skills.all')} <span className="ml-1 font-mono opacity-70">{skills.length}</span>
              </FilterPill>
              {SOURCES.map(s => (
                <FilterPill key={s.id} active={filter === s.id} onClick={() => setFilter(s.id)}>
                  <span className={`mr-1 inline-block h-1.5 w-1.5 rounded-full ${s.color}`} />
                  {s.label}
                </FilterPill>
              ))}
            </div>
          </div>

          {/* skill list */}
          <div className="min-h-0 flex-1 overflow-y-auto p-2">
            {visibleSkills.map((skill) => {
              const isSelected = selected?.item.source === skill.source && selected.item.name === skill.name
              return (
                <button
                  key={`${skill.source}:${skill.name}`}
                  onClick={() => selectSkill(skill)}
                  className={`group mb-0.5 w-full rounded-lg px-3 py-2.5 text-left transition-all ${
                    isSelected
                      ? 'border-l-[3px] border-l-violet-500 bg-violet-50 pl-2.5 text-violet-900 dark:border-l-violet-400 dark:bg-violet-500/10 dark:text-violet-100'
                      : 'border-l-[3px] border-l-transparent pl-2.5 hover:bg-gray-100 dark:hover:bg-gray-800'
                  }`}
                >
                  <div className="flex items-center gap-2">
                    <span className={`inline-block h-2 w-2 shrink-0 rounded-full ${sourceColor(skill.source)}`} />
                    <FileCode2 size={14} className={isSelected ? 'text-violet-600 dark:text-violet-400' : 'text-gray-400'} />
                    <span className="min-w-0 flex-1 truncate text-sm font-medium">{skill.name}</span>
                    <ChevronRight size={14} className="shrink-0 text-gray-400 opacity-0 transition group-hover:opacity-100" />
                  </div>
                  <p className="mt-1 truncate pl-8 text-xs text-gray-500 dark:text-gray-400">
                    {skill.description || skill.source}
                  </p>
                </button>
              )
            })}
            {!visibleSkills.length && (
              <div className="px-4 py-12 text-center">
                <div className="mx-auto mb-3 flex h-14 w-14 items-center justify-center rounded-2xl bg-amber-100 dark:bg-amber-900/30">
                  <Sparkles size={24} className="text-amber-600 dark:text-amber-400" />
                </div>
                <p className="text-sm font-medium text-gray-700 dark:text-gray-200">{t('skills.emptyTitle')}</p>
                <p className="mt-1 text-xs leading-5 text-gray-500 dark:text-gray-400">{t('skills.emptyHint')}</p>
              </div>
            )}
          </div>
        </aside>

        {/* ── Center: content viewer ── */}
        <section className="min-h-0 min-w-0 overflow-y-auto p-5">
          {selected ? (
            <div className="space-y-4">
              {/* metadata card */}
              <div className="rounded-2xl border border-gray-200 bg-white p-5 dark:border-gray-700 dark:bg-gray-900">
                <div className="flex items-start gap-3">
                  <div className="rounded-lg bg-violet-500/10 p-2 text-violet-600 dark:text-violet-400">
                    <FileCode2 size={18} />
                  </div>
                  <div className="min-w-0 flex-1">
                    <h2 className="text-lg font-semibold text-gray-900 dark:text-gray-100">{selected.item.name}</h2>
                    <p className="mt-0.5 text-sm text-gray-500 dark:text-gray-400">
                      {selected.item.description || t('skills.noDescription')}
                    </p>
                    <div className="mt-3 flex flex-wrap items-center gap-3 text-xs">
                      <span className="inline-flex items-center gap-1 rounded-full bg-gray-100 px-2 py-0.5 font-medium text-gray-600 dark:bg-gray-800 dark:text-gray-300">
                        <Globe size={11} />
                        {selected.item.source}
                      </span>
                      <button
                        onClick={() => copyHash(selected.item.hash)}
                        className="inline-flex items-center gap-1 rounded-full bg-gray-100 px-2 py-0.5 font-mono text-gray-500 transition hover:bg-gray-200 dark:bg-gray-800 dark:text-gray-400 dark:hover:bg-gray-700"
                      >
                        <Hash size={11} />
                        {selected.item.hash.slice(0, 16)}…
                        {copied ? <CheckCheck size={12} className="text-emerald-500" /> : <Copy size={12} />}
                      </button>
                      <span className={`rounded-full px-2 py-0.5 font-medium ${selected.item.status === 'published' ? 'bg-emerald-500/10 text-emerald-700 dark:text-emerald-300' : 'bg-amber-500/10 text-amber-800 dark:text-amber-300'}`}>
                        {selected.item.status === 'published' ? '已发布' : '草稿'} · v{selected.item.version}
                      </span>
                    </div>
                  </div>
                </div>
              </div>

              <div className="rounded-xl border border-gray-200 bg-white p-4 dark:border-gray-700 dark:bg-gray-900">
                <div className="flex flex-wrap items-center gap-2">
                  <ShieldCheck size={15} className="text-violet-600 dark:text-violet-400" />
                  <h3 className="text-sm font-semibold text-gray-800 dark:text-gray-100">发布与 Agent 装备</h3>
                  <span className="text-xs text-gray-500 dark:text-gray-400">只有已发布且已装备的 Skill 会进入目标 Agent 同步预览。</span>
                </div>
                <div className="mt-3 flex flex-wrap items-center gap-2">
                  <button disabled={loading || selected.item.status === 'published'} onClick={() => { void updateSkillStatus('published') }} className="inline-flex items-center gap-1.5 rounded-md bg-violet-600 px-2.5 py-1.5 text-xs font-medium text-white hover:bg-violet-500 disabled:opacity-50"><Check size={13} />发布</button>
                  <button disabled={loading || selected.item.status === 'draft'} onClick={() => { void updateSkillStatus('draft') }} className="inline-flex items-center gap-1.5 rounded-md border border-gray-300 px-2.5 py-1.5 text-xs font-medium text-gray-700 hover:bg-gray-50 disabled:opacity-50 dark:border-gray-600 dark:text-gray-200 dark:hover:bg-gray-800">转为草稿</button>
                  <button disabled={loading} onClick={() => { void rollbackLatest() }} className="ml-auto inline-flex items-center gap-1.5 rounded-md border border-gray-300 px-2.5 py-1.5 text-xs font-medium text-gray-700 hover:bg-gray-50 disabled:opacity-50 dark:border-gray-600 dark:text-gray-200 dark:hover:bg-gray-800"><RotateCcw size={13} />恢复上一版</button>
                </div>
                <div className="mt-3 flex flex-wrap gap-2">
                  {SOURCES.map((agent) => {
                    const equipped = selected.item.assigned_agents.includes(agent.id)
                    return <button key={agent.id} type="button" disabled={loading} onClick={() => { void updateAssignment(agent.id, !equipped) }} className={`inline-flex items-center gap-1.5 rounded-full border px-2.5 py-1 text-xs font-medium transition disabled:opacity-50 ${equipped ? 'border-violet-500 bg-violet-50 text-violet-700 dark:bg-violet-500/20 dark:text-violet-200' : 'border-gray-300 text-gray-600 hover:bg-gray-50 dark:border-gray-600 dark:text-gray-300 dark:hover:bg-gray-800'}`}><span className={`h-1.5 w-1.5 rounded-full ${agent.color}`} />{equipped && <Check size={11} />}{agent.label}</button>
                  })}
                </div>
              </div>

              {/* content card */}
              <div className="rounded-2xl border border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-900">
                <div className="flex items-center gap-2 border-b border-gray-100 px-5 py-3 dark:border-gray-800">
                  <Layers size={14} className="text-gray-400" />
                  <span className="text-xs font-semibold uppercase tracking-wide text-gray-500">SKILL.md</span>
                </div>
                <pre className="overflow-x-auto whitespace-pre-wrap p-5 text-xs leading-6 text-gray-700 dark:text-gray-200 font-mono">
                  {selected.content}
                </pre>
              </div>
            </div>
          ) : (
            <div className="flex h-full flex-col items-center justify-center text-center">
              <div className="relative mb-4">
                <div className="absolute -inset-1 rounded-full bg-violet-100 opacity-50 blur dark:bg-violet-900/30" />
                <div className="relative flex h-16 w-16 items-center justify-center rounded-2xl bg-gray-100 dark:bg-gray-800">
                  <BookOpenText size={28} className="text-gray-400 dark:text-gray-500" />
                </div>
              </div>
              <h2 className="text-sm font-medium text-gray-700 dark:text-gray-200">{t('skills.selectTitle')}</h2>
              <p className="mt-1 max-w-sm text-xs leading-5 text-gray-500 dark:text-gray-400">{t('skills.selectHint')}</p>
            </div>
          )}
        </section>

        {/* ── Right: sync panel (desktop) ── */}
        <aside className="hidden min-h-0 min-w-0 overflow-y-auto border-l border-gray-200 bg-white p-4 dark:border-gray-800 dark:bg-gray-900 lg:block">
          <SyncPanel
            t={t}
            target={target}
            setTarget={(v) => { setTarget(v); setPreview(null) }}
            preview={preview}
            loading={loading}
            skillsCount={skills.length}
            onPreview={buildPreview}
            onSyncNew={() => applySync(false)}
            onSyncOverwrite={() => applySync(true)}
          />
        </aside>
      </div>
    </div>
  )
}

// ── Sub-components ──

function FilterPill({ active, onClick, children }: { active: boolean; onClick: () => void; children: React.ReactNode }) {
  return (
    <button
      onClick={onClick}
      className={`inline-flex items-center rounded-full px-2.5 py-1 text-xs font-medium transition-all ${
        active
          ? 'bg-violet-600 text-white shadow-sm shadow-violet-500/25'
          : 'bg-gray-100 text-gray-600 hover:bg-gray-200 dark:bg-gray-800 dark:text-gray-300 dark:hover:bg-gray-700'
      }`}
    >
      {children}
    </button>
  )
}

function SyncPanel({
  t, target, setTarget, preview, loading, skillsCount,
  onPreview, onSyncNew, onSyncOverwrite,
}: {
  t: (key: string) => string
  target: string
  setTarget: (v: string) => void
  preview: SkillSyncPreview | null
  loading: boolean
  skillsCount: number
  onPreview: () => void
  onSyncNew: () => void
  onSyncOverwrite: () => void
}) {
  return (
    <div className="space-y-4">
      {/* header */}
      <div className="flex items-center gap-2">
        <div className="rounded-lg bg-violet-500/10 p-1.5 text-violet-600 dark:text-violet-400">
          <ArrowRightLeft size={16} />
        </div>
        <div>
          <h2 className="text-sm font-semibold text-gray-800 dark:text-gray-100">{t('skills.syncTitle')}</h2>
        </div>
      </div>

      <p className="text-xs leading-5 text-gray-500 dark:text-gray-400">{t('skills.syncHint')}</p>

      {/* controls card */}
      <div className="rounded-xl border border-gray-200 bg-gray-50/50 p-3 dark:border-gray-700 dark:bg-gray-800/50">
        <label className="mb-2 block text-xs font-medium text-gray-600 dark:text-gray-400">目标 Agent</label>
        <select
          value={target}
          onChange={(e) => setTarget(e.target.value)}
          className="w-full rounded-lg border border-gray-300 bg-white px-3 py-2 text-sm text-gray-800 outline-none transition focus:border-violet-400 focus:ring-2 focus:ring-violet-400/20 dark:border-gray-600 dark:bg-gray-800 dark:text-gray-100"
        >
          <option value="codex">Codex</option>
          <option value="claude">Claude Code</option>
          <option value="qoder">Qoder</option>
          <option value="workbuddy">WorkBuddy</option>
        </select>
        <button
          onClick={onPreview}
          disabled={loading || !skillsCount}
          className="mt-2.5 inline-flex w-full items-center justify-center gap-2 rounded-lg border border-gray-300 bg-white px-3 py-2 text-xs font-medium text-gray-700 transition-all hover:bg-gray-100 active:scale-[0.98] disabled:opacity-50 dark:border-gray-600 dark:bg-gray-800 dark:text-gray-200 dark:hover:bg-gray-700"
        >
          <FolderSync size={13} className={loading ? 'animate-spin' : ''} />
          {t('skills.preview')}
        </button>
      </div>

      {/* preview results */}
      {preview && (
        <div className="space-y-3 rounded-xl border border-gray-200 p-4 dark:border-gray-700">
          <p className="text-xs font-medium text-gray-700 dark:text-gray-200">同步差异摘要</p>

          <div className="grid grid-cols-2 gap-2">
            <SyncStatBadge icon={<PlusCircle size={12} />} label={t('skills.create')} count={preview.create.length} color="emerald" />
            <SyncStatBadge icon={<PencilLine size={12} />} label={t('skills.update')} count={preview.update.length} color="amber" />
            <SyncStatBadge icon={<AlertTriangle size={12} />} label={t('skills.conflict')} count={preview.conflict.length} color="rose" />
            <SyncStatBadge icon={<ShieldCheck size={12} />} label={t('skills.unchanged')} count={preview.unchanged.length} color="gray" />
          </div>

          <div className="border-t border-gray-100 pt-3 dark:border-gray-700 space-y-2">
            <button
              onClick={onSyncNew}
              disabled={loading || !preview.create.length}
              className="inline-flex w-full items-center justify-center gap-2 rounded-lg bg-violet-600 px-3 py-2 text-xs font-medium text-white transition-all hover:bg-violet-500 active:scale-[0.98] disabled:opacity-50"
            >
              <Check size={13} />
              {t('skills.syncNew')}
            </button>
            {(preview.update.length > 0 || preview.conflict.length > 0) && (
              <button
                onClick={onSyncOverwrite}
                disabled={loading}
                className="inline-flex w-full items-center justify-center gap-2 rounded-lg border border-amber-500/60 bg-white px-3 py-2 text-xs font-medium text-amber-700 transition-all hover:bg-amber-50 active:scale-[0.98] disabled:opacity-50 dark:border-amber-400/40 dark:text-amber-300 dark:hover:bg-amber-900/20"
              >
                <FolderSync size={13} />
                {t('skills.syncOverwrite')}
              </button>
            )}
          </div>
        </div>
      )}
    </div>
  )
}

function SyncStatBadge({ icon, label, count, color }: { icon: React.ReactNode; label: string; count: number; color: string }) {
  const colorMap: Record<string, string> = {
    emerald: 'bg-emerald-50 text-emerald-700 border-emerald-200 dark:bg-emerald-500/10 dark:text-emerald-400 dark:border-emerald-500/20',
    amber:   'bg-amber-50   text-amber-700   border-amber-200   dark:bg-amber-500/10   dark:text-amber-400   dark:border-amber-500/20',
    rose:    'bg-rose-50    text-rose-700    border-rose-200    dark:bg-rose-500/10    dark:text-rose-400    dark:border-rose-500/20',
    gray:    'bg-gray-50    text-gray-600    border-gray-200    dark:bg-gray-800      dark:text-gray-400    dark:border-gray-700',
  }

  return (
    <div className={`flex items-center gap-2 rounded-lg border px-3 py-2 text-xs font-medium ${colorMap[color] ?? colorMap.gray}`}>
      <span className="shrink-0 opacity-70">{icon}</span>
      <span className="min-w-0 flex-1 truncate">{label}</span>
      <span className="font-mono font-semibold tabular-nums">{count}</span>
    </div>
  )
}
