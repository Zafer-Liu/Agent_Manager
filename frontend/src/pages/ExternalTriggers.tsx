import { useEffect, useState, useCallback } from 'react'
import { useTranslation } from 'react-i18next'
import { invoke } from '@tauri-apps/api/core'
import { useWorkflowStore, type RunSummary, type RawRunSummary, mapSummary, runStatusColor, formatTime } from '../store/workflowStore'
import { RefreshCw, Copy, Check, Power, RotateCcw, Shield } from 'lucide-react'

interface HookServerConfig {
  port: number
  auth_token: string | null
  enabled: boolean
  max_concurrent_runs: number
}

interface WorkflowTemplate {
  id: string
  name: string
  template_key: string | null
  allowed_sources?: string[]
}

export function ExternalTriggers() {
  const { t } = useTranslation()
  const { runs, fetchRuns } = useWorkflowStore()
  const [config, setConfig] = useState<HookServerConfig | null>(null)
  const [templates, setTemplates] = useState<WorkflowTemplate[]>([])
  const [hookRuns, setHookRuns] = useState<RunSummary[]>([])
  const [saving, setSaving] = useState(false)
  const [restarting, setRestarting] = useState(false)
  const [copied, setCopied] = useState(false)
  const [showToken, setShowToken] = useState(false)
  const [editPort, setEditPort] = useState('9420')
  const [editToken, setEditToken] = useState('')
  const [editEnabled, setEditEnabled] = useState(true)

  const loadData = useCallback(async () => {
    try {
      const [cfg, wfs, hrs] = await Promise.all([
        invoke<HookServerConfig>('get_hook_server_config'),
        invoke<WorkflowTemplate[]>('list_workflows'),
        invoke<RawRunSummary[]>('list_hook_triggered_runs'),
      ])
      setConfig(cfg)
      setEditPort(String(cfg.port))
      setEditToken(cfg.auth_token ?? '')
      setEditEnabled(cfg.enabled)
      setTemplates(wfs.filter((w) => w.template_key))
      setHookRuns(hrs.map(mapSummary))
    } catch (e) {
      console.error('Failed to load hook server config:', e)
    }
  }, [])

  useEffect(() => {
    loadData()
  }, [loadData])

  // 从 runs 中过滤 Hook 触发的（补充后端可能未实时刷新的）
  const allHookRuns = [...hookRuns]
  for (const r of runs) {
    if (r.trigger?.trigger === 'hook' && !allHookRuns.some((h) => h.runId === r.runId)) {
      allHookRuns.unshift(r)
    }
  }

  const curlExample = config
    ? `curl -X POST http://localhost:${config.port}/hook \
  -H "Content-Type: application/json"${config.auth_token ? ` \
  -H "Authorization: Bearer ${config.auth_token}"` : ''} \
  -d '{"title":"测试","description":"任务描述","template_key":"${templates[0]?.template_key ?? 'your-template-key'}"}'`
    : ''

  const handleSave = async () => {
    setSaving(true)
    try {
      const newConfig: HookServerConfig = {
        port: parseInt(editPort) || 9420,
        auth_token: editToken.trim() || null,
        enabled: editEnabled,
        max_concurrent_runs: config?.max_concurrent_runs ?? 5,
      }
      await invoke('set_hook_server_config', { config: newConfig })
      setConfig(newConfig)
    } catch (e) {
      console.error('Failed to save config:', e)
    }
    setSaving(false)
  }

  const handleRestart = async () => {
    setRestarting(true)
    try {
      await invoke('restart_hook_server')
      await loadData()
    } catch (e) {
      console.error('Failed to restart server:', e)
    }
    setRestarting(false)
  }

  const handleCopy = () => {
    navigator.clipboard.writeText(curlExample)
    setCopied(true)
    setTimeout(() => setCopied(false), 2000)
  }

  const generateToken = () => {
    const chars = 'abcdefghijklmnopqrstuvwxyz0123456789'
    let token = ''
    for (let i = 0; i < 32; i++) {
      token += chars[Math.floor(Math.random() * chars.length)]
    }
    setEditToken(token)
  }

  if (!config) {
    return (
      <div className="flex items-center justify-center h-full text-gray-400 text-sm">
        {t('common.loading')}
      </div>
    )
  }

  return (
    <div className="flex h-full flex-col overflow-y-auto bg-gray-50 dark:bg-gray-950 p-6 space-y-6">
      {/* Page header */}
      <div>
        <h2 className="text-base font-semibold text-gray-900 dark:text-gray-100">
          {t('settings.externalTriggers')}
        </h2>
        <p className="text-xs text-gray-500 mt-0.5">{t('settings.externalTriggersSubtitle')}</p>
      </div>

      {/* Hook Server status & config */}
      <section className="space-y-3">
        <h3 className="text-xs font-semibold uppercase tracking-wide text-gray-500">
          {t('settings.hookServer')}
        </h3>
        <div className="rounded-2xl border border-gray-200 bg-white p-4 dark:border-gray-700 dark:bg-gray-900 space-y-4">
          {/* Status row */}
          <div className="flex items-center justify-between">
            <div className="flex items-center gap-2">
              <span className={`h-2 w-2 rounded-full ${config.enabled ? 'bg-green-500' : 'bg-gray-400'}`} />
              <span className="text-sm font-medium text-gray-700 dark:text-gray-300">
                {config.enabled
                  ? t('settings.hookRunning') + ` :${config.port}`
                  : t('settings.hookDisabled')}
              </span>
            </div>
            <button
              onClick={handleRestart}
              disabled={restarting}
              className="flex items-center gap-1.5 text-xs px-3 py-1.5 rounded-lg bg-blue-50 text-blue-600 hover:bg-blue-100 dark:bg-blue-900/30 dark:text-blue-400 dark:hover:bg-blue-900/50 disabled:opacity-50"
            >
              <RotateCcw className={`h-3.5 w-3.5 ${restarting ? 'animate-spin' : ''}`} />
              {t('settings.hookRestart')}
            </button>
          </div>

          {/* Port + Enabled */}
          <div className="grid grid-cols-2 gap-4">
            <div>
              <label className="text-xs text-gray-500 dark:text-gray-400">{t('settings.hookPort')}</label>
              <input
                type="number"
                value={editPort}
                onChange={(e) => setEditPort(e.target.value)}
                className="mt-1 w-full rounded-lg border border-gray-300 px-3 py-1.5 text-sm dark:border-gray-600 dark:bg-gray-800 dark:text-gray-200"
              />
            </div>
            <div>
              <label className="text-xs text-gray-500 dark:text-gray-400">{t('settings.hookEnabled')}</label>
              <div className="mt-1 flex items-center gap-2 h-[34px]">
                <button
                  onClick={() => setEditEnabled(!editEnabled)}
                  className={`relative h-6 w-11 rounded-full transition-colors ${editEnabled ? 'bg-green-500' : 'bg-gray-300 dark:bg-gray-600'}`}
                >
                  <span className={`absolute top-0.5 left-0.5 h-5 w-5 rounded-full bg-white transition-transform ${editEnabled ? 'translate-x-5' : ''}`} />
                </button>
                <Power className="h-4 w-4 text-gray-400" />
              </div>
            </div>
          </div>

          {/* Auth token */}
          <div>
            <label className="text-xs text-gray-500 dark:text-gray-400 flex items-center gap-1">
              <Shield className="h-3 w-3" />
              {t('settings.hookAuthToken')}
            </label>
            <div className="mt-1 flex gap-2">
              <input
                type={showToken ? 'text' : 'password'}
                value={editToken}
                onChange={(e) => setEditToken(e.target.value)}
                placeholder={t('settings.hookTokenPlaceholder')}
                className="flex-1 rounded-lg border border-gray-300 px-3 py-1.5 text-sm font-mono dark:border-gray-600 dark:bg-gray-800 dark:text-gray-200"
              />
              <button
                onClick={() => setShowToken(!showToken)}
                className="text-xs px-2 rounded-lg border border-gray-300 text-gray-500 hover:bg-gray-100 dark:border-gray-600 dark:hover:bg-gray-800"
              >
                {showToken ? '−' : '○'}
              </button>
              <button
                onClick={generateToken}
                className="text-xs px-3 py-1.5 rounded-lg bg-gray-100 text-gray-600 hover:bg-gray-200 dark:bg-gray-800 dark:text-gray-300 dark:hover:bg-gray-700"
              >
                {t('settings.hookRegenerate')}
              </button>
            </div>
          </div>

          {/* Save button */}
          <button
            onClick={handleSave}
            disabled={saving}
            className="w-full rounded-lg bg-blue-600 px-3 py-2 text-sm font-medium text-white hover:bg-blue-500 disabled:opacity-50"
          >
            {saving ? t('common.saving') : t('common.save')}
          </button>
        </div>
      </section>

      {/* Registered templates */}
      <section className="space-y-3">
        <h3 className="text-xs font-semibold uppercase tracking-wide text-gray-500">
          {t('settings.registeredTemplates')}
        </h3>
        <div className="rounded-2xl border border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-900 overflow-hidden">
          {templates.length === 0 ? (
            <div className="p-4 text-sm text-gray-400 text-center">
              {t('settings.noTemplatesWithKey')}
            </div>
          ) : (
            <table className="w-full text-sm">
              <thead className="bg-gray-50 dark:bg-gray-800 border-b border-gray-200 dark:border-gray-700">
                <tr className="text-left text-xs text-gray-500">
                  <th className="px-4 py-2 font-medium">template_key</th>
                  <th className="px-4 py-2 font-medium">{t('workflow.run.template')}</th>
                  <th className="px-4 py-2 font-medium">{t('settings.allowedSources')}</th>
                </tr>
              </thead>
              <tbody>
                {templates.map((w) => (
                  <tr key={w.id} className="border-b border-gray-100 dark:border-gray-800">
                    <td className="px-4 py-2 font-mono text-xs text-blue-600 dark:text-blue-400">
                      {w.template_key}
                    </td>
                    <td className="px-4 py-2 text-gray-700 dark:text-gray-300">{w.name}</td>
                    <td className="px-4 py-2 text-xs text-gray-500 dark:text-gray-400">
                      {((w.allowed_sources as string[] | undefined)?.length ?? 0) === 0 ? t('settings.allowAll') : (w.allowed_sources as string[] | undefined)?.join(', ')}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      </section>

      {/* curl example */}
      <section className="space-y-3">
        <h3 className="text-xs font-semibold uppercase tracking-wide text-gray-500">
          {t('settings.curlExample')}
        </h3>
        <div className="relative rounded-2xl border border-gray-200 bg-gray-900 dark:border-gray-700 p-4">
          <pre className="text-xs text-green-400 font-mono whitespace-pre-wrap break-all pr-10">
            {curlExample}
          </pre>
          <button
            onClick={handleCopy}
            className="absolute top-3 right-3 rounded p-1.5 text-gray-400 hover:bg-gray-800 hover:text-gray-200"
          >
            {copied ? <Check className="h-4 w-4 text-green-400" /> : <Copy className="h-4 w-4" />}
          </button>
        </div>
      </section>

      {/* Recent hook-triggered runs */}
      <section className="space-y-3">
        <div className="flex items-center justify-between">
          <h3 className="text-xs font-semibold uppercase tracking-wide text-gray-500">
            {t('settings.recentHookRuns')}
          </h3>
          <button
            onClick={() => { loadData(); fetchRuns() }}
            className="flex items-center gap-1 text-xs text-gray-400 hover:text-gray-600 dark:hover:text-gray-300"
          >
            <RefreshCw className="h-3 w-3" />
            {t('common.refresh')}
          </button>
        </div>
        <div className="rounded-2xl border border-gray-200 bg-white dark:border-gray-700 dark:bg-gray-900 overflow-hidden">
          {allHookRuns.length === 0 ? (
            <div className="p-4 text-sm text-gray-400 text-center">
              {t('settings.noHookRuns')}
            </div>
          ) : (
            <table className="w-full text-sm">
              <thead className="bg-gray-50 dark:bg-gray-800 border-b border-gray-200 dark:border-gray-700">
                <tr className="text-left text-xs text-gray-500">
                  <th className="px-4 py-2 font-medium">{t('settings.colTime')}</th>
                  <th className="px-4 py-2 font-medium">{t('settings.colTemplate')}</th>
                  <th className="px-4 py-2 font-medium">{t('workflow.run.statusLabel')}</th>
                  <th className="px-4 py-2 font-medium">source</th>
                </tr>
              </thead>
              <tbody>
                {allHookRuns.slice(0, 20).map((r) => (
                  <tr key={r.runId} className="border-b border-gray-100 dark:border-gray-800">
                    <td className="px-4 py-2 text-xs text-gray-500 dark:text-gray-400">
                      {formatTime(r.createdAt)}
                    </td>
                    <td className="px-4 py-2 font-mono text-xs text-gray-600 dark:text-gray-400">
                      {r.templateKey ?? r.templateId}
                    </td>
                    <td className="px-4 py-2">
                      <span className={`text-xs px-2 py-0.5 rounded ${runStatusColor(r.status)}`}>
                        {t(`workflow.run.status.${r.status}`, r.status)}
                      </span>
                    </td>
                    <td className="px-4 py-2 text-xs text-gray-500 dark:text-gray-400">
                      {r.trigger?.trigger === 'hook' ? r.trigger.source : '-'}
                    </td>
                  </tr>
                ))}
              </tbody>
            </table>
          )}
        </div>
      </section>
    </div>
  )
}
