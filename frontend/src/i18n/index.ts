import i18n from 'i18next'
import { initReactI18next } from 'react-i18next'
import zh from './locales/zh'
import en from './locales/en'

export const LANGUAGES = [
  { code: 'zh', label: '中文' },
  { code: 'en', label: 'English' },
] as const

export type LangCode = (typeof LANGUAGES)[number]['code']

const STORAGE_KEY = 'app.lang'

function detectInitialLang(): LangCode {
  const saved = localStorage.getItem(STORAGE_KEY)
  if (saved === 'zh' || saved === 'en') return saved
  // 跟随浏览器语言，默认中文
  const nav = navigator.language?.toLowerCase() ?? ''
  return nav.startsWith('en') ? 'en' : 'zh'
}

i18n
  .use(initReactI18next)
  .init({
    resources: {
      zh: { translation: zh },
      en: { translation: en },
    },
    lng: detectInitialLang(),
    fallbackLng: 'zh',
    interpolation: { escapeValue: false },
    returnNull: false,
  })

export function setLanguage(lang: LangCode) {
  i18n.changeLanguage(lang)
  localStorage.setItem(STORAGE_KEY, lang)
}

export function getLanguage(): LangCode {
  return (i18n.language as LangCode) || 'zh'
}

export default i18n
