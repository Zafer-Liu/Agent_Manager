import { useTranslation } from 'react-i18next'
import { UpdateChecker } from '../components/UpdateChecker'

export function Settings() {
  const { t } = useTranslation()

  return (
    <div className="flex h-full flex-col overflow-y-auto bg-gray-50 dark:bg-gray-950 p-6 space-y-6">
      {/* Page header */}
      <div>
        <h2 className="text-base font-semibold text-gray-900 dark:text-gray-100">
          {t('settings.title')}
        </h2>
        <p className="text-xs text-gray-500 mt-0.5">{t('settings.subtitle')}</p>
      </div>

      {/* Updates section */}
      <section className="space-y-3">
        <h3 className="text-xs font-semibold uppercase tracking-wide text-gray-500">
          {t('settings.sectionUpdates')}
        </h3>
        <UpdateChecker autoCheck={false} />
      </section>

      {/* About section */}
      <section className="space-y-3">
        <h3 className="text-xs font-semibold uppercase tracking-wide text-gray-500">
          {t('settings.sectionAbout')}
        </h3>
        <div className="rounded-2xl border border-gray-200 bg-white p-4 dark:border-gray-700 dark:bg-gray-900 space-y-1.5">
          <AboutRow label={t('settings.aboutName')} value="智管-Agent Manager" />
          <AboutRow label={t('settings.aboutLicense')} value="MIT License" />
          <AboutRow
            label={t('settings.aboutSource')}
            value="GitHub"
            href="https://github.com/Zafer-Liu/Agent_Manager"
          />
        </div>
      </section>
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
