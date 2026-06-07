import { useState } from 'react'
import type { AgentConfig } from '../types/agent'
import { X, Plus, Minus, Loader2, FolderOpen, Sparkles, AlertCircle, Eye, EyeOff } from 'lucide-react'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'

interface Props {
  initial?: Partial<AgentConfig>
  onSave: (config: Partial<AgentConfig>) => Promise<void>
  onClose: () => void
}

interface ScanResult {
  name: string
  command: string
  args: string[]
  port?: number
  description: string
  project_type: string
}

const PROJECT_TYPE_LABELS: Record<string, string> = {
  python: '🐍 Python',
  node: '🟩 Node.js',
  rust: '🦀 Rust',
  go: '🐹 Go',
  binary: '⚙️ Executable',
  'npm-global': '📦 npm global',
  unknown: '❓ Unknown',
}

export function AgentForm({ initial, onSave, onClose }: Props) {
  const [name, setName] = useState(initial?.name ?? '')
  const [description, setDescription] = useState(initial?.description ?? '')
  const [command, setCommand] = useState(initial?.command ?? '')
  const [argInput, setArgInput] = useState((initial?.args ?? []).join(' '))
  const [workingDir, setWorkingDir] = useState(initial?.working_dir ?? '')
  const [port, setPort] = useState(initial?.port?.toString() ?? '')
  const [uiToken, setUiToken] = useState(initial?.ui_token ?? '')
  const [showToken, setShowToken] = useState(false)
  const [autoRestart, setAutoRestart] = useState(initial?.auto_restart ?? false)
  const [envPairs, setEnvPairs] = useState<[string, string][]>(
    Object.entries(initial?.env ?? {})
  )
  const [saving, setSaving] = useState(false)
  const [scanning, setScanning] = useState(false)
  const [error, setError] = useState('')
  const [scanHint, setScanHint] = useState('')

  async function pickDirectory() {
    try {
      const selected = await open({ directory: true, multiple: false })
      if (!selected || typeof selected !== 'string') return
      setWorkingDir(selected)
      await scanDirectory(selected)
    } catch {
      // user cancelled
    }
  }

  async function scanDirectory(dir: string) {
    if (!dir) return
    setScanning(true)
    setScanHint('')
    try {
      const result = await invoke<ScanResult>('scan_project_dir', { dir })
      if (!name) setName(result.name)
      if (!description && result.description) setDescription(result.description)
      if (!command && result.command) setCommand(result.command)
      if (!argInput && result.args.length) setArgInput(result.args.join(' '))
      if (!port && result.port) setPort(result.port.toString())

      if (result.project_type !== 'unknown') {
        setScanHint(`Detected: ${PROJECT_TYPE_LABELS[result.project_type] ?? result.project_type}`)
      } else {
        setScanHint('Could not detect project type — please fill in manually.')
      }
    } catch (e) {
      setScanHint(`Scan failed: ${e}`)
    } finally {
      setScanning(false)
    }
  }

  async function handleWorkingDirChange(val: string) {
    setWorkingDir(val)
    if (val.length > 3) {
      await scanDirectory(val)
    }
  }

  async function handleSubmit(e: React.FormEvent) {
    e.preventDefault()
    if (!name.trim()) { setError('Name is required'); return }
    if (!command.trim()) { setError('Command is required'); return }

    const env: Record<string, string> = {}
    envPairs.forEach(([k, v]) => { if (k.trim()) env[k.trim()] = v })

    setSaving(true)
    setError('')
    try {
      await onSave({
        id: initial?.id,
        name: name.trim(),
        description: description.trim(),
        command: command.trim(),
        args: argInput.trim() ? argInput.trim().split(/\s+/) : [],
        working_dir: workingDir.trim(),
        env,
        port: port ? Number(port) : undefined,
        ui_token: uiToken.trim() || undefined,
        auto_restart: autoRestart,
      })
      onClose()
    } catch (err) {
      setError(String(err))
      setSaving(false)
    }
  }

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 dark:bg-black/60">
      <div className="w-full max-w-lg rounded-2xl border border-gray-200 bg-white shadow-2xl dark:border-gray-700 dark:bg-gray-900">
        {/* Header */}
        <div className="flex items-center justify-between border-b border-gray-200 px-6 py-4 dark:border-gray-800">
          <h3 className="text-base font-semibold text-gray-900 dark:text-gray-100">
            {initial?.id ? 'Edit Agent' : 'New Agent'}
          </h3>
          <button onClick={onClose} className="rounded-md p-1 text-gray-400 hover:bg-gray-100 hover:text-gray-600 dark:hover:bg-gray-800 dark:hover:text-gray-300">
            <X className="h-5 w-5" />
          </button>
        </div>

        <form onSubmit={handleSubmit}>
          <div className="max-h-[65vh] overflow-y-auto px-6 py-5 space-y-4">

            <Field label="Working Directory">
              <div className="flex gap-2">
                <input
                  value={workingDir}
                  onChange={e => handleWorkingDirChange(e.target.value)}
                  placeholder="D:/projects/my-agent"
                  className="field-input flex-1 font-mono"
                />
                <button
                  type="button"
                  onClick={pickDirectory}
                  className="flex items-center rounded-lg border border-gray-200 px-3 py-2 text-gray-500 transition-colors hover:bg-gray-100 hover:text-gray-700 dark:border-gray-700 dark:text-gray-400 dark:hover:bg-gray-800 dark:hover:text-gray-200"
                  title="Browse folder"
                >
                  <FolderOpen className="h-4 w-4" />
                </button>
              </div>
              {(scanning || scanHint) && (
                <div className={`mt-2 flex items-center gap-1.5 text-xs rounded-lg px-3 py-2 ${
                  scanning
                    ? 'bg-blue-50 text-blue-600 dark:bg-blue-900/30 dark:text-blue-400'
                    : scanHint.startsWith('Detected')
                      ? 'bg-green-50 text-green-700 dark:bg-green-900/30 dark:text-green-400'
                      : 'bg-yellow-50 text-yellow-700 dark:bg-yellow-900/30 dark:text-yellow-400'
                }`}>
                  {scanning
                    ? <><Loader2 className="h-3.5 w-3.5 animate-spin" /> Scanning project...</>
                    : <><Sparkles className="h-3.5 w-3.5" /> {scanHint}</>
                  }
                </div>
              )}
            </Field>

            <div className="border-t border-gray-200 dark:border-gray-800" />

            <Field label="Name *">
              <input autoFocus={!!initial?.id} value={name} onChange={e => setName(e.target.value)}
                placeholder="My Agent" className="field-input" />
            </Field>

            <Field label="Description">
              <input value={description} onChange={e => setDescription(e.target.value)}
                placeholder="What does this agent do?" className="field-input" />
            </Field>

            <Field label="Command *">
              <input value={command} onChange={e => setCommand(e.target.value)}
                placeholder="python / node / uvicorn / cargo" className="field-input font-mono" />
            </Field>

            <Field label="Arguments">
              <input value={argInput} onChange={e => setArgInput(e.target.value)}
                placeholder="main.py --port 8080 --verbose" className="field-input font-mono" />
            </Field>

            <div className="grid grid-cols-2 gap-3">
              <Field label="Port (optional)">
                <input type="number" value={port} onChange={e => setPort(e.target.value)}
                  placeholder="8080" min={1} max={65535} className="field-input" />
              </Field>
              <Field label="">
                <label className="flex h-full items-end gap-2 pb-0.5 text-sm text-gray-600 cursor-pointer dark:text-gray-300">
                  <input type="checkbox" checked={autoRestart}
                    onChange={e => setAutoRestart(e.target.checked)} className="rounded" />
                  Auto-restart on crash
                </label>
              </Field>
            </div>

            <Field label="UI Access Key (optional)">
              <div className="flex gap-2">
                <input
                  type={showToken ? 'text' : 'password'}
                  value={uiToken} onChange={e => setUiToken(e.target.value)}
                  placeholder="ad01a3b0282356d6..."
                  className="field-input flex-1 font-mono" autoComplete="off"
                />
                <button type="button" onClick={() => setShowToken(v => !v)}
                  className="flex items-center rounded-lg border border-gray-200 px-3 py-2 text-gray-400 hover:bg-gray-100 hover:text-gray-600 dark:border-gray-700 dark:hover:bg-gray-800 dark:hover:text-gray-200"
                  title={showToken ? 'Hide' : 'Show'}>
                  {showToken ? <EyeOff className="h-4 w-4" /> : <Eye className="h-4 w-4" />}
                </button>
              </div>
              <p className="mt-1 text-xs text-gray-400 dark:text-gray-600">
                打开 UI 时自动填写到登录表单并连接，无需手动输入
              </p>
            </Field>

            <Field label="Environment Variables">
              <div className="space-y-2">
                {envPairs.map(([k, v], i) => (
                  <div key={i} className="flex gap-2">
                    <input value={k}
                      onChange={e => setEnvPairs(p => p.map((pair, j) => j === i ? [e.target.value, pair[1]] : pair))}
                      placeholder="KEY" className="field-input flex-1 font-mono" />
                    <input value={v}
                      onChange={e => setEnvPairs(p => p.map((pair, j) => j === i ? [pair[0], e.target.value] : pair))}
                      placeholder="value" className="field-input flex-1 font-mono" />
                    <button type="button"
                      onClick={() => setEnvPairs(p => p.filter((_, j) => j !== i))}
                      className="text-gray-400 hover:text-red-500 dark:text-gray-600 dark:hover:text-red-400">
                      <Minus className="h-4 w-4" />
                    </button>
                  </div>
                ))}
                <button type="button" onClick={() => setEnvPairs(p => [...p, ['', '']])}
                  className="flex items-center gap-1 text-xs text-gray-400 hover:text-gray-700 dark:text-gray-500 dark:hover:text-gray-300">
                  <Plus className="h-3.5 w-3.5" /> Add variable
                </button>
              </div>
            </Field>

            {error && (
              <div className="flex items-center gap-2 rounded-lg bg-red-50 px-3 py-2 text-sm text-red-600 dark:bg-red-900/30 dark:text-red-400">
                <AlertCircle className="h-4 w-4 shrink-0" />
                {error}
              </div>
            )}
          </div>

          {/* Footer */}
          <div className="flex justify-end gap-2 border-t border-gray-200 px-6 py-4 dark:border-gray-800">
            <button type="button" onClick={onClose}
              className="rounded-lg px-4 py-2 text-sm text-gray-500 hover:bg-gray-100 hover:text-gray-700 dark:text-gray-400 dark:hover:bg-gray-800 dark:hover:text-gray-200">
              Cancel
            </button>
            <button type="submit" disabled={saving}
              className="flex items-center gap-2 rounded-lg bg-blue-600 px-4 py-2 text-sm font-medium text-white hover:bg-blue-500 disabled:opacity-60">
              {saving && <Loader2 className="h-4 w-4 animate-spin" />}
              {saving ? 'Saving...' : 'Save Agent'}
            </button>
          </div>
        </form>
      </div>
    </div>
  )
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div>
      {label && <label className="mb-1 block text-sm font-medium text-gray-600 dark:text-gray-400">{label}</label>}
      {children}
    </div>
  )
}
