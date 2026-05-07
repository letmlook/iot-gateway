<script setup>
import { ref, inject, onMounted, computed } from 'vue'
import { useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { getErrorMessage } from '../i18n'
import { Connection, Upload, Cpu, Monitor, Setting, TrendCharts, Warning } from '@element-plus/icons-vue'
import { api } from '../api.js'
import PageHeader from '../components/PageHeader.vue'
import StatusIndicator from '../components/StatusIndicator.vue'

const router = useRouter()
const { t } = useI18n()
const health = ref(null)
const nodes = ref([])
const southPlugins = inject('southPlugins', ref([]))
const northPlugins = inject('northPlugins', ref([]))
const loading = ref(true)
const error = ref('')

const southNodes = computed(() => nodes.value.filter(n => n.kind === 'south'))
const northNodes = computed(() => nodes.value.filter(n => n.kind === 'north'))
const runningCount = computed(() => nodes.value.filter(n => n.state === 'running').length)
const errorCount = computed(() => nodes.value.filter(n => n.state === 'error').length)

// 快捷入口
const quickActions = [
  { action: 'add-south', icon: Connection, labelKey: 'dashboard.addSouthDevice', color: 'success' },
  { action: 'add-north', icon: Upload, labelKey: 'dashboard.addNorthApp', color: 'purple' },
  { action: 'monitor', icon: Monitor, labelKey: 'dashboard.openMonitor', color: 'accent' },
]

async function loadData() {
  loading.value = true
  error.value = ''
  try {
    const [h, nd] = await Promise.all([
      api.health().catch(() => null),
      api.nodes().catch(() => []),
    ])
    health.value = h
    nodes.value = nd
  } catch (e) {
    error.value = t('dashboard.loadOverviewFailed') + getErrorMessage(t, e)
  } finally {
    loading.value = false
  }
}

function go(path) {
  router.push(path)
}

function handleQuickAction(action) {
  switch (action) {
    case 'add-south':
      router.push('/south/new')
      break
    case 'add-north':
      router.push('/north/new')
      break
    case 'monitor':
      router.push('/monitor')
      break
  }
}

function goNode(node) {
  router.push(node.kind === 'south' ? `/south/${node.id}` : `/north/${node.id}`)
}

function getStateClass(state) {
  if (state === 'running') return 'running'
  if (state === 'error') return 'error'
  return 'stopped'
}

onMounted(loadData)
</script>

<template>
  <div class="dashboard-page">
    <PageHeader :title="t('dashboard.title')" :subtitle="t('dashboard.desc')">
      <template #actions>
        <el-button type="primary" @click="loadData" :loading="loading">
          {{ t('common.refresh') }}
        </el-button>
      </template>
    </PageHeader>

    <el-alert v-if="error" type="error" :title="error" closable show-icon @close="error = ''" class="mb-4" />

    <el-skeleton v-if="loading" :rows="8" animated />

    <template v-else>
      <!-- 统计卡片 -->
      <div class="stats-grid">
        <div class="stat-card" @click="go('/south')">
          <div class="stat-icon total">
            <el-icon :size="24"><Connection /></el-icon>
          </div>
          <div class="stat-content">
            <span class="stat-value">{{ health?.nodes_count ?? nodes.length }}</span>
            <span class="stat-label">{{ t('header.nodesTotal') }}</span>
          </div>
          <div class="stat-trend up">
            <span class="trend-arrow">↗</span>
          </div>
        </div>

        <div class="stat-card highlight" @click="go('/south')">
          <div class="stat-icon running">
            <el-icon :size="24"><TrendCharts /></el-icon>
          </div>
          <div class="stat-content">
            <span class="stat-value">{{ health?.nodes_running ?? runningCount }}</span>
            <span class="stat-label">{{ t('header.runningCount') }}</span>
          </div>
          <div class="stat-ring">
            <svg viewBox="0 0 36 36" class="circular-chart">
              <path class="circle-bg" d="M18 2.0845 a 15.9155 15.9155 0 0 1 0 31.831 a 15.9155 15.9155 0 0 1 0 -31.831" />
              <path class="circle" 
                :stroke-dasharray="`${nodes.length ? (runningCount / nodes.length * 100) : 0}, 100`"
                d="M18 2.0845 a 15.9155 15.9155 0 0 1 0 31.831 a 15.9155 15.9155 0 0 1 0 -31.831" />
            </svg>
          </div>
        </div>

        <div class="stat-card" @click="go('/plugins')">
          <div class="stat-icon south">
            <el-icon :size="24"><Cpu /></el-icon>
          </div>
          <div class="stat-content">
            <span class="stat-value">{{ health?.plugins_south ?? southPlugins.length }}</span>
            <span class="stat-label">{{ t('header.southPlugins') }}</span>
          </div>
        </div>

        <div class="stat-card" @click="go('/plugins')">
          <div class="stat-icon north">
            <el-icon :size="24"><Upload /></el-icon>
          </div>
          <div class="stat-content">
            <span class="stat-value">{{ health?.plugins_north ?? northPlugins.length }}</span>
            <span class="stat-label">{{ t('header.northPlugins') }}</span>
          </div>
        </div>
      </div>

      <!-- 快捷操作 -->
      <div class="section quick-actions-section">
        <h2 class="section-title">{{ t('dashboard.quickActions') }}</h2>
        <div class="quick-actions-grid">
          <button 
            v-for="action in quickActions" 
            :key="action.action"
            class="quick-action-btn"
            :class="action.color"
            @click="handleQuickAction(action.action)"
          >
            <el-icon :size="20"><component :is="action.icon" /></el-icon>
            <span>{{ t(action.labelKey) }}</span>
          </button>
        </div>
      </div>

      <!-- 设备概览 -->
      <div class="section devices-section">
        <div class="section-header">
          <h2 class="section-title">{{ t('dashboard.devicesOverview') }}</h2>
          <div class="section-tabs">
            <el-button-group>
              <el-button size="small" type="primary" plain @click="go('/south')">
                {{ t('dashboard.southDevices') }} ({{ southNodes.length }})
              </el-button>
              <el-button size="small" type="primary" plain @click="go('/north')">
                {{ t('dashboard.northApps') }} ({{ northNodes.length }})
              </el-button>
            </el-button-group>
          </div>
        </div>

        <div class="devices-grid">
          <!-- 南向设备列表 -->
          <div class="device-list-card">
            <div class="list-header">
              <span class="list-title">
                <span class="list-dot south"></span>
                {{ t('dashboard.southDevices') }}
              </span>
              <el-button type="primary" link size="small" @click="go('/south')">
                {{ t('common.viewAll') }} →
              </el-button>
            </div>
            <div v-if="southNodes.length" class="node-list">
              <div
                v-for="n in southNodes.slice(0, 5)"
                :key="n.id"
                class="node-item"
                @click="goNode(n)"
              >
                <div class="node-info">
                  <span class="node-name">{{ n.name }}</span>
                  <span class="node-plugin">{{ n.plugin_name }}</span>
                </div>
                <StatusIndicator :status="n.state" size="small" />
              </div>
            </div>
            <el-empty v-else :description="t('south.noNodes')" :image-size="48">
              <el-button type="primary" size="small" @click="go('/south/new')">
                {{ t('south.addDevice') }}
              </el-button>
            </el-empty>
          </div>

          <!-- 北向应用列表 -->
          <div class="device-list-card">
            <div class="list-header">
              <span class="list-title">
                <span class="list-dot north"></span>
                {{ t('dashboard.northApps') }}
              </span>
              <el-button type="primary" link size="small" @click="go('/north')">
                {{ t('common.viewAll') }} →
              </el-button>
            </div>
            <div v-if="northNodes.length" class="node-list">
              <div
                v-for="n in northNodes.slice(0, 5)"
                :key="n.id"
                class="node-item"
                @click="goNode(n)"
              >
                <div class="node-info">
                  <span class="node-name">{{ n.name }}</span>
                  <span class="node-plugin">{{ n.plugin_name }}</span>
                </div>
                <StatusIndicator :status="n.state" size="small" />
              </div>
            </div>
            <el-empty v-else :description="t('north.noNodes')" :image-size="48">
              <el-button type="primary" size="small" @click="go('/north/new')">
                {{ t('north.addApp') }}
              </el-button>
            </el-empty>
          </div>
        </div>
      </div>

      <!-- 系统状态 -->
      <div class="section system-section">
        <h2 class="section-title">{{ t('dashboard.systemStatus') }}</h2>
        <div class="system-cards">
          <div class="system-card">
            <div class="system-icon" :class="{ online: health?.status === 'ok' }">
              <el-icon :size="20"><Setting /></el-icon>
            </div>
            <div class="system-info">
              <span class="system-label">{{ t('dashboard.coreService') }}</span>
              <span class="system-value" :class="{ online: health?.status === 'ok' }">
                {{ health?.status === 'ok' ? t('common.normal') : t('common.abnormal') }}
              </span>
            </div>
          </div>

          <div class="system-card" v-if="errorCount > 0">
            <div class="system-icon error">
              <el-icon :size="20"><Warning /></el-icon>
            </div>
            <div class="system-info">
              <span class="system-label">{{ t('dashboard.errorDevices') }}</span>
              <span class="system-value error">{{ errorCount }} {{ t('common.items') }}</span>
            </div>
          </div>
        </div>
      </div>
    </template>
  </div>
</template>

<style scoped>
.dashboard-page {
  max-width: 1200px;
}

.mb-4 { margin-bottom: 1.5rem; }

/* 统计卡片网格 */
.stats-grid {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 1rem;
  margin-bottom: 1.5rem;
}

@media (max-width: 1024px) {
  .stats-grid { grid-template-columns: repeat(2, 1fr); }
}

@media (max-width: 640px) {
  .stats-grid { grid-template-columns: 1fr; }
}

.stat-card {
  background: var(--bg-surface);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-lg);
  padding: 1.25rem;
  display: flex;
  align-items: center;
  gap: 1rem;
  cursor: pointer;
  transition: all var(--transition-fast);
  position: relative;
  overflow: hidden;
}

.stat-card:hover {
  border-color: var(--border-default);
  box-shadow: var(--shadow-sm);
  transform: translateY(-2px);
}

.stat-card.highlight {
  background: linear-gradient(135deg, var(--bg-surface) 0%, rgba(56, 161, 105, 0.05) 100%);
  border-color: rgba(56, 161, 105, 0.2);
}

.stat-icon {
  width: 48px;
  height: 48px;
  border-radius: var(--radius-md);
  display: flex;
  align-items: center;
  justify-content: center;
  flex-shrink: 0;
}

.stat-icon.total {
  background: var(--primary-glow);
  color: var(--primary);
}

.stat-icon.running {
  background: rgba(56, 161, 105, 0.12);
  color: var(--success);
}

.stat-icon.south {
  background: var(--accent-glow);
  color: var(--accent);
}

.stat-icon.north {
  background: rgba(128, 90, 213, 0.12);
  color: var(--accent-purple);
}

.stat-content {
  flex: 1;
  min-width: 0;
}

.stat-value {
  display: block;
  font-size: 1.75rem;
  font-weight: 700;
  color: var(--text-primary);
  font-family: var(--font-mono);
  line-height: 1.2;
}

.stat-label {
  font-size: 0.8rem;
  color: var(--text-muted);
}

.stat-trend {
  font-size: 0.75rem;
  padding: 0.2rem 0.4rem;
  border-radius: var(--radius-xs);
}

.stat-trend.up {
  background: rgba(56, 161, 105, 0.1);
  color: var(--success);
}

/* 环形图 */
.stat-ring {
  width: 40px;
  height: 40px;
  flex-shrink: 0;
}

.circular-chart {
  width: 100%;
  height: 100%;
}

.circle-bg {
  fill: none;
  stroke: var(--border-subtle);
  stroke-width: 3;
}

.circle {
  fill: none;
  stroke: var(--success);
  stroke-width: 3;
  stroke-linecap: round;
  transform: rotate(-90deg);
  transform-origin: center;
  transition: stroke-dasharray 0.5s ease;
}

/* Section 通用样式 */
.section {
  margin-bottom: 1.5rem;
}

.section-title {
  font-size: 1rem;
  font-weight: 600;
  color: var(--text-primary);
  margin: 0 0 1rem;
}

.section-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 1rem;
  flex-wrap: wrap;
  gap: 0.5rem;
}

/* 快捷操作 */
.quick-actions-grid {
  display: flex;
  gap: 0.75rem;
  flex-wrap: wrap;
}

.quick-action-btn {
  display: inline-flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.65rem 1rem;
  border: 1px solid var(--border-default);
  border-radius: var(--radius-md);
  background: var(--bg-surface);
  color: var(--text-secondary);
  font-size: 0.875rem;
  font-weight: 500;
  cursor: pointer;
  transition: all var(--transition-fast);
}

.quick-action-btn:hover {
  border-color: var(--accent);
  color: var(--accent);
  background: var(--accent-glow);
}

.quick-action-btn.success:hover {
  border-color: var(--success);
  color: var(--success);
  background: rgba(56, 161, 105, 0.08);
}

.quick-action-btn.purple:hover {
  border-color: var(--accent-purple);
  color: var(--accent-purple);
  background: rgba(128, 90, 213, 0.08);
}

/* 设备列表 */
.devices-grid {
  display: grid;
  grid-template-columns: repeat(2, 1fr);
  gap: 1rem;
}

@media (max-width: 768px) {
  .devices-grid { grid-template-columns: 1fr; }
}

.device-list-card {
  background: var(--bg-surface);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-lg);
  overflow: hidden;
}

.list-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 0.85rem 1rem;
  background: var(--bg-elevated);
  border-bottom: 1px solid var(--border-subtle);
}

.list-title {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  font-weight: 600;
  font-size: 0.9rem;
  color: var(--text-primary);
}

.list-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
}

.list-dot.south {
  background: var(--success);
}

.list-dot.north {
  background: var(--accent-purple);
}

.node-list {
  padding: 0.5rem;
}

.node-item {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 0.65rem 0.75rem;
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: background var(--transition-fast);
}

.node-item:hover {
  background: var(--bg-hover);
}

.node-info {
  display: flex;
  flex-direction: column;
  gap: 0.15rem;
  min-width: 0;
}

.node-name {
  font-size: 0.9rem;
  font-weight: 500;
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.node-plugin {
  font-size: 0.75rem;
  color: var(--text-muted);
  font-family: var(--font-mono);
}

/* 系统状态 */
.system-cards {
  display: flex;
  gap: 1rem;
  flex-wrap: wrap;
}

.system-card {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  padding: 0.85rem 1.25rem;
  background: var(--bg-surface);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-md);
}

.system-icon {
  width: 36px;
  height: 36px;
  border-radius: var(--radius-sm);
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--bg-inset);
  color: var(--text-muted);
}

.system-icon.online {
  background: rgba(56, 161, 105, 0.12);
  color: var(--success);
}

.system-icon.error {
  background: rgba(229, 62, 62, 0.1);
  color: var(--danger);
}

.system-info {
  display: flex;
  flex-direction: column;
  gap: 0.1rem;
}

.system-label {
  font-size: 0.75rem;
  color: var(--text-muted);
}

.system-value {
  font-size: 0.9rem;
  font-weight: 500;
  color: var(--text-secondary);
}

.system-value.online {
  color: var(--success);
}

.system-value.error {
  color: var(--danger);
}
</style>
