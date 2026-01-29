<script setup>
import { ref, onMounted } from 'vue'
import { ElMessage } from 'element-plus'
import { Download, CopyDocument, Link } from '@element-plus/icons-vue'
import { api } from '../api.js'

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
    error.value = '加载系统信息失败: ' + e.message
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
    error.value = '导出配置失败: ' + e.message
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
    ElMessage.success('已复制到剪贴板')
  } catch (e) {
    ElMessage.error('复制失败')
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
  <div class="page-container">
    <div class="page-header">
      <div class="header-info">
        <p class="header-desc">查看系统信息、版本、监控指标，导出配置。</p>
      </div>
      <el-button type="primary" :icon="Download" @click="exportConfig">导出配置</el-button>
    </div>

    <el-alert v-if="error" type="error" :title="error" closable show-icon @close="error = ''" class="mb-2" />

    <el-skeleton v-if="loading" :rows="8" animated />

    <div v-else class="system-grid">
      <el-card shadow="hover" class="system-card">
        <template #header><span>版本信息</span></template>
        <div v-if="version" class="info-grid">
          <div class="info-item">
            <span class="info-label">版本号</span>
            <span class="info-value highlight">v{{ version.version }}</span>
          </div>
          <div v-if="version.build_date" class="info-item">
            <span class="info-label">构建日期</span>
            <span class="info-value">{{ version.build_date }}</span>
          </div>
          <div v-if="version.revision" class="info-item">
            <span class="info-label">Git 版本</span>
            <span class="info-value font-mono">{{ version.revision.slice(0, 8) }}</span>
          </div>
        </div>
        <div v-else class="no-data">无法获取版本信息</div>
      </el-card>

      <el-card shadow="hover" class="system-card">
        <template #header><span>运行状态</span></template>
        <div v-if="health" class="info-grid">
          <div class="info-item">
            <span class="info-label">系统状态</span>
            <el-tag :type="health.status === 'ok' ? 'success' : 'danger'" size="small">
              {{ health.status === 'ok' ? '正常运行' : '异常' }}
            </el-tag>
          </div>
          <div class="info-item">
            <span class="info-label">节点总数</span>
            <span class="info-value">{{ health.nodes_count || 0 }}</span>
          </div>
          <div class="info-item">
            <span class="info-label">运行中</span>
            <span class="info-value highlight">{{ health.nodes_running || 0 }}</span>
          </div>
          <div class="info-item">
            <span class="info-label">南向插件</span>
            <span class="info-value">{{ health.plugins_south || 0 }}</span>
          </div>
          <div class="info-item">
            <span class="info-label">北向插件</span>
            <span class="info-value">{{ health.plugins_north || 0 }}</span>
          </div>
        </div>
        <div v-else class="no-data">无法获取运行状态</div>
      </el-card>

      <el-card shadow="hover" class="system-card metrics-card">
        <template #header><span>Prometheus 指标</span></template>
        <div v-if="parseMetrics(metrics).length" class="metrics-list">
          <div class="metric-item" v-for="m in parseMetrics(metrics)" :key="m.name">
            <span class="metric-name">{{ m.name }}</span>
            <span class="metric-value">{{ m.value }}</span>
          </div>
        </div>
        <div v-else class="no-data">暂无监控指标</div>
        <el-link href="/api/metrics" target="_blank" type="primary" :icon="Link" class="mt-2">查看原始数据</el-link>
      </el-card>

      <el-card shadow="hover" class="system-card api-card">
        <template #header><span>API 端点</span></template>
        <div class="api-list">
          <div class="api-item" v-for="path in ['/api/health', '/api/version', '/api/metrics', '/api/export', '/api/nodes', '/api/plugins/south', '/api/plugins/north']" :key="path">
            <el-tag size="small" type="success">GET</el-tag>
            <code class="api-path">{{ path }}</code>
          </div>
        </div>
      </el-card>
    </div>

    <el-dialog v-model="showExportModal" title="导出配置" width="640px" destroy-on-close>
      <el-input :model-value="exportData" type="textarea" :rows="18" readonly class="font-mono export-textarea" />
      <template #footer>
        <el-button :icon="CopyDocument" @click="copyExport">复制</el-button>
        <el-button type="primary" :icon="Download" @click="downloadExport">下载 JSON</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<style scoped>
.mb-2 { margin-bottom: 1rem; }
.mt-2 { margin-top: 0.5rem; }
.system-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(300px, 1fr)); gap: 1rem; }
.info-grid { display: grid; gap: 0.75rem; }
.info-item { display: flex; justify-content: space-between; align-items: center; }
.info-label { font-size: 0.9rem; color: var(--text-muted); }
.info-value.highlight { color: var(--el-color-primary); font-weight: 500; }
.no-data { color: var(--text-muted); font-size: 0.9rem; }
.metrics-list { display: grid; grid-template-columns: repeat(auto-fill, minmax(180px, 1fr)); gap: 0.5rem; }
.metric-item { display: flex; justify-content: space-between; padding: 0.4rem 0.6rem; background: var(--el-fill-color-light); border-radius: 6px; font-size: 0.85rem; }
.metric-name { font-family: var(--font-mono); color: var(--text-secondary); }
.metric-value { font-weight: 600; color: var(--el-color-primary); }
.api-list { display: grid; gap: 0.4rem; }
.api-item { display: flex; align-items: center; gap: 0.5rem; }
.api-path { font-size: 0.85rem; color: var(--text-secondary); }
.metrics-card { grid-column: span 2; }
.api-card { grid-column: span 2; }
.font-mono { font-family: var(--font-mono); }
@media (max-width: 900px) { .metrics-card, .api-card { grid-column: span 1; } }
</style>
