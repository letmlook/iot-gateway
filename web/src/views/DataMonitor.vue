<script setup>
import { ref, onMounted, onUnmounted, computed, watch } from 'vue'
import { useRoute } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { ElMessage } from 'element-plus'
import { VideoPlay, VideoPause, Refresh, EditPen } from '@element-plus/icons-vue'
import { api } from '../api.js'

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
    error.value = t('south.loadFailed') + e.message
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
    error.value = t('south.loadFailed') + e.message
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
    // DataValue 格式 { type, value }：直接显示 value
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
    ElMessage.error(t('monitor.writeFailed') + e.message)
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
  <div class="page-container monitor-page">
    <div class="monitor-page-header">
      <h2 class="monitor-page-title">{{ t('monitor.title') }}</h2>
      <span v-if="lastUpdateTime" class="monitor-update-time">{{ t('monitor.updateTime') }} {{ lastUpdateTime.toLocaleString() }}</span>
    </div>

    <el-alert v-if="error" type="error" :title="error" closable show-icon @close="error = ''" class="monitor-alert" />

    <el-card shadow="never" class="monitor-controls">
      <div class="monitor-controls-inner">
        <div class="monitor-filter-group">
          <span class="monitor-filter-label">{{ t('monitor.filter') }}</span>
          <el-select v-model="selectedNode" :placeholder="t('south.selectPlugin')" class="monitor-select" @change="onNodeChange">
            <el-option v-for="n in nodes" :key="n.id" :label="n.name" :value="n.id" />
          </el-select>
          <el-select v-model="selectedGroup" placeholder="Group" clearable class="monitor-select monitor-select-group" @change="applyFilters">
            <el-option v-for="g in groups" :key="g.id" :label="g.name" :value="g.id" />
          </el-select>
          <el-input v-model="keywordSearch" :placeholder="t('south.keywordSearch')" clearable class="monitor-search" />
          <el-checkbox v-model="onlyShowErrors" class="monitor-checkbox">{{ t('monitor.onlyErrors') }}</el-checkbox>
        </div>
        <div class="monitor-refresh-group">
          <span class="monitor-filter-label">{{ t('monitor.refresh') }}</span>
          <el-select v-model="refreshInterval" class="monitor-interval-select" :disabled="autoRefresh">
            <el-option :value="1000" :label="locale === 'zh' ? '1 秒' : '1s'" />
            <el-option :value="2000" :label="locale === 'zh' ? '2 秒' : '2s'" />
            <el-option :value="5000" :label="locale === 'zh' ? '5 秒' : '5s'" />
            <el-option :value="10000" :label="locale === 'zh' ? '10 秒' : '10s'" />
          </el-select>
          <el-button
            :type="autoRefresh ? 'warning' : 'primary'"
            :icon="autoRefresh ? VideoPause : VideoPlay"
            :disabled="!tags.length"
            size="default"
            @click="toggleAutoRefresh"
          >
            {{ autoRefresh ? t('monitor.stopRefresh') : t('monitor.autoRefresh') }}
          </el-button>
          <el-button :icon="Refresh" :disabled="loading || !tags.length" @click="readValues">{{ t('monitor.manualRefresh') }}</el-button>
        </div>
      </div>
    </el-card>

    <div v-if="selectedNodeInfo" class="monitor-status-bar">
      <el-tag :type="selectedNodeInfo.state === 'running' ? 'success' : selectedNodeInfo.state === 'error' ? 'danger' : 'info'" size="small">
        {{ selectedNodeInfo.state === 'running' ? t('common.running') : selectedNodeInfo.state === 'error' ? t('common.error') : t('common.stopped') }}
      </el-tag>
      <span class="stat">{{ t('monitor.currentCount', { n: tags.length }) }}</span>
      <el-tag v-if="autoRefresh" type="primary" size="small" effect="plain">{{ t('monitor.autoRefresh') }} {{ refreshInterval / 1000 }}s</el-tag>
    </div>

    <el-card v-if="tags.length" shadow="never" class="monitor-table-card">
      <el-table :data="tags" size="small" stripe>
        <el-table-column prop="name" :label="t('common.name')" min-width="120" />
        <el-table-column prop="address" :label="t('monitor.address')" width="140">
          <template #default="{ row }"><span class="font-mono">{{ row.address || '-' }}</span></template>
        </el-table-column>
        <el-table-column prop="data_type" :label="t('monitor.type')" width="90">
          <template #default="{ row }"><el-tag size="small" type="info">{{ row.data_type || '-' }}</el-tag></template>
        </el-table-column>
        <el-table-column :label="t('monitor.multiplier')" width="80">
          <template #default>-</template>
        </el-table-column>
        <el-table-column :label="t('monitor.value')" min-width="160">
          <template #default="{ row }">
            <span :class="{ 'value-error': isValueError(tagValues[row.id]) }">
              {{ formatValue(tagValues[row.id]) }}
            </span>
          </template>
        </el-table-column>
        <el-table-column prop="description" :label="t('common.description')" width="100" show-overflow-tooltip>
          <template #default="{ row }">{{ row.description || '-' }}</template>
        </el-table-column>
        <el-table-column :label="t('common.operation')" width="200" align="right" fixed="right">
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
    </el-card>

    <el-skeleton v-else-if="loading" :rows="5" animated />

    <el-empty v-else :description="t('monitor.noData')" class="empty-block">
      <template #description>
        <p v-if="!nodes.length">{{ t('monitor.createSouthFirst') }}</p>
        <p v-else>{{ t('monitor.addTagsFirst') }}</p>
      </template>
    </el-empty>
  </div>
</template>

<style scoped>
.monitor-page { padding-bottom: 1.5rem; }

.monitor-page-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  gap: 1rem;
  margin-bottom: 1rem;
  flex-wrap: wrap;
}
.monitor-page-title { margin: 0; font-size: 1.25rem; font-weight: 600; color: var(--text-primary); }
.monitor-update-time { font-size: 0.8125rem; color: var(--text-muted); }

.monitor-alert { margin-bottom: 1rem; }

.monitor-controls :deep(.el-card__body) { padding: 1rem 1.25rem; }
.monitor-controls-inner {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 1rem 1.5rem;
  row-gap: 0.75rem;
}
.monitor-filter-group,
.monitor-refresh-group {
  display: flex;
  align-items: center;
  gap: 0.5rem 0.75rem;
  flex-wrap: wrap;
}
.monitor-filter-label {
  font-size: 0.8125rem;
  color: var(--text-muted);
  margin-right: 0.25rem;
  flex-shrink: 0;
}
.monitor-select { width: 140px; }
.monitor-select-group { width: 120px; }
.monitor-search { width: 160px; }
.monitor-checkbox { margin-left: 0.25rem; }
.monitor-interval-select { width: 90px; }

.monitor-status-bar {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  margin: 0.75rem 0 0.5rem;
  font-size: 0.875rem;
}
.monitor-status-bar .stat { color: var(--text-secondary); }
.monitor-table-card { margin-top: 0; }
.monitor-table-card :deep(.el-card__body) { padding: 0.75rem 1rem; }

.font-mono { font-family: var(--font-mono); }
.value-error { color: var(--el-color-danger); }
.empty-block { padding: 3rem; margin-top: 0.5rem; }
.write-cell { display: flex; align-items: center; gap: 0.5rem; justify-content: flex-end; }
.write-input { width: 100px; min-width: 80px; }
</style>
