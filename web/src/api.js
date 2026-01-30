const BASE = '/api'

async function req(method, path, body) {
  const opts = { method, headers: {} }
  if (body && (method === 'POST' || method === 'PUT')) {
    opts.headers['Content-Type'] = 'application/json'
    opts.body = JSON.stringify(body)
  }
  const r = await fetch(`${BASE}${path}`, opts)
  if (r.status === 204) return null
  const text = await r.text()
  if (!r.ok) throw new Error(text || r.statusText)
  return text ? JSON.parse(text) : null
}

export const api = {
  // 健康与版本
  health: () => req('GET', '/health'),
  version: () => req('GET', '/version'),
  metrics: () => fetch(`${BASE}/metrics`).then(r => r.text()),
  backup: (password) =>
    fetch(`${BASE}/backup`, {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(password != null && password !== '' ? { password } : {}),
    }).then(async (r) => {
      if (!r.ok) throw new Error(await r.text() || r.statusText)
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
      if (!r.ok) throw new Error(text || r.statusText)
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
}
