<script setup>
import { ref, computed, onMounted, onUnmounted, nextTick } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage } from 'element-plus'
import { Refresh, Search } from '@element-plus/icons-vue'
import { api } from '../api.js'
import { getErrorMessage } from '../i18n'
import PageHeader from '../components/PageHeader.vue'
import * as echarts from 'echarts/core'
import { LineChart } from 'echarts/charts'
import {
  GridComponent,
  TooltipComponent,
  LegendComponent,
  DataZoomComponent,
} from 'echarts/components'
import { CanvasRenderer } from 'echarts/renderers'

// 按需注册（与 Element Plus 按需引入同一口径）：只打包折线/网格/提示/图例/缩放 + Canvas 渲染
echarts.use([LineChart, GridComponent, TooltipComponent, LegendComponent, DataZoomComponent, CanvasRenderer])

const { t } = useI18n()

// ---------- 状态 ----------
const stats = ref(null)
const seriesList = ref([])
const nodeNames = ref({})
const selectedKeys = ref([])
const range = ref('1h')
const loading = ref(false)
const error = ref('')
const chartEl = ref(null)
let chart = null

const RANGES = [
  { key: '30m', ms: 30 * 60_000 },
  { key: '1h', ms: 3_600_000 },
  { key: '6h', ms: 6 * 3_600_000 },
  { key: '24h', ms: 24 * 3_600_000 },
]

const disabled = computed(() => !!stats.value && stats.value.enabled === false)

// key: `node_id|group_id|tag`；label: `节点名 / 点位名`（同节点同名点位跨组时追加组短 id 区分）
const options = computed(() =>
  seriesList.value.map((s) => ({
    key: `${s.node_id}|${s.group_id}|${s.tag}`,
    label: `${nodeNames.value[s.node_id] || s.node_id.slice(0, 8)} / ${s.tag}`,
    count: s.count,
    lastTs: s.max_ts,
  })),
)

const selectedLabels = computed(() => {
  const byKey = Object.fromEntries(options.value.map((o) => [o.key, o.label]))
  const seen = {}
  return selectedKeys.value.map((k) => {
    const base = byKey[k] || k
    seen[base] = (seen[base] || 0) + 1
    // 同名序列（同节点同点位不同组）追加短 id 保证图例可区分
    const g = k.split('|')[1] || ''
    return seen[base] > 1 ? `${base} #${g.slice(0, 4)}` : base
  })
})

function formatBytes(n) {
  if (!n && n !== 0) return '-'
  if (n < 1024) return `${n} B`
  if (n < 1024 * 1024) return `${(n / 1024).toFixed(1)} KB`
  if (n < 1024 * 1024 * 1024) return `${(n / 1024 / 1024).toFixed(1)} MB`
  return `${(n / 1024 / 1024 / 1024).toFixed(2)} GB`
}

function formatTime(ms) {
  if (!ms) return '-'
  const d = new Date(ms)
  const p = (x) => String(x).padStart(2, '0')
  return `${d.getMonth() + 1}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`
}

// ---------- 加载 ----------
async function loadStats() {
  try {
    stats.value = await api.historyStats()
  } catch (_) {
    stats.value = null
  }
}

async function loadSeriesList() {
  try {
    const r = await api.historySeriesList(1000)
    seriesList.value = r.series || []
  } catch (e) {
    seriesList.value = []
    if (!disabled.value) {
      error.value = t('history.loadFailed') + getErrorMessage(t, e)
    }
  }
}

async function loadNodeNames() {
  try {
    const all = await api.nodes()
    const map = {}
    for (const n of all) map[n.id] = n.name
    nodeNames.value = map
  } catch (_) {
    nodeNames.value = {}
  }
}

async function query() {
  if (!selectedKeys.value.length) {
    ElMessage.warning(t('history.noSelection'))
    return
  }
  loading.value = true
  error.value = ''
  try {
    const r = RANGES.find((x) => x.key === range.value) || RANGES[1]
    const to = Date.now()
    const from = to - r.ms
    const results = await Promise.all(
      selectedKeys.value.map(async (key, i) => {
        const [nodeId, groupId, tag] = key.split('|')
        const resp = await api.historySeries(nodeId, groupId, tag, from, to, 2000)
        return { label: selectedLabels.value[i] || tag, ...resp }
      }),
    )
    await nextTick()
    renderChart(results)
  } catch (e) {
    error.value = t('history.loadFailed') + getErrorMessage(t, e)
  } finally {
    loading.value = false
  }
}

// ---------- 图表 ----------
// 每个序列画三条线：min（透明）+ max-min（带面积，堆叠成 min/max 置信带）+ avg（主线）。
// 置信带不进图例、不进 tooltip，视觉上托住主线。
function renderChart(results) {
  if (!chartEl.value) return
  if (!chart) chart = echarts.init(chartEl.value)
  const series = []
  const legendNames = []

  results.forEach((r) => {
    const pts = r.points || []
    legendNames.push(r.label)
    series.push({
      name: r.label,
      type: 'line',
      showSymbol: false,
      data: pts.map((p) => [p.ts, p.avg, p.count]),
      lineStyle: { width: 1.6 },
      emphasis: { focus: 'none' },
    })
    if (pts.length > 1) {
      series.push(
        {
          name: `${r.label}__band__min`,
          type: 'line',
          stack: `band_${r.label}`,
          data: pts.map((p) => [p.ts, p.min]),
          lineStyle: { opacity: 0 },
          symbol: 'none',
          silent: true,
          legendHoverLink: false,
        },
        {
          name: `${r.label}__band__range`,
          type: 'line',
          stack: `band_${r.label}`,
          data: pts.map((p) => [p.ts, Math.max(0, p.max - p.min)]),
          lineStyle: { opacity: 0 },
          areaStyle: { opacity: 0.12 },
          symbol: 'none',
          silent: true,
          legendHoverLink: false,
        },
      )
    }
  })

  chart.setOption(
    {
      animation: false,
      tooltip: {
        trigger: 'axis',
        valueFormatter: undefined,
        formatter: (params) => {
          const rows = params.filter((p) => !p.seriesName.includes('__band__'))
          if (!rows.length) return ''
          const time = formatTime(rows[0].value?.[0] ?? rows[0].axisValue)
          const lines = rows.map(
            (p) =>
              `${p.marker}${p.seriesName}&nbsp;<b>${Number(p.value?.[1] ?? 0).toPrecision(6)}</b>` +
              (p.value?.[2] != null ? `&nbsp;<span style="opacity:.6">(${p.value[2]})</span>` : ''),
          )
          return `${time}<br/>${lines.join('<br/>')}`
        },
      },
      legend: { data: legendNames, type: 'scroll' },
      grid: { top: 44, left: 64, right: 24, bottom: 64 },
      xAxis: { type: 'time', axisLabel: { formatter: (v) => formatTime(v) } },
      yAxis: { type: 'value', scale: true },
      dataZoom: [
        { type: 'inside' },
        { type: 'slider', height: 20, bottom: 12 },
      ],
      series,
    },
    { notMerge: true },
  )
}

function onResize() {
  chart && chart.resize()
}

onMounted(async () => {
  window.addEventListener('resize', onResize)
  await Promise.all([loadStats(), loadNodeNames()])
  await loadSeriesList()
})

onUnmounted(() => {
  window.removeEventListener('resize', onResize)
  if (chart) {
    chart.dispose()
    chart = null
  }
})
</script>

<template>
  <div class="history-page">
    <PageHeader :title="t('history.title')" :subtitle="t('history.subtitle')">
      <template #actions>
        <el-button :icon="Refresh" :loading="loading" @click="loadSeriesList(); loadStats()">
          {{ t('history.refreshSeries') }}
        </el-button>
      </template>
    </PageHeader>

    <el-alert
      v-if="disabled"
      type="warning"
      :title="t('history.disabledTitle')"
      :description="t('history.disabledHint')"
      show-icon
      :closable="false"
      class="history-alert"
    />

    <el-card shadow="never" class="history-card">
      <div class="history-toolbar">
        <el-select
          v-model="selectedKeys"
          multiple
          filterable
          clearable
          collapse-tags
          collapse-tags-tooltip
          :max-collapse-tags="3"
          :placeholder="t('history.selectPlaceholder')"
          :loading="loading"
          :disabled="disabled"
          class="history-select"
        >
          <el-option
            v-for="o in options"
            :key="o.key"
            :value="o.key"
            :label="o.label"
          >
            <span>{{ o.label }}</span>
            <span class="history-option-meta">{{ o.count }} · {{ formatTime(o.lastTs) }}</span>
          </el-option>
        </el-select>

        <el-radio-group v-model="range" :disabled="disabled">
          <el-radio-button
            v-for="r in RANGES"
            :key="r.key"
            :value="r.key"
          >{{ t(`history.range_${r.key}`) }}</el-radio-button>
        </el-radio-group>

        <el-button
          type="primary"
          :icon="Search"
          :loading="loading"
          :disabled="disabled || !selectedKeys.length"
          @click="query"
        >
          {{ t('history.query') }}
        </el-button>
      </div>

      <div v-if="stats" class="history-meta">
        <el-tag v-if="!disabled" type="info" effect="plain">{{ t('history.rows') }}: {{ stats.rows?.toLocaleString() }}</el-tag>
        <el-tag v-if="!disabled" type="info" effect="plain">{{ t('history.disk') }}: {{ formatBytes(stats.bytes) }}</el-tag>
        <el-tag type="info" effect="plain">{{ t('history.retention') }}: {{ stats.retention_hours }} {{ t('history.hours') }}</el-tag>
      </div>

      <div v-if="error" class="history-error">{{ error }}</div>

      <el-empty v-if="disabled || (!loading && !seriesList.length)" :description="disabled ? t('history.disabledTitle') : t('history.noSeries')" />
      <el-empty v-else-if="!chart" :description="t('history.noData')" />
      <div v-show="chart || loading" ref="chartEl" class="history-chart" />
    </el-card>
  </div>
</template>

<style scoped>
.history-page {
  display: flex;
  flex-direction: column;
  gap: 16px;
}
.history-alert {
  border-radius: 8px;
}
.history-toolbar {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: 12px;
  margin-bottom: 12px;
}
.history-select {
  flex: 1;
  min-width: 280px;
}
.history-option-meta {
  float: right;
  opacity: 0.6;
  font-size: 12px;
}
.history-meta {
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
  margin-bottom: 8px;
}
.history-chart {
  width: 100%;
  height: 420px;
}
.history-error {
  color: var(--el-color-danger);
  font-size: 13px;
  margin-bottom: 8px;
}
</style>
