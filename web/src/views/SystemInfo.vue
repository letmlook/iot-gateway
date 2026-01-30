<script setup>
import { ref, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { getErrorMessage } from '../i18n'
import { Link } from '@element-plus/icons-vue'
import { api } from '../api.js'

const { t } = useI18n()

const version = ref(null)
const health = ref(null)
const metrics = ref('')
const hardwareInfo = ref(null)
const loading = ref(false)
const error = ref('')

async function loadSystemInfo() {
  loading.value = true
  error.value = ''
  try {
    const [v, h, m, hw] = await Promise.all([
      api.version().catch(() => null),
      api.health().catch(() => null),
      api.metrics().catch(() => ''),
      api.hardwareInfo?.().catch(() => null),
    ])
    version.value = v
    health.value = h
    metrics.value = m
    hardwareInfo.value = hw
  } catch (e) {
    error.value = t('sysInfo.loadFailed') + getErrorMessage(t, e)
  } finally {
    loading.value = false
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

function formatUptime(seconds) {
  if (!seconds) return '-'
  const hours = Math.floor(seconds / 3600)
  const minutes = Math.floor((seconds % 3600) / 60)
  const secs = Math.floor(seconds % 60)
  if (hours > 0) {
    return `${hours}${t('sysInfo.hours')} ${minutes}${t('sysInfo.minutes')} ${secs}${t('sysInfo.seconds')}`
  } else if (minutes > 0) {
    return `${minutes}${t('sysInfo.minutes')} ${secs}${t('sysInfo.seconds')}`
  }
  return `${secs}${t('sysInfo.seconds')}`
}

function formatBytes(bytes) {
  if (bytes === undefined || bytes === null) return '-'
  const units = ['B', 'KB', 'MB', 'GB', 'TB']
  let i = 0
  let size = bytes
  while (size >= 1024 && i < units.length - 1) {
    size /= 1024
    i++
  }
  return `${size.toFixed(2)} ${units[i]}`
}

onMounted(loadSystemInfo)
</script>

<template>
  <div class="page-container sysinfo-page">
    <div class="sysinfo-body">
      <el-alert v-if="error" type="error" :title="error" closable show-icon @close="error = ''" class="mb-2" />

      <el-skeleton v-if="loading" :rows="8" animated />

      <div v-else class="sysinfo-content">
        <!-- 基础信息 -->
        <div class="info-section">
          <h3 class="section-title">
            {{ t('sysInfo.basicInfo') }}
            <span v-if="version" class="version-badge">v{{ version.version }}</span>
          </h3>

          <div class="info-grid">
            <!-- 系统状态 -->
            <div class="info-row">
              <span class="info-label">{{ t('sysInfo.systemStatus') }}</span>
              <span class="info-value">
                <el-tag :type="health?.status === 'ok' ? 'success' : 'danger'" size="small">
                  {{ health?.status === 'ok' ? t('common.running') : t('common.error') }}
                </el-tag>
              </span>
            </div>

            <!-- 数采引擎状态 -->
            <div class="info-row">
              <span class="info-label">{{ t('sysInfo.dataEngineStatus') }}</span>
              <span class="info-value">
                <el-tag type="success" size="small">{{ t('common.running') }}</el-tag>
              </span>
            </div>

            <!-- 数采引擎运行时长 -->
            <div class="info-row">
              <span class="info-label">{{ t('sysInfo.dataEngineUptime') }}</span>
              <span class="info-value">{{ formatUptime(health?.uptime) }}</span>
            </div>

            <!-- 节点统计 -->
            <div class="info-row">
              <span class="info-label">{{ t('sysInfo.nodesTotal') }}</span>
              <span class="info-value">{{ health?.nodes_count || 0 }}</span>
            </div>

            <div class="info-row">
              <span class="info-label">{{ t('sysInfo.nodesRunning') }}</span>
              <span class="info-value highlight">{{ health?.nodes_running || 0 }}</span>
            </div>

            <!-- 插件统计 -->
            <div class="info-row">
              <span class="info-label">{{ t('sysInfo.southPlugins') }}</span>
              <span class="info-value">{{ health?.plugins_south || 0 }}</span>
            </div>

            <div class="info-row">
              <span class="info-label">{{ t('sysInfo.northPlugins') }}</span>
              <span class="info-value">{{ health?.plugins_north || 0 }}</span>
            </div>
          </div>
        </div>

        <!-- 硬件信息 -->
        <div class="info-section">
          <h3 class="section-title">{{ t('sysInfo.hardwareInfo') }}</h3>
          <div class="info-grid">
            <div class="info-row">
              <span class="info-label">{{ t('sysInfo.hardwareId') }}</span>
              <span class="info-value font-mono">{{ hardwareInfo?.hardware_id || version?.hardware_id || '-' }}</span>
            </div>

            <div class="info-row">
              <span class="info-label">{{ t('sysInfo.architecture') }}</span>
              <span class="info-value font-mono">{{ hardwareInfo?.arch || version?.arch || '-' }}</span>
            </div>

            <div class="info-row">
              <span class="info-label">{{ t('sysInfo.osVersion') }}</span>
              <span class="info-value">{{ hardwareInfo?.os_version || version?.platform || '-' }}</span>
            </div>

            <div class="info-row">
              <span class="info-label">{{ t('sysInfo.kernelVersion') }}</span>
              <span class="info-value font-mono">{{ hardwareInfo?.kernel_version || '-' }}</span>
            </div>

            <div class="info-row">
              <span class="info-label">{{ t('sysInfo.cpuUsage') }}</span>
              <span class="info-value">
                <template v-if="hardwareInfo?.cpu_usage !== undefined">
                  <el-progress :percentage="Math.round(hardwareInfo.cpu_usage * 100)" :stroke-width="10" style="width: 200px"/>
                  <span class="progress-text">{{ (hardwareInfo.cpu_usage * 100).toFixed(1) }}%</span>
                </template>
                <template v-else>-</template>
              </span>
            </div>

            <div class="info-row">
              <span class="info-label">{{ t('sysInfo.memoryUsage') }}</span>
              <span class="info-value">
                <template v-if="hardwareInfo?.memory_used !== undefined && hardwareInfo?.memory_total !== undefined">
                  <el-progress :percentage="Math.round(hardwareInfo.memory_used / hardwareInfo.memory_total * 100)" :stroke-width="10" style="width: 200px"/>
                  <span class="progress-text">{{ formatBytes(hardwareInfo.memory_used) }} / {{ formatBytes(hardwareInfo.memory_total) }}</span>
                </template>
                <template v-else>-</template>
              </span>
            </div>
          </div>
        </div>

        <!-- 版本信息 -->
        <div class="info-section">
          <h3 class="section-title">{{ t('sysInfo.versionInfo') }}</h3>
          <div class="info-grid">
            <div v-if="version?.version" class="info-row">
              <span class="info-label">{{ t('sysInfo.version') }}</span>
              <span class="info-value highlight">v{{ version.version }}</span>
            </div>

            <div v-if="version?.build_date" class="info-row">
              <span class="info-label">{{ t('sysInfo.buildDate') }}</span>
              <span class="info-value">{{ version.build_date }}</span>
            </div>

            <div v-if="version?.revision" class="info-row">
              <span class="info-label">{{ t('sysInfo.gitRevision') }}</span>
              <span class="info-value font-mono">{{ version.revision.slice(0, 8) }}</span>
            </div>
          </div>
        </div>

        <!-- Prometheus 指标 -->
        <div class="info-section metrics-section">
          <h3 class="section-title">{{ t('sysInfo.prometheusMetrics') }}</h3>
          <div v-if="parseMetrics(metrics).length" class="metrics-list">
            <div class="metric-item" v-for="m in parseMetrics(metrics)" :key="m.name">
              <span class="metric-name">{{ m.name }}</span>
              <span class="metric-value">{{ m.value }}</span>
            </div>
          </div>
          <div v-else class="no-data">{{ t('sysInfo.noMetrics') }}</div>
          <el-link href="/api/metrics" target="_blank" type="primary" :icon="Link" class="mt-2">
            {{ t('sysInfo.viewRaw') }}
          </el-link>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.sysinfo-page {
  padding: 0;
}
.sysinfo-body {
  max-width: 900px;
}
.sysinfo-content {
  display: flex;
  flex-direction: column;
  gap: 1.5rem;
}
.info-section {
  background: var(--el-bg-color);
  border: 1px solid var(--el-border-color-light);
  border-radius: 8px;
  padding: 1.25rem;
}
.section-title {
  font-size: 1rem;
  font-weight: 600;
  margin: 0 0 1rem 0;
  color: var(--el-text-color-primary);
  display: flex;
  align-items: center;
  gap: 0.75rem;
}
.version-badge {
  font-size: 0.85rem;
  font-weight: 500;
  color: var(--el-color-primary);
  background: var(--el-color-primary-light-9);
  padding: 0.15rem 0.5rem;
  border-radius: 4px;
}
.info-grid {
  display: grid;
  gap: 0.75rem;
}
.info-row {
  display: flex;
  align-items: center;
  padding: 0.5rem 0;
  border-bottom: 1px solid var(--el-border-color-lighter);
}
.info-row:last-child {
  border-bottom: none;
}
.info-label {
  min-width: 160px;
  font-size: 0.9rem;
  color: var(--el-text-color-secondary);
}
.info-value {
  flex: 1;
  font-size: 0.9rem;
}
.info-value.highlight {
  color: var(--el-color-primary);
  font-weight: 500;
}
.info-value .progress-text {
  margin-left: 0.75rem;
  color: var(--el-text-color-secondary);
  font-size: 0.85rem;
}
.font-mono {
  font-family: ui-monospace, monospace;
}
.metrics-list {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
  gap: 0.5rem;
}
.metric-item {
  display: flex;
  justify-content: space-between;
  padding: 0.5rem 0.75rem;
  background: var(--el-fill-color-light);
  border-radius: 6px;
  font-size: 0.85rem;
}
.metric-name {
  font-family: ui-monospace, monospace;
  color: var(--el-text-color-secondary);
}
.metric-value {
  font-weight: 600;
  color: var(--el-color-primary);
}
.no-data {
  color: var(--el-text-color-secondary);
  font-size: 0.9rem;
}
.mb-2 {
  margin-bottom: 1rem;
}
.mt-2 {
  margin-top: 0.75rem;
}
</style>
