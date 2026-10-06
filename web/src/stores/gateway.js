/**
 * 网关全局状态：节点、插件、健康状态。
 * 所有需要这些数据的组件通过 storeToRefs(gateway) 获取响应式引用。
 */
import { defineStore } from 'pinia'
import { ref, computed } from 'vue'
import { api } from '../api.js'

export const useGatewayStore = defineStore('gateway', () => {
  // State
  const southPlugins = ref([])
  const northPlugins = ref([])
  const nodes = ref([])
  const health = ref(null)
  const lastRefresh = ref('')
  const loading = ref(false)
  const error = ref(null)

  // Getters
  const southNodes = computed(() => nodes.value.filter(n => n.kind === 'south'))
  const northNodes = computed(() => nodes.value.filter(n => n.kind === 'north'))
  const runningCount = computed(() => nodes.value.filter(n => n.state === 'running').length)
  const totalCount = computed(() => nodes.value.length)

  // Actions
  async function loadAll() {
    loading.value = true
    error.value = null
    try {
      const [sp, np, nd, h] = await Promise.all([
        api.pluginsSouth().catch(() => []),
        api.pluginsNorth().catch(() => []),
        api.nodes().catch(() => []),
        api.health().catch(() => null),
      ])
      southPlugins.value = sp
      northPlugins.value = np
      nodes.value = nd
      health.value = h
      lastRefresh.value = new Date().toLocaleTimeString()
    } catch (e) {
      error.value = e
    } finally {
      loading.value = false
    }
  }

  async function loadNodes() {
    try {
      nodes.value = await api.nodes().catch(() => [])
    } catch (e) {
      console.error('loadNodes failed', e)
    }
  }

  async function loadPlugins() {
    try {
      const [sp, np] = await Promise.all([
        api.pluginsSouth().catch(() => []),
        api.pluginsNorth().catch(() => []),
      ])
      southPlugins.value = sp
      northPlugins.value = np
    } catch (e) {
      console.error('loadPlugins failed', e)
    }
  }

  async function loadHealth() {
    try {
      health.value = await api.health().catch(() => null)
    } catch (e) {
      console.error('loadHealth failed', e)
    }
  }

  // Update a single node in the list (e.g., after start/stop)
  function updateNode(updated) {
    const idx = nodes.value.findIndex(n => n.id === updated.id)
    if (idx >= 0) {
      nodes.value[idx] = updated
    }
  }

  // Remove a node from the list
  function removeNode(nodeId) {
    nodes.value = nodes.value.filter(n => n.id !== nodeId)
  }

  // Add a new node
  function addNode(node) {
    nodes.value.push(node)
  }

  return {
    // State
    southPlugins,
    northPlugins,
    nodes,
    health,
    lastRefresh,
    loading,
    error,
    // Getters
    southNodes,
    northNodes,
    runningCount,
    totalCount,
    // Actions
    loadAll,
    loadNodes,
    loadPlugins,
    loadHealth,
    updateNode,
    removeNode,
    addNode,
  }
})
