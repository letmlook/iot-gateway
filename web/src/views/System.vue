<script setup>
import { ref, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage } from 'element-plus'
import { Download, CopyDocument, Link } from '@element-plus/icons-vue'
import { api } from '../api.js'

const { t } = useI18n()

const version = ref(null)
const health = ref(null)
const metrics = ref('')
const exportData = ref(null)
const loading = ref(false)
const error = ref('')
const showExportModal = ref(false)

async function loadSystemInfo() {
  loading.value = true
  error.value = ''
  try {
    const [v, h, m] = await Promise.all([
      api.version().catch(() => null),
      api.health().catch(() => null),
      api.metrics().catch(() => ''),
    ])
    version.value = v
    health.value = h
    metrics.value = m
  } catch (e) {
    error.value = t('system.loadFailed') + e.message
  } finally {
    loading.value = false
  }
}

async function exportConfig() {
  try {
    const data = await api.export()
    exportData.value = JSON.stringify(data, null, 2)
    showExportModal.value = true
  } catch (e) {
    error.value = t('system.exportFailed') + e.message
  }
}

function downloadExport() {
  const blob = new Blob([exportData.value], { type: 'application/json' })
  const url = URL.createObjectURL(blob)
  const a = document.createElement('a')
  a.href = url
  a.download = `gateway-config-${new Date().toISOString().slice(0, 10)}.json`
  a.click()
  URL.revokeObjectURL(url)
}

async function copyExport() {
  try {
    await navigator.clipboard.writeText(exportData.value)
    ElMessage.success(t('monitor.copied'))
  } catch (e) {
    ElMessage.error(t('monitor.copyFailed'))
  }
}

function parseMetrics(text) {
  if (!text) return []
  return text.split('\n')
    .filter(l => l && !l.startsWith('#'))
    .map(l => {
      const m = l.match(/^(\w+)\s+(\S+)$/)
      return m ? { name: m[1], value: m[2] } : null
    })
    .filter(Boolean)
}

onMounted(loadSystemInfo)
</script>

<template>
  <div class="page-container system-page">
    <div class="system-body">
      <div class="page-header system-header">
        <p class="header-desc">{{ t('system.desc') }}</p>
      </div>

      <div class="system-actions">
        <el-button type="primary" :icon="Download" @click="exportConfig">{{ t('system.exportConfig') }}</el-button>
      </div>

      <el-alert v-if="error" type="error" :title="error" closable show-icon @close="error = ''" class="mb-2" />

      <el-skeleton v-if="loading" :rows="8" animated />

      <div v-else class="system-grid">
      <el-card shadow="hover" class="system-card">
        <template #header><span>{{ t('system.versionInfo') }}</span></template>
        <div v-if="version" class="info-grid">
          <div class="info-item">
            <span class="info-label">{{ t('system.version') }}</span>
            <span class="info-value highlight">v{{ version.version }}</span>
          </div>
          <div v-if="version.build_date" class="info-item">
            <span class="info-label">{{ t('system.buildDate') }}</span>
            <span class="info-value">{{ version.build_date }}</span>
          </div>
          <div v-if="version.revision" class="info-item">
            <span class="info-label">{{ t('system.gitRevision') }}</span>
            <span class="info-value font-mono">{{ version.revision.slice(0, 8) }}</span>
          </div>
        </div>
        <div v-else class="no-data">{{ t('system.noVersion') }}</div>
      </el-card>

      <el-card shadow="hover" class="system-card">
        <template #header><span>{{ t('system.runStatus') }}</span></template>
        <div v-if="health" class="info-grid">
          <div class="info-item">
            <span class="info-label">{{ t('system.systemStatus') }}</span>
            <el-tag :type="health.status === 'ok' ? 'success' : 'danger'" size="small">
              {{ health.status === 'ok' ? t('common.normal') : t('common.error') }}
            </el-tag>
          </div>
          <div class="info-item">
            <span class="info-label">{{ t('system.nodesTotal') }}</span>
            <span class="info-value">{{ health.nodes_count || 0 }}</span>
          </div>
          <div class="info-item">
            <span class="info-label">{{ t('system.running') }}</span>
            <span class="info-value highlight">{{ health.nodes_running || 0 }}</span>
          </div>
          <div class="info-item">
            <span class="info-label">{{ t('system.southPlugins') }}</span>
            <span class="info-value">{{ health.plugins_south || 0 }}</span>
          </div>
          <div class="info-item">
            <span class="info-label">{{ t('system.northPlugins') }}</span>
            <span class="info-value">{{ health.plugins_north || 0 }}</span>
          </div>
        </div>
        <div v-else class="no-data">{{ t('system.noHealth') }}</div>
      </el-card>

      <el-card shadow="hover" class="system-card metrics-card">
        <template #header><span>{{ t('system.prometheusMetrics') }}</span></template>
        <div v-if="parseMetrics(metrics).length" class="metrics-list">
          <div class="metric-item" v-for="m in parseMetrics(metrics)" :key="m.name">
            <span class="metric-name">{{ m.name }}</span>
            <span class="metric-value">{{ m.value }}</span>
          </div>
        </div>
        <div v-else class="no-data">{{ t('system.noMetrics') }}</div>
        <el-link href="/api/metrics" target="_blank" type="primary" :icon="Link" class="mt-2">{{ t('system.viewRaw') }}</el-link>
      </el-card>
    </div>
    </div>

    <el-dialog v-model="showExportModal" :title="t('system.exportConfig')" width="640px" destroy-on-close>
      <el-input :model-value="exportData" type="textarea" :rows="18" readonly class="font-mono export-textarea" />
      <template #footer>
        <el-button :icon="CopyDocument" @click="copyExport">{{ t('system.copyExport') }}</el-button>
        <el-button type="primary" :icon="Download" @click="downloadExport">{{ t('system.downloadJson') }}</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<style scoped>
/* 系统页整体与下方内容同宽，导出配置按钮与卡片右侧对齐 */
.system-page {
  width: 100%;
  min-width: 0;
  max-width: 100%;
  box-sizing: border-box;
}

/* 头部与卡片同一包裹层，保证同宽 */
.system-body {
  width: 100%;
  min-width: 0;
  max-width: 100%;
  overflow: hidden;
  display: flex;
  flex-direction: column;
  flex: 1;
}

.system-header {
  width: 100%;
  max-width: 100%;
  min-width: 0;
  box-sizing: border-box;
  margin-bottom: 0.5rem;
}

/* 导出配置按钮放在描述下方、与卡片同宽区域（位置 2） */
.system-actions {
  width: 100%;
  margin-bottom: 1.5rem;
}

.mb-2 { margin-bottom: 1rem; }
.mt-2 { margin-top: 0.5rem; }
.system-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(300px, 1fr)); gap: 1rem; width: 100%; }
.info-grid { display: grid; gap: 0.75rem; }
.info-item { display: flex; justify-content: space-between; align-items: center; }
.info-label { font-size: 0.9rem; color: var(--text-muted); }
.info-value.highlight { color: var(--el-color-primary); font-weight: 500; }
.no-data { color: var(--text-muted); font-size: 0.9rem; }
.metrics-list { display: grid; grid-template-columns: repeat(auto-fill, minmax(180px, 1fr)); gap: 0.5rem; }
.metric-item { display: flex; justify-content: space-between; padding: 0.4rem 0.6rem; background: var(--el-fill-color-light); border-radius: 6px; font-size: 0.85rem; }
.metric-name { font-family: var(--font-mono); color: var(--text-secondary); }
.metric-value { font-weight: 600; color: var(--el-color-primary); }
.metrics-card { grid-column: span 2; }
.font-mono { font-family: var(--font-mono); }
@media (max-width: 900px) { .metrics-card { grid-column: span 1; } }
</style>
