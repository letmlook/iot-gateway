# Alarm Events Page Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a standalone `/monitor/alarms` page that makes persisted alarm events visible and operable through the existing REST alarm lifecycle.

**Architecture:** Reuse the existing `AlarmEventList.vue` component as the core UI and add a small `AlarmEvents.vue` page shell around it. REST polling remains the reliable source of truth; the existing alarm WebSocket stays opportunistic and silent on failure. Routing, navigation, and i18n are updated without touching backend alarm semantics or `LiveMonitor.vue`.

**Tech Stack:** Vue 3 `<script setup>`, Vue Router, vue-i18n, Element Plus, existing `web/src/api.js`, existing Rust alarm REST endpoints.

---

## File Structure

- Create: `web/src/views/AlarmEvents.vue`
  - Owns the standalone page layout and standard `PageHeader` wrapper.
  - Depends on `AlarmEventList.vue` for data loading and event operations.

- Modify: `web/src/components/AlarmEventList.vue`
  - Aligns severity/status labels with backend values.
  - Converts hardcoded Chinese strings to i18n.
  - Updates merge behavior so REST polling refreshes existing event state.
  - Keeps WebSocket failures silent and polling active.

- Modify: `web/src/router.js`
  - Adds authenticated route `/monitor/alarms`.

- Modify: `web/src/App.vue`
  - Adds the Alarm Events navigation item in the system/monitoring section near Data Flow Metrics.

- Modify: `web/src/locales/zh.js`
  - Adds `menu.alarmEvents`, `menu.alarmEventsDesc`, and `alarms.*` translations.

- Modify: `web/src/locales/en.js`
  - Adds English counterparts for the new keys.

- Modify: `docs/product-capability-matrix.md`
  - Updates Alarm events from Missing to Beta/Experimental based on implemented UI visibility.

Notes:

- Git commits require explicit user authorization in this environment. Each task includes a commit command for authorized execution only.
- Do not modify backend alarm store, alarm routes, flow alarm operator, or `LiveMonitor.vue` for this plan.

---

### Task 1: Add the standalone route, page shell, navigation, and i18n keys

**Files:**
- Create: `web/src/views/AlarmEvents.vue`
- Modify: `web/src/router.js`
- Modify: `web/src/App.vue`
- Modify: `web/src/locales/zh.js`
- Modify: `web/src/locales/en.js`

- [ ] **Step 1: Create the standalone Alarm Events view**

Create `web/src/views/AlarmEvents.vue` with this content:

```vue
<script setup>
import { useI18n } from 'vue-i18n'
import PageHeader from '../components/PageHeader.vue'
import AlarmEventList from '../components/AlarmEventList.vue'

const { t } = useI18n()
</script>

<template>
  <div class="alarm-events-page">
    <PageHeader :title="t('alarms.title')" :subtitle="t('alarms.desc')" />

    <div class="alarm-events-card">
      <AlarmEventList />
    </div>
  </div>
</template>

<style scoped>
.alarm-events-page {
  display: flex;
  flex-direction: column;
  min-height: calc(100vh - 120px);
}

.alarm-events-card {
  flex: 1;
  min-height: 0;
}
</style>
```

- [ ] **Step 2: Add the authenticated router entry**

In `web/src/router.js`, add this route after the existing `/monitor/flow` route and before `/flows`:

```js
  {
    path: '/monitor/alarms',
    name: 'AlarmEvents',
    component: () => import('./views/AlarmEvents.vue'),
    meta: { title: '告警事件' }
  },
```

The surrounding route section should become:

```js
  {
    path: '/monitor/flow',
    name: 'DataFlowMetrics',
    component: () => import('./views/DataFlowMetrics.vue'),
    meta: { title: '数据流指标' }
  },
  {
    path: '/monitor/alarms',
    name: 'AlarmEvents',
    component: () => import('./views/AlarmEvents.vue'),
    meta: { title: '告警事件' }
  },
  {
    path: '/flows',
    name: 'FlowList',
    component: () => import('./views/FlowList.vue'),
    meta: { title: '数据流编排' }
  },
```

- [ ] **Step 3: Add the navigation item**

In `web/src/App.vue`, add the route to `systemNavItems` immediately after `/monitor/flow`:

```js
const systemNavItems = [
  { path: '/settings/info', labelKey: 'menu.systemInfo' },
  { path: '/monitor/flow', labelKey: 'menu.dataFlowMetrics' },
  { path: '/monitor/alarms', labelKey: 'menu.alarmEvents' },
  { path: '/settings/license', labelKey: 'menu.license' },
  { path: '/settings/users', labelKey: 'menu.users' },
  { path: '/settings/logs', labelKey: 'menu.logs' },
  { path: '/settings/config', labelKey: 'menu.systemConfig' },
]
```

- [ ] **Step 4: Add Chinese menu and alarms i18n keys**

In `web/src/locales/zh.js`, add these entries inside the existing `menu` object after `dataFlowMetricsDesc`:

```js
    alarmEvents: '告警事件',
    alarmEventsDesc: '告警事件与处理',
```

Then add this new top-level `alarms` object after the existing `liveMonitor` object:

```js
  alarms: {
    title: '告警事件',
    desc: '查看 Flow 运行产生的持久化告警事件，并执行确认或解决操作。',
    eventCount: '{n} 条记录',
    noEvents: '暂无告警事件',
    loadFailed: '加载告警事件失败: ',
    ackFailed: '确认告警失败: ',
    resolveFailed: '解决告警失败: ',
    unknownAlarm: '未知告警',
    filterHint: '点击筛选',
    actions: {
      refresh: '刷新',
      clear: '清空',
      ack: '确认',
      resolve: '解决',
    },
    fields: {
      value: '值',
      status: '状态',
      tag: '点位',
      threshold: '阈值',
      rule: '规则',
      source: '来源',
    },
    levels: {
      critical: '严重',
      high: '高',
      medium: '中',
      low: '低',
      info: '提示',
    },
    status: {
      active: '活跃',
      acknowledged: '已确认',
      resolved: '已解决',
    },
  },
```

Ensure commas are valid around the inserted object. The existing `liveMonitor` object currently ends with `clear: '清除全部',`; after adding `alarms`, the `liveMonitor` closing brace must be followed by a comma.

- [ ] **Step 5: Add English menu and alarms i18n keys**

In `web/src/locales/en.js`, add these entries inside the existing `menu` object after `dataFlowMetricsDesc`:

```js
    alarmEvents: 'Alarm Events',
    alarmEventsDesc: 'Alarm events and actions',
```

Then add this new top-level `alarms` object after the existing `liveMonitor` object:

```js
  alarms: {
    title: 'Alarm Events',
    desc: 'View persisted alarm events generated by Flow runtime and acknowledge or resolve them.',
    eventCount: '{n} records',
    noEvents: 'No alarm events',
    loadFailed: 'Load alarm events failed: ',
    ackFailed: 'Acknowledge alarm failed: ',
    resolveFailed: 'Resolve alarm failed: ',
    unknownAlarm: 'Unknown alarm',
    filterHint: 'Click to filter',
    actions: {
      refresh: 'Refresh',
      clear: 'Clear',
      ack: 'Ack',
      resolve: 'Resolve',
    },
    fields: {
      value: 'Value',
      status: 'Status',
      tag: 'Tag',
      threshold: 'Threshold',
      rule: 'Rule',
      source: 'Source',
    },
    levels: {
      critical: 'Critical',
      high: 'High',
      medium: 'Medium',
      low: 'Low',
      info: 'Info',
    },
    status: {
      active: 'Active',
      acknowledged: 'Acknowledged',
      resolved: 'Resolved',
    },
  },
```

- [ ] **Step 6: Run the frontend build checkpoint**

Run:

```bash
cd web && npm run build
```

Expected result: Vite build completes successfully. If it fails with a syntax error in a locale file, fix the comma/object placement and rerun the command until it passes.

- [ ] **Step 7: Authorized commit checkpoint**

Only if the user explicitly authorizes commits, run:

```bash
git add web/src/views/AlarmEvents.vue web/src/router.js web/src/App.vue web/src/locales/zh.js web/src/locales/en.js
git commit -m "feat(web): add alarm events page shell"
```

Append the required co-author trailer to the commit message when committing:

```text
Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
```

---

### Task 2: Make `AlarmEventList.vue` production-usable for backend alarm events

**Files:**
- Modify: `web/src/components/AlarmEventList.vue`

- [ ] **Step 1: Replace the script section**

Replace lines 1-153 of `web/src/components/AlarmEventList.vue` with this script:

```vue
<script setup>
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage } from 'element-plus'
import { api } from '../api.js'

const { t } = useI18n()

const LEVELS = ['critical', 'high', 'medium', 'low', 'info']

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
    status: String(event?.status || 'active').toLowerCase(),
  }
}

function mergeIncoming(incoming) {
  const normalized = incoming.map(normalizeEvent)
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
    mergeIncoming(data?.items || data?.events || [])
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
```

- [ ] **Step 2: Replace the template section**

Replace the existing `<template>...</template>` block in `web/src/components/AlarmEventList.vue` with this template:

```vue
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
```

- [ ] **Step 3: Patch styles for full-page use and error display**

In the `<style scoped>` block of `web/src/components/AlarmEventList.vue`, update `.alarm-event-list` so it can fill the new page card:

```css
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
```

Add this class after `.list-header`:

```css
.event-error {
  margin: 8px 12px 0;
  flex-shrink: 0;
}
```

Replace the `.event-detail` rule with this version so details wrap cleanly on the standalone page:

```css
.event-detail {
  font-size: 11px;
  color: #888;
  margin-top: 4px;
  display: flex;
  flex-wrap: wrap;
  gap: 8px;
}
```

Add this rule near `.event-detail` so action buttons have spacing:

```css
.event-actions {
  margin-top: 4px;
  display: flex;
  gap: 4px;
}
```

- [ ] **Step 4: Run the frontend build checkpoint**

Run:

```bash
cd web && npm run build
```

Expected result: Vite build completes successfully. If it fails with an undefined identifier in `AlarmEventList.vue`, check that the script and template names match exactly: `statusTagTypes`, `canAck`, `canResolve`, `levelLabel`, `statusLabel`, `loadEvents`, `loading`, and `error`.

- [ ] **Step 5: Authorized commit checkpoint**

Only if the user explicitly authorizes commits, run:

```bash
git add web/src/components/AlarmEventList.vue
git commit -m "fix(web): make alarm event list use persisted lifecycle"
```

Append the required co-author trailer to the commit message when committing:

```text
Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
```

---

### Task 3: Update the capability matrix and run final verification

**Files:**
- Modify: `docs/product-capability-matrix.md`

- [ ] **Step 1: Update the Product Areas row**

In `docs/product-capability-matrix.md`, replace this row:

```md
| Alarm events | Missing | Persist/query/ack/resolve/WebSocket lifecycle planned. |
```

with:

```md
| Alarm events | Beta | Basic persisted lifecycle exists: create/list/ack/resolve plus standalone UI visibility. Query filters, dedupe/recovery semantics, and WebSocket auth hardening remain future work. |
```

- [ ] **Step 2: Update the v0.6 acceptance checklist wording**

In the v0.6 required checklist, replace this bullet:

```md
- Alarm events have at least a basic persisted lifecycle.
```

with:

```md
- Alarm events have at least a basic persisted lifecycle and standalone UI visibility.
```

- [ ] **Step 3: Run frontend build**

Run:

```bash
cd web && npm run build
```

Expected result: Vite build completes successfully.

- [ ] **Step 4: Run the alarm store test**

Run:

```bash
cargo test -p gateway-server alarm_store_persists_and_lists_events
```

Expected result: the named test passes. If unrelated compilation errors appear, capture the exact output in the final implementation summary.

- [ ] **Step 5: Manual route smoke test if the app is already running**

If a gateway server and frontend are already running in the session, open `/monitor/alarms` in the browser and verify:

- The page loads without router errors.
- The page sends `GET /api/alarm/events`.
- Empty state renders when no events exist.
- Existing events show severity/status labels when events exist.

If the app is not already running, do not start it unless the user asks for manual verification.

- [ ] **Step 6: Authorized commit checkpoint**

Only if the user explicitly authorizes commits, run:

```bash
git add docs/product-capability-matrix.md
git commit -m "docs: update alarm visibility capability status"
```

Append the required co-author trailer to the commit message when committing:

```text
Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>
```

---

## Self-Review Checklist

- Spec coverage:
  - Standalone `/monitor/alarms` route: Task 1.
  - Navigation entry: Task 1.
  - New `AlarmEvents.vue` view: Task 1.
  - Improved backend-compatible `AlarmEventList.vue`: Task 2.
  - REST polling reliable path: Task 2.
  - WebSocket opportunistic and silent failure: Task 2.
  - zh/en i18n: Task 1 and Task 2.
  - Product capability matrix update: Task 3.
  - No `LiveMonitor.vue` changes: File structure and scope notes.
  - No backend query/auth/lifecycle changes: File structure and scope notes.

- Placeholder scan:
  - This plan contains no placeholder sections, incomplete requirements, or deferred implementation details inside the in-scope work.

- Type and property consistency:
  - Backend event fields used consistently: `id`, `level`, `severity`, `status`, `timestamp`, `created_at`, `message`, `tag`, `value`, `threshold`, `rule_id`, `source_type`.
  - Frontend helper names used consistently between script/template: `LEVELS`, `levelColors`, `statusTagTypes`, `loading`, `error`, `loadEvents`, `levelLabel`, `statusLabel`, `levelStyle`, `formatTime`, `toggleLevel`, `canAck`, `canResolve`, `ackEvent`, `resolveEvent`, `clearEvents`, `stats`, `filteredEvents`.
