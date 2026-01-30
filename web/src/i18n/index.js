import { createI18n } from 'vue-i18n'
import zh from '../locales/zh.js'
import en from '../locales/en.js'

const savedLocale = localStorage.getItem('locale') || 'zh'

export const i18n = createI18n({
  legacy: false,
  locale: savedLocale,
  fallbackLocale: 'zh',
  messages: { zh, en },
})

export function setLocale(locale) {
  i18n.global.locale.value = locale
  localStorage.setItem('locale', locale)
}

/**
 * 根据接口错误码返回中英文提示语；若为 ApiError 且存在对应 errors.xxx 则用 i18n，否则用 error.message。
 * @param {Function} t - useI18n().t
 * @param {Error|{ code?: string, message?: string }} error - 接口抛出的错误
 * @returns {string}
 */
export function getErrorMessage(t, error) {
  if (!error) return ''
  const code = error.code
  const msg = error.message || ''
  if (code && typeof code === 'string') {
    const key = `errors.${code}`
    const translated = t(key)
    if (translated !== key) return translated
  }
  return msg || t('errors.unknown')
}
