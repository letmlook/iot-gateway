/**
 * WebSocket 客户端单例。
 *
 * 协议：
 * - 连接地址：/api/v1/ws（Vite proxy ws:true 转发到后端）
 * - 认证：建立连接后客户端立即发送首帧 { type: "auth", token: "Bearer xxx" }
 * - 服务器认证超时 10s，超时则关闭连接
 * - 服务器定期发送心跳 ping，客户端需在 25s 内回应 pong
 * - 客户端每 25s 发一次 ping
 * - 客户端可订阅 topics：node-values, group-values, system-metrics
 *
 * 用法：
 *   import { gatewayWs } from './ws/gatewayWs.js'
 *   gatewayWs.connect()
 *   gatewayWs.on('node-values', (data) => { ... })
 */

const BASE = '/api/v1'

// WS 帧类型
const FrameType = {
  AUTH: 'auth',
  PING: 'ping',
  PONG: 'pong',
  SUBSCRIBE: 'subscribe',
  UNSUBSCRIBE: 'unsubscribe',
  NODE_VALUES: 'node-values',
  GROUP_VALUES: 'group-values',
  SYSTEM_METRICS: 'system-metrics',
  ERROR: 'error',
}

class GatewayWs {
  constructor() {
    this._ws = null
    this._connected = false
    this._authenticated = false
    this._handlers = new Map()          // type -> [handler]
    this._authTimeout = null
    this._pingInterval = null
    this._pongTimer = null
    this._lastServerPingTs = null
    this._closed = false
  }

  /** 连接 WS 并发送认证帧 */
  connect() {
    if (this._ws && this._ws.readyState === WebSocket.OPEN) return
    this._closed = false

    const token = localStorage.getItem('gateway_token') || ''
    const url = `${window.location.protocol === 'https:' ? 'wss:' : 'ws:'}//${window.location.host}${BASE}/ws`
    this._ws = new WebSocket(url)

    this._ws.onopen = () => {
      this._connected = true
      // 立即发送认证帧
      this._send({ type: FrameType.AUTH, token })
      // 10s 认证超时
      this._authTimeout = setTimeout(() => {
        if (!this._authenticated) {
          console.warn('[WS] auth timeout, closing')
          this._ws.close()
        }
      }, 10_000)
    }

    this._ws.onmessage = (evt) => {
      let frame
      try { frame = JSON.parse(evt.data) } catch { return }
      const { type } = frame

      if (type === FrameType.PING) {
        this._lastServerPingTs = Date.now()
        // 25s 内未收到服务器 ping 则认为断连
        clearTimeout(this._pongTimer)
        this._pongTimer = setTimeout(() => {
          console.warn('[WS] server ping timeout, reconnecting')
          this._reconnect()
        }, 25_000)
        // 回应 pong
        this._send({ type: FrameType.PONG })
        return
      }

      if (type === FrameType.PONG) return  // 忽略客户端 ping 的响应

      if (type === FrameType.AUTH) {
        if (frame.success) {
          this._authenticated = true
          this._startPingLoop()
          this._emit('connected', {})
        } else {
          console.error('[WS] auth failed', frame.message)
          this._ws.close()
        }
        return
      }

      // 分发给注册的 handler（最多 256 帧缓冲在边界内）
      const handlers = this._handlers.get(type) || []
      for (const h of handlers) {
        try { h(frame) } catch (e) { console.error('[WS] handler error', e) }
      }
      const wildcard = this._handlers.get('*') || []
      for (const h of wildcard) {
        try { h(frame) } catch (e) { console.error('[WS] handler error', e) }
      }
    }

    this._ws.onerror = (e) => {
      console.error('[WS] error', e)
    }

    this._ws.onclose = () => {
      this._connected = false
      this._authenticated = false
      this._cleanup()
      this._emit('disconnected', {})
      if (!this._closed) {
        // 自动重连
        setTimeout(() => this.connect(), 3000)
      }
    }
  }

  /** 主动关闭（不再自动重连） */
  disconnect() {
    this._closed = true
    if (this._ws) this._ws.close()
  }

  /** 订阅 topics */
  subscribe(...topics) {
    this._send({ type: FrameType.SUBSCRIBE, topics })
  }

  /** 取消订阅 */
  unsubscribe(...topics) {
    this._send({ type: FrameType.UNSUBSCRIBE, topics })
  }

  /** 注册帧处理器 */
  on(type, handler) {
    if (!this._handlers.has(type)) this._handlers.set(type, [])
    this._handlers.get(type).push(handler)
    return () => this.off(type, handler)
  }

  /** 注销帧处理器 */
  off(type, handler) {
    const list = this._handlers.get(type) || []
    this._handlers.set(type, list.filter(h => h !== handler))
  }

  get connected() { return this._connected }
  get authenticated() { return this._authenticated }

  _send(frame) {
    if (this._ws && this._ws.readyState === WebSocket.OPEN) {
      this._ws.send(JSON.stringify(frame))
    }
  }

  _emit(type, data) {
    const handlers = this._handlers.get(type) || []
    for (const h of handlers) {
      try { h(data) } catch (e) { console.error('[WS] emit error', e) }
    }
  }

  _startPingLoop() {
    this._pingInterval = setInterval(() => {
      if (this._authenticated) {
        this._send({ type: FrameType.PING })
      }
    }, 25_000)
  }

  _cleanup() {
    clearTimeout(this._authTimeout)
    clearTimeout(this._pongTimer)
    clearInterval(this._pingInterval)
  }

  _reconnect() {
    this._cleanup()
    if (this._ws) this._ws.close()
    this._connected = false
    this._authenticated = false
    this.connect()
  }
}

// 单例
export const gatewayWs = new GatewayWs()
export { FrameType }
