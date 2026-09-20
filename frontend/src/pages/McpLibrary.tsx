import { memo, useCallback, useEffect, useMemo, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import { useTranslation } from 'react-i18next'
import { McpServersManager, type McpServer } from '../components/McpServersManager'
import {
  Plug, Plus, Download, RefreshCw, Search, Trash2, Pencil, Loader2,
  Terminal, Globe, X, Check, FolderOpen,
} from 'lucide-react'
import { useMemoryStore } from '../store/memoryStore'
import type { McpAgentStatus, McpCatalogEntry, McpImportCandidate } from '../types/memory'

/** 装备目标：id 与后端 MemoryMcpTarget 一致；remote 标记该目标支持 SSE/HTTP。 */
const TARGETS = [
  { id: 'claude_cli', label: 'Claude Code', color: 'bg-amber-500', remote: true },
  { id: 'claude_desktop', label: 'Claude Desktop', color: 'bg-orange-500', remote: true },
  { id: 'codex_cli', label: 'Codex CLI', color: 'bg-emerald-500', remote: false },
  { id: 'codex_desktop', label: 'Codex Desktop', color: 'bg-teal-500', remote: false },
  { id: 'qoder', label: 'Qoder', color: 'bg-sky-500', remote: false },
  { id: 'workbuddy', label: 'WorkBuddy', color: 'bg-rose-500', remote: false },
  { id: 'minimax', label: 'MiniMax Code', color: 'bg-indigo-500', remote: false },
  { id: 'kimi', label: 'Kimi', color: 'bg-cyan-500', remote: false },
  { id: 'zcode', label: 'ZCode', color: 'bg-fuchsia-500', remote: false },
] as const

function targetOf(id: string) {
  return TARGETS.find((t) => t.id === id)
}

/** 表单态：数组/键值字段用多行文本编辑。 */
interface EntryForm {
  name: string
  description: string
  transport: string
  command: string
  argsText: string
  envText: string
  url: string
  headersText: string
}

const EMPTY_FORM: EntryForm = {
  name: '', description: '', transport: 'stdio', command: '',
  argsText: '', envText: '', url: '', headersText: '',
}

function formFromEntry(entry: McpCatalogEntry): EntryForm {
  return {
    name: entry.name,
    description: entry.description,
    transport: entry.transport || 'stdio',
    command: entry.command,
    argsText: entry.args.join('\n'),
    envText: Object.entries(entry.env).map(([k, v]) => `${k}=${v}`).join('\n'),
    url: entry.url,
    headersText: Object.entries(entry.headers).map(([k, v]) => `${k}: ${v}`).join('\n'),
  }
}

function entryFromForm(form: EntryForm, existing?: McpCatalogEntry): McpCatalogEntry {
  const lines = (text: string) => text.split('\n').map((l) => l.trim()).filter(Boolean)
  const pairs = (text: string, sep: ':' | '=') => {
    const map: Record<string, string> = {}
    for (const line of lines(text)) {
      const idx = line.indexOf(sep)
      if (idx <= 0) continue
      map[line.slice(0, idx).trim()] = line.slice(idx + 1).trim()
    }
    return map
  }
  return {
    name: form.name.trim(),
    description: form.description.trim(),
    transport: form.transport,
    command: form.command.trim(),
    args: lines(form.argsText),
    env: pairs(form.envText, '='),
    url: form.url.trim(),
    headers: pairs(form.headersText, ':'),
    assigned_agents: existing?.assigned_agents ?? [],
    created_at: existing?.created_at ?? '',
    updated_at: existing?.updated_at ?? '',
  }
}

/** 本地智能解析（不依赖 LLM）：JSON 片段 / 命令行 / 裸 URL。 */
function smartParse(text: string): Partial<EntryForm> {
  const trimmed = text.trim()
  if (!trimmed) return {}
  // 1. JSON：单服务器对象或 { mcpServers: { name: {...} } }
  if (trimmed.startsWith('{')) {
    try {
      const parsed: unknown = JSON.parse(trimmed)
      if (parsed && typeof parsed === 'object') {
        const root = parsed as Record<string, unknown>
        const servers = (root.mcpServers ?? root) as Record<string, unknown>
        const [name, cfg] = Object.entries(servers)[0] ?? []
        if (cfg && typeof cfg === 'object' && ('command' in cfg || 'url' in cfg)) {
          const c = cfg as Record<string, unknown>
          const transport = c.type === 'sse' || c.type === 'http' ? String(c.type)
            : typeof c.url === 'string' && c.url ? 'http' : 'stdio'
          return {
            name: typeof name === 'string' ? name.toLowerCase() : '',
            description: typeof c.description === 'string' ? c.description : '',
            transport,
            command: typeof c.command === 'string' ? c.command : '',
            argsText: Array.isArray(c.args) ? c.args.map(String).join('\n') : '',
            envText: c.env && typeof c.env === 'object'
              ? Object.entries(c.env as Record<string, unknown>).map(([k, v]) => `${k}=${String(v)}`).join('\n') : '',
            url: typeof c.url === 'string' ? c.url : '',
            headersText: c.headers && typeof c.headers === 'object'
              ? Object.entries(c.headers as Record<string, unknown>).map(([k, v]) => `${k}: ${String(v)}`).join('\n') : '',
          }
        }
      }
    } catch { /* 非 JSON，继续尝试其他形态 */ }
  }
  // 2. 裸 URL → 远程
  if (/^https?:\/\//.test(trimmed) && !/\s/.test(trimmed)) {
    return { transport: 'sse', url: trimmed }
  }
  // 3. 命令行（npx/uvx/node/python…）→ stdio
  if (/^(npx|uvx|node|python3?|uv|deno|bunx?)(\s|$)/.test(trimmed)) {
    const tokens = trimmed.split(/\s+/)
    return { transport: 'stdio', command: tokens[0], argsText: tokens.slice(1).join('\n') }
  }
  return {}
}

/** 目录扫描结果（复用后端 scan_mcp_local，不传 provider 即不调 LLM）。 */
interface ScanResult {
  transport: string
  name: string
  command: string
  args: string[]
  env: Record<string, string>
  url: string
  headers: Record<string, string>
  description: string
  warnings: string[]
  confidence: number
}

function applyScan(form: EntryForm, result: ScanResult): EntryForm {
  return {
    ...form,
    name: result.name,
    description: result.description,
    transport: result.transport || 'stdio',
    command: result.command,
    argsText: result.args.join('\n'),
    envText: Object.entries(result.env ?? {}).map(([k, v]) => `${k}=${v}`).join('\n'),
    url: result.url,
    headersText: Object.entries(result.headers ?? {}).map(([k, v]) => `${k}: ${v}`).join('\n'),
  }
}

// ── 添加 / 编辑对话框 ─────────────────────────────────────────────────────────

function AddEditDialog({ initial, editing, onClose, onSaved }: {
  initial: McpCatalogEntry | null
  editing: boolean
  onClose: () => void
  onSaved: (message: string) => void
}) {
  const { t } = useTranslation()
  const upsertMcpEntry = useMemoryStore((s) => s.upsertMcpEntry)
  const [tab, setTab] = useState<'manual' | 'paste' | 'scan'>('manual')
  const [form, setForm] = useState<EntryForm>(initial ? formFromEntry(initial) : EMPTY_FORM)
  const [pasteText, setPasteText] = useState('')
  const [scanDir, setScanDir] = useState('')
  const [busy, setBusy] = useState(false)
  const [error, setError] = useState('')
  const set = (patch: Partial<EntryForm>) => setForm((prev) => ({ ...prev, ...patch }))

  const isRemote = form.transport === 'sse' || form.transport === 'http'

  function applyPaste() {
    const patch = smartParse(pasteText)
    if (Object.keys(patch).length === 0) {
      setError(t('mcpLibrary.parseUnsupported'))
      return
    }
    setError('')
    set(patch)
    setTab('manual')
  }

  async function pickDir() {
    const picked = await open({ directory: true, multiple: false })
    if (typeof picked === 'string') {
      setScanDir(picked)
      setBusy(true)
      setError('')
      try {
        const result = await invoke<ScanResult>('scan_mcp_local', { dir: picked, provider: null })
        setForm((prev) => applyScan(prev, result))
        setTab('manual')
      } catch (e) {
        setError(t('mcpLibrary.scanFailed', { error: String(e) }))
      } finally {
        setBusy(false)
      }
    }
  }

  async function save() {
    setBusy(true)
    setError('')
    try {
      const entry = entryFromForm(form, initial ?? undefined)
      await upsertMcpEntry(entry)
      onSaved(entry.name)
      onClose()
    } catch (e) {
      setError(t('mcpLibrary.saveFailed', { error: String(e) }))
    } finally {
      setBusy(false)
    }
  }

  const input = 'w-full rounded-lg border border-gray-300 bg-white px-3 py-2 text-sm text-gray-800 outline-none transition focus:border-violet-400 dark:border-gray-600 dark:bg-gray-800 dark:text-gray-100'
  const label = 'mb-1 block text-xs font-medium text-gray-500 dark:text-gray-400'

  return (
    <div className="fixed inset-0 z-40 flex items-center justify-center p-4" role="dialog" aria-modal="true" aria-label={t('mcpLibrary.addTitle')}>
      <button aria-label={t('mcpLibrary.cancel')} onClick={onClose} className="absolute inset-0 cursor-default bg-gray-950/40 backdrop-blur-[2px]" />
      <div className="relative flex max-h-[85vh] w-full max-w-lg flex-col overflow-hidden rounded-2xl border border-gray-200 bg-white shadow-2xl dark:border-gray-700 dark:bg-gray-900">
        <div className="flex items-center gap-2 border-b border-gray-100 px-5 py-4 dark:border-gray-800">
          <div className="rounded-lg bg-violet-500/10 p-1.5 text-violet-600 dark:text-violet-400">
            <Plug size={16} />
          </div>
          <h2 className="text-sm font-semibold text-gray-900 dark:text-gray-100">
            {editing ? t('mcpLibrary.editTitle') : t('mcpLibrary.addTitle')}
          </h2>
          <button onClick={onClose} className="ml-auto rounded-md p-1 text-gray-400 transition hover:text-gray-600 dark:hover:text-gray-300" aria-label={t('mcpLibrary.cancel')}>
            <X size={16} />
          </button>
        </div>

        {!editing && (
          <div className="flex gap-1 border-b border-gray-100 px-5 pt-3 dark:border-gray-800">
            {([['manual', t('mcpLibrary.tabManual')], ['paste', t('mcpLibrary.tabPaste')], ['scan', t('mcpLibrary.tabScan')]] as const).map(([id, title]) => (
              <button
                key={id}
                onClick={() => setTab(id)}
                className={`rounded-t-lg px-3 py-1.5 text-xs font-medium transition ${tab === id ? 'border-b-2 border-violet-500 text-violet-700 dark:text-violet-300' : 'text-gray-500 hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-200'}`}
              >
                {title}
              </button>
            ))}
          </div>
        )}

        <div className="min-h-0 flex-1 space-y-3 overflow-y-auto px-5 py-4">
          {tab === 'paste' && !editing && (
            <div className="space-y-2">
              <textarea
                value={pasteText}
                onChange={(e) => setPasteText(e.target.value)}
                placeholder={t('mcpLibrary.pastePlaceholder')}
                rows={6}
                className={`${input} font-mono text-xs`}
              />
              <p className="text-[11px] text-gray-400">{t('mcpLibrary.pasteHint')}</p>
              <button onClick={applyPaste} className="inline-flex items-center gap-1 rounded-md bg-violet-600 px-3 py-1.5 text-xs font-medium text-white transition hover:bg-violet-500">
                <Check size={12} />{t('mcpLibrary.tabPaste')}
              </button>
            </div>
          )}
          {tab === 'scan' && !editing && (
            <div className="space-y-2">
              <button onClick={() => { void pickDir() }} disabled={busy} className="inline-flex w-full items-center justify-center gap-2 rounded-lg border border-dashed border-gray-300 px-3 py-6 text-sm text-gray-500 transition hover:border-violet-400 hover:text-violet-600 disabled:opacity-50 dark:border-gray-600 dark:text-gray-400 dark:hover:border-violet-400">
                {busy ? <Loader2 size={16} className="animate-spin" /> : <FolderOpen size={16} />}
                {scanDir || t('mcpLibrary.scanPick')}
              </button>
              <p className="text-[11px] text-gray-400">{t('mcpLibrary.scanHint')}</p>
            </div>
          )}
          {tab === 'manual' && (
            <>
              <div>
                <label className={label}>{t('mcpLibrary.fieldName')}</label>
                <input value={form.name} onChange={(e) => set({ name: e.target.value })} placeholder="playwright" className={`${input} font-mono ${editing ? 'opacity-60' : ''}`} disabled={editing} />
                <p className="mt-1 text-[11px] text-gray-400">{t('mcpLibrary.fieldNameHint')}</p>
              </div>
              <div>
                <label className={label}>{t('mcpLibrary.fieldDescription')}</label>
                <input value={form.description} onChange={(e) => set({ description: e.target.value })} className={input} />
              </div>
              <div>
                <label className={label}>{t('mcpLibrary.fieldTransport')}</label>
                <select value={form.transport} onChange={(e) => set({ transport: e.target.value })} className={input}>
                  <option value="stdio">{t('mcpLibrary.transportStdio')}</option>
                  <option value="sse">{t('mcpLibrary.transportSse')}</option>
                  <option value="http">{t('mcpLibrary.transportHttp')}</option>
                </select>
              </div>
              {isRemote ? (
                <>
                  <div>
                    <label className={label}>{t('mcpLibrary.fieldUrl')}</label>
                    <input value={form.url} onChange={(e) => set({ url: e.target.value })} placeholder="https://example.com/mcp" className={`${input} font-mono text-xs`} />
                  </div>
                  <div>
                    <label className={label}>{t('mcpLibrary.fieldHeaders')}</label>
                    <textarea value={form.headersText} onChange={(e) => set({ headersText: e.target.value })} rows={2} placeholder="Authorization: Bearer …" className={`${input} font-mono text-xs`} />
                  </div>
                </>
              ) : (
                <>
                  <div>
                    <label className={label}>{t('mcpLibrary.fieldCommand')}</label>
                    <input value={form.command} onChange={(e) => set({ command: e.target.value })} placeholder="npx" className={`${input} font-mono text-xs`} />
                  </div>
                  <div>
                    <label className={label}>{t('mcpLibrary.fieldArgs')}</label>
                    <textarea value={form.argsText} onChange={(e) => set({ argsText: e.target.value })} rows={2} placeholder={'-y\n@playwright/mcp'} className={`${input} font-mono text-xs`} />
                  </div>
                  <div>
                    <label className={label}>{t('mcpLibrary.fieldEnv')}</label>
                    <textarea value={form.envText} onChange={(e) => set({ envText: e.target.value })} rows={2} placeholder="API_KEY=your_key" className={`${input} font-mono text-xs`} />
                  </div>
                </>
              )}
            </>
          )}
          {error && <p className="rounded-lg bg-red-50 px-3 py-2 text-xs text-red-600 dark:bg-red-900/30 dark:text-red-300">{error}</p>}
        </div>

        <div className="flex justify-end gap-2 border-t border-gray-100 px-5 py-3 dark:border-gray-800">
          <button onClick={onClose} className="rounded-lg px-3 py-1.5 text-xs font-medium text-gray-500 transition hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-200">{t('mcpLibrary.cancel')}</button>
          <button onClick={() => { void save() }} disabled={busy} className="inline-flex items-center gap-1 rounded-lg bg-violet-600 px-3 py-1.5 text-xs font-medium text-white transition hover:bg-violet-500 disabled:opacity-50">
            {busy ? <Loader2 size={12} className="animate-spin" /> : <Check size={12} />}
            {t('mcpLibrary.save')}
          </button>
        </div>
      </div>
    </div>
  )
}

// ── 装备对话框 ────────────────────────────────────────────────────────────────

function EquipDialog({ entry, onClose, onNotice }: {
  entry: McpCatalogEntry
  onClose: () => void
  onNotice: (message: string) => void
}) {
  const { t } = useTranslation()
  const { mcpStatuses, equipMcp, unequipMcp } = useMemoryStore()
  const [targets, setTargets] = useState<Set<string>>(new Set(entry.assigned_agents))
  const [busyAgent, setBusyAgent] = useState<string | null>(null)
  const [applying, setApplying] = useState(false)

  const isRemote = entry.transport === 'sse' || entry.transport === 'http'
  const statusOf = (agent: string): McpAgentStatus | undefined =>
    mcpStatuses.find((s) => s.name === entry.name && s.agent === agent)

  function toggle(id: string) {
    setTargets((prev) => {
      const next = new Set(prev)
      if (next.has(id)) next.delete(id)
      else next.add(id)
      return next
    })
  }

  async function rewrite(agent: string) {
    setBusyAgent(agent)
    try {
      await equipMcp(entry.name, agent)
    } catch (error) {
      onNotice(t('mcpLibrary.equipFailed', { error: String(error) }))
    } finally {
      setBusyAgent(null)
    }
  }

  async function apply() {
    const changes: { agent: string; equipped: boolean }[] = []
    for (const target of TARGETS) {
      const assigned = targets.has(target.id)
      const was = entry.assigned_agents.includes(target.id)
      if (assigned !== was) changes.push({ agent: target.id, equipped: assigned })
    }
    if (changes.length === 0) {
      onClose()
      return
    }
    setApplying(true)
    let failed = ''
    for (const change of changes) {
      try {
        if (change.equipped) await equipMcp(entry.name, change.agent)
        else await unequipMcp(entry.name, change.agent)
      } catch (error) {
        failed = t('mcpLibrary.equipFailed', { error: String(error) })
        break
      }
    }
    setApplying(false)
    if (failed) onNotice(failed)
    else onNotice(t('mcpLibrary.equipDone', { count: changes.length }))
    onClose()
  }

  const stateBadge = (state: string | undefined) => {
    if (!state) return null
    if (state === 'installed') return <span className="rounded-full bg-emerald-500/10 px-1.5 py-0.5 text-[10px] font-medium text-emerald-700 dark:text-emerald-300">{t('mcpLibrary.stateInstalled')}</span>
    if (state === 'differs') return <span className="rounded-full bg-amber-500/10 px-1.5 py-0.5 text-[10px] font-medium text-amber-700 dark:text-amber-300">{t('mcpLibrary.stateDiffers')}</span>
    return <span className="rounded-full bg-gray-500/10 px-1.5 py-0.5 text-[10px] font-medium text-gray-500 dark:text-gray-400">{t('mcpLibrary.stateMissing')}</span>
  }

  return (
    <div className="fixed inset-0 z-40 flex items-center justify-center p-4" role="dialog" aria-modal="true" aria-label={t('mcpLibrary.equipTitle')}>
      <button aria-label={t('mcpLibrary.cancel')} onClick={onClose} className="absolute inset-0 cursor-default bg-gray-950/40 backdrop-blur-[2px]" />
      <div className="relative flex max-h-[85vh] w-full max-w-md flex-col overflow-hidden rounded-2xl border border-gray-200 bg-white shadow-2xl dark:border-gray-700 dark:bg-gray-900">
        <div className="flex items-center gap-2 border-b border-gray-100 px-5 py-4 dark:border-gray-800">
          <div className="rounded-lg bg-violet-500/10 p-1.5 text-violet-600 dark:text-violet-400">
            <Plug size={16} />
          </div>
          <h2 className="min-w-0 truncate text-sm font-semibold text-gray-900 dark:text-gray-100">
            {t('mcpLibrary.equipTitle')} · <span className="font-mono">{entry.name}</span>
          </h2>
          <button onClick={onClose} className="ml-auto rounded-md p-1 text-gray-400 transition hover:text-gray-600 dark:hover:text-gray-300" aria-label={t('mcpLibrary.cancel')}>
            <X size={16} />
          </button>
        </div>
        <div className="min-h-0 flex-1 space-y-1 overflow-y-auto px-5 py-4">
          <p className="mb-2 text-xs leading-5 text-gray-500 dark:text-gray-400">{t('mcpLibrary.equipHint')}</p>
          {TARGETS.map((target) => {
            const unsupported = isRemote && !target.remote
            const checked = targets.has(target.id)
            const status = statusOf(target.id)
            return (
              <div key={target.id} className={`flex items-center gap-2 rounded-lg px-2 py-2 transition ${checked ? 'bg-violet-50 dark:bg-violet-500/10' : 'hover:bg-gray-50 dark:hover:bg-gray-800'}`}>
                <input
                  type="checkbox"
                  checked={checked}
                  disabled={unsupported}
                  onChange={() => toggle(target.id)}
                  className="h-3.5 w-3.5 cursor-pointer accent-violet-600 disabled:cursor-not-allowed"
                  aria-label={target.label}
                />
                <span className={`h-2 w-2 shrink-0 rounded-full ${target.color}`} />
                <span className={`min-w-0 flex-1 truncate text-sm ${unsupported ? 'text-gray-400 dark:text-gray-500' : 'text-gray-800 dark:text-gray-100'}`}>{target.label}</span>
                {unsupported ? (
                  <span className="text-[10px] text-gray-400 dark:text-gray-500">{t('mcpLibrary.remoteUnsupported')}</span>
                ) : (
                  <>
                    {stateBadge(checked ? status?.state : undefined)}
                    {checked && busyAgent !== target.id && (
                      <button
                        onClick={() => { void rewrite(target.id) }}
                        title={t('mcpLibrary.rewrite')}
                        className="rounded-md p-1 text-gray-400 transition hover:text-violet-600 dark:hover:text-violet-400"
                        aria-label={t('mcpLibrary.rewrite')}
                      >
                        <RefreshCw size={12} />
                      </button>
                    )}
                    {busyAgent === target.id && <Loader2 size={12} className="animate-spin text-violet-500" />}
                  </>
                )}
              </div>
            )
          })}
        </div>
        <div className="flex justify-end gap-2 border-t border-gray-100 px-5 py-3 dark:border-gray-800">
          <button onClick={onClose} className="rounded-lg px-3 py-1.5 text-xs font-medium text-gray-500 transition hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-200">{t('mcpLibrary.cancel')}</button>
          <button onClick={() => { void apply() }} disabled={applying} className="inline-flex items-center gap-1 rounded-lg bg-violet-600 px-3 py-1.5 text-xs font-medium text-white transition hover:bg-violet-500 disabled:opacity-50">
            {applying ? <Loader2 size={12} className="animate-spin" /> : <Check size={12} />}
            {t('mcpLibrary.equipApply')}
          </button>
        </div>
      </div>
    </div>
  )
}

// ── 从 Agent 导入对话框 ───────────────────────────────────────────────────────

function ImportDialog({ onClose, onNotice }: { onClose: () => void; onNotice: (message: string) => void }) {
  const { t } = useTranslation()
  const { importMcpFromAgents, upsertMcpEntry } = useMemoryStore()
  const [scanning, setScanning] = useState(true)
  const [candidates, setCandidates] = useState<McpImportCandidate[]>([])
  const [picked, setPicked] = useState<Set<string>>(new Set())
  const [importing, setImporting] = useState(false)

  useEffect(() => {
    let mounted = true
    importMcpFromAgents()
      .then((found) => {
        if (!mounted) return
        setCandidates(found)
        setPicked(new Set(found.filter((c) => !c.already_in_catalog).map((c) => c.entry.name)))
        setScanning(false)
      })
      .catch(() => { if (mounted) setScanning(false) })
    return () => { mounted = false }
  }, [importMcpFromAgents])

  function toggle(name: string) {
    setPicked((prev) => {
      const next = new Set(prev)
      if (next.has(name)) next.delete(name)
      else next.add(name)
      return next
    })
  }

  async function importSelected() {
    setImporting(true)
    let count = 0
    try {
      for (const candidate of candidates) {
        if (picked.has(candidate.entry.name)) {
          await upsertMcpEntry(candidate.entry)
          count += 1
        }
      }
      onNotice(t('mcpLibrary.importDone', { count }))
      onClose()
    } catch (error) {
      onNotice(t('mcpLibrary.importFailed', { error: String(error) }))
    } finally {
      setImporting(false)
    }
  }

  return (
    <div className="fixed inset-0 z-40 flex items-center justify-center p-4" role="dialog" aria-modal="true" aria-label={t('mcpLibrary.importTitle')}>
      <button aria-label={t('mcpLibrary.cancel')} onClick={onClose} className="absolute inset-0 cursor-default bg-gray-950/40 backdrop-blur-[2px]" />
      <div className="relative flex max-h-[85vh] w-full max-w-md flex-col overflow-hidden rounded-2xl border border-gray-200 bg-white shadow-2xl dark:border-gray-700 dark:bg-gray-900">
        <div className="flex items-center gap-2 border-b border-gray-100 px-5 py-4 dark:border-gray-800">
          <div className="rounded-lg bg-violet-500/10 p-1.5 text-violet-600 dark:text-violet-400">
            <Download size={16} />
          </div>
          <h2 className="text-sm font-semibold text-gray-900 dark:text-gray-100">{t('mcpLibrary.importTitle')}</h2>
          <button onClick={onClose} className="ml-auto rounded-md p-1 text-gray-400 transition hover:text-gray-600 dark:hover:text-gray-300" aria-label={t('mcpLibrary.cancel')}>
            <X size={16} />
          </button>
        </div>
        <div className="min-h-0 flex-1 overflow-y-auto px-5 py-4">
          <p className="mb-2 text-xs leading-5 text-gray-500 dark:text-gray-400">{t('mcpLibrary.importHint')}</p>
          {scanning && (
            <div className="flex items-center gap-2 py-8 text-sm text-gray-500 dark:text-gray-400">
              <Loader2 size={16} className="animate-spin" />{t('mcpLibrary.importScanning')}
            </div>
          )}
          {!scanning && candidates.length === 0 && (
            <p className="py-8 text-center text-sm text-gray-500 dark:text-gray-400">{t('mcpLibrary.importEmpty')}</p>
          )}
          {!scanning && candidates.map((candidate) => {
            const target = targetOf(candidate.agent)
            return (
              <label key={candidate.entry.name} className="mb-1 flex cursor-pointer items-center gap-2 rounded-lg px-2 py-2 transition hover:bg-gray-50 dark:hover:bg-gray-800">
                <input
                  type="checkbox"
                  checked={picked.has(candidate.entry.name)}
                  onChange={() => toggle(candidate.entry.name)}
                  className="h-3.5 w-3.5 cursor-pointer accent-violet-600"
                  aria-label={candidate.entry.name}
                />
                <span className={`h-2 w-2 shrink-0 rounded-full ${target?.color ?? 'bg-gray-400'}`} />
                <span className="min-w-0 flex-1">
                  <span className="block truncate font-mono text-sm text-gray-800 dark:text-gray-100">{candidate.entry.name}</span>
                  <span className="block truncate text-xs text-gray-500 dark:text-gray-400">
                    {t('mcpLibrary.importFrom')} {target?.label ?? candidate.agent}
                    {candidate.entry.description ? ` · ${candidate.entry.description}` : ''}
                  </span>
                </span>
                {candidate.already_in_catalog && (
                  <span className="rounded-full bg-gray-500/10 px-1.5 py-0.5 text-[10px] font-medium text-gray-500 dark:text-gray-400">{t('mcpLibrary.importAlready')}</span>
                )}
              </label>
            )
          })}
        </div>
        <div className="flex justify-end gap-2 border-t border-gray-100 px-5 py-3 dark:border-gray-800">
          <button onClick={onClose} className="rounded-lg px-3 py-1.5 text-xs font-medium text-gray-500 transition hover:text-gray-700 dark:text-gray-400 dark:hover:text-gray-200">{t('mcpLibrary.cancel')}</button>
          <button onClick={() => { void importSelected() }} disabled={importing || picked.size === 0} className="inline-flex items-center gap-1 rounded-lg bg-violet-600 px-3 py-1.5 text-xs font-medium text-white transition hover:bg-violet-500 disabled:opacity-50">
            {importing ? <Loader2 size={12} className="animate-spin" /> : <Download size={12} />}
            {t('mcpLibrary.importApply', { count: picked.size })}
          </button>
        </div>
      </div>
    </div>
  )
}

// ── 主页面 ────────────────────────────────────────────────────────────────────

export const McpLibrary = memo(function McpLibrary({ active = true }: { active?: boolean }) {
  const { t } = useTranslation()
  const {
    mcpCatalog, mcpStatuses, loadMcpCatalog, checkMcpStatuses, deleteMcpEntry,
  } = useMemoryStore()

  const [booting, setBooting] = useState(true)
  const [loading, setLoading] = useState(false)
  const [notice, setNotice] = useState('')
  const [query, setQuery] = useState('')
  const [addOpen, setAddOpen] = useState(false)
  const [editing, setEditing] = useState<McpCatalogEntry | null>(null)
  const [equipping, setEquipping] = useState<McpCatalogEntry | null>(null)
  const [importOpen, setImportOpen] = useState(false)
  // MCP 服务器中心双视图：runtime 服务器配置（工作流使用）与目录分发
  const [view, setView] = useState<'servers' | 'catalog'>('servers')
  const [runtimeServers, setRuntimeServers] = useState<McpServer[]>([])
  const [providers, setProviders] = useState<{ id: string; name: string; model: string; base_url: string; api_key: string; is_custom: boolean; enabled: boolean; context_window?: number; max_output_tokens?: number }[]>([])

  const reloadServers = useCallback(async () => {
    try {
      const [ms, ps] = await Promise.all([
        invoke<McpServer[]>('list_mcp_servers'),
        invoke<{ id: string; name: string; model: string; base_url: string; api_key: string; is_custom: boolean; enabled: boolean; context_window?: number; max_output_tokens?: number }[]>('list_llm_providers'),
      ])
      setRuntimeServers(ms)
      setProviders(ps)
    } catch { /* 配置读取失败时保持空列表 */ }
  }, [])

  useEffect(() => {
    if (active) void reloadServers()
  }, [active, reloadServers])

  const activeProvider = providers.find(p => p.enabled && !!p.api_key.trim()) ?? null

  useEffect(() => {
    let mounted = true
    Promise.all([loadMcpCatalog(), checkMcpStatuses().catch(() => {})])
      .catch(() => setNotice(t('mcpLibrary.importFailed', { error: '' })))
      .finally(() => { if (mounted) setBooting(false) })
    return () => { mounted = false }
  }, [loadMcpCatalog, checkMcpStatuses, t])

  // 每次进入页面刷新一次状态（只读配置文件，毫秒级）；不做定时轮询。
  useEffect(() => {
    if (active) void checkMcpStatuses().catch(() => {})
  }, [active, checkMcpStatuses])

  useEffect(() => {
    if (!notice) return
    const timer = window.setTimeout(() => setNotice(''), 6000)
    return () => window.clearTimeout(timer)
  }, [notice])

  const refreshAll = useCallback(async () => {
    setLoading(true)
    try {
      await Promise.all([loadMcpCatalog(), checkMcpStatuses()])
    } catch (error) {
      setNotice(String(error))
    } finally {
      setLoading(false)
    }
  }, [loadMcpCatalog, checkMcpStatuses])

  const visible = useMemo(() => {
    const keyword = query.trim().toLowerCase()
    if (!keyword) return mcpCatalog
    return mcpCatalog.filter((entry) =>
      entry.name.toLowerCase().includes(keyword)
      || entry.description.toLowerCase().includes(keyword))
  }, [mcpCatalog, query])

  const statusOf = useCallback((name: string, agent: string) =>
    mcpStatuses.find((s) => s.name === name && s.agent === agent), [mcpStatuses])

  async function handleDelete(entry: McpCatalogEntry) {
    if (!window.confirm(t('mcpLibrary.deleteConfirm', { name: entry.name }))) return
    setLoading(true)
    try {
      await deleteMcpEntry(entry.name)
      setNotice(t('mcpLibrary.deleteDone', { name: entry.name }))
    } catch (error) {
      setNotice(t('mcpLibrary.deleteFailed', { error: String(error) }))
    } finally {
      setLoading(false)
    }
  }

  if (booting && view === 'catalog') {
    return (
      <div className="flex h-full items-center justify-center gap-2 text-sm text-gray-500 dark:text-gray-400">
        <Loader2 size={16} className="animate-spin" />{t('mcpLibrary.booting')}
      </div>
    )
  }

  return (
    <div className="flex h-full min-h-0 flex-col">
      {/* header */}
      <div className="flex flex-wrap items-center gap-2 border-b border-gray-200 px-5 py-3 dark:border-gray-800">
        <div className="mr-auto flex min-w-0 items-center gap-4">
          <div className="min-w-0">
            <h1 className="flex items-center gap-2 text-base font-semibold text-gray-900 dark:text-gray-100">
              <Plug size={18} className="text-violet-500" />{t('mcpLibrary.title')}
            </h1>
            <p className="truncate text-xs text-gray-500 dark:text-gray-400">{t('mcpLibrary.subtitle')}</p>
          </div>
          {/* 视图切换：本地服务器 / 目录与分发 */}
          <div className="flex shrink-0 rounded-lg border border-gray-200 p-0.5 dark:border-gray-700">
            {([
              { id: 'servers' as const, label: t('mcpLibrary.tabServers') },
              { id: 'catalog' as const, label: t('mcpLibrary.tabCatalog') },
            ]).map(v => (
              <button key={v.id} onClick={() => setView(v.id)}
                className={`rounded-md px-3 py-1.5 text-xs font-medium transition-colors ${
                  view === v.id
                    ? 'bg-violet-600 text-white'
                    : 'text-gray-500 hover:text-gray-800 dark:text-gray-400 dark:hover:text-gray-200'
                }`}>
                {v.label}
              </button>
            ))}
          </div>
        </div>
        {view === 'catalog' && (
          <>
            <div className="relative">
              <Search size={14} className="pointer-events-none absolute left-2.5 top-1/2 -translate-y-1/2 text-gray-400" />
              <input
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder={t('mcpLibrary.search')}
                className="w-56 rounded-lg border border-gray-300 bg-white py-1.5 pl-8 pr-3 text-sm text-gray-800 outline-none transition focus:border-violet-400 dark:border-gray-600 dark:bg-gray-800 dark:text-gray-100"
              />
            </div>
            <button
              onClick={() => { void refreshAll() }}
              disabled={loading}
              className="inline-flex items-center gap-1 rounded-lg border border-gray-300 bg-white px-2.5 py-1.5 text-xs font-medium text-gray-700 transition hover:bg-gray-50 disabled:opacity-50 dark:border-gray-600 dark:bg-gray-800 dark:text-gray-200 dark:hover:bg-gray-700"
            >
              <RefreshCw size={13} className={loading ? 'animate-spin' : ''} />{t('mcpLibrary.refreshStatus')}
            </button>
            <button
              onClick={() => setImportOpen(true)}
              disabled={loading}
              className="inline-flex items-center gap-1 rounded-lg border border-gray-300 bg-white px-2.5 py-1.5 text-xs font-medium text-gray-700 transition hover:bg-gray-50 disabled:opacity-50 dark:border-gray-600 dark:bg-gray-800 dark:text-gray-200 dark:hover:bg-gray-700"
            >
              <Download size={13} />{t('mcpLibrary.importFromAgents')}
            </button>
            <button
              onClick={() => { setEditing(null); setAddOpen(true) }}
              className="inline-flex items-center gap-1 rounded-lg bg-violet-600 px-2.5 py-1.5 text-xs font-medium text-white transition hover:bg-violet-500"
            >
              <Plus size={13} />{t('mcpLibrary.add')}
            </button>
          </>
        )}
      </div>

      {view === 'servers' ? (
        <div className="min-h-0 flex-1 overflow-hidden">
          <McpServersManager servers={runtimeServers} onReload={reloadServers} activeProvider={activeProvider} />
        </div>
      ) : (
        <>
        {notice && (
        <div className="flex items-center gap-2 border-b border-emerald-200 bg-emerald-50 px-5 py-2 text-xs text-emerald-700 dark:border-emerald-500/20 dark:bg-emerald-500/10 dark:text-emerald-300">
          <Check size={12} className="shrink-0" />
          <span className="min-w-0 flex-1 break-all">{notice}</span>
          <button onClick={() => setNotice('')} className="shrink-0 text-emerald-500 transition hover:text-emerald-700 dark:hover:text-emerald-300" aria-label="dismiss"><X size={12} /></button>
        </div>
      )}

      {/* list */}
      <div className="min-h-0 flex-1 overflow-y-auto p-5">
        {visible.length === 0 ? (
          <div className="mx-auto max-w-md py-16 text-center">
            <div className="mx-auto mb-3 flex h-14 w-14 items-center justify-center rounded-2xl bg-violet-100 dark:bg-violet-900/30">
              <Plug size={24} className="text-violet-600 dark:text-violet-400" />
            </div>
            <p className="text-sm font-medium text-gray-700 dark:text-gray-200">{t('mcpLibrary.emptyTitle')}</p>
            <p className="mt-1 text-xs leading-5 text-gray-500 dark:text-gray-400">{t('mcpLibrary.emptyHint')}</p>
          </div>
        ) : (
          <div className="grid grid-cols-1 gap-3 xl:grid-cols-2">
            {visible.map((entry) => {
              const isRemote = entry.transport === 'sse' || entry.transport === 'http'
              return (
                <div key={entry.name} className="rounded-2xl border border-gray-200 bg-white p-4 transition hover:border-violet-300 dark:border-gray-700 dark:bg-gray-900 dark:hover:border-violet-500/40 [content-visibility:auto] [contain-intrinsic-size:auto_180px]">
                  <div className="flex items-start gap-3">
                    <div className={`rounded-lg p-2 ${isRemote ? 'bg-sky-500/10 text-sky-600 dark:text-sky-400' : 'bg-violet-500/10 text-violet-600 dark:text-violet-400'}`}>
                      {isRemote ? <Globe size={16} /> : <Terminal size={16} />}
                    </div>
                    <div className="min-w-0 flex-1">
                      <div className="flex items-center gap-2">
                        <h3 className="min-w-0 truncate font-mono text-sm font-semibold text-gray-900 dark:text-gray-100">{entry.name}</h3>
                        <span className="shrink-0 rounded-full bg-gray-100 px-1.5 py-0.5 text-[10px] font-medium text-gray-600 dark:bg-gray-800 dark:text-gray-300">
                          {entry.transport === 'stdio' ? 'stdio' : entry.transport.toUpperCase()}
                        </span>
                      </div>
                      <p className="mt-0.5 truncate text-xs text-gray-500 dark:text-gray-400">{entry.description || t('mcpLibrary.noDescription')}</p>
                      <p className="mt-1.5 truncate font-mono text-[11px] text-gray-400 dark:text-gray-500">
                        {isRemote ? entry.url : [entry.command, ...entry.args].filter(Boolean).join(' ')}
                      </p>
                    </div>
                  </div>
                  {/* per-agent chips */}
                  <div className="mt-3 flex flex-wrap gap-1.5">
                    {TARGETS.map((target) => {
                      const assigned = entry.assigned_agents.includes(target.id)
                      if (!assigned) return null
                      const state = statusOf(entry.name, target.id)?.state
                      const tone = state === 'installed'
                        ? 'border-emerald-500/50 bg-emerald-50 text-emerald-700 dark:border-emerald-500/30 dark:bg-emerald-500/10 dark:text-emerald-300'
                        : state === 'differs'
                          ? 'border-amber-500/50 bg-amber-50 text-amber-700 dark:border-amber-500/30 dark:bg-amber-500/10 dark:text-amber-300'
                          : 'border-gray-300 bg-gray-50 text-gray-500 dark:border-gray-600 dark:bg-gray-800 dark:text-gray-400'
                      const dot = state === 'installed' ? 'bg-emerald-500' : state === 'differs' ? 'bg-amber-500' : 'bg-gray-400'
                      return (
                        <span key={target.id} title={state === 'differs' ? t('mcpLibrary.stateDiffers') : state === 'missing' ? t('mcpLibrary.stateMissing') : t('mcpLibrary.stateInstalled')} className={`inline-flex items-center gap-1 rounded-full border px-1.5 py-0.5 text-[10px] font-medium ${tone}`}>
                          <span className={`h-1.5 w-1.5 rounded-full ${dot}`} />
                          {target.label}
                        </span>
                      )
                    })}
                    {entry.assigned_agents.length === 0 && (
                      <span className="text-[10px] text-gray-400 dark:text-gray-500">{t('mcpLibrary.notEquipped')}</span>
                    )}
                  </div>
                  {/* actions */}
                  <div className="mt-3 flex items-center gap-1.5">
                    <button onClick={() => setEquipping(entry)} className="inline-flex items-center gap-1 rounded-md bg-violet-600 px-2 py-1 text-[11px] font-medium text-white transition hover:bg-violet-500">
                      <Plug size={11} />{t('mcpLibrary.equip')}
                    </button>
                    <button onClick={() => { setEditing(entry); setAddOpen(true) }} className="inline-flex items-center gap-1 rounded-md border border-gray-300 px-2 py-1 text-[11px] font-medium text-gray-600 transition hover:bg-gray-50 dark:border-gray-600 dark:text-gray-300 dark:hover:bg-gray-800">
                      <Pencil size={11} />{t('mcpLibrary.edit')}
                    </button>
                    <button onClick={() => { void handleDelete(entry) }} disabled={loading} className="ml-auto inline-flex items-center gap-1 rounded-md px-2 py-1 text-[11px] font-medium text-gray-400 transition hover:text-red-600 dark:hover:text-red-400 disabled:opacity-50">
                      <Trash2 size={11} />{t('mcpLibrary.delete')}
                    </button>
                  </div>
                </div>
              )
            })}
          </div>
        )}
        <p className="mt-4 text-center text-[11px] text-gray-400 dark:text-gray-500">{t('mcpLibrary.statusHint')}</p>
      </div>
        </>
      )}

      {addOpen && (
        <AddEditDialog
          initial={editing}
          editing={editing !== null}
          onClose={() => { setAddOpen(false); setEditing(null) }}
          onSaved={(name) => setNotice(t('mcpLibrary.saveDone', { name }))}
        />
      )}
      {equipping && (
        <EquipDialog entry={equipping} onClose={() => setEquipping(null)} onNotice={setNotice} />
      )}
      {importOpen && (
        <ImportDialog onClose={() => setImportOpen(false)} onNotice={setNotice} />
      )}
    </div>
  )
})
