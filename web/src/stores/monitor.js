/**
 * 数据监控状态：节点/组/标签树 + 实时值。
 * 支持 WebSocket 实时推送，也支持 HTTP 轮询降级。
 */
import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { api } from '../api.js'

export const useMonitorStore = defineStore('monitor', () => {
  // State
  const nodes = ref([])          // south nodes with groups/tags
  const groups = ref([])         // current node's groups
  const tags = ref([])           // current group's tags
  const tagValues = ref({})      // tagId -> { value, ts, available }
  const selectedNodeId = ref(null)
  const selectedGroupId = ref(null)
  const loading = ref(false)
  const error = ref(null)
  const connected = ref(false)   // WS connection status
  let pollTimer = null
  let ws = null

  // Getters
  const southNodes = computed(() => nodes.value.filter(n => n.kind === 'south'))
  const currentNode = computed(() => nodes.value.find(n => n.id === selectedNodeId.value))
  const currentGroup = computed(() => groups.value.find(g => g.id === selectedGroupId.value))

  // Actions
  async function loadNodes() {
    try {
      const all = await api.nodes()
      nodes.value = all.filter(n => n.kind === 'south')
      if (nodes.value.length && !selectedNodeId.value) {
        selectedNodeId.value = nodes.value[0].id
      }
    } catch (e) {
      error.value = e
    }
  }

  async function loadGroups(nodeId) {
    if (!nodeId) return
    try {
      groups.value = await api.groups(nodeId).catch(() => [])
      if (groups.value.length && !selectedGroupId.value) {
        selectedGroupId.value = groups.value[0].id
      }
    } catch (e) {
      error.value = e
    }
  }

  async function loadTags(nodeId, groupId) {
    if (!nodeId || !groupId) return
    try {
      tags.value = await api.tags(nodeId, { group_id: groupId }).catch(() => [])
    } catch (e) {
      error.value = e
    }
  }

  async function selectNode(nodeId) {
    selectedNodeId.value = nodeId
    selectedGroupId.value = null
    tags.value = []
    tagValues.value = {}
    await loadGroups(nodeId)
  }

  async function selectGroup(groupId) {
    selectedGroupId.value = groupId
    await loadTags(selectedNodeId.value, groupId)
  }

  // HTTP 轮询降级
  async function pollValues() {
    if (!selectedNodeId.value) return
    try {
      const vals = await api.nodeValues(selectedNodeId.value)
      if (Array.isArray(vals)) {
        for (const v of vals) {
          tagValues.value[v.tagId] = v
        }
      }
    } catch (e) {
      console.error('pollValues failed', e)
    }
  }

  function startPolling(intervalMs = 2000) {
    stopPolling()
    pollTimer = setInterval(pollValues, intervalMs)
  }

  function stopPolling() {
    if (pollTimer) {
      clearInterval(pollTimer)
      pollTimer = null
    }
  }

  // WS 占位（后续 gatewayWs.js 实现后接入）
  function connectWs() {
    // TODO: gatewayWs.js
  }
  function disconnectWs() {
    if (ws) {
      ws.close()
      ws = null
      connected.value = false
    }
  }

  function updateTagValue(tagId, value) {
    tagValues.value[tagId] = value
  }

  return {
    // State
    nodes,
    groups,
    tags,
    tagValues,
    selectedNodeId,
    selectedGroupId,
    loading,
    error,
    connected,
    // Getters
    southNodes,
    currentNode,
    currentGroup,
    // Actions
    loadNodes,
    loadGroups,
    loadTags,
    selectNode,
    selectGroup,
    pollValues,
    startPolling,
    stopPolling,
    connectWs,
    disconnectWs,
    updateTagValue,
  }
})
