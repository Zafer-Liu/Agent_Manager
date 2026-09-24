import { useState } from 'react'
import { useTranslation } from 'react-i18next'
import { StepWelcome } from './onboarding/StepWelcome'
import { StepAgents } from './onboarding/StepAgents'
import { StepLlm } from './onboarding/StepLlm'
import { StepDone } from './onboarding/StepDone'

const STORAGE_KEY = 'onboarding-complete'

type Step = 'welcome' | 'agents' | 'llm' | 'done'

const STEPS: Step[] = ['welcome', 'agents', 'llm', 'done']

/** First-run onboarding wizard. Shows once; dismissed permanently via localStorage. */
export function Onboarding() {
  const { t } = useTranslation()
  const [visible, setVisible] = useState(() => !localStorage.getItem(STORAGE_KEY))
  const [step, setStep] = useState<Step>('welcome')

  if (!visible) return null

  const finish = () => {
    localStorage.setItem(STORAGE_KEY, '1')
    setVisible(false)
  }

  const stepIndex = STEPS.indexOf(step)

  return (
    <div className="fixed inset-0 z-50 flex items-center justify-center bg-black/40 backdrop-blur-sm">
      <div className="w-full max-w-md rounded-2xl bg-white p-8 shadow-2xl dark:bg-gray-900">
        {/* Step indicator + skip */}
        <div className="mb-6 flex items-center justify-between">
          <div className="flex gap-1.5">
            {STEPS.map((s, i) => (
              <div
                key={s}
                className={`h-1.5 rounded-full transition-all ${
                  i <= stepIndex
                    ? 'w-6 bg-blue-500'
                    : 'w-3 bg-gray-200 dark:bg-gray-700'
                }`}
              />
            ))}
          </div>
          {step !== 'done' && (
            <button
              onClick={finish}
              className="text-xs text-gray-400 transition-colors hover:text-gray-600 dark:hover:text-gray-300"
            >
              {t('onboarding.skip')}
            </button>
          )}
        </div>

        {/* Step content */}
        {step === 'welcome' && <StepWelcome onNext={() => setStep('agents')} />}
        {step === 'agents' && <StepAgents onNext={() => setStep('llm')} />}
        {step === 'llm' && <StepLlm onNext={() => setStep('done')} />}
        {step === 'done' && <StepDone onFinish={finish} />}
      </div>
    </div>
  )
}
