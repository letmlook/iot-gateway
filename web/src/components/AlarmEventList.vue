<script setup>
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage } from 'element-plus'
import { api } from '../api.js'

const { t } = useI18n()

const LEVELS = ['critical', 'high', 'medium', 'low', 'info']
const STATUSES = ['active', 'acknowledged', 'resolved']

const levelColors = {
  critical: '#ff4d4f',
  high: '#ff7a45',
  medium: '#ffa940',
  low: '#52c41a',
  info: '#409eff',
}

const statusTagTypes = {
  active: 'danger',
  acknowledged: 'warning',
  resolved: 'success',
}

const events = ref([])
const levelFilter = ref([])
const loading = ref(false)
const error = ref('')
const maxEvents = 200
let pollTimer = null
let alarmSocket = null
const POLL_INTERVAL = 3000

function normalizeLevel(event) {
  const raw = String(event?.level || event?.severity || 'info').toLowerCase()
  if (raw === 'major') return 'high'
  if (raw === 'minor' || raw === 'warning') return 'medium'
  return LEVELS.includes(raw) ? raw : 'info'
}

function normalizeStatus(event) {
  const raw = String(event?.status || 'active').toLowerCase()
  return STATUSES.includes(raw) ? raw : 'active'
}

function eventTimestamp(event) {
  return event?.timestamp || event?.created_at || event?.createdAt || ''
}

function eventTimeValue(event) {
  const ts = eventTimestamp(event)
  const value = ts ? new Date(ts).getTime() : 0
  return Number.isFinite(value) ? value : 0
}

function normalizeEvent(event) {
  const level = normalizeLevel(event)
  return {
    ...event,
    level,
    severity: String(event?.severity || level).toLowerCase(),
    timestamp: eventTimestamp(event),
    status: normalizeStatus(event),
  }
}

function eventsFromResponse(data) {
  if (Array.isArray(data)) return data
  return data?.items || data?.events || []
}

function mergeIncoming(incoming) {
  const source = Array.isArray(incoming) ? incoming : eventsFromResponse(incoming)
  const normalized = source.map(normalizeEvent)
  if (!normalized.length) return

  const byId = new Map()
  const withoutId = []

  for (const existing of events.value) {
    if (existing.id) byId.set(existing.id, { ...existing, isNew: false })
    else withoutId.push({ ...existing, isNew: false })
  }

  let hasFresh = false
  for (const event of normalized) {
    if (event.id && byId.has(event.id)) {
      byId.set(event.id, { ...byId.get(event.id), ...event, isNew: false })
    } else if (event.id) {
      byId.set(event.id, { ...event, isNew: true })
      hasFresh = true
    } else {
      withoutId.unshift({ ...event, isNew: true })
      hasFresh = true
    }
  }

  events.value = [...byId.values(), ...withoutId]
    .sort((a, b) => eventTimeValue(b) - eventTimeValue(a))
    .slice(0, maxEvents)

  if (hasFresh) {
    setTimeout(() => {
      events.value.forEach(e => { e.isNew = false })
    }, 2000)
  }
}

async function loadEvents() {
  loading.value = true
  error.value = ''
  try {
    const data = await api.alarmEvents()
    mergeIncoming(eventsFromResponse(data))
  } catch (e) {
    error.value = t('alarms.loadFailed') + (e?.message || String(e))
  } finally {
    loading.value = false
  }
}

function startAlarmSocket() {
  stopAlarmSocket()
  try {
    const protocol = window.location.protocol === 'https:' ? 'wss:' : 'ws:'
    alarmSocket = new WebSocket(`${protocol}//${window.location.host}/api/ws/alarms`)
    alarmSocket.onmessage = (event) => {
      try {
        const payload = JSON.parse(event.data)
        if (payload.event) mergeIncoming([payload.event])
      } catch (_) {}
    }
    alarmSocket.onerror = () => {
      stopAlarmSocket()
    }
    alarmSocket.onclose = () => {
      alarmSocket = null
    }
  } catch (_) {
    stopAlarmSocket()
  }
}

function stopAlarmSocket() {
  if (alarmSocket) {
    alarmSocket.onclose = null
    alarmSocket.onerror = null
    alarmSocket.close()
    alarmSocket = null
  }
}

function startPolling() {
  stopPolling()
  loadEvents()
  startAlarmSocket()
  pollTimer = setInterval(loadEvents, POLL_INTERVAL)
}

function stopPolling() {
  if (pollTimer) {
    clearInterval(pollTimer)
    pollTimer = null
  }
  stopAlarmSocket()
}

const filteredEvents = computed(() => {
  if (!levelFilter.value.length) return events.value
  return events.value.filter(e => levelFilter.value.includes(e.level))
})

function levelLabel(level) {
  return t(`alarms.levels.${level}`)
}

function statusLabel(status) {
  return t(`alarms.status.${status}`, status)
}

function levelStyle(level) {
  const color = levelColors[level] || '#999'
  return {
    backgroundColor: color + '22',
    color,
    borderColor: color + '66',
  }
}

function formatTime(ts) {
  if (!ts) return '-'
  const d = new Date(ts)
  if (Number.isNaN(d.getTime())) return '-'
  return d.toLocaleString()
}

function toggleLevel(level) {
  const idx = levelFilter.value.indexOf(level)
  if (idx >= 0) {
    levelFilter.value.splice(idx, 1)
  } else {
    levelFilter.value.push(level)
  }
}

function canAck(event) {
  return event.status === 'active'
}

function canResolve(event) {
  return event.status !== 'resolved'
}

async function ackEvent(event) {
  if (!event.id || !canAck(event)) return
  try {
    const data = await api.ackAlarmEvent(event.id)
    mergeIncoming([data.event || data])
  } catch (e) {
    ElMessage.error(t('alarms.ackFailed') + (e?.message || String(e)))
  }
}

async function resolveEvent(event) {
  if (!event.id || !canResolve(event)) return
  try {
    const data = await api.resolveAlarmEvent(event.id)
    mergeIncoming([data.event || data])
  } catch (e) {
    ElMessage.error(t('alarms.resolveFailed') + (e?.message || String(e)))
  }
}

function clearEvents() {
  events.value = []
}

const stats = computed(() => {
  const s = {}
  LEVELS.forEach(l => { s[l] = events.value.filter(e => e.level === l).length })
  return s
})

onMounted(startPolling)
onUnmounted(stopPolling)
</script>

<template>
  <div class="alarm-event-list">
    <div class="list-header">
      <div class="level-badges">
        <span
          v-for="level in LEVELS"
          :key="level"
          class="level-badge"
          :style="[levelStyle(level), { opacity: levelFilter.length && !levelFilter.includes(level) ? 0.35 : 1 }]"
          :title="t('alarms.filterHint')"
          @click="toggleLevel(level)"
        >
          <span class="badge-dot" :style="{ backgroundColor: levelColors[level] }"></span>
          {{ levelLabel(level) }}
          <span class="badge-count">{{ stats[level] }}</span>
        </span>
      </div>
      <div class="header-actions">
        <span class="event-count">{{ t('alarms.eventCount', { n: filteredEvents.length }) }}</span>
        <el-button size="small" @click="clearEvents">{{ t('alarms.actions.clear') }}</el-button>
        <el-button size="small" :loading="loading" @click="loadEvents">{{ t('alarms.actions.refresh') }}</el-button>
      </div>
    </div>

    <el-alert
      v-if="error"
      type="error"
      :title="error"
      closable
      show-icon
      class="event-error"
      @close="error = ''"
    />

    <div class="event-scroll">
      <transition-group name="event-pop" tag="div">
        <div
          v-for="event in filteredEvents"
          :key="event.id || `${event.node_id}-${event.timestamp}`"
          class="event-item"
          :class="{ 'event-new': event.isNew, [`level-${event.level}`]: true }"
        >
          <div class="event-indicator" :style="{ backgroundColor: levelColors[event.level] || '#999' }"></div>
          <div class="event-body">
            <div class="event-top">
              <span class="event-level" :style="levelStyle(event.level)">
                {{ levelLabel(event.level) }}
              </span>
              <el-tag size="small" :type="statusTagTypes[event.status] || 'info'">
                {{ statusLabel(event.status) }}
              </el-tag>
              <span class="event-node" v-if="event.node_name">{{ event.node_name }}</span>
              <span class="event-time">{{ formatTime(event.timestamp) }}</span>
            </div>

            <div class="event-message">{{ event.message || event.desc || event.content || t('alarms.unknownAlarm') }}</div>

            <div class="event-detail">
              <span v-if="event.tag">{{ t('alarms.fields.tag') }}: {{ event.tag }}</span>
              <span v-if="event.value !== null && event.value !== undefined" class="event-value">{{ t('alarms.fields.value') }}: {{ event.value }}</span>
              <span v-if="event.threshold">{{ t('alarms.fields.threshold') }}: {{ event.threshold }}</span>
              <span v-if="event.rule_id">{{ t('alarms.fields.rule') }}: {{ event.rule_id }}</span>
              <span v-if="event.source_type">{{ t('alarms.fields.source') }}: {{ event.source_type }}</span>
            </div>

            <div class="event-actions" v-if="event.id">
              <el-button size="small" text :disabled="!canAck(event)" @click="ackEvent(event)">
                {{ t('alarms.actions.ack') }}
              </el-button>
              <el-button size="small" text :disabled="!canResolve(event)" @click="resolveEvent(event)">
                {{ t('alarms.actions.resolve') }}
              </el-button>
            </div>
          </div>
        </div>
      </transition-group>

      <el-empty v-if="!filteredEvents.length && !loading" :description="t('alarms.noEvents')" />
    </div>
  </div>
</template>

<style scoped>
.alarm-event-list {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 520px;
  border: 1px solid var(--border-subtle, #ddd);
  border-radius: 6px;
  overflow: hidden;
  background: #fff;
}

.list-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  padding: 8px 12px;
  background: #fafafa;
  border-bottom: 1px solid var(--border-subtle, #ddd);
  gap: 8px;
  flex-shrink: 0;
}

.event-error {
  margin: 8px 12px 0;
  flex-shrink: 0;
}

.level-badges {
  display: flex;
  gap: 6px;
  flex-wrap: wrap;
}

.level-badge {
  display: inline-flex;
  align-items: center;
  gap: 4px;
  padding: 2px 8px;
  border-radius: 100px;
  font-size: 12px;
  cursor: pointer;
  border: 1px solid;
  transition: opacity 0.2s;
  user-select: none;
}

.badge-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
}

.badge-count {
  font-weight: 600;
  margin-left: 2px;
}

.header-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}

.event-count {
  font-size: 12px;
  color: #666;
}

.event-scroll {
  flex: 1;
  overflow-y: auto;
  padding: 4px 0;
}

.event-item {
  display: flex;
  align-items: flex-start;
  gap: 0;
  padding: 8px 12px;
  border-bottom: 1px solid #f0f0f0;
  transition: background 0.3s;
}

.event-item:last-child {
  border-bottom: none;
}

.event-item.event-new {
  background: #fffbe6;
  animation: highlight-pulse 2s ease-out forwards;
}

@keyframes highlight-pulse {
  0% { background: #fff7e6; }
  100% { background: transparent; }
}

.event-indicator {
  width: 4px;
  border-radius: 2px;
  align-self: stretch;
  min-height: 40px;
  flex-shrink: 0;
  margin-right: 10px;
}

.event-body {
  flex: 1;
  min-width: 0;
}

.event-top {
  display: flex;
  align-items: center;
  gap: 8px;
  margin-bottom: 2px;
  flex-wrap: wrap;
}

.event-level {
  font-size: 11px;
  padding: 1px 6px;
  border-radius: 4px;
  border: 1px solid;
  font-weight: 600;
}

.event-node {
  font-size: 12px;
  color: #333;
  font-weight: 500;
}

.event-time {
  font-size: 11px;
  color: #999;
  margin-left: auto;
}

.event-message {
  font-size: 13px;
  color: #222;
  line-height: 1.4;
  word-break: break-word;
}

.event-detail {
  font-size: 11px;
  color: #888;
  margin-top: 4px;
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}

.event-actions {
  margin-top: 4px;
  display: flex;
  gap: 4px;
}

.event-value {
  font-family: monospace;
  background: #f5f5f5;
  padding: 0 4px;
  border-radius: 2px;
}

/* Transition animations */
.event-pop-enter-active {
  animation: event-in 0.4s ease-out;
}

@keyframes event-in {
  0% { opacity: 0; transform: translateY(-8px); }
  100% { opacity: 1; transform: translateY(0); }
}

.event-pop-leave-active {
  transition: opacity 0.3s;
}

.event-pop-leave-to {
  opacity: 0;
}
</style>
