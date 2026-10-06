/**
 * 认证状态管理。
 * - token 持久化在 localStorage
 * - 组件通过 storeToRefs(auth) 获取响应式状态
 */
import { defineStore } from 'pinia'
import { ref, computed } from 'vue'

export const useAuthStore = defineStore('auth', () => {
  const TOKEN_KEY = 'gateway_token'
  const USER_KEY = 'gateway_user'
  const AUTH_KEY = 'gateway_authenticated'

  // State
  const token = ref(localStorage.getItem(TOKEN_KEY) || null)
  const username = ref(localStorage.getItem(USER_KEY) || null)
  const authenticated = ref(localStorage.getItem(AUTH_KEY) === '1')

  // Getters
  const isAuthenticated = computed(() =>
    authenticated.value || (token.value !== null && token.value !== '')
  )
  const bearerToken = computed(() => token.value ? `Bearer ${token.value}` : null)

  // Actions
  function setAuth(newToken, user) {
    token.value = newToken
    username.value = user || null
    authenticated.value = true
    try {
      if (newToken) localStorage.setItem(TOKEN_KEY, newToken)
      if (user) localStorage.setItem(USER_KEY, user)
      localStorage.setItem(AUTH_KEY, '1')
    } catch (_) {}
  }

  function clearAuth() {
    token.value = null
    username.value = null
    authenticated.value = false
    try {
      localStorage.removeItem(TOKEN_KEY)
      localStorage.removeItem(USER_KEY)
      localStorage.removeItem(AUTH_KEY)
    } catch (_) {}
  }

  return {
    token,
    username,
    authenticated,
    isAuthenticated,
    bearerToken,
    setAuth,
    clearAuth,
  }
})
