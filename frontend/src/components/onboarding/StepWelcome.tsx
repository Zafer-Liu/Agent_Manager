import { useTranslation } from 'react-i18next'
import { Bot } from 'lucide-react'

export function StepWelcome({ onNext }: { onNext: () => void }) {
  const { t } = useTranslation()
  return (
    <div className="flex flex-col items-center text-center">
      <div className="mb-6 flex h-16 w-16 items-center justify-center rounded-2xl bg-blue-50 dark:bg-blue-500/15">
        <Bot className="h-8 w-8 text-blue-600 dark:text-blue-400" />
      </div>
      <h2 className="text-xl font-bold text-gray-900 dark:text-gray-100">
        {t('onboarding.welcome.title')}
      </h2>
      <p className="mt-3 text-sm leading-relaxed text-gray-500 dark:text-gray-400">
        {t('onboarding.welcome.pitch')}
      </p>
      <button
        onClick={onNext}
        className="mt-8 w-full rounded-xl bg-blue-600 px-4 py-2.5 text-sm font-medium text-white transition-colors hover:bg-blue-500"
      >
        {t('onboarding.welcome.start')}
      </button>
    </div>
  )
}
