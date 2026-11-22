<script setup>
// 告警中心（v2.0 T1-④）：REST 轮询为可靠来源（redesign 规格结论），
// WS alarm 帧机会式刷新已接入（与 DataMonitor 同模式：on('connected') 重订阅、
// 卸载只 off handler 不退订 topic——topic 订阅被其它页面覆盖是既有单例语义，
// 重连时各自 on('connected') 里恢复）。
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage } from 'element-plus'
import { Refresh } from '@element-plus/icons-vue'
import { api } from '../api.js'
import { getErrorMessage } from '../i18n'
import { gatewayWs, FrameType } from '../ws/gatewayWs.js'
import PageHeader from '../components/PageHeader.vue'

const { t } = useI18n()

const alarms = ref([])
const loading = ref(false)
const error = ref('')
const stateFilter = ref('')
const levelFilter = ref('')
let timer = null

const LEVELS = ['critical', 'high', 'medium', 'low', 'info']
const STATES = ['active', 'acknowledged', 'resolved']

const levelTagType = (l) =>
  ({ critical: 'danger', high: 'danger', medium: 'warning', low: 'info', info: 'info' }[l] || 'info')
const stateTagType = (s) =>
  ({ active: 'danger', acknowledged: 'warning', resolved: 'success' }[s] || 'info')

const filtered = computed(() => {
  let rows = alarms.value
  if (stateFilter.value) rows = rows.filter((a) => a.state === stateFilter.value)
  if (levelFilter.value) rows = rows.filter((a) => a.level === levelFilter.value)
  return rows
})

function fmtTime(ms) {
  if (!ms) return '-'
  const d = new Date(ms)
  const p = (x) => String(x).padStart(2, '0')
  return `${d.getMonth() + 1}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}:${p(d.getSeconds())}`
}

async function load() {
  loading.value = true
  error.value = ''
  try {
    const r = await api.alarms({ limit: 500 })
    alarms.value = r.alarms || []
  } catch (e) {
    error.value = t('alarms.loadFailed') + getErrorMessage(t, e)
  } finally {
    loading.value = false
  }
}

async function ack(a) {
  try {
    await api.ackAlarm(a.id)
    ElMessage.success(t('alarms.ackOk'))
    await load()
  } catch (e) {
    ElMessage.error(getErrorMessage(t, e))
  }
}

async function resolve(a) {
  try {
    await api.resolveAlarm(a.id)
    ElMessage.success(t('alarms.resolveOk'))
    await load()
  } catch (e) {
    ElMessage.error(getErrorMessage(t, e))
  }
}

// ---------- WS 机会式刷新（B4） ----------
// alarm 帧到达时机会式刷新列表；500ms 防抖合并告警风暴，避免打爆 REST 端点。
// REST 5s 轮询保留不动——它是可靠来源，WS 只是让「新告警」从最长 5s 延迟缩到亚秒。
let wsDebounce = null
function onWsAlarm() {
  if (wsDebounce) return
  wsDebounce = setTimeout(() => {
    wsDebounce = null
    load()
  }, 500)
}
function onWsConnected() {
  // 认证成功后订阅 alarms topic（重连恢复走同一入口；subscribe 覆盖单例的重连恢复帧
  // 是既有语义，DataMonitor 靠同样的 on('connected') 重订阅兜底）
  if (gatewayWs.authenticated) {
    gatewayWs.subscribe([FrameType.ALARMS_TOPIC])
  }
}

onMounted(() => {
  load()
  timer = setInterval(load, 5000)
  gatewayWs.connect() // 单例幂等，不会重复连接
  gatewayWs.on('connected', onWsConnected)
  gatewayWs.on(FrameType.ALARM, onWsAlarm)
  onWsConnected() // 已认证连接（如从其它页面切回）立即订阅
})
onUnmounted(() => {
  gatewayWs.off('connected', onWsConnected)
  gatewayWs.off(FrameType.ALARM, onWsAlarm)
  if (wsDebounce) {
    clearTimeout(wsDebounce)
    wsDebounce = null
  }
  if (timer) clearInterval(timer)
  timer = null
})
</script>

<template>
  <div class="alarms-page">
    <PageHeader :title="t('alarms.title')" :subtitle="t('alarms.subtitle')">
      <template #actions>
        <el-button :icon="Refresh" :loading="loading" @click="load">
          {{ t('alarms.refresh') }}
        </el-button>
      </template>
    </PageHeader>

    <el-card shadow="never">
      <div class="alarms-toolbar">
        <el-radio-group v-model="stateFilter">
          <el-radio-button :value="''">{{ t('alarms.all') }}</el-radio-button>
          <el-radio-button v-for="s in STATES" :key="s" :value="s">
            {{ t(`alarms.state_${s}`) }}
          </el-radio-button>
        </el-radio-group>
        <el-select
          v-model="levelFilter"
          clearable
          :placeholder="t('alarms.levelFilter')"
          class="alarms-level-select"
        >
          <el-option v-for="l in LEVELS" :key="l" :value="l" :label="t(`alarms.level_${l}`)" />
        </el-select>
      </div>

      <div v-if="error" class="alarms-error">{{ error }}</div>

      <el-table :data="filtered" v-loading="loading" size="default">
        <el-table-column :label="t('alarms.level')" width="100">
          <template #default="{ row }">
            <el-tag :type="levelTagType(row.level)">{{ t(`alarms.level_${row.level}`) }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column prop="tag" :label="t('alarms.tag')" width="160" />
        <el-table-column :label="t('alarms.value')" width="120">
          <template #default="{ row }">
            {{ row.value != null ? row.value : '-' }}
            <span v-if="row.threshold != null" class="alarms-threshold">
              / {{ row.threshold }}
            </span>
          </template>
        </el-table-column>
        <el-table-column prop="message" :label="t('alarms.message')" min-width="220" show-overflow-tooltip />
        <el-table-column prop="node_name" :label="t('alarms.node')" width="140" />
        <el-table-column :label="t('alarms.occurrences')" width="90">
          <template #default="{ row }">
            ×{{ row.occurrences }}
          </template>
        </el-table-column>
        <el-table-column :label="t('alarms.firstTs')" width="150">
          <template #default="{ row }">{{ fmtTime(row.first_ts) }}</template>
        </el-table-column>
        <el-table-column :label="t('alarms.lastTs')" width="150">
          <template #default="{ row }">{{ fmtTime(row.last_ts) }}</template>
        </el-table-column>
        <el-table-column :label="t('alarms.state')" width="110">
          <template #default="{ row }">
            <el-tag :type="stateTagType(row.state)">{{ t(`alarms.state_${row.state}`) }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="t('alarms.actions')" width="160" fixed="right">
          <template #default="{ row }">
            <el-button
              v-if="row.state === 'active'"
              size="small"
              type="warning"
              @click="ack(row)"
            >{{ t('alarms.ack') }}</el-button>
            <el-button
              v-if="row.state !== 'resolved'"
              size="small"
              type="success"
              @click="resolve(row)"
            >{{ t('alarms.resolve') }}</el-button>
          </template>
        </el-table-column>
        <template #empty>
          <el-empty :description="t('alarms.empty')" />
        </template>
      </el-table>
    </el-card>
  </div>
</template>

<style scoped>
.alarms-page {
  display: flex;
  flex-direction: column;
  gap: 16px;
}
.alarms-toolbar {
  display: flex;
  flex-wrap: wrap;
  gap: 12px;
  align-items: center;
  margin-bottom: 12px;
}
.alarms-level-select {
  width: 160px;
}
.alarms-error {
  color: var(--el-color-danger);
  font-size: 13px;
  margin-bottom: 8px;
}
.alarms-threshold {
  opacity: 0.6;
  font-size: 12px;
}
</style>
