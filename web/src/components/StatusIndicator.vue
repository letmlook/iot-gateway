<script setup>
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'

const props = defineProps({
  status: { type: String, default: 'stopped' }, // running | stopped | error | syncing
  size: { type: String, default: 'default' }, // small | default | large
  showText: { type: Boolean, default: true },
  textOverride: { type: String, default: '' }
})

const { t } = useI18n()

const statusText = computed(() => {
  if (props.textOverride) return props.textOverride
  const map = {
    running: t('common.running'),
    stopped: t('common.stopped'),
    error: t('common.error'),
    syncing: t('common.syncing')
  }
  return map[props.status] || props.status
})

const statusClass = computed(() => props.status || 'stopped')
</script>

<template>
  <div class="status-indicator" :class="[statusClass, size, { 'with-text': showText }]">
    <span class="status-icon">
      <!-- 运行中：播放图标 -->
      <svg v-if="status === 'running'" viewBox="0 0 16 16" fill="currentColor">
        <path d="M11.596 8.697l-6.363 3.692c-.54.313-1.233-.066-1.233-.697V4.308c0-.63.692-1.01 1.233-.696l6.363 3.692a.802.802 0 010 1.393z"/>
      </svg>
      <!-- 已停止：方块图标 -->
      <svg v-else-if="status === 'stopped'" viewBox="0 0 16 16" fill="currentColor">
        <rect x="3" y="3" width="10" height="10" rx="2"/>
      </svg>
      <!-- 错误：警告图标 -->
      <svg v-else-if="status === 'error'" viewBox="0 0 16 16" fill="currentColor">
        <path fill-rule="evenodd" d="M8 1a7 7 0 100 14A7 7 0 008 1zM7 5a1 1 0 012 0v3a1 1 0 01-2 0V5zm1 7a1 1 0 100-2 1 1 0 000 2z" clip-rule="evenodd"/>
      </svg>
      <!-- 同步中：旋转图标 -->
      <svg v-else-if="status === 'syncing'" viewBox="0 0 16 16" fill="currentColor" class="spin">
        <path fill-rule="evenodd" d="M13.854 2.146a.5.5 0 010 .708l-3 3a.5.5 0 01-.708-.708L12.293 3H8.5A5.5 5.5 0 003 8.5a.5.5 0 01-1 0A6.5 6.5 0 018.5 2h3.793l-2.147-2.146a.5.5 0 01.708-.708l3 3zM2.146 13.854a.5.5 0 010-.708l3-3a.5.5 0 01.708.708L3.707 13H7.5A5.5 5.5 0 0013 7.5a.5.5 0 011 0A6.5 6.5 0 017.5 14H3.707l2.147 2.146a.5.5 0 01-.708.708l-3-3z" clip-rule="evenodd"/>
      </svg>
      <!-- 默认：圆点 -->
      <span v-else class="status-dot-simple"></span>
    </span>
    <span v-if="showText" class="status-text">{{ statusText }}</span>
  </div>
</template>

<style scoped>
.status-indicator {
  display: inline-flex;
  align-items: center;
  gap: 0.4rem;
  padding: 0.3rem 0.65rem;
  border-radius: 100px;
  font-size: 0.8rem;
  font-weight: 500;
  transition: all var(--transition-fast);
}

.status-indicator:not(.with-text) {
  padding: 0.35rem;
}

/* 尺寸变体 */
.status-indicator.small {
  padding: 0.2rem 0.5rem;
  font-size: 0.75rem;
  gap: 0.3rem;
}

.status-indicator.small .status-icon {
  width: 12px;
  height: 12px;
}

.status-indicator.large {
  padding: 0.4rem 0.85rem;
  font-size: 0.875rem;
  gap: 0.5rem;
}

.status-indicator.large .status-icon {
  width: 18px;
  height: 18px;
}

/* 状态颜色 */
.status-indicator.running {
  background: rgba(56, 161, 105, 0.12);
  color: var(--success);
}

.status-indicator.stopped {
  background: var(--bg-inset);
  color: var(--text-muted);
  border: 1px dashed var(--border-default);
}

.status-indicator.error {
  background: rgba(229, 62, 62, 0.1);
  color: var(--danger);
}

.status-indicator.syncing {
  background: var(--accent-glow);
  color: var(--accent-dim);
}

/* 图标 */
.status-icon {
  width: 14px;
  height: 14px;
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.status-icon svg {
  width: 100%;
  height: 100%;
}

.status-dot-simple {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: currentColor;
}

/* 动画效果 */
.status-indicator.running .status-icon {
  animation: pulse-glow 2s ease-in-out infinite;
}

@keyframes pulse-glow {
  0%, 100% {
    opacity: 1;
    transform: scale(1);
  }
  50% {
    opacity: 0.7;
    transform: scale(1.1);
  }
}

.status-indicator.error {
  animation: error-blink 1.5s ease-in-out infinite;
}

@keyframes error-blink {
  0%, 100% {
    box-shadow: 0 0 0 0 rgba(229, 62, 62, 0);
  }
  50% {
    box-shadow: 0 0 0 4px rgba(229, 62, 62, 0.12);
  }
}

.spin {
  animation: spin 1s linear infinite;
}

@keyframes spin {
  from { transform: rotate(0deg); }
  to { transform: rotate(360deg); }
}
</style>
