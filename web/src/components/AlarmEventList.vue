<script setup>
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { api } from '../api.js'

const { t } = useI18n()

// Alarm levels with colors
const LEVELS = ['critical', 'major', 'minor', 'info']

const levelColors = {
  critical: '#ff4d4f',
  major: '#ff7a45',
  minor: '#ffa940',
  info: '#52c41a'
}

const levelLabels = {
  critical: '严重',
  major: '重要',
  minor: '一般',
  info: '提示'
}

// State
const events = ref([])
const levelFilter = ref([])
const maxEvents = 200
let pollTimer = null
let lastPoll = 0
const POLL_INTERVAL = 3000 // 3s

// Load alarm events from API
async function loadEvents() {
  try {
    const data = await api.alarmEvents ? await api.alarmEvents() : { events: [] }
    // Merge new events, mark as new for highlight
    const incoming = (data.events || []).map(e => ({
      ...e,
      isNew: true
    }))
    // Mark existing as not new
    events.value.forEach(e => { e.isNew = false })
    // Prepend new ones
    events.value = [...incoming, ...events.value].slice(0, maxEvents)
    // After 2s, clear isNew flag
    setTimeout(() => {
      events.value.forEach(e => { e.isNew = false })
    }, 2000)
  } catch (e) {
    console.error('Failed to load alarm events', e)
  }
}

function startPolling() {
  stopPolling()
  loadEvents()
  pollTimer = setInterval(loadEvents, POLL_INTERVAL)
}

function stopPolling() {
  if (pollTimer) {
    clearInterval(pollTimer)
    pollTimer = null
  }
}

// Filtered events
const filteredEvents = computed(() => {
  if (!levelFilter.value.length) return events.value
  return events.value.filter(e => levelFilter.value.includes(e.level))
})

// Level badge style
function levelStyle(level) {
  const color = levelColors[level] || '#999'
  return {
    backgroundColor: color + '22',
    color: color,
    borderColor: color + '66'
  }
}

// Format time
function formatTime(ts) {
  if (!ts) return '-'
  const d = new Date(ts)
  return d.toLocaleTimeString()
}

// Toggle level filter
function toggleLevel(level) {
  const idx = levelFilter.value.indexOf(level)
  if (idx >= 0) {
    levelFilter.value.splice(idx, 1)
  } else {
    levelFilter.value.push(level)
  }
}

// Clear all
function clearEvents() {
  events.value = []
}

// Stats
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
    <!-- Header with stats -->
    <div class="list-header">
      <div class="level-badges">
        <span
          v-for="level in LEVELS"
          :key="level"
          class="level-badge"
          :style="[levelStyle(level), { opacity: levelFilter.length && !levelFilter.includes(level) ? 0.35 : 1 }]"
          @click="toggleLevel(level)"
          title="点击筛选"
        >
          <span class="badge-dot" :style="{ backgroundColor: levelColors[level] }"></span>
          {{ levelLabels[level] }}
          <span class="badge-count">{{ stats[level] }}</span>
        </span>
      </div>
      <div class="header-actions">
        <span class="event-count">{{ filteredEvents.length }} 条记录</span>
        <el-button size="small" @click="clearEvents">清空</el-button>
        <el-button size="small" @click="loadEvents">刷新</el-button>
      </div>
    </div>

    <!-- Event list -->
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
                {{ levelLabels[event.level] || event.level }}
              </span>
              <span class="event-node" v-if="event.node_name">{{ event.node_name }}</span>
              <span class="event-time">{{ formatTime(event.timestamp) }}</span>
            </div>
            <div class="event-message">{{ event.message || event.desc || event.content || '未知告警' }}</div>
            <div class="event-detail" v-if="event.detail || event.value">
              <span v-if="event.detail">{{ event.detail }}</span>
              <span v-if="event.value" class="event-value">值: {{ event.value }}</span>
            </div>
          </div>
        </div>
      </transition-group>

      <el-empty v-if="!filteredEvents.length" description="暂无告警事件" />
    </div>
  </div>
</template>

<style scoped>
.alarm-event-list {
  display: flex;
  flex-direction: column;
  height: 100%;
  min-height: 200px;
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
  margin-top: 2px;
  display: flex;
  gap: 8px;
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
