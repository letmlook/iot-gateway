<script setup>
import { ref } from 'vue'
import { useRouter, useRoute } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { api } from '../api.js'
import { ElMessage } from 'element-plus'

const { t } = useI18n()
const router = useRouter()
const route = useRoute()
const username = ref('')
const password = ref('')
const loading = ref(false)

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
  <div class="login-page">
    <div class="login-left">
      <div class="login-left-bg"></div>
      <div class="login-left-overlay"></div>
      <div class="login-brand">PMI-edge</div>
      <div class="login-left-decoration">
        <div class="deco-circle deco-circle-1"></div>
        <div class="deco-circle deco-circle-2"></div>
        <div class="deco-line deco-line-1"></div>
        <div class="deco-line deco-line-2"></div>
      </div>
    </div>
    <div class="login-right">
      <div class="login-form-wrap">
        <h1 class="login-title">{{ t('login.title') }}</h1>
        <form class="login-form" @submit.prevent="onSubmit">
          <div class="login-field">
            <input
              v-model="username"
              type="text"
              class="login-input"
              :placeholder="t('login.usernamePlaceholder')"
              autocomplete="username"
            />
          </div>
          <div class="login-field">
            <input
              v-model="password"
              type="password"
              class="login-input"
              :placeholder="t('login.passwordPlaceholder')"
              autocomplete="current-password"
            />
          </div>
          <button type="submit" class="login-btn" :disabled="loading">
            {{ loading ? t('common.loading') + '...' : t('login.submit') }}
          </button>
        </form>
      </div>
    </div>
  </div>
</template>

<style scoped>
.login-page {
  display: flex;
  min-height: 100vh;
  width: 100%;
  background: #f0f4f8;
}

.login-left {
  width: 45%;
  max-width: 560px;
  min-width: 320px;
  position: relative;
  overflow: hidden;
  display: flex;
  align-items: center;
  justify-content: center;
}

.login-left-bg {
  position: absolute;
  inset: 0;
  background: 
    linear-gradient(135deg, 
      rgba(59, 130, 246, 0.9) 0%, 
      rgba(37, 99, 235, 0.85) 50%, 
      rgba(29, 78, 216, 0.9) 100%
    ),
    url('data:image/svg+xml,<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 100 100"><defs><pattern id="grid" width="10" height="10" patternUnits="userSpaceOnUse"><path d="M 10 0 L 0 0 0 10" fill="none" stroke="rgba(255,255,255,0.1)" stroke-width="0.5"/></pattern></defs><rect width="100" height="100" fill="url(%23grid)"/></svg>');
  background-size: cover, 40px 40px;
}

.login-left-overlay {
  position: absolute;
  inset: 0;
  background: 
    radial-gradient(circle at 30% 20%, rgba(255, 255, 255, 0.1) 0%, transparent 40%),
    radial-gradient(circle at 70% 80%, rgba(0, 0, 0, 0.1) 0%, transparent 40%);
  pointer-events: none;
}

.login-left-decoration {
  position: absolute;
  inset: 0;
  pointer-events: none;
  overflow: hidden;
}

.deco-circle {
  position: absolute;
  border-radius: 50%;
  border: 1px solid rgba(255, 255, 255, 0.15);
}

.deco-circle-1 {
  width: 300px;
  height: 300px;
  top: -80px;
  left: -80px;
}

.deco-circle-2 {
  width: 200px;
  height: 200px;
  bottom: 10%;
  right: -60px;
  border-width: 2px;
  border-color: rgba(255, 255, 255, 0.1);
}

.deco-line {
  position: absolute;
  background: rgba(255, 255, 255, 0.08);
}

.deco-line-1 {
  width: 1px;
  height: 40%;
  left: 25%;
  top: 30%;
}

.deco-line-2 {
  width: 30%;
  height: 1px;
  right: 10%;
  bottom: 35%;
}

.login-brand {
  position: absolute;
  top: 2.5rem;
  left: 2.5rem;
  font-size: 1.75rem;
  font-weight: 700;
  color: #fff;
  letter-spacing: -0.02em;
  z-index: 2;
  text-shadow: 0 2px 8px rgba(0, 0, 0, 0.2);
}

.login-right {
  flex: 1;
  background: #fff;
  display: flex;
  align-items: center;
  justify-content: center;
  padding: 3rem;
  box-shadow: -4px 0 24px rgba(0, 0, 0, 0.06);
}

.login-form-wrap {
  width: 100%;
  max-width: 340px;
}

.login-title {
  font-size: 1.5rem;
  font-weight: 600;
  color: #3b82f6;
  margin-bottom: 2.5rem;
  letter-spacing: -0.02em;
  text-align: center;
}

.login-form {
  display: flex;
  flex-direction: column;
  gap: 1.5rem;
}

.login-field {
  width: 100%;
}

.login-input {
  width: 100%;
  height: 48px;
  padding: 0 1rem;
  padding-left: 1.25rem;
  font-size: 0.95rem;
  color: #334155;
  background: #fff;
  border: 1px solid #e2e8f0;
  border-left: 3px solid #3b82f6;
  border-radius: 4px;
  outline: none;
  transition: all 0.2s ease;
  box-sizing: border-box;
}

.login-input::placeholder {
  color: #94a3b8;
}

.login-input:hover {
  border-color: #cbd5e1;
  border-left-color: #2563eb;
}

.login-input:focus {
  border-color: #93c5fd;
  border-left-color: #2563eb;
  box-shadow: 0 0 0 3px rgba(59, 130, 246, 0.1);
}

.login-btn {
  height: 48px;
  margin-top: 1rem;
  font-size: 1rem;
  font-weight: 500;
  color: #fff;
  background: #3b82f6;
  border: none;
  border-radius: 4px;
  cursor: pointer;
  transition: all 0.2s ease;
  box-shadow: 0 2px 8px rgba(59, 130, 246, 0.3);
}

.login-btn:hover:not(:disabled) {
  background: #2563eb;
  box-shadow: 0 4px 12px rgba(59, 130, 246, 0.4);
  transform: translateY(-1px);
}

.login-btn:active:not(:disabled) {
  transform: translateY(0);
}

.login-btn:disabled {
  opacity: 0.7;
  cursor: not-allowed;
}

@media (max-width: 900px) {
  .login-left {
    width: 40%;
    min-width: 280px;
  }
}

@media (max-width: 768px) {
  .login-page {
    flex-direction: column;
  }
  .login-left {
    width: 100%;
    max-width: none;
    min-height: 200px;
  }
  .login-right {
    flex: 1;
    padding: 2rem;
  }
  .login-brand {
    top: 1.5rem;
    left: 1.5rem;
    font-size: 1.5rem;
  }
}
</style>
