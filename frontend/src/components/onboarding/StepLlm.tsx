import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { invoke } from '@tauri-apps/api/core'
import { CheckCircle2, Loader2, XCircle } from 'lucide-react'

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

const PRESETS = [
  { id: 'deepseek', name: 'DeepSeek', base_url: 'https://api.deepseek.com/v1', model: 'deepseek-chat' },
  { id: 'openai', name: 'OpenAI', base_url: 'https://api.openai.com/v1', model: 'gpt-4o-mini' },
  { id: 'ollama', name: 'Ollama', base_url: 'http://127.0.0.1:11434/v1', model: 'qwen2.5' },
] as const

export function StepLlm({ onNext }: { onNext: () => void }) {
  const { t } = useTranslation()
  const [selected, setSelected] = useState<string | null>(null)
  const [apiKey, setApiKey] = useState('')
  const [testing, setTesting] = useState(false)
  const [testResult, setTestResult] = useState<{ ok: boolean; msg: string } | null>(null)
  const [saving, setSaving] = useState(false)

  const preset = PRESETS.find(p => p.id === selected)
  const needsKey = selected !== 'ollama'

  const buildProvider = (): LlmProvider => ({
    id: preset?.id ?? '',
    name: preset?.name ?? '',
    base_url: preset?.base_url ?? '',
    model: preset?.model ?? '',
    api_key: apiKey,
    is_custom: false,
    enabled: true,
    context_window: 128000,
    max_output_tokens: 16384,
  })

  const handleTest = async () => {
    setTesting(true)
    setTestResult(null)
    try {
      const msg = await invoke<string>('test_llm_provider', { provider: buildProvider() })
      setTestResult({ ok: true, msg })
    } catch (e) {
      setTestResult({ ok: false, msg: String(e) })
    } finally {
      setTesting(false)
    }
  }

  const handleSave = async () => {
    setSaving(true)
    try {
      await invoke('save_llm_provider', { provider: buildProvider() })
      onNext()
    } catch {
      // save failed — still advance
      onNext()
    } finally {
      setSaving(false)
    }
  }

  return (
    <div>
      <h2 className="text-lg font-bold text-gray-900 dark:text-gray-100">
        {t('onboarding.llm.title')}
      </h2>
      <p className="mt-2 text-sm text-gray-500 dark:text-gray-400">
        {t('onboarding.llm.desc')}
      </p>

      {/* Provider cards */}
      <div className="mt-5 grid grid-cols-3 gap-2">
        {PRESETS.map(p => (
          <button
            key={p.id}
            onClick={() => { setSelected(p.id); setTestResult(null) }}
            className={`rounded-xl border px-3 py-3 text-center text-sm font-medium transition-colors ${
              selected === p.id
                ? 'border-blue-500 bg-blue-50 text-blue-700 dark:border-blue-400 dark:bg-blue-500/15 dark:text-blue-300'
                : 'border-gray-200 text-gray-600 hover:border-gray-300 hover:bg-gray-50 dark:border-gray-700 dark:text-gray-400 dark:hover:border-gray-600 dark:hover:bg-gray-800'
            }`}
          >
            {p.name}
          </button>
        ))}
      </div>

      {/* API key input */}
      {selected && needsKey && (
        <div className="mt-4">
          <label className="mb-1.5 block text-xs font-medium text-gray-500 dark:text-gray-400">
            {t('onboarding.llm.apiKey')}
          </label>
          <input
            type="password"
            value={apiKey}
            onChange={e => setApiKey(e.target.value)}
            placeholder="sk-..."
            className="w-full rounded-lg border border-gray-200 bg-white px-3 py-2 text-sm outline-none transition focus:border-blue-500 focus:ring-2 focus:ring-blue-500/20 dark:border-gray-700 dark:bg-gray-800 dark:text-gray-200"
          />
        </div>
      )}

      {/* Test result */}
      {testResult && (
        <div className={`mt-3 flex items-start gap-2 rounded-lg px-3 py-2 text-xs ${
          testResult.ok
            ? 'bg-emerald-50 text-emerald-700 dark:bg-emerald-500/10 dark:text-emerald-400'
            : 'bg-red-50 text-red-700 dark:bg-red-500/10 dark:text-red-400'
        }`}>
          {testResult.ok
            ? <CheckCircle2 className="mt-0.5 h-3.5 w-3.5 shrink-0" />
            : <XCircle className="mt-0.5 h-3.5 w-3.5 shrink-0" />}
          <span className="break-all">{testResult.msg}</span>
        </div>
      )}

      {/* Actions */}
      <div className="mt-6 flex gap-2">
        {selected && (
          <>
            <button
              onClick={handleTest}
              disabled={testing || (needsKey && !apiKey)}
              className="flex-1 rounded-xl border border-gray-200 px-4 py-2.5 text-sm font-medium text-gray-700 transition-colors hover:bg-gray-50 disabled:opacity-50 dark:border-gray-700 dark:text-gray-300 dark:hover:bg-gray-800"
            >
              {testing ? <Loader2 className="mx-auto h-4 w-4 animate-spin" /> : t('onboarding.llm.test')}
            </button>
            <button
              onClick={handleSave}
              disabled={saving || (needsKey && !apiKey)}
              className="flex-1 rounded-xl bg-blue-600 px-4 py-2.5 text-sm font-medium text-white transition-colors hover:bg-blue-500 disabled:opacity-50"
            >
              {saving ? <Loader2 className="mx-auto h-4 w-4 animate-spin" /> : t('onboarding.llm.saveAndNext')}
            </button>
          </>
        )}
        {!selected && (
          <button
            onClick={onNext}
            className="w-full rounded-xl border border-gray-200 px-4 py-2.5 text-sm font-medium text-gray-500 transition-colors hover:bg-gray-50 dark:border-gray-700 dark:text-gray-400 dark:hover:bg-gray-800"
          >
            {t('onboarding.llm.skip')}
          </button>
        )}
      </div>
    </div>
  )
}
