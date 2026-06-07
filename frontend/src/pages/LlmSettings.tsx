import { useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { Plus, Trash2, CheckCircle, XCircle, Loader2, Eye, EyeOff, AlertCircle } from 'lucide-react'

interface LlmProvider {
  id: string
  name: string
  base_url: string
  model: string
  api_key: string
  is_custom: boolean
  enabled: boolean
  context_window?: number
  max_output_tokens?: number
}

const EMPTY_CUSTOM: LlmProvider = {
  id: '', name: '', base_url: '', model: '',
  api_key: '', is_custom: true, enabled: true,
}

export function LlmSettings() {
  const [providers, setProviders] = useState<LlmProvider[]>([])
  const [showCustomForm, setShowCustomForm] = useState(false)
  const [editingCustom, setEditingCustom] = useState<LlmProvider | null>(null)
  const [form, setForm] = useState<LlmProvider>({ ...EMPTY_CUSTOM })
  const [testing, setTesting] = useState<string | null>(null)
  const [testResult, setTestResult] = useState<Record<string, { ok: boolean; msg: string }>>({})
  const [saving, setSaving] = useState(false)
  const [error, setError] = useState('')
  const [showKeys, setShowKeys] = useState<Record<string, boolean>>({})

  async function load() {
    const list = await invoke<LlmProvider[]>('list_llm_providers')
    setProviders(list)
  }

  useEffect(() => { load() }, [])

  async function saveBuiltin(p: LlmProvider) {
    await invoke('save_llm_provider', { provider: p })
    await load()
  }

  async function testProvider(p: LlmProvider) {
    setTesting(p.id)
    try {
      const msg = await invoke<string>('test_llm_provider', { provider: p })
      setTestResult(r => ({ ...r, [p.id]: { ok: true, msg } }))
    } catch (e) {
      setTestResult(r => ({ ...r, [p.id]: { ok: false, msg: String(e) } }))
    } finally {
      setTesting(null)
    }
  }

  async function saveCustom() {
    if (!form.name.trim()) { setError('Name is required'); return }
    if (!form.base_url.trim()) { setError('Base URL is required'); return }
    if (!form.model.trim()) { setError('Model ID is required'); return }
    if (!form.api_key.trim()) { setError('API Key is required'); return }

    const id = editingCustom?.id || `custom_${form.name.toLowerCase().replace(/\s+/g, '_')}_${Date.now()}`
    setSaving(true)
    setError('')
    try {
      await invoke('save_llm_provider', { provider: { ...form, id, is_custom: true } })
      await load()
      setShowCustomForm(false)
      setEditingCustom(null)
      setForm({ ...EMPTY_CUSTOM })
    } catch (e) {
      setError(String(e))
    } finally {
      setSaving(false)
    }
  }

  async function deleteCustom(id: string) {
    await invoke('delete_llm_provider', { id })
    await load()
  }

  const builtins = providers.filter(p => !p.is_custom)
  const customs = providers.filter(p => p.is_custom)

  return (
    <div className="flex h-full flex-col overflow-y-auto bg-gray-50 dark:bg-gray-950 p-6 space-y-6">
      <div>
        <h2 className="text-base font-semibold text-gray-900 dark:text-gray-100">LLM Providers</h2>
        <p className="text-xs text-gray-500 mt-0.5">Configure API keys for language models used by MCP Agent</p>
      </div>

      {/* Built-in providers */}
      <div className="space-y-3">
        <h3 className="text-xs font-semibold uppercase tracking-wide text-gray-500">Built-in</h3>
        {builtins.map(p => (
          <BuiltinCard
            key={p.id}
            provider={p}
            showKey={showKeys[p.id] ?? false}
            onToggleKey={() => setShowKeys(s => ({ ...s, [p.id]: !s[p.id] }))}
            testResult={testResult[p.id]}
            testing={testing === p.id}
            onSave={saveBuiltin}
            onTest={testProvider}
          />
        ))}
      </div>

      {/* Custom providers */}
      <div className="space-y-3">
        <div className="flex items-center justify-between">
          <h3 className="text-xs font-semibold uppercase tracking-wide text-gray-500">Custom (OpenAI-compatible)</h3>
          <button
            onClick={() => { setForm({ ...EMPTY_CUSTOM }); setEditingCustom(null); setShowCustomForm(true); setError('') }}
            className="flex items-center gap-1 text-xs text-blue-600 hover:text-blue-500"
          >
            <Plus className="h-3.5 w-3.5" /> Add
          </button>
        </div>

        {customs.map(p => (
          <div key={p.id} className="rounded-2xl border border-gray-200 bg-white p-4 dark:border-gray-700 dark:bg-gray-900">
            <div className="flex items-center justify-between">
              <div>
                <p className="text-sm font-medium text-gray-900 dark:text-gray-100">{p.name}</p>
                <p className="text-xs font-mono text-gray-400">{p.model} · {p.base_url}</p>
              </div>
              <div className="flex items-center gap-2">
                <button
                  onClick={() => testProvider(p)}
                  disabled={testing === p.id}
                  className="text-xs text-gray-500 hover:text-blue-600"
                >
                  {testing === p.id ? <Loader2 className="h-3.5 w-3.5 animate-spin" /> : 'Test'}
                </button>
                <button
                  onClick={() => { setForm({ ...p }); setEditingCustom(p); setShowCustomForm(true); setError('') }}
                  className="text-xs text-gray-500 hover:text-gray-700"
                >Edit</button>
                <button onClick={() => deleteCustom(p.id)} className="text-gray-400 hover:text-red-500">
                  <Trash2 className="h-3.5 w-3.5" />
                </button>
                <EnableToggle
                  enabled={p.enabled}
                  onChange={v => saveBuiltin({ ...p, enabled: v })}
                />
              </div>
            </div>
            {testResult[p.id] && <TestBadge result={testResult[p.id]} />}
          </div>
        ))}

        {customs.length === 0 && !showCustomForm && (
          <p className="text-xs text-gray-400 text-center py-4">No custom models yet</p>
        )}
      </div>

      {/* Custom form */}
      {showCustomForm && (
        <div className="rounded-2xl border border-blue-200 bg-white p-5 dark:border-blue-800 dark:bg-gray-900 space-y-3">
          <h4 className="text-sm font-semibold text-gray-900 dark:text-gray-100">
            {editingCustom ? `Edit: ${editingCustom.name}` : 'Add Custom Model'}
          </h4>
          <div className="grid grid-cols-2 gap-3">
            <Field label="Display Name">
              <input value={form.name} onChange={e => setForm(f => ({ ...f, name: e.target.value }))}
                placeholder="DeepSeek / Gemini..." className="field-input" />
            </Field>
            <Field label="Model ID">
              <input value={form.model} onChange={e => setForm(f => ({ ...f, model: e.target.value }))}
                placeholder="deepseek-chat" className="field-input font-mono" />
            </Field>
          </div>
          <Field label="Base URL">
            <input value={form.base_url} onChange={e => setForm(f => ({ ...f, base_url: e.target.value }))}
              placeholder="https://api.deepseek.com" className="field-input font-mono" />
          </Field>
          <Field label="API Key">
            <input type="password" value={form.api_key} onChange={e => setForm(f => ({ ...f, api_key: e.target.value }))}
              placeholder="sk-..." className="field-input font-mono" />
          </Field>
          {error && (
            <div className="flex items-center gap-2 rounded-lg bg-red-50 px-3 py-2 text-xs text-red-600 dark:bg-red-900/20 dark:text-red-400">
              <AlertCircle className="h-3.5 w-3.5 shrink-0" />{error}
            </div>
          )}
          <div className="flex gap-2">
            <button onClick={saveCustom} disabled={saving}
              className="flex items-center gap-1.5 rounded-lg bg-blue-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-blue-500 disabled:opacity-60">
              {saving && <Loader2 className="h-3 w-3 animate-spin" />} Save
            </button>
            <button onClick={() => { testProvider({ ...form, id: editingCustom?.id ?? 'temp' }) }}
              disabled={testing !== null}
              className="rounded-lg border border-gray-200 px-3 py-1.5 text-xs text-gray-600 hover:bg-gray-50 dark:border-gray-700 dark:text-gray-400">
              Test connection
            </button>
            <button onClick={() => { setShowCustomForm(false); setError('') }}
              className="rounded-lg px-3 py-1.5 text-xs text-gray-500 hover:bg-gray-100">Cancel</button>
          </div>
        </div>
      )}
    </div>
  )
}

function BuiltinCard({ provider, showKey, onToggleKey, testResult, testing, onSave, onTest }: {
  provider: LlmProvider
  showKey: boolean
  onToggleKey: () => void
  testResult?: { ok: boolean; msg: string }
  testing: boolean
  onSave: (p: LlmProvider) => void
  onTest: (p: LlmProvider) => void
}) {
  const [key, setKey] = useState(provider.api_key)
  const [model, setModel] = useState(provider.model)

  useEffect(() => { setKey(provider.api_key); setModel(provider.model) }, [provider])

  function save() {
    onSave({ ...provider, api_key: key, model, enabled: !!key })
  }

  return (
    <div className="rounded-2xl border border-gray-200 bg-white p-4 dark:border-gray-700 dark:bg-gray-900 space-y-3">
      <div className="flex items-center justify-between">
        <div className="flex items-center gap-2">
          <span className="text-sm font-semibold text-gray-900 dark:text-gray-100">{provider.name}</span>
          {provider.enabled && provider.api_key && (
            <span className="rounded-full bg-green-100 px-2 py-0.5 text-xs text-green-700 dark:bg-green-900/30 dark:text-green-400">active</span>
          )}
        </div>
        <EnableToggle enabled={provider.enabled} onChange={v => onSave({ ...provider, enabled: v })} />
      </div>

      <div className="grid grid-cols-2 gap-3">
        <Field label="Model">
          <input value={model} onChange={e => setModel(e.target.value)}
            className="field-input font-mono text-xs" placeholder={provider.model} />
        </Field>
        <Field label="API Key">
          <div className="relative">
            <input
              type={showKey ? 'text' : 'password'}
              value={key}
              onChange={e => setKey(e.target.value)}
              placeholder="sk-..."
              className="field-input font-mono text-xs pr-8"
            />
            <button onClick={onToggleKey} className="absolute right-2 top-1/2 -translate-y-1/2 text-gray-400 hover:text-gray-600">
              {showKey ? <EyeOff className="h-3.5 w-3.5" /> : <Eye className="h-3.5 w-3.5" />}
            </button>
          </div>
        </Field>
      </div>

      {testResult && <TestBadge result={testResult} />}

      <div className="flex gap-2">
        <button onClick={save}
          className="rounded-lg bg-gray-900 px-3 py-1.5 text-xs font-medium text-white hover:bg-gray-700 dark:bg-gray-100 dark:text-gray-900">
          Save
        </button>
        <button onClick={() => onTest({ ...provider, api_key: key, model })} disabled={testing || !key}
          className="flex items-center gap-1 rounded-lg border border-gray-200 px-3 py-1.5 text-xs text-gray-600 hover:bg-gray-50 disabled:opacity-40 dark:border-gray-700 dark:text-gray-400">
          {testing ? <Loader2 className="h-3 w-3 animate-spin" /> : null} Test
        </button>
      </div>
    </div>
  )
}

function TestBadge({ result }: { result: { ok: boolean; msg: string } }) {
  return (
    <div className={`flex items-center gap-1.5 rounded-lg px-3 py-2 text-xs ${
      result.ok ? 'bg-green-50 text-green-700 dark:bg-green-900/20 dark:text-green-400'
                : 'bg-red-50 text-red-600 dark:bg-red-900/20 dark:text-red-400'
    }`}>
      {result.ok ? <CheckCircle className="h-3.5 w-3.5 shrink-0" /> : <XCircle className="h-3.5 w-3.5 shrink-0" />}
      {result.msg}
    </div>
  )
}

function EnableToggle({ enabled, onChange }: { enabled: boolean; onChange: (v: boolean) => void }) {
  return (
    <button
      onClick={() => onChange(!enabled)}
      className={`relative h-5 w-9 rounded-full transition-colors ${enabled ? 'bg-blue-600' : 'bg-gray-200 dark:bg-gray-700'}`}
    >
      <span className={`absolute top-0.5 h-4 w-4 rounded-full bg-white shadow transition-transform ${enabled ? 'translate-x-4' : 'translate-x-0.5'}`} />
    </button>
  )
}

function Field({ label, children }: { label: string; children: React.ReactNode }) {
  return (
    <div>
      <label className="mb-1 block text-xs font-medium text-gray-500 dark:text-gray-400">{label}</label>
      {children}
    </div>
  )
}
