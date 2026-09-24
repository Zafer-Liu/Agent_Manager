import { useEffect, useState } from 'react'
import { useTranslation } from 'react-i18next'
import { invoke } from '@tauri-apps/api/core'
import { CheckCircle2, Loader2, Bot } from 'lucide-react'
import type { AgentState } from '../../types/agent'

export function StepAgents({ onNext }: { onNext: () => void }) {
  const { t } = useTranslation()
  const [agents, setAgents] = useState<AgentState[] | null>(null)

  useEffect(() => {
    invoke<AgentState[]>('list_agents')
      .then(setAgents)
      .catch(() => setAgents([]))
  }, [])

  return (
    <div>
      <h2 className="text-lg font-bold text-gray-900 dark:text-gray-100">
        {t('onboarding.agents.title')}
      </h2>
      <p className="mt-2 text-sm text-gray-500 dark:text-gray-400">
        {t('onboarding.agents.desc')}
      </p>

      <div className="mt-5 min-h-[120px]">
        {agents === null ? (
          <div className="flex items-center justify-center py-8">
            <Loader2 className="h-5 w-5 animate-spin text-blue-500" />
          </div>
        ) : agents.length === 0 ? (
          <div className="rounded-xl border border-dashed border-gray-200 px-4 py-8 text-center dark:border-gray-700">
            <Bot className="mx-auto h-8 w-8 text-gray-300 dark:text-gray-600" />
            <p className="mt-2 text-sm text-gray-400 dark:text-gray-500">
              {t('onboarding.agents.empty')}
            </p>
          </div>
        ) : (
          <div className="space-y-2">
            {agents.map(agent => (
              <div
                key={agent.config.id}
                className="flex items-center gap-3 rounded-xl border border-gray-100 bg-gray-50 px-4 py-3 dark:border-gray-800 dark:bg-gray-800/50"
              >
                <CheckCircle2 className="h-4 w-4 shrink-0 text-emerald-500" />
                <div className="min-w-0 flex-1">
                  <p className="truncate text-sm font-medium text-gray-900 dark:text-gray-100">
                    {agent.config.name}
                  </p>
                  <p className="truncate text-xs text-gray-400 dark:text-gray-500">
                    {agent.config.command}
                  </p>
                </div>
              </div>
            ))}
          </div>
        )}
      </div>

      <button
        onClick={onNext}
        className="mt-6 w-full rounded-xl bg-blue-600 px-4 py-2.5 text-sm font-medium text-white transition-colors hover:bg-blue-500"
      >
        {t('onboarding.agents.next')}
      </button>
    </div>
  )
}
