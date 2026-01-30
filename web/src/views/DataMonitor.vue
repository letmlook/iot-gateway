<script setup>
import { ref, onMounted, onUnmounted, computed, watch } from 'vue'
import { useRoute } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { getErrorMessage } from '../i18n'
import { ElMessage } from 'element-plus'
import { VideoPlay, VideoPause, Refresh, Search } from '@element-plus/icons-vue'
import { api } from '../api.js'
import StatusIndicator from '../components/StatusIndicator.vue'

const route = useRoute()
const { t, locale } = useI18n()
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
    error.value = t('south.loadFailed') + getErrorMessage(t, e)
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
    error.value = t('south.loadFailed') + getErrorMessage(t, e)
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
    if ('value' in val && (val.type || Object.keys(val).length <= 2)) {
      const v = val.value
      if (v === undefined || v === null) return '-'
      if (typeof v === 'number') return Number.isInteger(v) ? v : Number(v).toFixed(4)
      if (typeof v === 'boolean') return v ? 'true' : 'false'
      if (Array.isArray(v)) return v.length ? v.map(x => formatValue(x)).join(', ') : '-'
      return String(v)
    }
    return JSON.stringify(val)
  }
  if (typeof val === 'number') return Number.isInteger(val) ? val : val.toFixed(4)
  return String(val)
}

function getValueClass(val) {
  if (isValueError(val)) return 'value-error'
  if (val === undefined || val === null) return 'value-empty'
  const v = typeof val === 'object' && 'value' in val ? val.value : val
  if (typeof v === 'boolean') return v ? 'value-true' : 'value-false'
  return 'value-normal'
}

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
    ElMessage.warning(t('monitor.inputValidValue'))
    return
  }
  writingTagId.value = tag.id
  try {
    await api.writeTags(selectedNode.value, [[tag.id, dv]])
    ElMessage.success(t('monitor.writeSuccess'))
    writeInputs.value[tag.id] = ''
    await readValues()
  } catch (e) {
    ElMessage.error(t('monitor.writeFailed') + getErrorMessage(t, e))
  } finally {
    writingTagId.value = null
  }
}

function initWriteInput(tag) {
  if (writeInputs.value[tag.id] === undefined) {
    const v = tagValues.value[tag.id]
    if (v !== undefined && v !== null) {
      writeInputs.value[tag.id] = formatValue(v)
    } else {
      writeInputs.value[tag.id] = ''
    }
  }
}

const selectedNodeInfo = computed(() => nodes.value.find(n => n.id === selectedNode.value))

onMounted(loadNodes)
onUnmounted(() => { if (timer) clearInterval(timer) })
</script>

<template>
  <div class="monitor-page">
    <!-- 页面头部 -->
    <div class="page-header">
      <div class="header-content">
        <h1 class="page-title">{{ t('monitor.title') }}</h1>
        <span v-if="lastUpdateTime" class="update-time">
          {{ t('monitor.updateTime') }} {{ lastUpdateTime.toLocaleTimeString() }}
        </span>
      </div>
    </div>

    <el-alert v-if="error" type="error" :title="error" closable show-icon @close="error = ''" class="mb-4" />

    <!-- 控制面板 -->
    <div class="control-panel">
      <div class="control-row">
        <div class="control-group">
          <label>{{ t('monitor.selectDevice') }}</label>
          <el-select v-model="selectedNode" class="control-select" @change="onNodeChange">
            <el-option v-for="n in nodes" :key="n.id" :label="n.name" :value="n.id" />
          </el-select>
        </div>
        <div class="control-group">
          <label>{{ t('monitor.group') }}</label>
          <el-select v-model="selectedGroup" :placeholder="t('monitor.all')" clearable class="control-select" @change="applyFilters">
            <el-option v-for="g in groups" :key="g.id" :label="g.name" :value="g.id" />
          </el-select>
        </div>
        <div class="control-group">
          <label>{{ t('common.search') }}</label>
          <el-input
            v-model="keywordSearch"
            :placeholder="t('south.keywordSearch')"
            :prefix-icon="Search"
            clearable
            class="control-input"
          />
        </div>
        <div class="control-group checkbox-group">
          <el-checkbox v-model="onlyShowErrors">{{ t('monitor.onlyErrors') }}</el-checkbox>
        </div>
      </div>
      
      <div class="control-row control-row-actions">
        <div class="refresh-controls">
          <el-select v-model="refreshInterval" class="interval-select" :disabled="autoRefresh">
            <el-option :value="1000" :label="t('monitor.intervalSeconds', { n: 1 })" />
            <el-option :value="2000" :label="t('monitor.intervalSeconds', { n: 2 })" />
            <el-option :value="5000" :label="t('monitor.intervalSeconds', { n: 5 })" />
            <el-option :value="10000" :label="t('monitor.intervalSeconds', { n: 10 })" />
          </el-select>
          <el-button
            :type="autoRefresh ? 'warning' : 'primary'"
            :icon="autoRefresh ? VideoPause : VideoPlay"
            :disabled="!tags.length"
            @click="toggleAutoRefresh"
          >
            {{ autoRefresh ? t('monitor.stopRefresh') : t('monitor.autoRefresh') }}
          </el-button>
          <el-button :icon="Refresh" :disabled="loading || !tags.length" @click="readValues">
            {{ t('monitor.manualRefresh') }}
          </el-button>
        </div>
      </div>
    </div>

    <!-- 状态栏 -->
    <div v-if="selectedNodeInfo" class="status-bar">
      <StatusIndicator :status="selectedNodeInfo.state" size="small" />
      <span class="divider"></span>
      <span class="stat-item">{{ t('monitor.currentCount', { n: tags.length }) }}</span>
      <span v-if="autoRefresh" class="auto-badge">
        <span class="pulse-dot"></span>
        {{ t('monitor.autoRefresh') }} {{ refreshInterval / 1000 }}s
      </span>
    </div>

    <!-- 数据表格 -->
    <div v-if="tags.length" class="data-table-wrapper">
      <el-table :data="tags" size="small" stripe :header-cell-style="{ background: 'var(--bg-elevated)' }">
        <el-table-column prop="name" :label="t('common.name')" min-width="120">
          <template #default="{ row }">
            <span class="tag-name">{{ row.name }}</span>
          </template>
        </el-table-column>
        <el-table-column prop="address" :label="t('monitor.address')" min-width="100">
          <template #default="{ row }">
            <span class="mono">{{ row.address || '-' }}</span>
          </template>
        </el-table-column>
        <el-table-column prop="data_type" :label="t('monitor.type')" width="90">
          <template #default="{ row }">
            <span class="type-badge">{{ row.data_type || '-' }}</span>
          </template>
        </el-table-column>
        <el-table-column :label="t('monitor.value')" min-width="140">
          <template #default="{ row }">
            <span :class="['value-cell', getValueClass(tagValues[row.id])]">
              {{ formatValue(tagValues[row.id]) }}
            </span>
          </template>
        </el-table-column>
        <el-table-column prop="description" :label="t('common.description')" min-width="100" show-overflow-tooltip>
          <template #default="{ row }">
            <span class="desc-text">{{ row.description || '-' }}</span>
          </template>
        </el-table-column>
        <el-table-column :label="t('common.operation')" width="180" fixed="right">
          <template #default="{ row }">
            <div class="write-cell">
              <el-input
                v-model="writeInputs[row.id]"
                size="small"
                :placeholder="t('common.inputToWrite')"
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
                {{ t('common.write') }}
              </el-button>
            </div>
          </template>
        </el-table-column>
      </el-table>
    </div>

    <el-skeleton v-else-if="loading" :rows="5" animated />

    <el-empty v-else :description="t('monitor.noData')" class="empty-state">
      <template #description>
        <p v-if="!nodes.length">{{ t('monitor.createSouthFirst') }}</p>
        <p v-else>{{ t('monitor.addTagsFirst') }}</p>
      </template>
    </el-empty>
  </div>
</template>

<style scoped>
.monitor-page {
  max-width: 1400px;
}

.mb-4 { margin-bottom: 1.5rem; }

/* 页面头部 */
.page-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 1.25rem;
  flex-wrap: wrap;
  gap: 0.5rem;
}

.header-content {
  display: flex;
  align-items: center;
  gap: 1rem;
  flex-wrap: wrap;
}

.page-title {
  font-size: 1.5rem;
  font-weight: 700;
  color: var(--text-primary);
  margin: 0;
}

.update-time {
  font-size: 0.8rem;
  color: var(--text-muted);
  background: var(--bg-inset);
  padding: 0.25rem 0.6rem;
  border-radius: 100px;
}

/* 控制面板 */
.control-panel {
  background: var(--bg-surface);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-lg);
  padding: 1rem 1.25rem;
  margin-bottom: 1rem;
}

.control-row {
  display: flex;
  flex-wrap: wrap;
  align-items: flex-end;
  gap: 1rem;
}

.control-row + .control-row {
  margin-top: 0.75rem;
  padding-top: 0.75rem;
  border-top: 1px solid var(--border-subtle);
}

.control-row-actions {
  justify-content: flex-end;
}

.control-group {
  display: flex;
  flex-direction: column;
  gap: 0.35rem;
}

.control-group label {
  font-size: 0.75rem;
  font-weight: 500;
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.03em;
}

.control-select {
  width: 160px;
}

.control-input {
  width: 180px;
}

.checkbox-group {
  justify-content: flex-end;
  padding-bottom: 0.35rem;
}

.refresh-controls {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.interval-select {
  width: 90px;
}

/* 状态栏 */
.status-bar {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  padding: 0.6rem 1rem;
  background: var(--bg-elevated);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-md);
  margin-bottom: 1rem;
  font-size: 0.85rem;
}

.divider {
  width: 1px;
  height: 16px;
  background: var(--border-default);
}

.stat-item {
  color: var(--text-secondary);
}

.auto-badge {
  display: inline-flex;
  align-items: center;
  gap: 0.35rem;
  padding: 0.2rem 0.5rem;
  background: var(--accent-glow);
  color: var(--accent-dim);
  border-radius: 100px;
  font-size: 0.75rem;
  font-weight: 500;
}

.pulse-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
  background: var(--accent);
  animation: pulse 1s infinite;
}

@keyframes pulse {
  0%, 100% { opacity: 1; transform: scale(1); }
  50% { opacity: 0.5; transform: scale(1.2); }
}

/* 数据表格 */
.data-table-wrapper {
  background: var(--bg-surface);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-lg);
  overflow: hidden;
}

.tag-name {
  font-weight: 500;
  color: var(--text-primary);
}

.mono {
  font-family: var(--font-mono);
  font-size: 0.85rem;
  color: var(--text-secondary);
}

.type-badge {
  display: inline-block;
  padding: 0.15rem 0.45rem;
  background: var(--bg-inset);
  border-radius: var(--radius-xs);
  font-size: 0.75rem;
  font-family: var(--font-mono);
  color: var(--text-secondary);
}

.value-cell {
  font-family: var(--font-mono);
  font-size: 0.9rem;
  font-weight: 600;
}

.value-cell.value-normal {
  color: var(--success);
}

.value-cell.value-error {
  color: var(--danger);
}

.value-cell.value-empty {
  color: var(--text-muted);
}

.value-cell.value-true {
  color: var(--success);
}

.value-cell.value-false {
  color: var(--danger);
}

.desc-text {
  font-size: 0.85rem;
  color: var(--text-muted);
}

.write-cell {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  justify-content: flex-end;
}

.write-input {
  width: 100px;
}

/* 空状态 */
.empty-state {
  padding: 3rem;
}

@media (max-width: 768px) {
  .control-row {
    flex-direction: column;
    align-items: stretch;
  }
  
  .control-select,
  .control-input {
    width: 100%;
  }
  
  .refresh-controls {
    flex-wrap: wrap;
  }
}
</style>
