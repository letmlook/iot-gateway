/**
 * useFlowWebSocket.js
 * 
 * Composable for real-time flow data via WebSocket with:
 * - BroadcastChannel multi-tab sharing
 * - Message throttling (max 100 msg/s)
 * - ECharts sampling (max 500 points per line)
 * - Page Visibility API pause when tab hidden
 */
import { ref, onMounted, onUnmounted, computed } from 'vue'

// Throttle: ensures at most N calls per second
function createThrottle(fn, maxPerSec) {
  const interval = 1000 / maxPerSec
  let lastCall = 0
  let timer = null
  let lastArgs = null
  function throttled(...args) {
    const now = Date.now()
    lastArgs = args
    if (now - lastCall >= interval) {
      lastCall = now
      fn.apply(this, args)
    } else if (!timer) {
      timer = setTimeout(() => {
        lastCall = Date.now()
        timer = null
        if (lastArgs) fn.apply(this, lastArgs)
      }, interval - (now - lastCall))
    }
  }
  throttled.cancel = () => {
    if (timer) { clearTimeout(timer); timer = null }
  }
  return throttled
}

// Sliding window sample: keep at most maxPoints per series
function sampleData(seriesData, maxPoints = 500) {
  if (!Array.isArray(seriesData) || seriesData.length <= maxPoints) return seriesData
  const step = seriesData.length / maxPoints
  return seriesData.filter((_, i) => i % step < 1).slice(-maxPoints)
}

export function useFlowWebSocket(flowId, options = {}) {
  const {
    maxMsgPerSec = 100,
    maxChartPoints = 500,
    onMessage = null,
    onConnect = null,
    onDisconnect = null,
  } = options

  const wsUrl = ref('')
  const connected = ref(false)
  const latestData = ref(null)
  const chartSeries = ref({}) // { nodeId: [{ timestamp, value }] }
  const pendingMessages = ref([])
  const hidden = ref(!document.hidden)

  let ws = null
  let reconnectTimer = null
  let broadcastChannel = null
  let throttledHandler = null

  // Visibility change handler
  function onVisibilityChange() {
    hidden.value = document.hidden
    if (!document.hidden && connected.value) {
      // Tab became visible again — request fresh data
      sendToChannel({ type: 'request_sync', flowId: flowId.value })
    }
  }

  // BroadcastChannel for multi-tab sharing
  function setupBroadcastChannel() {
    try {
      const channelName = `flow_ws_${flowId.value}`
      broadcastChannel = new BroadcastChannel(channelName)
      broadcastChannel.onmessage = (event) => {
        const msg = event.data
        if (!msg || msg.flowId !== flowId.value) return
        if (msg.type === 'ws_message' && onMessage) {
          onMessage(msg.data)
        } else if (msg.type === 'request_sync' && connected.value) {
          // Another tab needs the latest data — push current state
          broadcastCurrentState()
        }
      }
    } catch (e) {
      console.warn('BroadcastChannel not supported:', e)
    }
  }

  function sendToChannel(data) {
    try {
      broadcastChannel?.postMessage({ ...data, flowId: flowId.value })
    } catch (_) {}
  }

  function broadcastCurrentState() {
    if (latestData.value) {
      sendToChannel({ type: 'ws_message', data: latestData.value })
    }
  }

  // Connect WebSocket
  function connect(url) {
    if (ws) ws.close()
    wsUrl.value = url

    try {
      ws = new WebSocket(url)

      ws.onopen = () => {
        connected.value = true
        if (onConnect) onConnect()
        // Announce to other tabs
        sendToChannel({ type: 'connected' })
      }

      ws.onclose = () => {
        connected.value = false
        if (onDisconnect) onDisconnect()
        sendToChannel({ type: 'disconnected' })
        // Auto reconnect after 3s
        reconnectTimer = setTimeout(() => {
          if (!hidden.value) connect(url)
        }, 3000)
      }

      ws.onerror = (e) => {
        console.error('WebSocket error', e)
      }

      // Throttled message handler — max 100msg/s
      throttledHandler = createThrottle((rawData) => {
        latestData.value = rawData
        // Process chart data
        processChartData(rawData)
        // Broadcast to other tabs
        sendToChannel({ type: 'ws_message', data: rawData })
        // Call user callback
        if (onMessage) onMessage(rawData)
      }, maxMsgPerSec)

      ws.onmessage = (event) => {
        if (hidden.value) return // Skip processing when tab hidden
        try {
          const data = JSON.parse(event.data)
          throttledHandler(data)
        } catch (e) {
          console.error('Failed to parse WS message', e)
        }
      }
    } catch (e) {
      console.error('WebSocket connect error', e)
    }
  }

  // Process data into chart series
  function processChartData(data) {
    if (!data || hidden.value) return
    // Expecting data: { node_id, timestamp, values: [{ tag, value }] } or similar
    const nodeId = data.node_id || data.nodeId || 'default'
    const timestamp = data.timestamp || Date.now()
    const values = data.values || data.data || []

    const current = chartSeries.value[nodeId] || []

    if (Array.isArray(values)) {
      values.forEach(v => {
        const tag = v.tag || v.name || 'unknown'
        const key = `${nodeId}:${tag}`
        const point = [timestamp, v.value ?? v.val ?? 0]
        if (!chartSeries.value[key]) {
          chartSeries.value[key] = []
        }
        chartSeries.value[key].push(point)
        // Sample to maxChartPoints
        if (chartSeries.value[key].length > maxChartPoints) {
          chartSeries.value[key] = sampleData(chartSeries.value[key], maxChartPoints)
        }
      })
    } else {
      // Single value
      current.push([timestamp, values.value ?? values])
      chartSeries.value[nodeId] = sampleData(current, maxChartPoints)
    }
  }

  function disconnect() {
    if (reconnectTimer) clearTimeout(reconnectTimer)
    reconnectTimer = null
    if (ws) {
      ws.onclose = null // Prevent auto-reconnect
      ws.close()
      ws = null
    }
    connected.value = false
    sendToChannel({ type: 'disconnected' })
  }

  // Get sampled series data for ECharts
  function getChartSeries(tagKey) {
    return chartSeries.value[tagKey] || []
  }

  // Summary stats
  const msgPerSec = computed(() => {
    // Rough estimate based on connected state
    return connected.value ? `${maxMsgPerSec}/s max` : '0'
  })

  onMounted(() => {
    document.addEventListener('visibilitychange', onVisibilityChange)
    setupBroadcastChannel()
  })

  onUnmounted(() => {
    document.removeEventListener('visibilitychange', onVisibilityChange)
    disconnect()
    broadcastChannel?.close()
    throttledHandler?.cancel?.()
  })

  return {
    wsUrl,
    connected,
    latestData,
    chartSeries,
    hidden,
    msgPerSec,
    connect,
    disconnect,
    getChartSeries,
  }
}
