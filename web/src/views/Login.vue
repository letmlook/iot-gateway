<script setup>
import { ref, onMounted } from 'vue'
import { useRouter, useRoute } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { api } from '../api.js'
import { ElMessage } from 'element-plus'
import { initMode } from '../themes.js'
import logBg from '../assets/logbg.png'

const { t } = useI18n()
const router = useRouter()
const route = useRoute()
const username = ref('')
const password = ref('')
const loading = ref(false)

onMounted(() => {
  initMode()
})

async function onSubmit() {
  if (!username.value.trim()) {
    ElMessage.warning(t('login.username') + ' ' + t('common.name'))
    return
  }
  loading.value = true
  try {
    const res = await api.login(username.value.trim(), password.value)
    if (res && res.token !== undefined) {
      if (res.token === null) {
        localStorage.setItem('gateway_authenticated', '1')
        localStorage.setItem('gateway_user', res.user?.username || username.value.trim())
      }
      router.replace((route.query.redirect && String(route.query.redirect)) || '/dashboard')
    } else {
      ElMessage.error(t('login.failed'))
    }
  } catch (e) {
    const msg = e?.message || e?.code || t('login.failed')
    ElMessage.error(msg)
  } finally {
    loading.value = false
  }
}
</script>

<template>
  <div class="login-page" :style="{ backgroundImage: `url(${logBg})` }">
    <div class="login-page-overlay"></div>

    <!-- 左侧：全屏背景上的大标题区 -->
    <div class="login-left">
      <div class="login-branding">
        <p class="login-greeting">Hi, {{ t('login.greeting') }}!</p>
        <h1 class="login-system-name">IoT Gateway</h1>
        <p class="login-system-subtitle">{{ t('login.subtitle') }}</p>
      </div>
    </div>

    <!-- 右侧：浮动白卡表单 -->
    <div class="login-right">
      <div class="login-card">
        <h2 class="login-card-title">{{ t('login.title') }}</h2>
        <form class="login-form" @submit.prevent="onSubmit">
          <div class="login-field">
            <label class="field-label">{{ t('login.username') }}</label>
            <div class="input-wrap">
              <span class="input-bar" aria-hidden="true"></span>
              <input
                v-model="username"
                type="text"
                class="login-input"
                :placeholder="t('login.usernamePlaceholder')"
                autocomplete="username"
              />
            </div>
          </div>
          <div class="login-field">
            <label class="field-label">{{ t('login.password') }}</label>
            <div class="input-wrap">
              <span class="input-bar" aria-hidden="true"></span>
              <input
                v-model="password"
                type="password"
                class="login-input"
                :placeholder="t('login.passwordPlaceholder')"
                autocomplete="current-password"
              />
            </div>
          </div>
          <button type="submit" class="login-btn" :disabled="loading">
            <span v-if="loading" class="btn-loading"></span>
            {{ loading ? t('common.loading') + '...' : t('login.submit') }}
          </button>
        </form>
      </div>
    </div>
  </div>
</template>

<style scoped>
.login-page {
  min-height: 100vh;
  width: 100%;
  display: flex;
  position: relative;
  background-size: cover;
  background-position: center;
  background-color: var(--primary-dim);
}

.login-page-overlay {
  position: absolute;
  inset: 0;
  background: linear-gradient(135deg, rgba(26, 32, 44, 0.85) 0%, rgba(45, 55, 72, 0.75) 50%, rgba(26, 32, 44, 0.85) 100%);
  pointer-events: none;
}

/* 左侧：大标题区，占 2/3 页面宽 */
.login-left {
  flex: 0 0 66.666%;
  display: flex;
  align-items: center;
  justify-content: flex-start;
  padding: 4rem 2rem 4rem 3rem;
  position: relative;
  z-index: 1;
}

.login-branding {
  max-width: 100%;
}

.login-greeting {
  font-size: 3.5rem;
  font-weight: 500;
  color: rgba(255, 255, 255, 0.95);
  margin: 0 0 2.5rem;
  letter-spacing: 0.02em;
}

.login-system-name {
  font-size: 7.5rem;
  font-weight: 700;
  color: #fff;
  margin: 0 0 2rem;
  letter-spacing: -0.03em;
  line-height: 1.1;
  text-shadow: 0 4px 32px rgba(0, 0, 0, 0.35);
}

.login-system-subtitle {
  font-size: 2.5rem;
  font-weight: 400;
  color: rgba(255, 255, 255, 0.85);
  margin: 0;
  letter-spacing: 0.01em;
}

/* 右侧：1/3 页面宽，登录卡左对齐 */
.login-right {
  flex: 0 0 33.334%;
  display: flex;
  align-items: center;
  justify-content: flex-start;
  padding: 3rem 2rem 3rem 2rem;
  position: relative;
  z-index: 1;
}

.login-card {
  width: 100%;
  max-width: 380px;
  background: var(--bg-surface);
  border-radius: 14px;
  box-shadow: 0 8px 40px rgba(0, 0, 0, 0.15), 0 2px 12px rgba(0, 0, 0, 0.08);
  border: 1px solid var(--border-subtle);
  padding: 3rem;
}

.login-card-title {
  font-size: 1.25rem;
  font-weight: 600;
  color: var(--text-primary);
  margin: 0 0 2rem;
  text-align: center;
  letter-spacing: -0.01em;
}

.login-form {
  display: flex;
  flex-direction: column;
  gap: 1.5rem;
}

.login-field {
  width: 100%;
}

.field-label {
  display: block;
  font-size: 0.875rem;
  font-weight: 500;
  color: var(--text-primary);
  margin-bottom: 0.5rem;
}

.input-wrap {
  display: flex;
  align-items: stretch;
  height: 48px;
  background: var(--bg-surface);
  border: 1px solid var(--border-default);
  border-radius: 8px;
  overflow: hidden;
  transition: border-color 0.2s ease, box-shadow 0.2s ease;
}

.input-wrap:focus-within {
  border-color: var(--accent);
  box-shadow: 0 0 0 3px var(--accent-glow);
}

.input-bar {
  width: 3px;
  flex-shrink: 0;
  background: var(--accent);
}

.login-input {
  flex: 1;
  min-width: 0;
  padding: 0 1rem;
  font-size: 0.9375rem;
  color: var(--text-primary);
  background: transparent;
  border: none;
  outline: none;
}

.login-input::placeholder {
  color: var(--text-muted);
}

.login-btn {
  height: 48px;
  margin-top: 0.25rem;
  font-size: 1rem;
  font-weight: 600;
  color: var(--on-primary);
  background: var(--accent);
  border: none;
  border-radius: 8px;
  cursor: pointer;
  transition: background 0.2s ease, box-shadow 0.2s ease;
  box-shadow: var(--shadow-accent);
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 0.5rem;
}

.login-btn:hover:not(:disabled) {
  background: var(--accent-dim);
  box-shadow: 0 4px 16px var(--shadow-accent);
}

.login-btn:disabled {
  opacity: 0.7;
  cursor: not-allowed;
}

.btn-loading {
  width: 18px;
  height: 18px;
  border: 2px solid rgba(255, 255, 255, 0.35);
  border-top-color: #fff;
  border-radius: 50%;
  animation: spin 0.7s linear infinite;
}

@keyframes spin {
  to { transform: rotate(360deg); }
}

@media (max-width: 1024px) {
  .login-left {
    padding: 3rem 2rem 3rem 2.5rem;
  }
  .login-greeting {
    font-size: 2.5rem;
  }
  .login-system-name {
    font-size: 5rem;
  }
  .login-system-subtitle {
    font-size: 1.75rem;
  }
}

@media (max-width: 768px) {
  .login-page {
    flex-direction: column;
  }
  .login-left {
    flex: none;
    min-height: 280px;
    padding: 3rem 2rem;
    justify-content: center;
    text-align: center;
  }
  .login-branding {
    max-width: none;
    margin-right: 0;
  }
  .login-greeting {
    font-size: 2rem;
  }
  .login-system-name {
    font-size: 3.5rem;
  }
  .login-system-subtitle {
    font-size: 1.25rem;
  }
  .login-right {
    flex: 1;
    max-width: none;
    padding: 2rem 4.5rem 2rem 1.5rem;
  }
  .login-card {
    padding: 2.5rem;
  }
}
</style>
