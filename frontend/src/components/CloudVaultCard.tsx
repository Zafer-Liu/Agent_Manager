import { useCallback, useEffect, useState } from 'react'
import { invoke } from '@tauri-apps/api/core'
import { useTranslation } from 'react-i18next'
import { Cloud, Loader2, RefreshCw } from 'lucide-react'

interface VaultStatusView {
  configured: boolean
  enabled: boolean
  dirty: number
  last_sync_at: string | null
  last_report: string | null
  auto_interval_min: number
}

interface SyncReport {
  pushed: number
  pulled: number
  skipped: number
  conflicts: number
  usage_imported: number
  errors: string[]
}

type Phase =
  | { kind: 'idle' }
  | { kind: 'syncing' }
  | { kind: 'done'; report: SyncReport }
  | { kind: 'error'; message: string }

export function CloudVaultCard({ onSynced }: { onSynced?: () => void }) {
  const { t } = useTranslation()
  const [status, setStatus] = useState<VaultStatusView | null>(null)
  const [phase, setPhase] = useState<Phase>({ kind: 'idle' })

  const refresh = useCallback(() => {
    invoke<VaultStatusView>('cloud_vault_status')
      .then(setStatus)
      .catch(() => setStatus(null))
  }, [])

  useEffect(refresh, [refresh])

  const handleSync = async () => {
    setPhase({ kind: 'syncing' })
    try {
      const report = await invoke<SyncReport>('cloud_vault_sync')
      setPhase({ kind: 'done', report })
      refresh()
      onSynced?.()
    } catch (e) {
      setPhase({ kind: 'error', message: String(e) })
    }
  }

  const configured = status?.configured ?? false
  const enabled = status?.enabled ?? false
  const dotClass = !configured || !enabled
    ? 'bg-gray-400'
    : phase.kind === 'syncing'
      ? 'bg-blue-500 animate-pulse'
      : phase.kind === 'error'
        ? 'bg-red-500'
        : phase.kind === 'done' && phase.report.errors.length > 0
          ? 'bg-amber-500'
          : 'bg-green-500'

  return (
    <section className="order-15 rounded-xl border border-cyan-200 bg-cyan-50/40 p-4 dark:border-cyan-900/70 dark:bg-cyan-950/20">
      <div className="flex flex-wrap items-center gap-2">
        <div className="mt-0.5 rounded-md bg-cyan-500/10 p-1.5 text-cyan-600 dark:text-cyan-300"><Cloud size={14} /></div>
        <h2 className="text-sm font-semibold text-gray-800 dark:text-gray-100">{t('memory.cloudCard.title')}</h2>
        <span className={`h-2 w-2 rounded-full ${dotClass}`} title={t('memory.cloudCard.status')} />
        {status?.dirty ? (
          <span className="rounded bg-amber-100 px-1.5 py-0.5 text-xs text-amber-800 dark:bg-amber-950/40 dark:text-amber-300">
            {t('memory.cloudCard.dirty', { count: status.dirty })}
          </span>
        ) : null}
        <div className="ml-auto flex items-center gap-2">
          <button
            type="button"
            onClick={() => { void handleSync() }}
            disabled={!enabled || phase.kind === 'syncing'}
            className="inline-flex items-center gap-1.5 rounded-md bg-cyan-600 px-2.5 py-1.5 text-xs font-medium text-white hover:bg-cyan-500 disabled:opacity-50"
          >
            {phase.kind === 'syncing' ? <Loader2 size={13} className="animate-spin" /> : <RefreshCw size={13} />}
            {phase.kind === 'syncing' ? t('memory.cloudCard.syncing') : t('memory.cloudCard.syncNow')}
          </button>
        </div>
      </div>
      {!configured ? (
        <p className="mt-2 text-xs text-gray-500 dark:text-gray-400">{t('memory.cloudCard.notConfigured')}</p>
      ) : !enabled ? (
        <p className="mt-2 text-xs text-gray-500 dark:text-gray-400">{t('memory.cloudCard.disabled')}</p>
      ) : (
        <div className="mt-2 space-y-1 text-xs text-gray-500 dark:text-gray-400">
          <p>
            {status && status.auto_interval_min > 0
              ? t('memory.cloudCard.autoOn', { min: status.auto_interval_min })
              : t('memory.cloudCard.autoOff')}
          </p>
          {status?.last_sync_at && (
            <p>
              {t('memory.cloudCard.lastSync', { time: new Date(status.last_sync_at).toLocaleString() })}
              {status.last_report ? ` · ${status.last_report}` : ''}
            </p>
          )}
          {phase.kind === 'done' && (
            <p className={phase.report.errors.length ? 'text-amber-700 dark:text-amber-300' : 'text-green-600 dark:text-green-400'}>
              {t('memory.cloudCard.report', {
                pushed: phase.report.pushed,
                pulled: phase.report.pulled,
                usage: phase.report.usage_imported,
                skipped: phase.report.skipped,
                conflicts: phase.report.conflicts,
                errors: phase.report.errors.length,
              })}
              {phase.report.errors.length ? ` — ${phase.report.errors[0]}` : ''}
            </p>
          )}
          {phase.kind === 'error' && (
            <p className="text-red-600 dark:text-red-400">{phase.message}</p>
          )}
        </div>
      )}
    </section>
  )
}
