import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { Plus, Trash2, Save, X, Minus, FolderOpen, AlertCircle, RefreshCw } from 'lucide-react'
import { open } from '@tauri-apps/plugin-dialog'

interface McpServer {
  name: string
  command: string
  args: string[]
  env: Record<string, string>
}

const EMPTY: McpServer = { name: '', command: '', args: [], env: {} }

export function McpManager() {
  const [servers, setServers] = useState<McpServer[]>([])
  const [configPath, setConfigPath] = useState('')
  const [editing, setEditing] = useState<McpServer | null>(null)
  const [isNew, setIsNew] = useState(false)
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState('')
  const [deleteConfirm, setDeleteConfirm] = useState<string | null>(null)

  async function load() {
    const [list, path] = await Promise.all([
      invoke<McpServer[]>('list_mcp_servers'),
      invoke<string>('get_mcp_config_path'),
    ])
    setServers(list)
    setConfigPath(path)
  }

  useEffect(() => { load() }, [])

  function startNew() {
    setEditing({ ...EMPTY })
    setIsNew(true)
    setError('')
  }

  function startEdit(s: McpServer) {
    setEditing({ ...s, env: { ...s.env } })
    setIsNew(false)
    setError('')
  }

  async function save() {
    if (!editing) return
    if (!editing.name.trim()) { setError('Name is required'); return }
    if (!editing.command.trim()) { setError('Command is required'); return }
    setSaving(true)
    setError('')
    try {
      await invoke('save_mcp_server', { server: editing })
      await load()
      setEditing(null)
    } catch (e) {
      setError(String(e))
    } finally {
      setSaving(false)
    }
  }

  async function del(name: string) {
    await invoke('delete_mcp_server', { name })
    setDeleteConfirm(null)
    await load()
    if (editing?.name === name) setEditing(null)
  }

  async function pickExecutable() {
    if (!editing) return
    const selected = await open({ multiple: false })
    if (selected && typeof selected === 'string') {
      setEditing(e => e ? { ...e, command: selected } : e)
    }
  }

  return (
    <div className="flex h-full">
      {/* Server list */}
      <div className="w-64 shrink-0 border-r border-gray-200 dark:border-gray-700 flex flex-col bg-white dark:bg-gray-900">
        <div className="flex items-center justify-between border-b border-gray-200 dark:border-gray-700 px-4 py-3">
          <div>
            <h2 className="text-sm font-semibold text-gray-900 dark:text-gray-100">MCP Servers</h2>
            <p className="text-xs text-gray-500">{servers.length} configured</p>
          </div>
          <div className="flex gap-1">
            <button onClick={load} className="rounded-md p-1.5 text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-800">
              <RefreshCw className="h-4 w-4" />
            </button>
            <button onClick={startNew} className="rounded-md p-1.5 text-gray-400 hover:bg-gray-100 dark:hover:bg-gray-800" title="Add server">
              <Plus className="h-4 w-4" />
            </button>
          </div>
        </div>

        <div className="flex-1 overflow-y-auto p-2 space-y-1">
          {servers.length === 0 && (
            <p className="py-8 text-center text-xs text-gray-400">No MCP servers configured</p>
          )}
          {servers.map(s => (
            <button
              key={s.name}
              onClick={() => startEdit(s)}
              className={`group flex w-full items-center gap-3 rounded-xl px-3 py-2.5 text-left transition-colors ${
                editing?.name === s.name && !isNew
                  ? 'border-l-2 border-blue-500 bg-blue-50 dark:bg-blue-900/20'
                  : 'hover:bg-gray-50 dark:hover:bg-gray-800'
              }`}
            >
              <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg bg-purple-100 text-sm dark:bg-purple-900/30">
                🔌
              </div>
              <div className="min-w-0 flex-1">
                <p className="truncate text-sm font-medium text-gray-900 dark:text-gray-100">{s.name}</p>
                <p className="truncate text-xs text-gray-400 font-mono">{s.command}</p>
              </div>
              <button
                onClick={e => { e.stopPropagation(); setDeleteConfirm(s.name) }}
                className="shrink-0 rounded p-1 text-gray-300 opacity-0 hover:bg-red-50 hover:text-red-500 group-hover:opacity-100 dark:hover:bg-red-900/20"
              >
                <Trash2 className="h-3.5 w-3.5" />
              </button>
            </button>
          ))}
        </div>

        <div className="border-t border-gray-200 dark:border-gray-700 p-3">
          <button
            onClick={startNew}
            className="flex w-full items-center justify-center gap-2 rounded-xl bg-purple-600 px-3 py-2 text-sm font-medium text-white hover:bg-purple-500"
          >
            <Plus className="h-4 w-4" /> Add Server
          </button>
        </div>
      </div>

      {/* Edit panel */}
      <div className="flex-1 overflow-y-auto bg-gray-50 dark:bg-gray-950">
        {editing ? (
          <div className="max-w-2xl p-6 space-y-5">
            <div className="flex items-center justify-between">
              <h3 className="text-base font-semibold text-gray-900 dark:text-gray-100">
                {isNew ? 'New MCP Server' : `Edit: ${editing.name}`}
              </h3>
              <button onClick={() => setEditing(null)} className="text-gray-400 hover:text-gray-600">
                <X className="h-5 w-5" />
              </button>
            </div>

            <Field label="Server Name *">
              <input
                value={editing.name}
                onChange={e => setEditing(v => v ? { ...v, name: e.target.value } : v)}
                disabled={!isNew}
                placeholder="e.g. filesystem, brave-search"
                className="field-input disabled:opacity-50 disabled:cursor-not-allowed"
              />
              {!isNew && <p className="mt-1 text-xs text-gray-400">Name cannot be changed after creation.</p>}
            </Field>

            <Field label="Command *">
              <div className="flex gap-2">
                <input
                  value={editing.command}
                  onChange={e => setEditing(v => v ? { ...v, command: e.target.value } : v)}
                  placeholder="npx / node / python / /path/to/binary"
                  className="field-input flex-1 font-mono"
                />
                <button
                  onClick={pickExecutable}
                  className="flex items-center gap-1.5 rounded-lg border border-gray-300 px-3 py-2 text-sm text-gray-600 hover:bg-gray-50 dark:border-gray-600 dark:text-gray-400 dark:hover:bg-gray-800"
                >
                  <FolderOpen className="h-4 w-4" />
                </button>
              </div>
            </Field>

            <Field label="Arguments">
              <ArgsEditor
                args={editing.args}
                onChange={args => setEditing(v => v ? { ...v, args } : v)}
              />
            </Field>

            <Field label="Environment Variables">
              <EnvEditor
                env={editing.env}
                onChange={env => setEditing(v => v ? { ...v, env } : v)}
              />
            </Field>

            {/* Preview */}
            <div className="rounded-xl border border-gray-200 dark:border-gray-700 bg-white dark:bg-gray-900 p-4">
              <p className="mb-2 text-xs font-medium uppercase tracking-wide text-gray-500">Config preview</p>
              <pre className="overflow-x-auto text-xs text-gray-600 dark:text-gray-300">
                {JSON.stringify({
                  [editing.name || 'server-name']: {
                    command: editing.command,
                    args: editing.args,
                    ...(Object.keys(editing.env).length ? { env: editing.env } : {}),
                  }
                }, null, 2)}
              </pre>
            </div>

            {error && (
              <div className="flex items-center gap-2 rounded-lg bg-red-50 px-3 py-2 text-sm text-red-600 dark:bg-red-900/20 dark:text-red-400">
                <AlertCircle className="h-4 w-4 shrink-0" />
                {error}
              </div>
            )}

            <div className="flex gap-2">
              <button
                onClick={save}
                disabled={saving}
                className="flex items-center gap-2 rounded-lg bg-purple-600 px-4 py-2 text-sm font-medium text-white hover:bg-purple-500 disabled:opacity-60"
              >
                <Save className="h-4 w-4" />
                {saving ? 'Saving...' : 'Save to claude_desktop_config.json'}
              </button>
              <button
                onClick={() => setEditing(null)}
                className="rounded-lg px-4 py-2 text-sm text-gray-600 hover:bg-gray-200 dark:text-gray-400 dark:hover:bg-gray-800"
              >
                Cancel
              </button>
            </div>

            <p className="text-xs text-gray-400">
              Config file: <span className="font-mono">{configPath}</span>
            </p>
          </div>
        ) : (
          <div className="flex h-full items-center justify-center text-gray-400">
            <div className="text-center">
              <div className="text-4xl mb-3">🔌</div>
              <p className="text-sm">Select a server to edit, or add a new one</p>
            </div>
          </div>
        )}
      </div>

      {/* Delete confirm modal */}
      {deleteConfirm && (
        <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40">
          <div className="rounded-2xl border border-gray-200 bg-white p-6 shadow-xl dark:border-gray-700 dark:bg-gray-900 w-80">
            <h3 className="font-semibold text-gray-900 dark:text-gray-100 mb-2">Delete MCP Server</h3>
            <p className="text-sm text-gray-500 mb-4">
              Remove <span className="font-mono font-medium">{deleteConfirm}</span> from claude_desktop_config.json?
            </p>
            <div className="flex gap-2 justify-end">
              <button onClick={() => setDeleteConfirm(null)} className="px-3 py-1.5 text-sm text-gray-500 hover:bg-gray-100 rounded-lg">Cancel</button>
              <button onClick={() => del(deleteConfirm)} className="px-3 py-1.5 text-sm bg-red-600 text-white rounded-lg hover:bg-red-500">Delete</button>
            </div>
          </div>
        </div>
      )}
    </div>
  )
}

function ArgsEditor({ args, onChange }: { args: string[]; onChange: (a: string[]) => void }) {
  return (
    <div className="space-y-2">
      {args.map((arg, i) => (
        <div key={i} className="flex gap-2">
          <input
            value={arg}
            onChange={e => onChange(args.map((a, j) => j === i ? e.target.value : a))}
            placeholder={`arg ${i + 1}`}
            className="field-input flex-1 font-mono"
          />
          <button onClick={() => onChange(args.filter((_, j) => j !== i))} className="text-gray-400 hover:text-red-500">
            <Minus className="h-4 w-4" />
          </button>
        </div>
      ))}
      <button onClick={() => onChange([...args, ''])} className="flex items-center gap-1 text-xs text-gray-500 hover:text-gray-700 dark:hover:text-gray-300">
        <Plus className="h-3.5 w-3.5" /> Add argument
      </button>
    </div>
  )
}

function EnvEditor({ env, onChange }: { env: Record<string, string>; onChange: (e: Record<string, string>) => void }) {
  const pairs = Object.entries(env)
  const update = (pairs: [string, string][]) => {
    const obj: Record<string, string> = {}
    pairs.forEach(([k, v]) => { if (k.trim()) obj[k.trim()] = v })
    onChange(obj)
  }
  return (
    <div className="space-y-2">
      {pairs.map(([k, v], i) => (
        <div key={i} className="flex gap-2">
          <input value={k} onChange={e => update(pairs.map((p, j) => j === i ? [e.target.value, p[1]] : p))}
            placeholder="KEY" className="field-input flex-1 font-mono" />
          <input value={v} onChange={e => update(pairs.map((p, j) => j === i ? [p[0], e.target.value] : p))}
            placeholder="value" className="field-input flex-1 font-mono" />
          <button onClick={() => update(pairs.filter((_, j) => j !== i))} className="text-gray-400 hover:text-red-500">
            <Minus className="h-4 w-4" />
          </button>
        </div>
      ))}
      <button onClick={() => update([...pairs, ['', '']])} className="flex items-center gap-1 text-xs text-gray-500 hover:text-gray-700 dark:hover:text-gray-300">
        <Plus className="h-3.5 w-3.5" /> Add variable
      </button>
    </div>
  )
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div>
      <label className="mb-1.5 block text-sm font-medium text-gray-700 dark:text-gray-300">{label}</label>
      {children}
    </div>
  )
}
