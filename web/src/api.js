const BASE = '/api'
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

async function req(method, path, body) {
  const opts = { method, headers: {} }
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
  if (!r.ok) throw await parseErrorResponse(r, text)
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
  nodes: () => req('GET', '/nodes'),
  node: (id) => req('GET', `/nodes/${id}`),
  createNode: (body) => req('POST', '/nodes', body),
  updateNode: (id, body) => req('PUT', `/nodes/${id}`, body),
  deleteNode: (id) => req('DELETE', `/nodes/${id}`),

  // 节点操作
  startNode: (id) => req('POST', `/nodes/${id}/start`),
  stopNode: (id) => req('POST', `/nodes/${id}/stop`),
  nodeSetting: (id) => req('GET', `/nodes/${id}/setting`),
  updateNodeSetting: (id, config) => req('PUT', `/nodes/${id}/setting`, config),

  // 组 CRUD
  groups: (nodeId) => req('GET', `/nodes/${nodeId}/groups`),
  group: (nodeId, gid) => req('GET', `/nodes/${nodeId}/groups/${gid}`),
  createGroup: (nodeId, body) => req('POST', `/nodes/${nodeId}/groups`, body),
  updateGroup: (nodeId, gid, body) => req('PUT', `/nodes/${nodeId}/groups/${gid}`, body),
  deleteGroup: (nodeId, gid) => req('DELETE', `/nodes/${nodeId}/groups/${gid}`),

  // 标签 CRUD
  tags: (nodeId) => req('GET', `/nodes/${nodeId}/tags`),
  tag: (nodeId, tid) => req('GET', `/nodes/${nodeId}/tags/${tid}`),
  createTag: (nodeId, body) => req('POST', `/nodes/${nodeId}/tags`, body),
  batchCreateTags: (nodeId, tags) => req('POST', `/nodes/${nodeId}/tags/batch`, { tags }),
  updateTag: (nodeId, tid, body) => req('PUT', `/nodes/${nodeId}/tags/${tid}`, body),
  deleteTag: (nodeId, tid) => req('DELETE', `/nodes/${nodeId}/tags/${tid}`),

  // 读写标签
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
    const headers = {}
    if (token) headers['Authorization'] = `Bearer ${token}`
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
    const headers = {}
    if (token) {
      headers['Authorization'] = `Bearer ${token}`
    }
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
  users: () => req('GET', '/users'),
  createUser: (body) => req('POST', '/users', body),
  getUser: (id) => req('GET', `/users/${id}`),
  updateUser: (id, body) => req('PUT', `/users/${id}`, body),
  deleteUser: (id) => req('DELETE', `/users/${id}`),
  changePassword: (id, password) => req('PUT', `/users/${id}/password`, { password: password || '' }),

  // 日志管理
  downloadLog: (type = 'all') => {
    const token = getStoredToken()
    const headers = {}
    if (token) {
      headers['Authorization'] = `Bearer ${token}`
    }
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
