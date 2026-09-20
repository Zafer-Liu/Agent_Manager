import { useTranslation } from 'react-i18next'
import { invoke } from '@tauri-apps/api/core'
import { save, open } from '@tauri-apps/plugin-dialog'
import { Download, Upload, Loader2, CheckCircle2, AlertCircle, Brain, Info, DatabaseBackup, Cloud } from 'lucide-react'
import { useState, useCallback, useEffect, type ReactNode } from 'react'
import { LlmSettings } from './LlmSettings'

export function Settings() {
  const { t } = useTranslation()

  return (
    <div className="flex h-full flex-col overflow-y-auto bg-gray-50 dark:bg-gray-950">
      <div className="mx-auto w-full max-w-3xl px-6 py-8 space-y-6">
      {/* Page header */}
      <div>
        <h2 className="text-lg font-semibold text-gray-900 dark:text-gray-100">
          {t('settings.title')}
        </h2>
        <p className="text-xs text-gray-500 mt-0.5">{t('settings.subtitle')}</p>
      </div>

      {/* LLM & Memory */}
      <SettingsSection
        icon={<Brain size={15} className="text-violet-600 dark:text-violet-400" />}
        title={t('settings.sectionLlm')}
        desc={t('settings.sectionLlmHint')}
      >
        <LlmSettings embedded />
      </SettingsSection>

      {/* Backup & Restore */}
      <SettingsSection
        icon={<DatabaseBackup size={15} className="text-blue-600 dark:text-blue-400" />}
        title={t('settings.sectionBackup')}
        desc={t('settings.sectionBackupHint')}
      >
        <Card>
        <BackupRestore />
        </Card>
      </SettingsSection>

      {/* Cloud Memory Vault */}
      <SettingsSection
        icon={<Cloud size={15} className="text-cyan-600 dark:text-cyan-400" />}
        title={t('settings.cloudVault.title')}
        desc={t('settings.cloudVault.hint')}
      >
        <Card>
        <CloudVaultSettings />
        </Card>
      </SettingsSection>

      {/* About */}
      <SettingsSection
        icon={<Info size={15} className="text-gray-500" />}
        title={t('settings.sectionAbout')}
      >
        <Card>
        <div className="space-y-2">
          <AboutRow label={t('settings.aboutName')} value={t('app.title')} />
          <AboutRow label={t('settings.aboutLicense')} value="MIT License" />
          <AboutRow
            label={t('settings.aboutSource')}
            value="GitHub"
            href="https://github.com/Zafer-Liu/Agent_Manager"
          />
        </div>
        </Card>
      </SettingsSection>
      </div>
    </div>
  )
}

/// Section header with icon badge — no card wrapper so children manage
/// their own visual containment (avoids cards-in-cards nesting).
function SettingsSection({
  icon,
  title,
  desc,
  children,
}: {
  icon: ReactNode
  title: string
  desc?: string
  children: ReactNode
}) {
  return (
    <section className="space-y-3">
      <div className="flex items-center gap-2.5">
        <div className="flex h-7 w-7 shrink-0 items-center justify-center rounded-md bg-gray-100 dark:bg-gray-800">
          {icon}
        </div>
        <div>
          <h3 className="text-sm font-semibold text-gray-800 dark:text-gray-200">{title}</h3>
          {desc && (
            <p className="text-xs leading-5 text-gray-500 dark:text-gray-400">{desc}</p>
          )}
        </div>
      </div>
      {children}
    </section>
  )
}

function Card({ children }: { children: ReactNode }) {
  return (
    <div className="rounded-lg border border-gray-200 bg-white p-4 dark:border-gray-800 dark:bg-gray-900">
      {children}
    </div>
  )
}

interface VaultSettingsView {
  url: string
  enabled: boolean
  pat_set: boolean
  password_set: boolean
  auto_interval_min: number
}

type VaultStatus =
  | { kind: 'idle' }
  | { kind: 'working'; label: string }
  | { kind: 'success'; label: string }
  | { kind: 'error'; label: string }

function CloudVaultSettings() {
  const { t } = useTranslation()
  const [url, setUrl] = useState('')
  const [pat, setPat] = useState('')
  const [password, setPassword] = useState('')
  const [passwordSet, setPasswordSet] = useState(false)
  const [autoInterval, setAutoInterval] = useState(0)
  const [enabled, setEnabled] = useState(false)
  const [patSet, setPatSet] = useState(false)
  const [status, setStatus] = useState<VaultStatus>({ kind: 'idle' })
  const [loaded, setLoaded] = useState(false)

  useEffect(() => {
    invoke<VaultSettingsView>('cloud_vault_get_settings').then((v) => {
      setUrl(v.url)
      setEnabled(v.enabled)
      setPatSet(v.pat_set)
      setPasswordSet(v.password_set)
      setAutoInterval(v.auto_interval_min)
      setLoaded(true)
    })
  }, [])

  if (!loaded) {
    return <div className="text-xs text-gray-400">{t('settings.cloudVault.loading')}</div>
  }

  const handleTest = async () => {
    setStatus({ kind: 'working', label: t('settings.cloudVault.testing') })
    try {
      const token = pat || (patSet ? 'saved' : '')
      const version = await invoke<string>('cloud_vault_test_connection', { url, pat: token })
      if (version === 'saved') {
        setStatus({ kind: 'error', label: t('settings.cloudVault.needPat') })
        return
      }
      setStatus({ kind: 'success', label: t('settings.cloudVault.testOk', { version }) })
    } catch (e) {
      setStatus({ kind: 'error', label: String(e) })
    }
  }

  const handleSave = async () => {
    setStatus({ kind: 'working', label: t('settings.cloudVault.saving') })
    try {
      await invoke('cloud_vault_save_settings', {
        url,
        pat: pat || null,
        password: password || null,
        autoIntervalMin: autoInterval,
        enabled,
      })
      if (pat) {
        setPat('')
        setPatSet(true)
      }
      if (password) {
        setPassword('')
        setPasswordSet(true)
      }
      setStatus({ kind: 'success', label: t('settings.cloudVault.saved') })
    } catch (e) {
      setStatus({ kind: 'error', label: String(e) })
    }
  }

  return (
    <div className="space-y-3">
      <div className="space-y-1.5">
        <label className="text-xs font-medium text-gray-600 dark:text-gray-300">
          {t('settings.cloudVault.url')}
        </label>
        <input
          value={url}
          onChange={(e) => setUrl(e.target.value)}
          placeholder="http://192.168.x.x:8787"
          className="w-full rounded-lg border border-gray-200 bg-white px-3 py-1.5 text-sm text-gray-900 outline-none focus:border-cyan-400 dark:border-gray-700 dark:bg-gray-800 dark:text-gray-100"
        />
      </div>
      <div className="space-y-1.5">
        <label className="text-xs font-medium text-gray-600 dark:text-gray-300">
          {t('settings.cloudVault.pat')}
          {patSet && !pat && (
            <span className="ml-2 font-normal text-green-600 dark:text-green-400">
              {t('settings.cloudVault.patSaved')}
            </span>
          )}
        </label>
        <input
          value={pat}
          onChange={(e) => setPat(e.target.value)}
          type="password"
          placeholder={patSet ? '••••••••' : 'vault-...'}
          className="w-full rounded-lg border border-gray-200 bg-white px-3 py-1.5 text-sm text-gray-900 outline-none focus:border-cyan-400 dark:border-gray-700 dark:bg-gray-800 dark:text-gray-100"
        />
      </div>
      <div className="space-y-1.5">
        <label className="text-xs font-medium text-gray-600 dark:text-gray-300">
          {t('settings.cloudVault.password')}
          {passwordSet && !password ? (
            <span className="ml-2 font-normal text-green-600 dark:text-green-400">
              {t('settings.cloudVault.patSaved')}
            </span>
          ) : !passwordSet ? (
            <span className="ml-2 font-normal text-amber-600 dark:text-amber-400">
              {t('settings.cloudVault.passwordNotSet')}
            </span>
          ) : null}
        </label>
        <input
          value={password}
          onChange={(e) => setPassword(e.target.value)}
          type="password"
          placeholder={passwordSet ? '••••••••' : t('settings.cloudVault.passwordPlaceholder')}
          className="w-full rounded-lg border border-gray-200 bg-white px-3 py-1.5 text-sm text-gray-900 outline-none focus:border-cyan-400 dark:border-gray-700 dark:bg-gray-800 dark:text-gray-100"
        />
      </div>
      <div className="space-y-1.5">
        <label className="text-xs font-medium text-gray-600 dark:text-gray-300">
          {t('settings.cloudVault.autoInterval')}
        </label>
        <select
          value={autoInterval}
          onChange={(e) => setAutoInterval(Number(e.target.value))}
          className="w-full rounded-lg border border-gray-200 bg-white px-3 py-1.5 text-sm text-gray-900 outline-none dark:border-gray-700 dark:bg-gray-800 dark:text-gray-100"
        >
          {[0, 5, 15, 30, 60, 360].map((min) => (
            <option key={min} value={min}>
              {min === 0
                ? t('settings.cloudVault.autoOff')
                : t('settings.cloudVault.autoEvery', { min })}
            </option>
          ))}
        </select>
      </div>
      <label className="flex items-center gap-2 text-xs text-gray-600 dark:text-gray-300">
        <input
          type="checkbox"
          checked={enabled}
          onChange={(e) => setEnabled(e.target.checked)}
          className="h-3.5 w-3.5 rounded border-gray-300"
        />
        {t('settings.cloudVault.enable')}
      </label>
      <div className="flex flex-wrap gap-2">
        <button
          onClick={handleTest}
          className="inline-flex items-center gap-1.5 rounded-lg border border-gray-200 px-3 py-1.5 text-xs font-medium text-gray-600 hover:bg-gray-50 disabled:opacity-40 dark:border-gray-700 dark:text-gray-300 dark:hover:bg-gray-800"
        >
          {status.kind === 'working' ? <Loader2 size={14} className="animate-spin" /> : <Cloud size={14} />}
          {t('settings.cloudVault.test')}
        </button>
        <button
          onClick={handleSave}
          className="inline-flex items-center gap-1.5 rounded-lg bg-cyan-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-cyan-500"
        >
          {t('settings.cloudVault.save')}
        </button>
      </div>
      {status.kind === 'success' && (
        <div className="flex items-start gap-2 text-xs text-green-600 dark:text-green-400">
          <CheckCircle2 size={14} className="mt-0.5 shrink-0" />
          <span>{status.label}</span>
        </div>
      )}
      {status.kind === 'error' && (
        <div className="flex items-start gap-2 text-xs text-red-600 dark:text-red-400">
          <AlertCircle size={14} className="mt-0.5 shrink-0" />
          <span className="break-all">{status.label}</span>
        </div>
      )}
      <p className="text-xs text-gray-400 dark:text-gray-500">
        {t('settings.cloudVault.passwordStoreHint')}
      </p>
    </div>
  )
}

function AboutRow({
  label,
  value,
  href,
}: {
  label: string
  value: string
  href?: string
}) {
  return (
    <div className="flex items-center justify-between text-sm">
      <span className="text-gray-500 dark:text-gray-400">{label}</span>
      {href ? (
        <a
          href={href}
          target="_blank"
          rel="noreferrer"
          className="font-medium text-blue-600 hover:underline dark:text-blue-400"
        >
          {value}
        </a>
      ) : (
        <span className="font-medium text-gray-900 dark:text-gray-100">{value}</span>
      )}
    </div>
  )
}

interface ExportManifest {
  version: number
  created_at: string
  app_version: string
  tables: string[]
  skill_count: number
  backup_count: number
}

interface ImportResult {
  tables_restored: number
  skills_restored: number
  backups_restored: number
  app_restart_required: boolean
}

type Status =
  | { kind: 'idle' }
  | { kind: 'working'; label: string }
  | { kind: 'success'; label: string }
  | { kind: 'error'; label: string }

function BackupRestore() {
  const { t } = useTranslation()
  const [status, setStatus] = useState<Status>({ kind: 'idle' })

  const handleExport = useCallback(async () => {
    setStatus({ kind: 'working', label: t('backup.exporting') })
    try {
      const path = await save({
        title: t('backup.exportTitle'),
        defaultPath: 'agent-manager-backup.zip',
        filters: [{ name: 'ZIP', extensions: ['zip'] }],
      })
      if (!path) {
        setStatus({ kind: 'idle' })
        return
      }
      const manifest = await invoke<ExportManifest>('config_export', { destPath: path })
      setStatus({
        kind: 'success',
        label: t('backup.exportDone', {
          tables: manifest.tables.length,
          skills: manifest.skill_count,
          backups: manifest.backup_count,
        }),
      })
    } catch (e) {
      setStatus({ kind: 'error', label: String(e) })
    }
  }, [t])

  const handleImport = useCallback(async () => {
    setStatus({ kind: 'working', label: t('backup.importing') })
    try {
      const path = await open({
        title: t('backup.importTitle'),
        filters: [{ name: 'ZIP', extensions: ['zip'] }],
        multiple: false,
        directory: false,
      })
      if (!path || typeof path !== 'string') {
        setStatus({ kind: 'idle' })
        return
      }
      const result = await invoke<ImportResult>('config_import', { sourcePath: path })
      setStatus({
        kind: 'success',
        label: t('backup.importDone', {
          tables: result.tables_restored,
          skills: result.skills_restored,
          backups: result.backups_restored,
        }),
      })
    } catch (e) {
      setStatus({ kind: 'error', label: String(e) })
    }
  }, [t])

  const busy = status.kind === 'working'

  return (
    <div className="space-y-3">
      <div className="flex flex-wrap gap-2">
        <button
          onClick={handleExport}
          disabled={busy}
          className="inline-flex items-center gap-1.5 rounded-lg bg-blue-600 px-3 py-1.5 text-xs font-medium text-white hover:bg-blue-500 disabled:opacity-40"
        >
          {busy ? <Loader2 size={14} className="animate-spin" /> : <Download size={14} />}
          {t('backup.export')}
        </button>
        <button
          onClick={handleImport}
          disabled={busy}
          className="inline-flex items-center gap-1.5 rounded-lg border border-gray-200 px-3 py-1.5 text-xs font-medium text-gray-600 hover:bg-gray-50 disabled:opacity-40 dark:border-gray-700 dark:text-gray-300 dark:hover:bg-gray-800"
        >
          {busy ? <Loader2 size={14} className="animate-spin" /> : <Upload size={14} />}
          {t('backup.import')}
        </button>
      </div>

      {status.kind === 'success' && (
        <div className="flex items-start gap-2 text-xs text-green-600 dark:text-green-400">
          <CheckCircle2 size={14} className="mt-0.5 shrink-0" />
          <span>{status.label}</span>
        </div>
      )}
      {status.kind === 'error' && (
        <div className="flex items-start gap-2 text-xs text-red-600 dark:text-red-400">
          <AlertCircle size={14} className="mt-0.5 shrink-0" />
          <span className="break-all">{status.label}</span>
        </div>
      )}
      {status.kind === 'working' && (
        <div className="flex items-center gap-2 text-xs text-gray-500">
          <Loader2 size={14} className="animate-spin" />
          <span>{status.label}</span>
        </div>
      )}

      <p className="text-xs text-gray-400 dark:text-gray-500">{t('backup.importWarning')}</p>
    </div>
  )
}
