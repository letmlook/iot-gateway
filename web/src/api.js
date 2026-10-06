const BASE = '/api/v1'
const TOKEN_KEY = 'gateway_token'

function getStoredToken() {
  try {
    return localStorage.getItem(TOKEN_KEY)
  } catch {
    return null
  }
}

/**
 * API 错误：包含后端返回的 code、message 与 HTTP status，便于前端按 code 做 i18n。
 * @property {string} code   - 错误码（如 config_invalid, not_found）
 * @property {string} message - 原始错误信息
 * @property {number} status - HTTP 状态码
 */
export class ApiError extends Error {
  constructor(code, message, status = 0) {
    super(message)
    this.name = 'ApiError'
    this.code = code
    this.message = message
    this.status = status
  }
}

async function parseErrorResponse(r, text) {
  let code = 'unknown'
  let message = text || r.statusText || 'Request failed'
  if (text) {
    try {
      const json = JSON.parse(text)
      if (json && typeof json.code === 'string') code = json.code
      if (json && typeof json.message === 'string') message = json.message
    } catch (_) {}
  }
  return new ApiError(code, message, r.status)
}

/**
 * 认证失效时的统一处理：清掉本地凭据并跳登录页。
 *
 * 放在请求层而不是各个视图里，是为了避免「有的页面跳登录、有的页面只报错」的不一致。
 * 用一个进程级标记避免并发请求把跳转叠加多次。
 */
let redirecting = false
function handleUnauthorized() {
  try {
    localStorage.removeItem(TOKEN_KEY)
    localStorage.removeItem('gateway_authenticated')
    localStorage.removeItem('gateway_user')
  } catch (_) {}
  if (redirecting) return
  redirecting = true
  // 使用 hash 路由，避免在任意子路径下刷新导致 404
  if (!window.location.hash.startsWith('#/login')) {
    window.location.hash = '#/login'
  }
  setTimeout(() => { redirecting = false }, 1000)
}

/**
 * 统一请求：附带凭据、解析错误体、处理 401/403、支持取消。
 * @param {string} method
 * @param {string} path 形如 `/nodes`
 * @param {object} [body]
 * @param {{signal?: AbortSignal}} [options]
 */
async function req(method, path, body, options = {}) {
  const opts = { method, headers: {} }
  if (options.signal) opts.signal = options.signal
  const token = getStoredToken()
  if (token) {
    opts.headers['Authorization'] = `Bearer ${token}`
  }
  if (body && (method === 'POST' || method === 'PUT')) {
    opts.headers['Content-Type'] = 'application/json'
    opts.body = JSON.stringify(body)
  }
  const r = await fetch(`${BASE}${path}`, opts)
  if (r.status === 204) return null
  const text = await r.text()
  if (!r.ok) {
    const err = await parseErrorResponse(r, text)
    if (r.status === 401) handleUnauthorized()
    // 权限不足要给出可理解的原因：后端返回的是「需要什么角色」这类信息
    if (r.status === 403) {
      err.code = err.code === 'unknown' ? 'forbidden' : err.code
    }
    throw err
  }
  return text ? JSON.parse(text) : null
}

export const api = {
  // 认证
  login: (username, password) =>
    fetch(`${BASE}/auth/login`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ username: username || '', password: password || '' }),
    }).then(async (r) => {
      const text = await r.text()
      if (!r.ok) throw await parseErrorResponse(r, text)
      const data = text ? JSON.parse(text) : null
      if (data && data.token != null) {
        try {
          localStorage.setItem(TOKEN_KEY, data.token)
          if (data.user) localStorage.setItem('gateway_user', data.user.username || '')
          localStorage.setItem('gateway_authenticated', '1')
        } catch (_) {}
      }
      return data
    }),
  isAuthenticated: () => {
    try {
      return (
        localStorage.getItem('gateway_authenticated') === '1' ||
        (getStoredToken() !== null && getStoredToken() !== '')
      )
    } catch {
      return false
    }
  },
  clearAuth: () => {
    try {
      localStorage.removeItem(TOKEN_KEY)
      localStorage.removeItem('gateway_authenticated')
      localStorage.removeItem('gateway_user')
    } catch (_) {}
  },

  // 健康与版本
  health: () => req('GET', '/health'),
  version: () => req('GET', '/version'),
  hardwareInfo: () => req('GET', '/hardware'),
  metrics: () => fetch(`${BASE}/metrics`).then(r => r.text()),
  dataFlow: () => req('GET', '/data-flow'),
  backup: (password) =>
    fetch(`${BASE}/backup`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(password != null && password !== '' ? { password } : {}),
    }).then(async (r) => {
      if (!r.ok) throw await parseErrorResponse(r, await r.text())
      const blob = await r.blob()
      const disposition = r.headers.get('Content-Disposition')
      const match = disposition && disposition.match(/filename="?([^";]+)"?/)
      const filename = match ? match[1].trim() : `gateway-backup-${Date.now()}.bin`
      return { blob, filename }
    }),
  restore: (formData) =>
    fetch(`${BASE}/restore`, {
      method: 'POST',
      body: formData,
    }).then(async (r) => {
      const text = await r.text()
      if (!r.ok) throw await parseErrorResponse(r, text)
      return text ? JSON.parse(text) : null
    }),

  // 插件
  pluginsSouth: () => req('GET', '/plugins/south'),
  pluginsNorth: () => req('GET', '/plugins/north'),
  pluginSouthSchema: (name) => req('GET', `/plugins/south/${name}/config_schema`),
  pluginSouthTagSchema: (name) => req('GET', `/plugins/south/${name}/tag_schema`),
  pluginNorthSchema: (name) => req('GET', `/plugins/north/${name}/config_schema`),

  // 节点 CRUD
  nodes: (params) => req('GET', `/nodes${params ? '?' + new URLSearchParams(params).toString() : ''}`)
    .then(r => r && r.items !== undefined ? r.items : r),
  node: (id) => req('GET', `/nodes/${id}`),
  createNode: (body) => req('POST', '/nodes', body),
  updateNode: (id, body) => req('PUT', `/nodes/${id}`, body),
  deleteNode: (id) => req('DELETE', `/nodes/${id}`),

  // 节点操作
  startNode: (id) => req('POST', `/nodes/${id}/start`),
  stopNode: (id) => req('POST', `/nodes/${id}/stop`),
  nodeConnectionStatus: (id) => req('GET', `/nodes/${id}/connection-status`),
  nodeSetting: (id) => req('GET', `/nodes/${id}/setting`),
  updateNodeSetting: (id, config) => req('PUT', `/nodes/${id}/setting`, config),

  // 组 CRUD
  groups: (nodeId, params) => req('GET', `/nodes/${nodeId}/groups${params ? '?' + new URLSearchParams(params).toString() : ''}`)
    .then(r => r && r.items !== undefined ? r.items : r),
  group: (nodeId, gid) => req('GET', `/nodes/${nodeId}/groups/${gid}`),
  createGroup: (nodeId, body) => req('POST', `/nodes/${nodeId}/groups`, body),
  updateGroup: (nodeId, gid, body) => req('PUT', `/nodes/${nodeId}/groups/${gid}`, body),
  deleteGroup: (nodeId, gid) => req('DELETE', `/nodes/${nodeId}/groups/${gid}`),

  // 标签 CRUD
  tags: (nodeId, params) => req('GET', `/nodes/${nodeId}/tags${params ? '?' + new URLSearchParams(params).toString() : ''}`)
    .then(r => r && r.items !== undefined ? r.items : r),
  tag: (nodeId, tid) => req('GET', `/nodes/${nodeId}/tags/${tid}`),
  createTag: (nodeId, body) => req('POST', `/nodes/${nodeId}/tags`, body),
  batchCreateTags: (nodeId, tags) => req('POST', `/nodes/${nodeId}/tags/batch`, { tags }),
  updateTag: (nodeId, tid, body) => req('PUT', `/nodes/${nodeId}/tags/${tid}`, body),
  deleteTag: (nodeId, tid) => req('DELETE', `/nodes/${nodeId}/tags/${tid}`),

  // 实时值：读采集缓存，不访问设备（管理台刷新用这个）
  nodeValues: (nodeId, options) => req('GET', `/nodes/${nodeId}/values`, undefined, options),

  // 读写标签（read_tags 会真实下发到设备，仅用于「立即读取」这类显式操作）
  readTags: (nodeId, tagIds) => req('POST', `/nodes/${nodeId}/read_tags`, { tag_ids: tagIds }),
  writeTags: (nodeId, values) => req('POST', `/nodes/${nodeId}/write_tags`, { values }),

  // 订阅（body: { subscriptions: [{ south_node_id, group_id }] }）
  subscriptions: (nodeId) => req('GET', `/nodes/${nodeId}/subscriptions`),
  setSubscriptions: (nodeId, subs) => req('PUT', `/nodes/${nodeId}/subscriptions`, { subscriptions: subs }),

  // 节点配置用文件上传：保存到 data/uploads，返回 { path } 供配置存储
  uploadConfigFile: (file) => {
    const formData = new FormData()
    formData.append('file', file)
    const token = getStoredToken()
    const headers = token ? { Authorization: `Bearer ${token}` } : {}
    return fetch(`${BASE}/upload`, { method: 'POST', headers, body: formData }).then(async (r) => {
      const text = await r.text()
      if (!r.ok) throw await parseErrorResponse(r, text)
      const data = text ? JSON.parse(text) : null
      return data && data.path != null ? data.path : ''
    })
  },

  // 离线授权（完全离线，无网络请求）
  licenseMachineId: () => req('GET', '/license/machine-id'),
  licenseStatus: () => req('GET', '/license/status'),
  uploadLicense: (formData) => {
    const token = getStoredToken()
    const headers = token ? { Authorization: `Bearer ${token}` } : {}
    return fetch(`${BASE}/license/upload`, {
      method: 'POST',
      headers,
      body: formData,
    }).then(async (r) => {
      const text = await r.text()
      if (!r.ok) throw await parseErrorResponse(r, text)
      return text ? JSON.parse(text) : null
    })
  },
  resetLicense: () => req('POST', '/license/reset'),

  // 用户管理（需认证）
  users: (params) => req('GET', `/users${params ? '?' + new URLSearchParams(params).toString() : ''}`)
    .then(r => r && r.items !== undefined ? r.items : r),
  createUser: (body) => req('POST', '/users', body),
  getUser: (id) => req('GET', `/users/${id}`),
  updateUser: (id, body) => req('PUT', `/users/${id}`, body),
  deleteUser: (id) => req('DELETE', `/users/${id}`),
  changePassword: (id, password) => req('PUT', `/users/${id}/password`, { password: password || '' }),

  // 日志管理
  downloadLog: (type = 'all') => {
    const token = getStoredToken()
    const headers = token ? { Authorization: `Bearer ${token}` } : {}
    return fetch(`${BASE}/logs/download?type=${type}`, { headers }).then(async (r) => {
      if (!r.ok) throw await parseErrorResponse(r, await r.text())
      const blob = await r.blob()
      const disposition = r.headers.get('Content-Disposition')
      const match = disposition && disposition.match(/filename="?([^";]+)"?/)
      const filename = match ? match[1].trim() : `gateway-${type}.log`
      return { blob, filename }
    })
  },
  getLogConfig: () => req('GET', '/logs/config'),
  setLogConfig: (config) => req('PUT', '/logs/config', config),

  // 系统配置
  getSystemConfig: () => req('GET', '/system/config'),
  setSystemConfig: (config) => req('PUT', '/system/config', config),
}
