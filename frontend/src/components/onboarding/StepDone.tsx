import { useTranslation } from 'react-i18next'
import { CheckCircle2 } from 'lucide-react'

export function StepDone({ onFinish }: { onFinish: () => void }) {
  const { t } = useTranslation()
  return (
    <div className="flex flex-col items-center text-center">
      <div className="mb-6 flex h-16 w-16 items-center justify-center rounded-2xl bg-emerald-50 dark:bg-emerald-500/15">
        <CheckCircle2 className="h-8 w-8 text-emerald-600 dark:text-emerald-400" />
      </div>
      <h2 className="text-xl font-bold text-gray-900 dark:text-gray-100">
        {t('onboarding.done.title')}
      </h2>
      <p className="mt-3 text-sm leading-relaxed text-gray-500 dark:text-gray-400">
        {t('onboarding.done.desc')}
      </p>
      <button
        onClick={onFinish}
        className="mt-8 w-full rounded-xl bg-blue-600 px-4 py-2.5 text-sm font-medium text-white transition-colors hover:bg-blue-500"
      >
        {t('onboarding.done.enter')}
      </button>
    </div>
  )
}
