<script setup>
import { ref, onMounted, onUnmounted, computed, watch } from 'vue'
import { useRoute } from 'vue-router'
import { ElMessage } from 'element-plus'
import { VideoPlay, VideoPause, Refresh, EditPen } from '@element-plus/icons-vue'
import { api } from '../api.js'

const route = useRoute()
const nodes = ref([])
const groups = ref([])
const selectedNode = ref(null)
const selectedGroup = ref('')
const tags = ref([])
const allTags = ref([])
const tagValues = ref({})
const loading = ref(false)
const error = ref('')
const autoRefresh = ref(false)
const refreshInterval = ref(2000)
const writingTagId = ref(null)
const writeInputs = ref({})
const keywordSearch = ref('')
const onlyShowErrors = ref(false)
const lastUpdateTime = ref(null)
let timer = null

async function loadNodes() {
  try {
    const all = await api.nodes()
    nodes.value = all.filter(n => n.kind === 'south')
    const nodeId = route.query.nodeId
    if (nodes.value.length) {
      if (nodeId && nodes.value.some(n => n.id === nodeId)) {
        selectedNode.value = nodeId
      } else if (!selectedNode.value) {
        selectedNode.value = nodes.value[0].id
      }
      await loadGroups()
      await loadTags()
    }
  } catch (e) {
    error.value = '加载节点失败: ' + e.message
  }
}

async function loadGroups() {
  if (!selectedNode.value) {
    groups.value = []
    return
  }
  try {
    groups.value = await api.groups(selectedNode.value)
    if (groups.value.length && !selectedGroup.value) selectedGroup.value = ''
  } catch {
    groups.value = []
  }
}

async function loadTags() {
  if (!selectedNode.value) return
  loading.value = true
  try {
    allTags.value = await api.tags(selectedNode.value)
    tags.value = allTags.value
    if (selectedGroup.value) {
      tags.value = tags.value.filter(t => t.group_id === selectedGroup.value)
    }
    if (keywordSearch.value.trim()) {
      const k = keywordSearch.value.trim().toLowerCase()
      tags.value = tags.value.filter(t =>
        (t.name && t.name.toLowerCase().includes(k)) ||
        (t.address && t.address.toLowerCase().includes(k))
      )
    }
    if (onlyShowErrors.value) {
      tags.value = tags.value.filter(t => isValueError(tagValues.value[t.id]))
    }
    if (tags.value.length) await readValues()
  } catch (e) {
    error.value = '加载标签失败: ' + e.message
  } finally {
    loading.value = false
  }
}

function applyFilters() {
  let list = [...allTags.value]
  if (selectedGroup.value) {
    list = list.filter(t => t.group_id === selectedGroup.value)
  }
  if (keywordSearch.value.trim()) {
    const k = keywordSearch.value.trim().toLowerCase()
    list = list.filter(t =>
      (t.name && t.name.toLowerCase().includes(k)) ||
      (t.address && t.address.toLowerCase().includes(k))
    )
  }
  if (onlyShowErrors.value) {
    list = list.filter(t => isValueError(tagValues.value[t.id]))
  }
  tags.value = list
}

function isValueError(val) {
  if (val === undefined || val === null) return false
  if (typeof val === 'object' && (val.error || val.Error || val.message)) return true
  if (typeof val === 'string' && val.startsWith('Error')) return true
  return false
}

watch([selectedGroup, keywordSearch, onlyShowErrors], () => applyFilters())
watch(tagValues, () => { if (onlyShowErrors.value) applyFilters() }, { deep: true })

async function readValues() {
  if (!selectedNode.value || !tags.value.length) return
  try {
    const ids = tags.value.map(t => t.id)
    const result = await api.readTags(selectedNode.value, ids)
    const newValues = { ...tagValues.value }
    result.forEach(([tid, val]) => { newValues[tid] = val })
    tagValues.value = newValues
    lastUpdateTime.value = new Date()
  } catch (e) {
    console.error('读取标签失败', e)
  }
}

function onNodeChange() {
  selectedGroup.value = ''
  tags.value = []
  allTags.value = []
  tagValues.value = {}
  writeInputs.value = {}
  loadGroups()
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
  if (typeof val === 'object') {
    if (val.error) return val.error
    if (val.Error) return val.Error
    if (val.message) return val.message
    return JSON.stringify(val)
  }
  if (typeof val === 'number') return Number.isInteger(val) ? val : val.toFixed(4)
  return String(val)
}

function getValueType(val) {
  if (isValueError(val)) return 'danger'
  if (val === undefined || val === null) return 'info'
  if (typeof val === 'boolean') return val ? 'success' : 'danger'
  return 'primary'
}

// 将用户输入按 data_type 转为 DataValue JSON
function parseDataValue(dataType, raw) {
  if (raw === '' || raw === undefined || raw === null) return null
  const s = String(raw).trim()
  const t = (dataType || 'Float64').toLowerCase()
  try {
    if (t === 'bool' || t === 'boolean') {
      const v = s.toLowerCase()
      if (v === 'true' || v === '1') return { type: 'Bool', value: true }
      if (v === 'false' || v === '0') return { type: 'Bool', value: false }
      return null
    }
    if (t === 'int8') return { type: 'Int8', value: parseInt(s, 10) }
    if (t === 'int16') return { type: 'Int16', value: parseInt(s, 10) }
    if (t === 'int32') return { type: 'Int32', value: parseInt(s, 10) }
    if (t === 'int64') return { type: 'Int64', value: parseInt(s, 10) }
    if (t === 'uint8') return { type: 'UInt8', value: parseInt(s, 10) >>> 0 }
    if (t === 'uint16') return { type: 'UInt16', value: parseInt(s, 10) >>> 0 }
    if (t === 'uint32') return { type: 'UInt32', value: parseInt(s, 10) >>> 0 }
    if (t === 'uint64') return { type: 'UInt64', value: parseInt(s, 10) }
    if (t === 'float32') return { type: 'Float32', value: parseFloat(s) }
    if (t === 'float64' || t === 'float') return { type: 'Float64', value: parseFloat(s) }
    if (t === 'string') return { type: 'String', value: s }
    return { type: 'Float64', value: parseFloat(s) }
  } catch {
    return null
  }
}

async function writeTag(tag) {
  const raw = writeInputs.value[tag.id]
  const dv = parseDataValue(tag.data_type, raw)
  if (dv === null) {
    ElMessage.warning('请输入合法值（与数据类型一致）')
    return
  }
  writingTagId.value = tag.id
  try {
    await api.writeTags(selectedNode.value, [[tag.id, dv]])
    ElMessage.success('写入成功')
    writeInputs.value[tag.id] = ''
    await readValues()
  } catch (e) {
    ElMessage.error('写入失败: ' + e.message)
  } finally {
    writingTagId.value = null
  }
}

function initWriteInput(tag) {
  if (writeInputs.value[tag.id] === undefined) {
    const v = tagValues.value[tag.id]
    if (v !== undefined && v !== null) writeInputs.value[tag.id] = String(v)
    else writeInputs.value[tag.id] = ''
  }
}

const selectedNodeInfo = computed(() => nodes.value.find(n => n.id === selectedNode.value))

onMounted(loadNodes)
onUnmounted(() => { if (timer) clearInterval(timer) })
</script>

<template>
  <div class="page-container">
    <div class="page-header monitor-header">
      <h2 class="page-title">数据监控</h2>
      <span v-if="lastUpdateTime" class="update-time">更新时间 {{ lastUpdateTime.toLocaleString('zh-CN') }}</span>
    </div>

    <el-alert v-if="error" type="error" :title="error" closable show-icon @close="error = ''" class="mb-2" />

    <el-card shadow="never" class="monitor-controls">
      <div class="control-row">
        <el-form label-position="top" inline>
          <el-form-item label="驱动实例">
            <el-select v-model="selectedNode" placeholder="请选择" style="width: 200px" @change="onNodeChange">
              <el-option v-for="n in nodes" :key="n.id" :label="n.name" :value="n.id" />
            </el-select>
          </el-form-item>
          <el-form-item label="组">
            <el-select v-model="selectedGroup" placeholder="全部" clearable style="width: 140px" @change="applyFilters">
              <el-option v-for="g in groups" :key="g.id" :label="g.name" :value="g.id" />
            </el-select>
          </el-form-item>
          <el-form-item label="搜索">
            <el-input v-model="keywordSearch" placeholder="输入关键字搜索" clearable style="width: 160px" />
          </el-form-item>
          <el-form-item label="">
            <el-checkbox v-model="onlyShowErrors">仅展示错误点位</el-checkbox>
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
      <span class="stat">当前 {{ tags.length }} 条</span>
      <el-tag v-if="autoRefresh" type="primary" size="small" effect="plain">自动刷新 {{ refreshInterval / 1000 }}s</el-tag>
    </div>

    <el-card v-if="tags.length" shadow="never" class="table-card">
      <el-table :data="tags" size="small" stripe>
        <el-table-column prop="name" label="名称" min-width="120" />
        <el-table-column prop="address" label="地址" width="140">
          <template #default="{ row }"><span class="font-mono">{{ row.address || '-' }}</span></template>
        </el-table-column>
        <el-table-column prop="data_type" label="类型" width="90">
          <template #default="{ row }"><el-tag size="small" type="info">{{ row.data_type || '-' }}</el-tag></template>
        </el-table-column>
        <el-table-column label="乘系数" width="80">
          <template #default>-</template>
        </el-table-column>
        <el-table-column label="值" min-width="160">
          <template #default="{ row }">
            <span :class="{ 'value-error': isValueError(tagValues[row.id]) }">
              {{ formatValue(tagValues[row.id]) }}
            </span>
          </template>
        </el-table-column>
        <el-table-column prop="description" label="描述" width="100" show-overflow-tooltip>
          <template #default="{ row }">{{ row.description || '-' }}</template>
        </el-table-column>
        <el-table-column label="操作" width="200" align="right" fixed="right">
          <template #default="{ row }">
            <div class="write-cell">
              <el-input
                v-model="writeInputs[row.id]"
                size="small"
                placeholder="输入后写入"
                class="write-input"
                @focus="initWriteInput(row)"
                @keyup.enter="writeTag(row)"
              />
              <el-button
                type="primary"
                size="small"
                :loading="writingTagId === row.id"
                @click="writeTag(row)"
              >
                Write
              </el-button>
            </div>
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
.page-title { margin: 0 0 0.5rem; font-size: 1.25rem; }
.monitor-header { display: flex; align-items: center; gap: 1rem; flex-wrap: wrap; }
.update-time { font-size: 0.85rem; color: var(--el-text-color-secondary); }
.control-row { display: flex; flex-wrap: wrap; align-items: flex-end; gap: 1rem; }
.control-actions { margin-left: auto; display: flex; gap: 0.5rem; }
.status-bar { display: flex; align-items: center; gap: 1rem; margin: 1rem 0; }
.table-card { margin-top: 0.5rem; }
.font-mono { font-family: var(--font-mono); }
.value-error { color: var(--el-color-danger); }
.empty-block { padding: 3rem; }
.write-cell { display: flex; align-items: center; gap: 0.5rem; justify-content: flex-end; }
.write-input { width: 100px; }
</style>
