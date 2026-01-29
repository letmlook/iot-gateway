<script setup>
import { ref, onMounted, onUnmounted, computed } from 'vue'
import { VideoPlay, VideoPause, Refresh } from '@element-plus/icons-vue'
import { api } from '../api.js'

const nodes = ref([])
const selectedNode = ref(null)
const tags = ref([])
const tagValues = ref({})
const loading = ref(false)
const error = ref('')
const autoRefresh = ref(false)
const refreshInterval = ref(2000)
let timer = null

async function loadNodes() {
  try {
    const all = await api.nodes()
    nodes.value = all.filter(n => n.kind === 'south')
    if (nodes.value.length && !selectedNode.value) {
      selectedNode.value = nodes.value[0].id
      await loadTags()
    }
  } catch (e) {
    error.value = '加载节点失败: ' + e.message
  }
}

async function loadTags() {
  if (!selectedNode.value) return
  loading.value = true
  try {
    tags.value = await api.tags(selectedNode.value)
    if (tags.value.length) await readValues()
  } catch (e) {
    error.value = '加载标签失败: ' + e.message
  } finally {
    loading.value = false
  }
}

async function readValues() {
  if (!selectedNode.value || !tags.value.length) return
  try {
    const ids = tags.value.map(t => t.id)
    const result = await api.readTags(selectedNode.value, ids)
    const newValues = {}
    result.forEach(([tid, val]) => { newValues[tid] = val })
    tagValues.value = newValues
  } catch (e) {
    console.error('读取标签失败', e)
  }
}

function onNodeChange() {
  tags.value = []
  tagValues.value = {}
  loadTags()
}

function toggleAutoRefresh() {
  autoRefresh.value = !autoRefresh.value
  if (autoRefresh.value) timer = setInterval(readValues, refreshInterval.value)
  else {
    clearInterval(timer)
    timer = null
  }
}

function formatValue(val) {
  if (val === undefined || val === null) return '-'
  if (typeof val === 'object') return JSON.stringify(val)
  if (typeof val === 'number') return Number.isInteger(val) ? val : val.toFixed(4)
  return String(val)
}

function getValueType(val) {
  if (val === undefined || val === null) return 'info'
  if (typeof val === 'boolean') return val ? 'success' : 'danger'
  return 'primary'
}

const selectedNodeInfo = computed(() => nodes.value.find(n => n.id === selectedNode.value))

onMounted(loadNodes)
onUnmounted(() => { if (timer) clearInterval(timer) })
</script>

<template>
  <div class="page-container">
    <div class="page-header">
      <div class="header-info">
        <p class="header-desc">实时监控南向设备数据，支持自动刷新。</p>
      </div>
    </div>

    <el-alert v-if="error" type="error" :title="error" closable show-icon @close="error = ''" class="mb-2" />

    <el-card shadow="never" class="monitor-controls">
      <div class="control-row">
        <el-form label-position="top" inline>
          <el-form-item label="选择设备">
            <el-select v-model="selectedNode" placeholder="请选择" style="width: 260px" @change="onNodeChange">
              <el-option v-for="n in nodes" :key="n.id" :label="`${n.name} (${n.plugin_name})`" :value="n.id" />
            </el-select>
          </el-form-item>
          <el-form-item label="刷新间隔">
            <el-select v-model="refreshInterval" style="width: 120px" :disabled="autoRefresh">
              <el-option :value="1000" label="1 秒" />
              <el-option :value="2000" label="2 秒" />
              <el-option :value="5000" label="5 秒" />
              <el-option :value="10000" label="10 秒" />
            </el-select>
          </el-form-item>
        </el-form>
        <div class="control-actions">
          <el-button
            :type="autoRefresh ? 'warning' : 'success'"
            :icon="autoRefresh ? VideoPause : VideoPlay"
            :disabled="!tags.length"
            @click="toggleAutoRefresh"
          >
            {{ autoRefresh ? '停止自动刷新' : '开始自动刷新' }}
          </el-button>
          <el-button :icon="Refresh" :disabled="loading || !tags.length" @click="readValues">手动刷新</el-button>
        </div>
      </div>
    </el-card>

    <div v-if="selectedNodeInfo" class="status-bar">
      <el-tag :type="selectedNodeInfo.state === 'running' ? 'success' : selectedNodeInfo.state === 'error' ? 'danger' : 'info'" size="small">
        {{ selectedNodeInfo.state === 'running' ? '运行中' : selectedNodeInfo.state === 'error' ? '错误' : '已停止' }}
      </el-tag>
      <span class="stat">标签总数：{{ tags.length }}</span>
      <el-tag v-if="autoRefresh" type="primary" size="small" effect="plain">自动刷新 {{ refreshInterval / 1000 }}s</el-tag>
    </div>

    <el-card v-if="tags.length" shadow="never" class="table-card">
      <el-table :data="tags" size="small" stripe>
        <el-table-column prop="name" label="标签名称" min-width="120" />
        <el-table-column prop="data_type" label="数据类型" width="100">
          <template #default="{ row }"><el-tag size="small" type="info">{{ row.data_type }}</el-tag></template>
        </el-table-column>
        <el-table-column prop="address" label="地址" width="140">
          <template #default="{ row }"><span class="font-mono">{{ row.address || '-' }}</span></template>
        </el-table-column>
        <el-table-column label="当前值" min-width="120">
          <template #default="{ row }">
            <el-tag :type="getValueType(tagValues[row.id])" size="small" effect="light">
              {{ formatValue(tagValues[row.id]) }}
            </el-tag>
          </template>
        </el-table-column>
      </el-table>
    </el-card>

    <el-skeleton v-else-if="loading" :rows="5" animated />

    <el-empty v-else description="暂无监控数据" class="empty-block">
      <template #description>
        <p v-if="!nodes.length">请先创建南向设备并添加标签。</p>
        <p v-else>请为选中的设备添加数据标签。</p>
      </template>
    </el-empty>
  </div>
</template>

<style scoped>
.mb-2 { margin-bottom: 1rem; }
.control-row { display: flex; flex-wrap: wrap; align-items: flex-end; gap: 1rem; }
.control-actions { margin-left: auto; display: flex; gap: 0.5rem; }
.status-bar { display: flex; align-items: center; gap: 1rem; margin: 1rem 0; }
.table-card { margin-top: 0.5rem; }
.font-mono { font-family: var(--font-mono); }
.empty-block { padding: 3rem; }
</style>
