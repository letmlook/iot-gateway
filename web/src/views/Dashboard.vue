<script setup>
import { ref, inject, onMounted, computed } from 'vue'
import { useRouter } from 'vue-router'
import { Connection, Upload, Cpu, Monitor, Setting, Document } from '@element-plus/icons-vue'
import { api } from '../api.js'

const router = useRouter()
const health = ref(null)
const version = ref(null)
const nodes = ref([])
const southPlugins = inject('southPlugins', ref([]))
const northPlugins = inject('northPlugins', ref([]))
const loading = ref(true)
const error = ref('')

const southNodes = computed(() => nodes.value.filter(n => n.kind === 'south'))
const northNodes = computed(() => nodes.value.filter(n => n.kind === 'north'))
const runningCount = computed(() => nodes.value.filter(n => n.state === 'running').length)

const quickLinks = [
  { path: '/south', icon: Connection, label: '南向设备', desc: '管理设备驱动', color: 'var(--el-color-success)' },
  { path: '/north', icon: Upload, label: '北向应用', desc: '数据上报应用', color: 'var(--el-color-primary)' },
  { path: '/monitor', icon: Monitor, label: '数据监控', desc: '实时数据与写值', color: 'var(--el-color-warning)' },
  { path: '/plugins', icon: Cpu, label: '插件管理', desc: '驱动与 Schema', color: 'var(--el-color-info)' },
  { path: '/system', icon: Setting, label: '系统管理', desc: '版本与导出', color: 'var(--el-color-info)' },
]

async function loadData() {
  loading.value = true
  error.value = ''
  try {
    const [h, v, nd] = await Promise.all([
      api.health().catch(() => null),
      api.version().catch(() => null),
      api.nodes().catch(() => []),
    ])
    health.value = h
    version.value = v
    nodes.value = nd
  } catch (e) {
    error.value = '加载概览失败: ' + e.message
  } finally {
    loading.value = false
  }
}

function go(path) {
  router.push(path)
}

function goNode(node) {
  router.push(node.kind === 'south' ? `/south/${node.id}` : `/north/${node.id}`)
}

function getStateType(state) {
  if (state === 'running') return 'success'
  if (state === 'error') return 'danger'
  return 'info'
}

onMounted(loadData)
</script>

<template>
  <div class="page-container dashboard">
    <div class="page-header">
      <div class="header-info">
        <h2 class="dashboard-title">概览</h2>
        <p class="header-desc">网关运行状态与快捷入口，对标 Neuron Dashboard 首页。</p>
      </div>
    </div>

    <el-alert v-if="error" type="error" :title="error" closable show-icon @close="error = ''" class="mb-2" />

    <el-skeleton v-if="loading" :rows="8" animated />

    <template v-else>
      <div class="stats-row">
        <el-card shadow="hover" class="stat-card total" @click="go('/south')">
          <div class="stat-icon">
            <Connection />
          </div>
          <div class="stat-content">
            <span class="stat-value">{{ health?.nodes_count ?? 0 }}</span>
            <span class="stat-label">节点总数</span>
          </div>
        </el-card>
        <el-card shadow="hover" class="stat-card running" @click="go('/south')">
          <div class="stat-icon">
            <Monitor />
          </div>
          <div class="stat-content">
            <span class="stat-value">{{ health?.nodes_running ?? 0 }}</span>
            <span class="stat-label">运行中</span>
          </div>
        </el-card>
        <el-card shadow="hover" class="stat-card south">
          <div class="stat-icon">
            <Connection />
          </div>
          <div class="stat-content">
            <span class="stat-value">{{ health?.plugins_south ?? 0 }}</span>
            <span class="stat-label">南向插件</span>
          </div>
        </el-card>
        <el-card shadow="hover" class="stat-card north" @click="go('/plugins')">
          <div class="stat-icon">
            <Upload />
          </div>
          <div class="stat-content">
            <span class="stat-value">{{ health?.plugins_north ?? 0 }}</span>
            <span class="stat-label">北向插件</span>
          </div>
        </el-card>
      </div>

      <div class="section">
        <h3 class="section-title">快捷入口</h3>
        <div class="quick-grid">
          <el-card
            v-for="link in quickLinks"
            :key="link.path"
            shadow="hover"
            class="quick-card"
            @click="go(link.path)"
          >
            <div class="quick-icon" :style="{ color: link.color }">
              <component :is="link.icon" />
            </div>
            <div class="quick-info">
              <span class="quick-label">{{ link.label }}</span>
              <span class="quick-desc">{{ link.desc }}</span>
            </div>
          </el-card>
        </div>
      </div>

      <div class="section two-col">
        <el-card shadow="hover" class="list-card">
          <template #header>
            <span>南向设备</span>
            <el-button type="primary" link size="small" @click="go('/south')">查看全部</el-button>
          </template>
          <div v-if="southNodes.length" class="node-list">
            <div
              v-for="n in southNodes.slice(0, 5)"
              :key="n.id"
              class="node-item"
              @click="goNode(n)"
            >
              <span class="node-name">{{ n.name }}</span>
              <el-tag :type="getStateType(n.state)" size="small">{{ n.state === 'running' ? '运行中' : '已停止' }}</el-tag>
            </div>
          </div>
          <el-empty v-else description="暂无南向设备" :image-size="60" />
        </el-card>
        <el-card shadow="hover" class="list-card">
          <template #header>
            <span>北向应用</span>
            <el-button type="primary" link size="small" @click="go('/north')">查看全部</el-button>
          </template>
          <div v-if="northNodes.length" class="node-list">
            <div
              v-for="n in northNodes.slice(0, 5)"
              :key="n.id"
              class="node-item"
              @click="goNode(n)"
            >
              <span class="node-name">{{ n.name }}</span>
              <el-tag :type="getStateType(n.state)" size="small">{{ n.state === 'running' ? '运行中' : '已停止' }}</el-tag>
            </div>
          </div>
          <el-empty v-else description="暂无北向应用" :image-size="60" />
        </el-card>
      </div>

      <div v-if="version" class="footer-info">
        <span>网关版本 v{{ version.version }}</span>
        <span v-if="version.revision" class="revision">{{ version.revision.slice(0, 8) }}</span>
      </div>
    </template>
  </div>
</template>

<style scoped>
.mb-2 { margin-bottom: 1rem; }
.dashboard-title { font-size: 1.35rem; margin-bottom: 0.25rem; }
.stats-row { display: grid; grid-template-columns: repeat(4, 1fr); gap: 1rem; margin-bottom: 1.5rem; }
@media (max-width: 900px) { .stats-row { grid-template-columns: repeat(2, 1fr); } }
.stat-card { cursor: pointer; transition: transform 0.2s; display: flex; align-items: center; gap: 1rem; padding: 1rem; }
.stat-card:hover { transform: translateY(-2px); }
.stat-icon { width: 48px; height: 48px; border-radius: 12px; display: flex; align-items: center; justify-content: center; font-size: 1.5rem; }
.stat-card.total .stat-icon { background: rgba(13, 148, 136, 0.12); color: var(--el-color-primary); }
.stat-card.running .stat-icon { background: rgba(5, 150, 105, 0.12); color: var(--el-color-success); }
.stat-card.south .stat-icon { background: rgba(217, 119, 6, 0.12); color: var(--el-color-warning); }
.stat-card.north .stat-icon { background: rgba(124, 58, 237, 0.12); color: #7c3aed; }
.stat-content { display: flex; flex-direction: column; gap: 0.25rem; }
.stat-value { font-size: 1.5rem; font-weight: 700; color: var(--text-primary); }
.stat-label { font-size: 0.8rem; color: var(--text-muted); }
.section { margin-bottom: 1.5rem; }
.section-title { font-size: 1rem; margin-bottom: 0.75rem; color: var(--text-secondary); }
.quick-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(160px, 1fr)); gap: 1rem; }
.quick-card { cursor: pointer; transition: transform 0.2s; padding: 1rem; display: flex; align-items: center; gap: 0.75rem; }
.quick-card:hover { transform: translateY(-2px); }
.quick-icon { width: 40px; height: 40px; border-radius: 10px; display: flex; align-items: center; justify-content: center; font-size: 1.25rem; background: var(--el-fill-color-light); }
.quick-info { display: flex; flex-direction: column; gap: 0.2rem; }
.quick-label { font-weight: 600; font-size: 0.95rem; }
.quick-desc { font-size: 0.8rem; color: var(--text-muted); }
.two-col { display: grid; grid-template-columns: 1fr 1fr; gap: 1rem; }
@media (max-width: 768px) { .two-col { grid-template-columns: 1fr; } }
.list-card :deep(.el-card__header) { display: flex; justify-content: space-between; align-items: center; }
.node-list { display: flex; flex-direction: column; gap: 0.5rem; }
.node-item { display: flex; justify-content: space-between; align-items: center; padding: 0.5rem 0.75rem; border-radius: 8px; cursor: pointer; transition: background 0.2s; }
.node-item:hover { background: var(--el-fill-color-light); }
.node-name { font-size: 0.9rem; font-weight: 500; }
.footer-info { font-size: 0.85rem; color: var(--text-muted); margin-top: 1rem; display: flex; align-items: center; gap: 0.5rem; }
.revision { font-family: var(--font-mono); font-size: 0.8rem; }
</style>
