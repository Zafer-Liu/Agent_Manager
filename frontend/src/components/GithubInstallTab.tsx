import { useState, useEffect } from 'react'
import { useTranslation } from 'react-i18next'
import { invoke } from '@tauri-apps/api/core'
import { open } from '@tauri-apps/plugin-dialog'
import type { AgentConfig } from '../types/agent'
import {
  GitFork, Star, GitBranch, Loader2, FolderOpen,
  CheckCircle2, AlertCircle, ExternalLink, ChevronRight,
  KeyRound, ChevronDown, ChevronUp,
} from 'lucide-react'

interface GithubRepoInfo {
  name: string
  full_name: string
  description: string
  stars: number
  language: string
  default_branch: string
  clone_url: string
  readme_excerpt: string
}

interface RecommendedAgent {
  name: string
  url: string
  description: string
  tags: string[]
}

const RECOMMENDED_AGENTS: RecommendedAgent[] = [
  {
    name: '智析 · 数据分析 Agent',
    url: 'https://github.com/Zafer-Liu/Data-Analysis-Agent',
    description: '本地运行的 AI 数据分析助手，支持上传 CSV/Excel，用自然语言提问，自动生成图表与洞察报告。',
    tags: ['Data Analysis', 'Streamlit', 'Python'],
  },
  {
    name: 'BrainBoost · AI 思维导图',
    url: 'https://github.com/Zafer-Liu/BrainBoost',
    description: '输入关键词，AI 自动生成思维导图与推演方案，支持语音输入、节点拖拽编辑、一键导出 Markdown / Word / PNG。',
    tags: ['Mind Map', 'React', 'TypeScript'],
  },
]

type Step = 'input' | 'fetching' | 'preview' | 'cloning' | 'done' | 'error'

interface Props {
  onPrefill: (partial: Partial<AgentConfig>) => void
}

export function GithubInstallTab({ onPrefill }: Props) {
  const { t } = useTranslation()
  const [url, setUrl] = useState('')
  const [step, setStep] = useState<Step>('input')
  const [repoInfo, setRepoInfo] = useState<GithubRepoInfo | null>(null)
  const [baseDir, setBaseDir] = useState('')
  const [errorMsg, setErrorMsg] = useState('')
  const [detectedProxy, setDetectedProxy] = useState('')
  const [clonedPath, setClonedPath] = useState('')

  useEffect(() => {
    invoke<string>('github_get_proxy').then(p => setDetectedProxy(p)).catch(() => {})
  }, [])

  async function pickBaseDir() {
    try {
      const selected = await open({ directory: true, multiple: false })
      if (selected && typeof selected === 'string') {
        setBaseDir(selected)
      }
    } catch (e) {
      const msg = String(e)
      if (!msg.includes('cancelled') && msg !== 'null' && msg !== 'undefined') {
        setErrorMsg(t('github.openFolderFailed', { error: msg }))
      }
    }
  }

  async function handleFetch(fetchUrl?: string) {
    const target = fetchUrl ?? url
    if (!target.trim()) return
    setStep('fetching')
    setErrorMsg('')
    try {
      const info = await invoke<GithubRepoInfo>('github_fetch_repo_info', { url: target.trim() })
      setRepoInfo(info)
      setStep('preview')
    } catch (e) {
      setErrorMsg(String(e))
      setStep('error')
    }
  }

  async function handleClone() {
    if (!repoInfo) return
    setStep('cloning')
    setErrorMsg('')
    try {
      const path = await invoke<string>('github_clone_repo', {
        cloneUrl: repoInfo.clone_url,
        repoName: repoInfo.name,
        targetDir: baseDir,
      })
      setClonedPath(path)
      setStep('done')

      // Scan cloned dir and prefill form
      try {
        const scan = await invoke<{
          name: string; command: string; args: string[]; port?: number; description: string
        }>('scan_project_dir', { dir: path })
        onPrefill({
          name: scan.name || repoInfo.name,
          description: scan.description || repoInfo.description,
          command: scan.command,
          args: scan.args,
          working_dir: path,
          port: scan.port,
        })
      } catch {
        onPrefill({
          name: repoInfo.name,
          description: repoInfo.description,
          working_dir: path,
        })
      }
    } catch (e) {
      setErrorMsg(String(e))
      setStep('error')
    }
  }

  function reset() {
    setStep('input')
    setUrl('')
    setRepoInfo(null)
    setErrorMsg('')
    setClonedPath('')
  }

  return (
    <div className="space-y-4">
      {/* Proxy badge */}
      <ProxyBadge proxy={detectedProxy} />

      {/* GitHub Token 配置（可选，折叠） */}
      <TokenConfig />

      {/* Recommended */}
      {step === 'input' && (
        <div>
          <p className="mb-2 text-xs font-medium text-gray-500 dark:text-gray-400">{t('github.recommended')}</p>
          <div className="space-y-2">
            {RECOMMENDED_AGENTS.map(agent => (
              <RecommendedCard
                key={agent.url}
                agent={agent}
                onSelect={() => {
                  setUrl(agent.url)
                  handleFetch(agent.url)
                }}
              />
            ))}
          </div>
        </div>
      )}

      {/* URL input */}
      {(step === 'input' || step === 'error') && (
        <div>
          <p className="mb-1 text-xs font-medium text-gray-500 dark:text-gray-400">{t('github.orInputUrl')}</p>
          <div className="flex gap-2">
            <input
              value={url}
              onChange={e => setUrl(e.target.value)}
              onKeyDown={e => e.key === 'Enter' && handleFetch()}
              placeholder="https://github.com/owner/repo"
              className="field-input flex-1 font-mono text-sm"
            />
            <button
              onClick={() => handleFetch()}
              disabled={!url.trim()}
              className="flex items-center gap-1.5 rounded-lg bg-blue-600 px-3 py-2 text-sm font-medium text-white hover:bg-blue-500 disabled:opacity-50"
            >
              <GitFork className="h-4 w-4" /> {t('github.fetchInfo')}
            </button>
          </div>
          {step === 'error' && errorMsg && (
            <div className="mt-2 flex items-start gap-2 rounded-lg bg-red-50 px-3 py-2 text-sm text-red-600 dark:bg-red-900/30 dark:text-red-400">
              <AlertCircle className="mt-0.5 h-4 w-4 shrink-0" />
              <span className="whitespace-pre-wrap leading-relaxed">{errorMsg}</span>
            </div>
          )}
        </div>
      )}

      {/* Fetching */}
      {step === 'fetching' && (
        <div className="flex items-center gap-2 rounded-lg bg-blue-50 px-4 py-3 text-sm text-blue-600 dark:bg-blue-900/30 dark:text-blue-400">
          <Loader2 className="h-4 w-4 animate-spin" />
          {t('github.fetching')}
        </div>
      )}

      {/* Preview */}
      {step === 'preview' && repoInfo && (
        <div className="space-y-3">
          <RepoPreview info={repoInfo} />

          {/* Clone destination */}
          <div>
            <p className="mb-1 text-xs font-medium text-gray-500 dark:text-gray-400">
              {t('github.storeDir')} <span className="font-normal text-gray-400">{t('github.storeDirHint')}</span>
            </p>
            <div className="flex gap-2">
              <input
                value={baseDir}
                onChange={e => setBaseDir(e.target.value)}
                placeholder={t('github.storeDirPlaceholder')}
                className="field-input flex-1 font-mono text-sm"
              />
              <button
                type="button"
                onClick={pickBaseDir}
                className="flex items-center rounded-lg border border-gray-200 px-3 py-2 text-gray-500 hover:bg-gray-100 hover:text-gray-700 dark:border-gray-700 dark:text-gray-400 dark:hover:bg-gray-800 dark:hover:text-gray-200"
                title={t('github.pickFolder')}
              >
                <FolderOpen className="h-4 w-4" />
              </button>
            </div>
          </div>

          <button
            onClick={handleClone}
            className="flex w-full items-center justify-center gap-2 rounded-lg bg-blue-600 py-2.5 text-sm font-medium text-white hover:bg-blue-500"
          >
            <GitBranch className="h-4 w-4" />
            {t('github.cloneToLocal')}
          </button>
        </div>
      )}

      {/* Cloning */}
      {step === 'cloning' && (
        <div className="flex items-center gap-2 rounded-lg bg-blue-50 px-4 py-3 text-sm text-blue-600 dark:bg-blue-900/30 dark:text-blue-400">
          <Loader2 className="h-4 w-4 animate-spin" />
          {t('github.cloning')}
        </div>
      )}

      {/* Done */}
      {step === 'done' && (
        <div className="space-y-3">
          <div className="flex items-start gap-2 rounded-lg bg-green-50 px-4 py-3 text-sm text-green-700 dark:bg-green-900/30 dark:text-green-400">
            <CheckCircle2 className="mt-0.5 h-4 w-4 shrink-0" />
            <div>
              <p className="font-medium">{t('github.cloneSuccess')}</p>
              <p className="mt-0.5 font-mono text-xs text-green-600 dark:text-green-500">{clonedPath}</p>
            </div>
          </div>
          <p className="text-xs text-gray-400 dark:text-gray-500">
            {t('github.autoFillHint')}
          </p>
          <button
            onClick={reset}
            className="text-xs text-blue-500 hover:text-blue-400 dark:text-blue-400 dark:hover:text-blue-300"
          >
            {t('github.installAnother')}
          </button>
        </div>
      )}
    </div>
  )
}

// ── Sub-components ────────────────────────────────────────────

function ProxyBadge({ proxy }: { proxy: string }) {
  const { t } = useTranslation()
  if (proxy) {
    return (
      <div className="flex items-center gap-1.5 text-xs text-green-600 dark:text-green-400">
        <span className="h-1.5 w-1.5 rounded-full bg-green-500" />
        {t('github.proxyReady')}<span className="font-mono">{proxy}</span>
      </div>
    )
  }
  return (
    <div className="flex items-center gap-1.5 text-xs text-amber-500 dark:text-amber-400">
      <span className="h-1.5 w-1.5 rounded-full bg-amber-400" />
      {t('github.proxyNone')}
    </div>
  )
}

function TokenConfig() {
  const { t } = useTranslation()
  const [expanded, setExpanded] = useState(false)
  const [token, setToken] = useState('')
  const [configured, setConfigured] = useState(false)
  const [fromEnv, setFromEnv] = useState(false)
  const [saving, setSaving] = useState(false)
  const [justSaved, setJustSaved] = useState(false)

  useEffect(() => {
    invoke<[boolean, boolean]>('github_token_status')
      .then(([has, env]) => { setConfigured(has); setFromEnv(env) })
      .catch(() => {})
  }, [])

  async function save() {
    setSaving(true)
    try {
      await invoke('github_save_token', { token })
      setConfigured(token.trim().length > 0)
      setFromEnv(false)
      setToken('')
      setJustSaved(true)
      setTimeout(() => setJustSaved(false), 2000)
    } catch { /* ignore */ }
    finally { setSaving(false) }
  }

  async function clear() {
    setSaving(true)
    try {
      await invoke('github_save_token', { token: '' })
      setConfigured(false)
      setFromEnv(false)
      setToken('')
    } catch { /* ignore */ }
    finally { setSaving(false) }
  }

  return (
    <div className="rounded-lg border border-gray-200 dark:border-gray-700">
      {/* 折叠头 */}
      <button
        type="button"
        onClick={() => setExpanded(o => !o)}
        className="flex w-full items-center gap-2 px-3 py-2 text-left"
      >
        <KeyRound className="h-3.5 w-3.5 text-gray-400" />
        <span className="text-xs font-medium text-gray-600 dark:text-gray-300">{t('github.tokenSection')}</span>
        {configured && (
          <span className="flex items-center gap-1 rounded-full bg-green-100 px-1.5 py-0.5 text-[10px] text-green-600 dark:bg-green-900/30 dark:text-green-400">
            <CheckCircle2 className="h-3 w-3" />
            {t('github.tokenConfigured')}{fromEnv ? `（${t('github.tokenFromEnv')}）` : ''}
          </span>
        )}
        <span className="ml-auto text-gray-400">
          {expanded ? <ChevronUp className="h-3.5 w-3.5" /> : <ChevronDown className="h-3.5 w-3.5" />}
        </span>
      </button>

      {/* 折叠体 */}
      {expanded && (
        <div className="space-y-2 border-t border-gray-100 px-3 py-3 dark:border-gray-800">
          <p className="text-[11px] leading-relaxed text-gray-400 dark:text-gray-500">
            {t('github.tokenHint')}
          </p>
          <div className="flex gap-2">
            <input
              type="password"
              value={token}
              onChange={e => setToken(e.target.value)}
              placeholder={t('github.tokenPlaceholder')}
              className="field-input flex-1 font-mono text-xs"
              autoComplete="off"
            />
            <button
              type="button"
              onClick={save}
              disabled={saving || !token.trim()}
              className="flex items-center gap-1 rounded-lg bg-blue-600 px-3 py-2 text-xs font-medium text-white hover:bg-blue-500 disabled:opacity-40"
            >
              {justSaved ? <><CheckCircle2 className="h-3.5 w-3.5" /> {t('github.tokenSaved')}</> : t('github.tokenSave')}
            </button>
            {configured && !fromEnv && (
              <button
                type="button"
                onClick={clear}
                disabled={saving}
                className="rounded-lg border border-gray-200 px-3 py-2 text-xs text-gray-500 hover:bg-gray-100 dark:border-gray-700 dark:text-gray-400 dark:hover:bg-gray-800"
              >
                {t('github.tokenClear')}
              </button>
            )}
          </div>
          <div className="flex items-center justify-between">
            <a
              href="https://github.com/settings/tokens/new?description=agent-manager&default_expires=none"
              target="_blank"
              rel="noopener noreferrer"
              className="flex items-center gap-1 text-[11px] text-blue-500 hover:text-blue-400 dark:text-blue-400"
            >
              <ExternalLink className="h-3 w-3" />
              {t('github.tokenGetLink')}
            </a>
            <span className="text-[10px] text-gray-400 dark:text-gray-600">{t('github.tokenNote')}</span>
          </div>
        </div>
      )}
    </div>
  )
}

function RecommendedCard({ agent, onSelect }: { agent: RecommendedAgent; onSelect: () => void }) {
  return (
    <button
      type="button"
      onClick={onSelect}
      className="group flex w-full items-start gap-3 rounded-xl border border-gray-200 bg-white p-3 text-left transition-colors hover:border-blue-300 hover:bg-blue-50 dark:border-gray-700 dark:bg-gray-800/50 dark:hover:border-blue-600 dark:hover:bg-blue-900/20"
    >
      <GitFork className="mt-0.5 h-4 w-4 shrink-0 text-gray-400 group-hover:text-blue-500" />
      <div className="min-w-0 flex-1">
        <p className="text-sm font-medium text-gray-800 dark:text-gray-200">{agent.name}</p>
        <p className="mt-0.5 text-xs text-gray-500 dark:text-gray-400 line-clamp-2">{agent.description}</p>
        <div className="mt-1.5 flex flex-wrap gap-1">
          {agent.tags.map(t => (
            <span key={t} className="rounded-md bg-gray-100 px-1.5 py-0.5 text-[10px] text-gray-500 dark:bg-gray-700 dark:text-gray-400">
              {t}
            </span>
          ))}
        </div>
      </div>
      <ChevronRight className="mt-1 h-3.5 w-3.5 shrink-0 text-gray-300 group-hover:text-blue-400" />
    </button>
  )
}

function RepoPreview({ info }: { info: GithubRepoInfo }) {
  return (
    <div className="rounded-xl border border-gray-200 bg-gray-50 p-4 dark:border-gray-700 dark:bg-gray-800/50">
      <div className="flex items-start justify-between gap-2">
        <div>
          <p className="font-semibold text-gray-900 dark:text-gray-100">{info.name}</p>
          <p className="text-xs text-gray-500 dark:text-gray-400">{info.full_name}</p>
        </div>
        <a
          href={`https://github.com/${info.full_name}`}
          target="_blank"
          rel="noopener noreferrer"
          onClick={e => e.stopPropagation()}
          className="shrink-0 rounded p-1 text-gray-400 hover:text-blue-500"
        >
          <ExternalLink className="h-3.5 w-3.5" />
        </a>
      </div>
      {info.description && (
        <p className="mt-2 text-sm text-gray-600 dark:text-gray-300">{info.description}</p>
      )}
      <div className="mt-3 flex items-center gap-3 text-xs text-gray-400">
        <span className="flex items-center gap-1">
          <Star className="h-3 w-3" /> {info.stars.toLocaleString()}
        </span>
        {info.language && <span>{info.language}</span>}
        <span className="flex items-center gap-1">
          <GitBranch className="h-3 w-3" /> {info.default_branch}
        </span>
      </div>
      {info.readme_excerpt && (
        <pre className="mt-3 max-h-24 overflow-y-auto whitespace-pre-wrap rounded-lg bg-white p-2 text-[11px] leading-relaxed text-gray-500 dark:bg-gray-900 dark:text-gray-400">
          {info.readme_excerpt}
        </pre>
      )}
    </div>
  )
}
