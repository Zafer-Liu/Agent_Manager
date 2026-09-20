import { useEffect, useRef, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import {
  Loader2, Wrench, AlertCircle, Plus, Minus, FolderOpen, Trash2,
} from 'lucide-react'
import { useTranslation } from 'react-i18next'

// ── Types ────────────────────────────────────────────────────────────────────

export interface McpServer {
  name: string; command: string; args: string[]; env: Record<string, string>
  transport?: string; url?: string; headers?: Record<string, string>; description?: string
}

interface LlmProvider {
  id: string; name: string; model: string; base_url: string
  api_key: string; is_custom: boolean; enabled: boolean
  context_window?: number; max_output_tokens?: number
}

type AddMode = 'local' | 'text' | 'manual'

interface ScanResult {
  name: string; command: string; args: string[]; env: Record<string, string>
  transport: string; url: string; headers: Record<string, string>
  description: string; warnings: string[]; confidence: number
}

const EMPTY_SERVER: McpServer = {
  name: '', command: '', args: [], env: {}, transport: 'stdio', url: '', headers: {}, description: '',
}

// ── MCP servers manager（工作流与 MCP 分发共用的运行时服务器配置）──────────────

export function McpServersManager({ servers, onReload, activeProvider }: {
  servers: McpServer[]
  onReload: () => void
  activeProvider: LlmProvider | null
}) {
  const { t } = useTranslation()
  const [editing, setEditing] = useState<McpServer | null>(null)
  const [isNew, setIsNew] = useState(false)
  const [addMode, setAddMode] = useState<AddMode>('local')
  const [saving, setSaving] = useState(false)
  const [scanning, setScanning] = useState(false)
  const [error, setError] = useState('')
  const [warnings, setWarnings] = useState<string[]>([])
  const [deleteConfirm, setDeleteConfirm] = useState<string | null>(null)
  const [configPath, setConfigPath] = useState('')
  const [localDir, setLocalDir] = useState('')
  const [parseText, setParseText] = useState('')

  useEffect(() => { invoke<string>('get_mcp_config_path').then(setConfigPath) }, [])

  function startNew() {
    setEditing({ ...EMPTY_SERVER })
    setIsNew(true)
    setError('')
    setWarnings([])
    setLocalDir('')
    setParseText('')
  }

  function startEdit(s: McpServer) {
    setEditing({ ...s, transport: s.transport || 'stdio', headers: s.headers || {}, url: s.url || '' })
    setIsNew(false)
    setError('')
    setWarnings([])
  }

  async function pickLocalDir() {
    const f = await open({ directory: true, multiple: false })
    if (f && typeof f === 'string') {
      setLocalDir(f)
      await scanLocal(f)
    }
  }

  async function scanLocal(dir: string) {
    setScanning(true); setError(''); setWarnings([])
    try {
      const result = await invoke<ScanResult>('scan_mcp_local', {
        dir,
        provider: activeProvider ?? null,
      })
      applyResult(result)
    } catch (e) {
      setError(String(e))
    } finally { setScanning(false) }
  }

  async function parseText_() {
    if (!parseText.trim()) { setError(t('mcpServers.pasteTextFirst')); return }

    // Try direct JSON parse first (standard mcpServers / single-server format)
    try {
      const json = JSON.parse(parseText)
      // { mcpServers: { name: { command, args, env, ... } } }
      const serversMap: Record<string, unknown> = json.mcpServers ?? json
      const entries = Object.entries(serversMap)
      if (entries.length > 0) {
        const [name, cfg] = entries[0] as [string, Record<string, unknown>]
        if (cfg && typeof cfg === 'object' && ('command' in cfg || 'url' in cfg)) {
          const isSSE = (cfg.transport === 'sse') || typeof cfg.url === 'string'
          applyResult({
            name,
            command: String(cfg.command ?? ''),
            args: Array.isArray(cfg.args) ? cfg.args.map(String) : [],
            env: (cfg.env && typeof cfg.env === 'object' && !Array.isArray(cfg.env))
              ? cfg.env as Record<string, string> : {},
            transport: isSSE ? 'sse' : 'stdio',
            url: typeof cfg.url === 'string' ? cfg.url : '',
            headers: (cfg.headers && typeof cfg.headers === 'object' && !Array.isArray(cfg.headers))
              ? cfg.headers as Record<string, string> : {},
            description: typeof cfg.description === 'string' ? cfg.description : '',
            warnings: [],
            confidence: 1,
          })
          return
        }
      }
    } catch { /* not valid JSON, fall through to LLM */ }

    if (!activeProvider) { setError(t('mcpServers.configureLlmFirst')); return }
    setScanning(true); setError(''); setWarnings([])
    try {
      const result = await invoke<ScanResult>('parse_mcp_text', {
        text: parseText,
        provider: activeProvider,
      })
      applyResult(result)
    } catch (e) {
      setError(String(e))
    } finally { setScanning(false) }
  }

  function applyResult(r: ScanResult) {
    setWarnings(r.warnings)
    setEditing({
      name: r.name, command: r.command, args: r.args, env: r.env,
      transport: r.transport, url: r.url, headers: r.headers, description: r.description,
    })
  }

  async function save() {
    if (!editing) return
    if (!editing.name.trim()) { setError(t('mcpServers.nameRequired')); return }
    const isSSE = editing.transport === 'sse'
    if (isSSE && !(editing.url ?? '').trim()) { setError(t('mcpServers.urlRequiredSse')); return }
    if (!isSSE && !editing.command.trim()) { setError(t('mcpServers.commandRequiredStdio')); return }
    setSaving(true)
    try {
      await invoke('save_mcp_server', { server: editing })
      await onReload()
      setEditing(null)
    } catch (e) { setError(String(e)) }
    finally { setSaving(false) }
  }

  async function del(name: string) {
    await invoke('delete_mcp_server', { name })
    setDeleteConfirm(null)
    await onReload()
    if (editing?.name === name) setEditing(null)
  }

  const isSSE = editing?.transport === 'sse'

  return (
    <div className="flex h-full overflow-hidden">
      {/* Server list */}
      <div className="w-56 shrink-0 border-r border-gray-200 dark:border-gray-700 flex flex-col">
        <div className="flex items-center justify-between px-3 py-2 border-b border-gray-100 dark:border-gray-800">
          <span className="text-xs font-medium text-gray-500">{t('mcpServers.serverCount', { count: servers.length })}</span>
          <button onClick={startNew} className="rounded p-1 text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-800">
            <Plus className="h-3.5 w-3.5" />
          </button>
        </div>
        <div className="flex-1 overflow-y-auto p-2 space-y-0.5">
          {servers.length === 0 && <p className="py-6 text-center text-xs text-gray-400">{t('mcpServers.noServersYet')}</p>}
          {servers.map(s => (
            <button key={s.name} onClick={() => startEdit(s)}
              className={`group flex w-full items-center gap-2 rounded-lg px-3 py-2 text-left transition-colors ${
                editing?.name === s.name && !isNew
                  ? 'bg-purple-50 text-purple-700 dark:bg-purple-900/20 dark:text-purple-300'
                  : 'hover:bg-gray-50 dark:hover:bg-gray-800'
              }`}>
              <span>{s.transport === 'sse' ? '🌐' : '🔌'}</span>
              <div className="min-w-0 flex-1">
                <p className="truncate text-xs font-medium text-gray-900 dark:text-gray-100">{s.name}</p>
                <p className="truncate text-xs text-gray-400 font-mono">
                  {s.transport === 'sse' ? s.url : `${s.command} ${s.args.slice(0, 1).join(' ')}`}
                </p>
              </div>
              <button onClick={e => { e.stopPropagation(); setDeleteConfirm(s.name) }}
                className="shrink-0 opacity-0 group-hover:opacity-100 text-gray-300 hover:text-red-500">
                <Trash2 className="h-3 w-3" />
              </button>
            </button>
          ))}
        </div>
        <div className="p-2 border-t border-gray-100 dark:border-gray-800">
          <button onClick={startNew} className="flex w-full items-center justify-center gap-1.5 rounded-lg bg-purple-600 py-1.5 text-xs font-medium text-white hover:bg-purple-500">
            <Plus className="h-3.5 w-3.5" /> {t('mcpServers.addServer')}
          </button>
        </div>
      </div>

      {/* Right panel */}
      <div className="flex-1 overflow-y-auto">
        {editing ? (
          <div className="max-w-xl p-5 space-y-4">
            <div className="flex items-center justify-between">
              <h3 className="text-sm font-semibold text-gray-900 dark:text-gray-100">
                {isNew ? t('mcpServers.addMcpServer') : t('mcpServers.editServer', { name: editing.name })}
              </h3>
            </div>

            {/* Add mode selector — only for new */}
            {isNew && (
              <div className="flex rounded-xl border border-gray-200 dark:border-gray-700 overflow-hidden">
                {([
                  { id: 'local' as AddMode, icon: <FolderOpen className="h-3.5 w-3.5" />, label: t('mcpServers.modeLocalDir') },
                  { id: 'text'  as AddMode, icon: <Wrench className="h-3.5 w-3.5" />,      label: t('mcpServers.modeSmartParse') },
                  { id: 'manual'as AddMode, icon: <Plus className="h-3.5 w-3.5" />,         label: t('mcpServers.modeManual') },
                ]).map(m => (
                  <button key={m.id} onClick={() => setAddMode(m.id)}
                    className={`flex flex-1 items-center justify-center gap-1.5 py-2 text-xs font-medium transition-colors ${
                      addMode === m.id
                        ? 'bg-purple-50 text-purple-700 dark:bg-purple-900/20 dark:text-purple-400'
                        : 'text-gray-500 hover:bg-gray-50 dark:text-gray-400 dark:hover:bg-gray-800'
                    }`}>
                    {m.icon}{m.label}
                  </button>
                ))}
              </div>
            )}

            {/* Local directory mode */}
            {isNew && addMode === 'local' && (
              <div className="space-y-2">
                <F label={t('mcpServers.localDirLabel')}>
                  <div className="flex gap-2">
                    <input value={localDir} onChange={e => setLocalDir(e.target.value)}
                      placeholder="D:/my-mcp-server" className="field-input flex-1 font-mono text-xs" />
                    <button onClick={pickLocalDir} className="rounded-lg border border-gray-200 px-2.5 text-gray-500 hover:bg-gray-50 dark:border-gray-700">
                      <FolderOpen className="h-4 w-4" />
                    </button>
                    <button onClick={() => localDir && scanLocal(localDir)} disabled={!localDir || scanning}
                      className="flex items-center gap-1 rounded-lg bg-purple-600 px-3 text-xs text-white hover:bg-purple-500 disabled:opacity-50">
                      {scanning ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : t('mcpServers.scan')}
                    </button>
                  </div>
                </F>
                <p className="text-xs text-gray-400">{activeProvider ? t('mcpServers.localDirHintAi') : t('mcpServers.localDirHint')}</p>
              </div>
            )}

            {/* Smart parse mode */}
            {isNew && addMode === 'text' && (
              <div className="space-y-2">
                <F label={t('mcpServers.pasteLabel')}>
                  <textarea value={parseText} onChange={e => setParseText(e.target.value)}
                    rows={5} placeholder={`npx @modelcontextprotocol/server-filesystem /path/to/dir\n\nor paste a JSON config snippet, README excerpt, SSE URL...`}
                    className="field-input font-mono text-xs resize-none" />
                </F>
                <button type="button" onClick={parseText_} disabled={scanning || !parseText.trim()}
                  className="flex items-center gap-1.5 rounded-lg bg-purple-600 px-3 py-1.5 text-xs text-white hover:bg-purple-500 disabled:opacity-50">
                  {scanning ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : <Wrench className="h-3.5 w-3.5" />}
                  {scanning ? t('mcpServers.parsing') : t('mcpServers.parseWithAi')}
                </button>
                {!activeProvider && <p className="text-xs text-yellow-600">⚠ {t('mcpServers.smartParseLlmHint')}</p>}
              </div>
            )}

            {error && (
              <div className="flex items-center gap-2 rounded-lg bg-red-50 dark:bg-red-900/20 px-3 py-2 text-xs text-red-600 dark:text-red-400">
                <AlertCircle className="h-3.5 w-3.5 shrink-0" />{error}
              </div>
            )}

            {/* Warnings */}
            {warnings.length > 0 && (
              <div className="rounded-lg bg-yellow-50 dark:bg-yellow-900/20 border border-yellow-200 dark:border-yellow-800 p-3 space-y-1">
                {warnings.map((w, i) => (
                  <p key={i} className="text-xs text-yellow-700 dark:text-yellow-400 flex items-start gap-1.5">
                    <AlertCircle className="h-3.5 w-3.5 shrink-0 mt-0.5" />{w}
                  </p>
                ))}
              </div>
            )}

            {/* Form fields — shown after scan or in manual mode */}
            {(addMode === 'manual' || !isNew || editing.name || editing.command || editing.url) && (
              <>
                {/* Transport selector */}
                <div className="flex items-center gap-3">
                  <span className="text-xs font-medium text-gray-500">{t('mcpServers.transport')}</span>
                  {['stdio', 'sse'].map(tp => (
                    <label key={tp} className="flex items-center gap-1.5 cursor-pointer">
                      <input type="radio" name="transport" value={tp}
                        checked={(editing.transport || 'stdio') === tp}
                        onChange={() => setEditing(v => v ? { ...v, transport: tp } : v)} />
                      <span className="text-xs text-gray-700 dark:text-gray-300">{tp === 'stdio' ? t('mcpServers.transportStdio') : t('mcpServers.transportSse')}</span>
                    </label>
                  ))}
                </div>

                <F label={t('mcpServers.serverIdLabel')}>
                  <input value={editing.name} disabled={!isNew}
                    onChange={e => setEditing(v => v ? { ...v, name: e.target.value } : v)}
                    placeholder="filesystem" className="field-input disabled:opacity-50" />
                </F>

                <F label={t('mcpServers.descriptionLabel')}>
                  <input value={editing.description || ''} onChange={e => setEditing(v => v ? { ...v, description: e.target.value } : v)}
                    placeholder={t('mcpServers.descriptionPlaceholder')} className="field-input" />
                </F>

                {isSSE ? (
                  <>
                    <F label={t('mcpServers.urlLabel')}>
                      <input value={editing.url || ''} onChange={e => setEditing(v => v ? { ...v, url: e.target.value } : v)}
                        placeholder="https://mcp.example.com/sse" className="field-input font-mono" />
                    </F>
                    <F label={t('mcpServers.headersLabel')}>
                      <EnvEditor
                        env={editing.headers || {}}
                        onChange={h => setEditing(v => v ? { ...v, headers: h } : v)}
                        keyPlaceholder="Authorization"
                        valPlaceholder="Bearer token..."
                      />
                    </F>
                  </>
                ) : (
                  <>
                    <F label={t('mcpServers.commandLabel')}>
                      <input value={editing.command} onChange={e => setEditing(v => v ? { ...v, command: e.target.value } : v)}
                        placeholder="npx / node / python / uvx" className="field-input font-mono" />
                    </F>
                    <F label={t('mcpServers.argumentsLabel')}>
                      <div className="space-y-1.5">
                        {editing.args.map((a, i) => (
                          <div key={i} className="flex gap-2">
                            <input value={a} onChange={e => setEditing(v => v ? { ...v, args: v.args.map((x, j) => j === i ? e.target.value : x) } : v)}
                              className="field-input flex-1 font-mono text-xs" />
                            <button onClick={() => setEditing(v => v ? { ...v, args: v.args.filter((_, j) => j !== i) } : v)}
                              className="text-gray-400 hover:text-red-500"><Minus className="h-4 w-4" /></button>
                          </div>
                        ))}
                        <button onClick={() => setEditing(v => v ? { ...v, args: [...v.args, ''] } : v)}
                          className="flex items-center gap-1 text-xs text-gray-400 hover:text-gray-700">
                          <Plus className="h-3 w-3" /> {t('mcpServers.addArg')}
                        </button>
                      </div>
                    </F>
                    <F label={t('mcpServers.envLabel')}>
                      <EnvEditor
                        env={editing.env}
                        onChange={env => setEditing(v => v ? { ...v, env } : v)}
                      />
                    </F>
                  </>
                )}

                <div className="flex gap-2">
                  <button onClick={save} disabled={saving}
                    className="flex items-center gap-1.5 rounded-lg bg-purple-600 px-3 py-1.5 text-xs text-white hover:bg-purple-500 disabled:opacity-60">
                    {saving && <Loader2 className="h-3 w-3 animate-spin" />} {t('mcpServers.save')}
                  </button>
                  <button onClick={() => setEditing(null)} className="rounded-lg px-3 py-1.5 text-xs text-gray-500 hover:bg-gray-100 dark:hover:bg-gray-800">{t('mcpServers.cancel')}</button>
                </div>
                <p className="text-xs text-gray-400">{t('mcpServers.configLabel')} <span className="font-mono">{configPath}</span></p>
              </>
            )}
          </div>
        ) : (
          <div className="flex h-full items-center justify-center text-gray-400">
            <div className="text-center space-y-2">
              <div className="text-4xl">🔌</div>
              <p className="text-sm">{t('mcpServers.selectServerToEdit')}</p>
              <p className="text-xs text-gray-400">{t('mcpServers.orClickAddServer')}</p>
            </div>
          </div>
        )}
      </div>

      {deleteConfirm && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40">
          <div className="rounded-2xl border border-gray-200 bg-white p-5 shadow-xl dark:border-gray-700 dark:bg-gray-900 w-72">
            <p className="text-sm font-semibold mb-1 text-gray-900 dark:text-gray-100">{t('mcpServers.deletePrefix')} <span className="font-mono">{deleteConfirm}</span>?</p>
            <p className="text-xs text-gray-500 mb-4">{t('mcpServers.deleteRemovesFrom')}</p>
            <div className="flex gap-2 justify-end">
              <button onClick={() => setDeleteConfirm(null)} className="px-3 py-1.5 text-xs text-gray-500 hover:bg-gray-100 rounded-lg">{t('mcpServers.cancel')}</button>
              <button onClick={() => del(deleteConfirm)} className="px-3 py-1.5 text-xs bg-red-600 text-white rounded-lg hover:bg-red-500">{t('mcpServers.delete')}</button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}

function EnvEditor({ env, onChange, keyPlaceholder = 'KEY', valPlaceholder = 'value' }: {
  env: Record<string, string>; onChange: (e: Record<string, string>) => void
  keyPlaceholder?: string; valPlaceholder?: string
}) {
  const [pairs, setPairs] = useState<[string, string][]>(() => Object.entries(env))
  const prevEnvRef = useRef(env)
  useEffect(() => {
    if (prevEnvRef.current !== env) {
      prevEnvRef.current = env
      setPairs(Object.entries(env))
    }
  }, [env])

  function update(newPairs: [string, string][]) {
    setPairs(newPairs)
    const obj: Record<string, string> = {}
    newPairs.forEach(([k, v]) => { if (k.trim()) obj[k.trim()] = v })
    onChange(obj)
  }

  return (
    <div className="space-y-1.5">
      {pairs.map(([k, v], i) => (
        <div key={i} className="flex gap-2">
          <input value={k} onChange={e => update(pairs.map((p, j) => j === i ? [e.target.value, p[1]] : p))}
            placeholder={keyPlaceholder} className="field-input flex-1 font-mono text-xs" />
          <input value={v} onChange={e => update(pairs.map((p, j) => j === i ? [p[0], e.target.value] : p))}
            placeholder={valPlaceholder} className="field-input flex-1 font-mono text-xs" />
          <button type="button" onClick={() => update(pairs.filter((_, j) => j !== i))} className="text-gray-400 hover:text-red-500">
            <Minus className="h-4 w-4" />
          </button>
        </div>
      ))}
      <button type="button" onClick={() => update([...pairs, ['', '']])}
        className="flex items-center gap-1 text-xs text-gray-400 hover:text-gray-700 dark:hover:text-gray-300">
        <Plus className="h-3 w-3" /> Add
      </button>
    </div>
  )
}

function F({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div>
      <label className="mb-1 block text-xs font-medium text-gray-500 dark:text-gray-400">{label}</label>
      {children}
    </div>
  )
}
