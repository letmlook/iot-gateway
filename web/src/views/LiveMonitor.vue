<script setup>
import { ref, computed, onMounted, onUnmounted, watch, nextTick } from 'vue'
import { useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { ElMessage } from 'element-plus'
import { useFlowWebSocket } from '../composables/useFlowWebSocket.js'
import { api } from '../api.js'
import PageHeader from '../components/PageHeader.vue'
import * as echarts from 'echarts'

const { t } = useI18n()
const router = useRouter()

// ── Data ──────────────────────────────────────────────────────────────────────
const flowNodes = ref([])       // { id, name, kind, tags[] }
const selectedNodeId = ref(null)
const selectedTagId = ref(null) // null = all tags of selected node
const tagList = ref([])          // all tags for selected node
const alarms = ref([])           // alarm records from WS

const charts = ref({})           // nodeId -> ECharts instance
const chartContainers = ref({})  // nodeId -> DOM element
const chartData = ref({})        // nodeId -> { timestamps[], seriesData{} }

const metrics = ref({
  totalDataPoints: 0,
  totalAlarms: 0,
  activeNodes: 0,
  msgPerSecond: 0,
})
const mpsCount = ref(0)          // messages in current second
const mpsTimer = ref(null)
const lastMsgTime = ref(null)

// ── WebSocket ─────────────────────────────────────────────────────────────────
const {
  connectionStatus,
  latestMetrics,
  dataPoints,
  connect,
  disconnect,
  subscribe,
  unsubscribe,
} = useFlowWebSocket({})

watch(connectionStatus, (s) => {
  if (s === 'connected') {
    ElMessage.success(t('liveMonitor.wsConnected'))
  } else if (s === 'disconnected') {
    // don't spam on intentional disconnects
  }
})

watch(dataPoints, (points) => {
  if (!points.length) return
  mpsCount.value++
  lastMsgTime.value = Date.now()

  // Process incoming data
  const p = points[0]
  if (!p || !p.node_id) return

  // Update metrics
  metrics.value.totalDataPoints = points.length

  // Push to chart
  const nid = p.node_id
  if (!chartData.value[nid]) {
    chartData.value[nid] = { timestamps: [], seriesData: {} }
  }
  const cd = chartData.value[nid]
  const ts = p.timestamp || new Date().toISOString()
  cd.timestamps.push(ts)

  // Keep last 200 points per node
  if (cd.timestamps.length > 200) {
    cd.timestamps.shift()
  }

  // Each field in payload gets its own series
  if (p.payload && typeof p.payload === 'object') {
    for (const [field, value] of Object.entries(p.payload)) {
      if (value === null || value === undefined) continue
      if (typeof value === 'object') continue  // skip nested objects
      if (!cd.seriesData[field]) cd.seriesData[field] = []
      cd.seriesData[field].push(value)
      if (cd.seriesData[field].length > 200) cd.seriesData[field].shift()
    }
  }

  // Refresh chart if this is the selected node
  if (nid === selectedNodeId.value) {
    updateChart(nid)
  }
})

watch(alarms, (list) => {
  metrics.value.totalAlarms = list.length
}, { immediate: true })

// ── Computed ───────────────────────────────────────────────────────────────────
const selectedNode = computed(() => flowNodes.value.find(n => n.id === selectedNodeId.value))

const selectedTagOptions = computed(() => {
  if (!selectedNode.value) return []
  return [{ id: null, name: t('liveMonitor.allTags') }, ...tagList.value]
})

const chartSeries = computed(() => {
  if (!selectedNodeId.value) return []
  const cd = chartData.value[selectedNodeId.value]
  if (!cd) return []
  return Object.keys(cd.seriesData).map((field, idx) => ({
    name: field,
    type: 'line',
    smooth: true,
    symbol: 'none',
    data: cd.seriesData[field] || [],
    lineStyle: { width: 1.5 },
  }))
})

const chartTimestamps = computed(() => {
  if (!selectedNodeId.value) return []
  return (chartData.value[selectedNodeId.value]?.timestamps || []).map(ts => {
    const d = new Date(ts)
    return `${d.getHours().toString().padStart(2,'0')}:${d.getMinutes().toString().padStart(2,'0')}:${d.getSeconds().toString().padStart(2,'0')}`
  })
})

// ── Lifecycle ─────────────────────────────────────────────────────────────────
onMounted(async () => {
  await loadFlowNodes()
  connect()
  startMpsTimer()
})

onUnmounted(() => {
  disconnect()
  stopMpsTimer()
  for (const inst of Object.values(charts.value)) {
    inst?.dispose()
  }
})

function startMpsTimer() {
  mpsTimer.value = setInterval(() => {
    metrics.value.msgPerSecond = mpsCount.value
    mpsCount.value = 0
  }, 1000)
}

function stopMpsTimer() {
  if (mpsTimer.value) clearInterval(mpsTimer.value)
}

// ── Data loading ──────────────────────────────────────────────────────────────
async function loadFlowNodes() {
  try {
    const flows = await api.flows()
    const list = flows.flows || []
    // Flatten all nodes from all flows
    const allNodes = []
    for (const f of list) {
      const fd = await api.flow(f.id).catch(() => null)
      if (!fd?.flow) continue
      for (const n of fd.flow.nodes || []) {
        allNodes.push({ ...n, flowId: f.id, flowName: f.name })
      }
    }
    flowNodes.value = allNodes
    metrics.value.activeNodes = allNodes.filter(n => n.status === 'running').length

    // Auto-select first node
    if (allNodes.length && !selectedNodeId.value) {
      selectNode(allNodes[0].id)
    }
  } catch (e) {
    console.error('loadFlowNodes failed', e)
  }
}

async function loadTagsForNode(nodeId) {
  try {
    const tags = await api.tags(nodeId)
    tagList.value = tags.tags || []
  } catch {
    tagList.value = []
  }
}

// ── Node selection ─────────────────────────────────────────────────────────────
function selectNode(nodeId) {
  selectedNodeId.value = nodeId
  selectedTagId.value = null
  loadTagsForNode(nodeId)

  // Init chart data if needed
  if (!chartData.value[nodeId]) {
    chartData.value[nodeId] = { timestamps: [], seriesData: {} }
  }

  // Init / show chart
  nextTick(() => initChart(nodeId))
}

// ── ECharts ────────────────────────────────────────────────────────────────────
function getOrCreateChart(nodeId) {
  if (charts.value[nodeId]) return charts.value[nodeId]
  const el = chartContainers.value[nodeId]
  if (!el) return null
  const instance = echarts.init(el, null, { renderer: 'canvas' })
  charts.value[nodeId] = instance
  return instance
}

function initChart(nodeId) {
  const chart = getOrCreateChart(nodeId)
  if (!chart) return
  chart.setOption({
    backgroundColor: 'transparent',
    grid: { top: 30, right: 20, bottom: 40, left: 60 },
    legend: { top: 0, textStyle: { fontSize: 10 } },
    tooltip: { trigger: 'axis', axisPointer: { type: 'cross' } },
    xAxis: {
      type: 'category',
      data: [],
      axisLabel: { fontSize: 10 },
    },
    yAxis: {
      type: 'value',
      axisLabel: { fontSize: 10 },
      splitLine: { lineStyle: { type: 'dashed', color: '#eee' } },
    },
    series: [],
    animation: false,
  })
}

function updateChart(nodeId) {
  const chart = charts.value[nodeId]
  if (!chart) return
  const cd = chartData.value[nodeId]
  if (!cd) return
  chart.setOption({
    xAxis: { type: 'category', data: chartTimestamps.value, axisLabel: { fontSize: 10 } },
    series: chartSeries.value,
  }, { replaceMerge: ['series'] })
}

// Resize charts when window resizes
function handleResize() {
  for (const inst of Object.values(charts.value)) {
    inst?.resize()
  }
}
window.addEventListener('resize', handleResize)
onUnmounted(() => window.removeEventListener('resize', handleResize))

// ── Alarm handling ─────────────────────────────────────────────────────────────
function acknowledgeAlarm(alarmId) {
  alarms.value = alarms.value.filter(a => a.id !== alarmId)
}

function clearAlarms() {
  alarms.value = []
}

// ── i18n labels ────────────────────────────────────────────────────────────────
function nodeKindIcon(kind) {
  return { south: '🔌', north: '📤', operator: '⚙️' }[kind] || '📦'
}

function severityTagType(severity) {
  return { critical: 'danger', warning: 'warning', info: 'info' }[severity] || 'info'
}

function formatTime(ts) {
  if (!ts) return ''
  const d = new Date(ts)
  return d.toLocaleTimeString()
}
</script>

<template>
  <div class="live-monitor">
    <PageHeader :title="t('liveMonitor.title')">
      <template #actions>
        <!-- Connection status badge -->
        <el-tag :type="connectionStatus === 'connected' ? 'success' : connectionStatus === 'error' ? 'danger' : 'info'" size="small">
          {{ connectionStatus === 'connected' ? t('liveMonitor.connected') : connectionStatus === 'connecting' ? t('liveMonitor.connecting') : t('liveMonitor.disconnected') }}
        </el-tag>

        <el-button size="small" @click="router.push('/flows')">
          ← {{ t('liveMonitor.backToFlows') }}
        </el-button>
      </template>
    </PageHeader>

    <el-alert v-if="!flowNodes.length" type="info" :title="t('liveMonitor.noFlows')" show-icon class="mb-4" />

    <template v-else>
      <!-- ── Metrics Dashboard ── -->
      <div class="metrics-strip">
        <div class="metric-chip">
          <span class="metric-chip-label">{{ t('liveMonitor.activeNodes') }}</span>
          <span class="metric-chip-value">{{ metrics.activeNodes }}</span>
        </div>
        <div class="metric-chip">
          <span class="metric-chip-label">{{ t('liveMonitor.totalDataPoints') }}</span>
          <span class="metric-chip-value">{{ metrics.totalDataPoints }}</span>
        </div>
        <div class="metric-chip">
          <span class="metric-chip-label">{{ t('liveMonitor.totalAlarms') }}</span>
          <span class="metric-chip-value" :class="{ 'text-danger': metrics.totalAlarms > 0 }">{{ metrics.totalAlarms }}</span>
        </div>
        <div class="metric-chip">
          <span class="metric-chip-label">{{ t('liveMonitor.msgPerSecond') }}</span>
          <span class="metric-chip-value">{{ metrics.msgPerSecond }}</span>
        </div>
      </div>

      <div class="monitor-body">
        <!-- ── Left: Node Tree ── -->
        <div class="panel node-panel">
          <div class="panel-header">{{ t('liveMonitor.nodeTree') }}</div>
          <div class="node-list">
            <div
              v-for="node in flowNodes"
              :key="node.id"
              class="node-item"
              :class="{ active: node.id === selectedNodeId }"
              @click="selectNode(node.id)"
            >
              <span class="node-icon">{{ nodeKindIcon(node.kind) }}</span>
              <div class="node-info">
                <span class="node-name">{{ node.name }}</span>
                <el-tag size="small" :type="node.status === 'running' ? 'success' : 'info'">{{ node.status || 'unknown' }}</el-tag>
              </div>
            </div>
          </div>
        </div>

        <!-- ── Center: ECharts Curve ── -->
        <div class="panel chart-panel">
          <div class="panel-header">
            <span>{{ t('liveMonitor.realtimeCurve') }}</span>
            <el-select v-if="tagList.length" v-model="selectedTagId" :placeholder="t('liveMonitor.allTags')" clearable size="small" style="width: 120px; margin-left: 8px">
              <el-option v-for="tag in tagList" :key="tag.id" :label="tag.name" :value="tag.id" />
            </el-select>
          </div>

          <div v-if="selectedNodeId" class="chart-wrapper">
            <!-- Chart per selected node; reuse same container key -->
            <div
              :ref="el => { chartContainers[selectedNodeId] = el }"
              class="chart-canvas"
            />
            <div v-if="!chartData[selectedNodeId]?.timestamps?.length" class="chart-empty">
              {{ t('liveMonitor.waitingForData') }}
            </div>
          </div>
          <div v-else class="chart-placeholder">
            {{ t('liveMonitor.selectNodeToView') }}
          </div>
        </div>

        <!-- ── Right: Alarm List ── -->
        <div class="panel alarm-panel">
          <div class="panel-header">
            <span>{{ t('liveMonitor.alarmList') }}</span>
            <div style="display:flex;gap:4px">
              <el-button size="small" link type="danger" @click="clearAlarms" :disabled="!alarms.length">
                {{ t('liveMonitor.clear') }}
              </el-button>
            </div>
          </div>

          <div v-if="alarms.length" class="alarm-list">
            <div
              v-for="alarm in alarms"
              :key="alarm.id"
              class="alarm-item"
              :class="'alarm-' + (alarm.severity || 'info')"
            >
              <div class="alarm-header">
                <el-tag size="small" :type="severityTagType(alarm.severity)">
                  {{ alarm.severity || 'info' }}
                </el-tag>
                <span class="alarm-time">{{ formatTime(alarm.ts) }}</span>
                <el-button size="small" link type="primary" @click="acknowledgeAlarm(alarm.id)">✓</el-button>
              </div>
              <div class="alarm-msg">{{ alarm.message || alarm.msg || JSON.stringify(alarm) }}</div>
              <div v-if="alarm.node_name" class="alarm-node">📌 {{ alarm.node_name }}</div>
            </div>
          </div>
          <el-empty v-else :description="t('liveMonitor.noAlarms')" :image-size="60" />
        </div>
      </div>
    </template>
  </div>
</template>

<style scoped>
.live-monitor {
  display: flex;
  flex-direction: column;
  height: calc(100vh - 80px);
  overflow: hidden;
}

.mb-4 { margin-bottom: 1rem; }

/* Metrics strip */
.metrics-strip {
  display: flex;
  gap: 0.75rem;
  margin-bottom: 1rem;
  flex-shrink: 0;
}

.metric-chip {
  background: var(--bg-surface);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-md);
  padding: 0.5rem 1rem;
  display: flex;
  flex-direction: column;
  gap: 0.2rem;
  min-width: 100px;
}

.metric-chip-label {
  font-size: 0.7rem;
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.metric-chip-value {
  font-size: 1.3rem;
  font-weight: 700;
  font-family: var(--font-mono);
  color: var(--text-primary);
}

.text-danger { color: var(--danger); }

/* Main body: 3-column layout */
.monitor-body {
  display: flex;
  flex: 1;
  gap: 0.75rem;
  min-height: 0;
  overflow: hidden;
}

/* Panels */
.panel {
  background: var(--bg-surface);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-lg);
  display: flex;
  flex-direction: column;
  overflow: hidden;
}

.panel-header {
  font-size: 0.85rem;
  font-weight: 600;
  color: var(--text-primary);
  padding: 0.6rem 1rem;
  border-bottom: 1px solid var(--border-subtle);
  display: flex;
  align-items: center;
  flex-shrink: 0;
}

/* Node panel */
.node-panel {
  width: 200px;
  flex-shrink: 0;
}

.node-list {
  overflow-y: auto;
  flex: 1;
  padding: 0.5rem;
}

.node-item {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.5rem 0.6rem;
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: background 0.15s;
}

.node-item:hover { background: var(--bg-elevated); }
.node-item.active { background: var(--accent-bg); border: 1px solid var(--accent); }

.node-icon { font-size: 1.1rem; }

.node-info {
  display: flex;
  flex-direction: column;
  gap: 2px;
  overflow: hidden;
}

.node-name {
  font-size: 0.8rem;
  font-weight: 500;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

/* Chart panel */
.chart-panel {
  flex: 1;
  min-width: 0;
}

.chart-wrapper {
  flex: 1;
  position: relative;
  min-height: 0;
}

.chart-canvas {
  width: 100%;
  height: 100%;
  min-height: 300px;
}

.chart-empty,
.chart-placeholder {
  position: absolute;
  inset: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  font-size: 0.85rem;
  color: var(--text-muted);
  background: var(--bg-surface);
}

.chart-placeholder {
  border-radius: var(--radius-lg);
}

/* Alarm panel */
.alarm-panel {
  width: 260px;
  flex-shrink: 0;
}

.alarm-list {
  overflow-y: auto;
  flex: 1;
  padding: 0.5rem;
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.alarm-item {
  background: var(--bg-elevated);
  border-radius: var(--radius-md);
  padding: 0.6rem 0.75rem;
  border-left: 3px solid var(--border-default);
}

.alarm-item.alarm-critical { border-left-color: var(--danger); }
.alarm-item.alarm-warning { border-left-color: var(--warning); }
.alarm-item.alarm-info { border-left-color: var(--accent); }

.alarm-header {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  margin-bottom: 4px;
}

.alarm-time {
  font-size: 0.7rem;
  color: var(--text-muted);
  margin-left: auto;
  font-family: var(--font-mono);
}

.alarm-msg {
  font-size: 0.78rem;
  color: var(--text-secondary);
  line-height: 1.4;
  word-break: break-all;
}

.alarm-node {
  font-size: 0.7rem;
  color: var(--text-muted);
  margin-top: 4px;
}
</style>
