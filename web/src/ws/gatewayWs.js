/**
 * WebSocket 客户端单例。
 *
 * 协议：
 * - 连接地址：/api/v1/ws（Vite proxy ws:true 转发到后端）
 * - 认证：建立连接后客户端立即发送首帧 { type: "auth", token: "Bearer xxx" }
 * - 服务器认证超时 10s，超时则关闭连接
 * - 服务器定期发送心跳 ping，客户端需在 25s 内回应 pong
 * - 客户端每 25s 发一次 ping
 * - 客户端可订阅 topics：values, nodes
 *
 * 重连策略：指数退避 1s → 2s → 4s → ... → 30s 封顶，成功后重置
 * visibilitychange：页面恢复可见时若已断则立即重连
 *
 * 用法：
 *   import { gatewayWs } from './ws/gatewayWs.js'
 *   gatewayWs.connect()
 *   gatewayWs.on('values', (data) => { ... })
 */

const BASE = '/api/v1'

// WS 帧类型（与后端 WsClientFrame serde tag 一致）
const FrameType = {
  AUTH: 'auth',
  PING: 'ping',
  PONG: 'pong',
  SUBSCRIBE: 'subscribe',
  UNSUBSCRIBE: 'unsubscribe',
  VALUES: 'values',
  NODES: 'nodes',
  HELLO: 'hello',
  ERROR: 'error',
}

class GatewayWs {
  constructor() {
    this._ws = null
    this._state = 'closed'   // 'connecting' | 'open' | 'closed' | 'error'
    this._authenticated = false
    this._handlers = new Map()          // type -> [handler]
    this._authTimeout = null
    this._pingInterval = null
    this._pongTimer = null
    this._lastServerPingTs = null
    this._closed = false
    // 重连状态
    this._reconnectDelay = 1000        // 指数退避：1s 起步
    this._reconnectMaxDelay = 30000   // 30s 封顶
    this._reconnectTimer = null
    this._lastSubscription = null      // 记住上次订阅以便重连后恢复
    // 绑定 visibility handler
    this._onVisibilityChange = this._onVisibilityChange.bind(this)
  }

  /** 连接 WS 并发送认证帧 */
  connect() {
    if (this._ws && (this._ws.readyState === WebSocket.CONNECTING || this._ws.readyState === WebSocket.OPEN)) {
      return
    }
    this._closed = false
    this._state = 'connecting'

    const token = localStorage.getItem('gateway_token') || ''
    const url = `${window.location.protocol === 'https:' ? 'wss:' : 'ws:'}//${window.location.host}${BASE}/ws`
    this._ws = new WebSocket(url)

    this._ws.onopen = () => {
      this._state = 'open'
      this._reconnectDelay = 1000  // 重置退避
      // 立即发送认证帧
      this._send({ type: FrameType.AUTH, token })
      // 10s 认证超时
      this._authTimeout = setTimeout(() => {
        if (!this._authenticated) {
          console.warn('[WS] auth timeout, closing')
          this._ws.close()
        }
      }, 10_000)
      // 监听页面可见性
      document.addEventListener('visibilitychange', this._onVisibilityChange)
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
          // 重连后恢复订阅
          if (this._lastSubscription) {
            this._send(this._lastSubscription)
          }
        } else {
          console.error('[WS] auth failed', frame.message)
          this._ws.close()
        }
        return
      }

      if (type === FrameType.HELLO) {
        // 服务器 hello 帧（认证成功后发送）
        this._emit('hello', frame)
        return
      }

      // 分发给注册的 handler
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
      this._state = 'error'
    }

    this._ws.onclose = (evt) => {
      this._state = 'closed'
      this._authenticated = false
      this._cleanup()
      document.removeEventListener('visibilitychange', this._onVisibilityChange)
      this._emit('disconnected', { code: evt.code })
      if (!this._closed) {
        // 指数退避重连
        this._scheduleReconnect()
      }
    }
  }

  /** 页面可见性变化时：若已断则立即重连 */
  _onVisibilityChange() {
    if (document.visibilityState === 'visible' && this._state !== 'open' && !this._closed) {
      console.debug('[WS] visibility change: reconnecting immediately')
      this._reconnectDelay = 1000  // 立即重连用最小延迟
      this._reconnect()
    }
  }

  /** 调度指数退避重连 */
  _scheduleReconnect() {
    clearTimeout(this._reconnectTimer)
    this._reconnectTimer = setTimeout(() => {
      if (!this._closed) {
        console.debug(`[WS] reconnecting in ${this._reconnectDelay}ms`)
        this.connect()
      }
    }, this._reconnectDelay)
    // 下次延迟翻倍，上限 30s
    this._reconnectDelay = Math.min(this._reconnectDelay * 2, this._reconnectMaxDelay)
  }

  /** 主动关闭（不再自动重连） */
  disconnect() {
    this._closed = true
    this._reconnectDelay = 1000
    clearTimeout(this._reconnectTimer)
    if (this._ws) this._ws.close()
    document.removeEventListener('visibilitychange', this._onVisibilityChange)
  }

  /** 订阅 topics（记住以便重连后恢复） */
  subscribe(topics, nodeIds = null, groupIds = null) {
    const frame = { type: FrameType.SUBSCRIBE, topics }
    if (nodeIds) frame.nodeIds = nodeIds
    if (groupIds) frame.groupIds = groupIds
    this._lastSubscription = frame
    this._send(frame)
  }

  /** 取消订阅 */
  unsubscribe(topics) {
    this._lastSubscription = null
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

  /** 当前状态：'connecting' | 'open' | 'closed' | 'error' */
  get state() { return this._state }
  get connected() { return this._state === 'open' }
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
    clearTimeout(this._reconnectTimer)
    clearInterval(this._pingInterval)
  }

  _reconnect() {
    this._cleanup()
    if (this._ws) this._ws.close()
    this._state = 'closed'
    this._authenticated = false
    this.connect()
  }
}

// 单例
export const gatewayWs = new GatewayWs()
export { FrameType }
