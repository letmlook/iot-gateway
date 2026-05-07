<script setup>
defineProps({
  runningCount: { type: Number, default: 0 },
  totalCount: { type: Number, default: 0 },
  uptime: { type: String, default: '' },
  lastRefresh: { type: String, default: '' },
  dataRate: { type: Number, default: 0 },
})
</script>

<template>
  <footer class="app-statusbar">
    <span class="statusbar-item">
      <span class="status-dot" :class="{ running: runningCount > 0 }"></span>
      Nodes: {{ runningCount }}/{{ totalCount }} running
    </span>
    <span v-if="dataRate > 0" class="statusbar-item">
      Data: {{ dataRate }} pts/s
    </span>
    <span v-if="uptime" class="statusbar-item">
      Uptime: {{ uptime }}
    </span>
    <span style="flex:1"></span>
    <span v-if="lastRefresh" class="statusbar-item">
      Last refresh: {{ lastRefresh }}
    </span>
  </footer>
</template>

<style scoped>
.statusbar-item {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  white-space: nowrap;
}
</style>
