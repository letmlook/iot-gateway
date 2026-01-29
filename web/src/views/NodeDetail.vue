<script setup>
import { ref, onMounted, computed, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Back, Setting, VideoPlay, VideoPause, Plus, Edit, Delete, RefreshRight, EditPen } from '@element-plus/icons-vue'
import { api } from '../api.js'
import NodeConfigForm from '../components/NodeConfigForm.vue'

const route = useRoute()
const router = useRouter()

const nodeId = computed(() => route.params.id)
const isNorth = computed(() => route.path.startsWith('/north'))

const node = ref(null)
const groups = ref([])
const tags = ref([])
const subscriptions = ref([])
const southNodes = ref([])
const loading = ref(false)
const error = ref('')
const activeTab = ref('groups')

// 模态框状态
const showGroupModal = ref(false)
const showTagModal = ref(false)
const showSubModal = ref(false)
const showSettingModal = ref(false)
const showNameModal = ref(false)
const editingGroup = ref(null)
const editingTag = ref(null)
const editNameValue = ref('')

// 表单数据
const groupForm = ref({ name: '', interval_ms: 1000, description: '' })
const tagForm = ref({ name: '', group_id: '', data_type: 'Float64', address: '', attr: 'read', description: '' })
const attrOptions = [
  { value: 'read', label: 'Read' },
  { value: 'write', label: 'Write' },
  { value: 'readwrite', label: 'ReadWrite' }
]
const showBatchTagModal = ref(false)
const batchTagGroupId = ref('')
const batchTagRows = ref([{ name: '', data_type: 'Float64', address: '', attr: 'read', description: '' }])
const settingForm = ref('')
/** 节点配置弹窗：表单模式下的配置对象（由 NodeConfigForm 填充） */
const settingConfigFromForm = ref({})
/** 节点配置弹窗：表单 / JSON 模式 */
const settingEditMode = ref('json') // 'form' | 'json'
const subForm = ref({ south_node_id: '', group_id: '' })
const southGroups = ref([])
const groupsBySouth = ref({})

const dataTypes = ['Bool', 'Int8', 'Int16', 'Int32', 'Int64', 'UInt8', 'UInt16', 'UInt32', 'UInt64', 'Float32', 'Float64', 'String', 'Bytes']

async function loadNode() {
  loading.value = true
  error.value = ''
  try {
    node.value = await api.node(nodeId.value)
    await loadGroups()
    await loadTags()
    if (isNorth.value) {
      await loadSubscriptions()
      await loadSouthNodes()
    }
  } catch (e) {
    error.value = '加载节点信息失败: ' + e.message
  } finally {
    loading.value = false
  }
}

async function loadGroups() {
  try {
    groups.value = await api.groups(nodeId.value)
  } catch (e) {
    console.error('加载组失败', e)
  }
}

async function loadTags() {
  try {
    tags.value = await api.tags(nodeId.value)
  } catch (e) {
    console.error('加载标签失败', e)
  }
}

async function loadSubscriptions() {
  try {
    subscriptions.value = await api.subscriptions(nodeId.value)
    if (isNorth.value && subscriptions.value.length) {
      const sids = [...new Set(subscriptions.value.map(s => s.south_node_id))]
      for (const sid of sids) {
        if (!groupsBySouth.value[sid]) {
          try {
            const gs = await api.groups(sid)
            groupsBySouth.value[sid] = gs
          } catch {
            groupsBySouth.value[sid] = []
          }
        }
      }
    }
  } catch (e) {
    console.error('加载订阅失败', e)
  }
}

async function loadSouthNodes() {
  try {
    const all = await api.nodes()
    southNodes.value = all.filter(n => n.kind === 'south')
  } catch (e) {
    console.error('加载南向节点失败', e)
  }
}

async function loadSouthGroups(southId) {
  if (!southId) {
    southGroups.value = []
    return
  }
  try {
    southGroups.value = await api.groups(southId)
  } catch (e) {
    console.error('加载南向组失败', e)
    southGroups.value = []
  }
}

// 组操作
function openGroupModal(group = null) {
  editingGroup.value = group
  if (group) {
    groupForm.value = { name: group.name, interval_ms: group.interval_ms || 1000, description: group.description || '' }
  } else {
    groupForm.value = { name: '', interval_ms: 1000, description: '' }
  }
  showGroupModal.value = true
}

async function saveGroup() {
  try {
    if (editingGroup.value) {
      await api.updateGroup(nodeId.value, editingGroup.value.id, groupForm.value)
    } else {
      await api.createGroup(nodeId.value, groupForm.value)
    }
    showGroupModal.value = false
    await loadGroups()
    ElMessage.success('保存成功')
  } catch (e) {
    error.value = '保存组失败: ' + e.message
  }
}

async function deleteGroup(gid) {
  try {
    await ElMessageBox.confirm('确定删除该组？组内标签也会被删除。', '确认删除', { type: 'warning', confirmButtonText: '删除', cancelButtonText: '取消' })
    await api.deleteGroup(nodeId.value, gid)
    await loadGroups()
    await loadTags()
    ElMessage.success('已删除')
  } catch (e) {
    if (e !== 'cancel') error.value = '删除组失败: ' + e.message
  }
}

// 标签操作
function openTagModal(tag = null) {
  editingTag.value = tag
  if (tag) {
    tagForm.value = {
      name: tag.name,
      group_id: tag.group_id,
      data_type: tag.data_type || 'Float64',
      address: tag.address || '',
      attr: (tag.attr || 'read').toLowerCase(),
      description: tag.description || ''
    }
  } else {
    tagForm.value = {
      name: '',
      group_id: groups.value[0]?.id || '',
      data_type: 'Float64',
      address: '',
      attr: 'read',
      description: ''
    }
  }
  showTagModal.value = true
}

async function saveTag() {
  try {
    const payload = { ...tagForm.value, attr: tagForm.value.attr }
    if (editingTag.value) {
      await api.updateTag(nodeId.value, editingTag.value.id, payload)
    } else {
      await api.createTag(nodeId.value, payload)
    }
    showTagModal.value = false
    await loadTags()
    ElMessage.success('保存成功')
  } catch (e) {
    error.value = '保存标签失败: ' + e.message
  }
}

async function deleteTag(tid) {
  try {
    await ElMessageBox.confirm('确定删除该标签？', '确认删除', { type: 'warning', confirmButtonText: '删除', cancelButtonText: '取消' })
    await api.deleteTag(nodeId.value, tid)
    await loadTags()
    ElMessage.success('已删除')
  } catch (e) {
    if (e !== 'cancel') error.value = '删除标签失败: ' + e.message
  }
}

// 读取标签值
const tagValues = ref({})
const readingTags = ref(false)

async function readAllTags() {
  if (!tags.value.length) return
  readingTags.value = true
  try {
    const ids = tags.value.map(t => t.id)
    const result = await api.readTags(nodeId.value, ids)
    tagValues.value = {}
    result.forEach(([tid, val]) => {
      tagValues.value[tid] = val
    })
  } catch (e) {
    error.value = '读取标签失败: ' + e.message
  } finally {
    readingTags.value = false
  }
}

// 订阅操作
function openSubModal() {
  subForm.value = { south_node_id: southNodes.value[0]?.id || '', group_id: '' }
  if (subForm.value.south_node_id) {
    loadSouthGroups(subForm.value.south_node_id)
  }
  showSubModal.value = true
}

watch(() => subForm.value.south_node_id, (newVal) => {
  loadSouthGroups(newVal)
  subForm.value.group_id = ''
})

async function addSubscription() {
  if (!subForm.value.south_node_id || !subForm.value.group_id) return
  try {
    const newSubs = [...subscriptions.value, { south_node_id: subForm.value.south_node_id, group_id: subForm.value.group_id }]
    await api.setSubscriptions(nodeId.value, newSubs)
    showSubModal.value = false
    await loadSubscriptions()
    ElMessage.success('订阅已添加')
  } catch (e) {
    error.value = '添加订阅失败: ' + e.message
  }
}

async function removeSub(index) {
  try {
    await ElMessageBox.confirm('确定取消该订阅？', '确认', { type: 'warning', confirmButtonText: '取消订阅', cancelButtonText: '返回' })
    const newSubs = subscriptions.value.filter((_, i) => i !== index)
    await api.setSubscriptions(nodeId.value, newSubs) // subscriptions 为 [{ south_node_id, group_id }]
    await loadSubscriptions()
    ElMessage.success('已取消订阅')
  } catch (e) {
    if (e !== 'cancel') error.value = '取消订阅失败: ' + e.message
  }
}

// 节点配置
async function openSettingModal() {
  if (!node.value) return
  try {
    const setting = await api.nodeSetting(nodeId.value)
    const configObj = setting?.config || {}
    settingForm.value = JSON.stringify(setting, null, 2)
    settingConfigFromForm.value = { ...configObj }
    const pluginName = node.value.plugin_name
    const kind = isNorth.value ? 'north' : 'south'
    try {
      const schema = kind === 'south'
        ? await api.pluginSouthSchema(pluginName)
        : await api.pluginNorthSchema(pluginName)
      settingEditMode.value = (schema && schema.params && schema.params.length > 0) ? 'form' : 'json'
    } catch {
      settingEditMode.value = 'json'
    }
    showSettingModal.value = true
  } catch (e) {
    error.value = '获取配置失败: ' + e.message
  }
}

watch(settingEditMode, (mode) => {
  if (mode === 'json' && Object.keys(settingConfigFromForm.value).length > 0) {
    settingForm.value = JSON.stringify({ config: settingConfigFromForm.value }, null, 2)
  }
})

function onSettingConfigFromForm(v) {
  settingConfigFromForm.value = v
}

async function saveSetting() {
  try {
    let body
    if (settingEditMode.value === 'form') {
      body = { config: { ...settingConfigFromForm.value } }
    } else {
      body = JSON.parse(settingForm.value)
    }
    await api.updateNodeSetting(nodeId.value, body)
    showSettingModal.value = false
    await loadNode()
    ElMessage.success('配置已保存')
  } catch (e) {
    error.value = '保存配置失败: ' + e.message
  }
}

async function toggleNode() {
  try {
    if (node.value.state === 'running') {
      await api.stopNode(nodeId.value)
      ElMessage.success('已停止')
    } else {
      await api.startNode(nodeId.value)
      ElMessage.success('已启动')
    }
    await loadNode()
  } catch (e) {
    error.value = '操作失败: ' + e.message
  }
}

// 编辑节点名称（对标 Neuron Update node）
function openNameModal() {
  editNameValue.value = node.value?.name || ''
  showNameModal.value = true
}

async function saveNodeName() {
  const name = (editNameValue.value || '').trim()
  if (!name) {
    ElMessage.warning('名称不能为空')
    return
  }
  try {
    await api.updateNode(nodeId.value, { name })
    showNameModal.value = false
    await loadNode()
    ElMessage.success('名称已更新')
  } catch (e) {
    error.value = '更新名称失败: ' + e.message
  }
}

function getGroupName(gid) {
  return groups.value.find(g => g.id === gid)?.name || gid.slice(0, 8)
}

function goToGroupTags(group) {
  activeTab.value = 'tags'
  // 可选：前端过滤只显示该组点位，这里不过滤，仅切换 tab
}

function openBatchTagModal() {
  batchTagGroupId.value = groups.value[0]?.id || ''
  batchTagRows.value = [{ name: '', data_type: 'Float64', address: '', attr: 'read', description: '' }]
  showBatchTagModal.value = true
}

function addBatchTagRow() {
  batchTagRows.value.push({ name: '', data_type: 'Float64', address: '', attr: 'read', description: '' })
}

function removeBatchTagRow(index) {
  if (batchTagRows.value.length <= 1) return
  batchTagRows.value.splice(index, 1)
}

async function saveBatchTags() {
  const valid = batchTagRows.value.filter(r => r.name && r.address)
  if (!valid.length) {
    ElMessage.warning('请至少填写一行名称与地址')
    return
  }
  if (!batchTagGroupId.value) {
    ElMessage.warning('请选择组')
    return
  }
  try {
    const payload = valid.map(r => ({
      name: r.name,
      address: r.address,
      group_id: batchTagGroupId.value,
      data_type: r.data_type || 'Float64',
      attr: r.attr || 'read',
      description: r.description || undefined
    }))
    await api.batchCreateTags(nodeId.value, payload)
    showBatchTagModal.value = false
    await loadTags()
    ElMessage.success(`已创建 ${payload.length} 个点位`)
  } catch (e) {
    error.value = '批量创建失败: ' + e.message
  }
}

function getSouthNodeName(sid) {
  return southNodes.value.find(n => n.id === sid)?.name || sid.slice(0, 8)
}

function getSouthGroupName(sid, gid) {
  const gs = groupsBySouth.value[sid] || []
  const g = gs.find(x => x.id === gid)
  return g?.name || (typeof gid === 'string' ? gid.slice(0, 8) : '-')
}

onMounted(loadNode)
watch(nodeId, loadNode)
</script>

<template>
  <div class="page-container">
    <div class="detail-header" v-if="node">
      <div class="header-left">
        <el-button :icon="Back" @click="router.push(isNorth ? '/north' : '/south')">返回</el-button>
        <div class="node-info">
          <h2 class="node-name">{{ node.name }}</h2>
          <el-button type="primary" link size="small" :icon="EditPen" @click="openNameModal">编辑名称</el-button>
          <el-tag size="small" type="info">{{ node.plugin_name }}</el-tag>
          <el-tag :type="node.state === 'running' ? 'success' : node.state === 'error' ? 'danger' : 'info'" size="small" effect="light">
            {{ node.state === 'running' ? '运行中' : node.state === 'error' ? '错误' : '已停止' }}
          </el-tag>
        </div>
      </div>
      <div class="header-right">
        <el-button :icon="Setting" @click="openSettingModal">配置</el-button>
        <el-button
          :type="node.state === 'running' ? 'warning' : 'success'"
          :icon="node.state === 'running' ? VideoPause : VideoPlay"
          @click="toggleNode"
        >
          {{ node.state === 'running' ? '停止' : '启动' }}
        </el-button>
      </div>
    </div>

    <el-alert v-if="error" type="error" :title="error" closable show-icon @close="error = ''" class="mb-2" />

    <el-skeleton v-if="loading" :rows="6" animated />

    <el-tabs v-else-if="node" v-model="activeTab" class="detail-tabs">
      <el-tab-pane name="groups">
        <template #label>组列表 <el-tag size="small" type="info">{{ groups.length }}</el-tag></template>
        <div class="tab-header">
          <h3>点位组</h3>
          <el-button type="primary" size="small" :icon="Plus" @click="openGroupModal()">添加组</el-button>
        </div>
        <el-table v-if="groups.length" :data="groups" size="small" stripe>
          <el-table-column prop="name" label="组名称" min-width="120" />
          <el-table-column label="点位数量" width="100">
            <template #default="{ row }">{{ tags.filter(t => t.group_id === row.id).length }}</template>
          </el-table-column>
          <el-table-column label="间隔(ms)" width="100">
            <template #default="{ row }">{{ row.interval_ms || 1000 }}</template>
          </el-table-column>
          <el-table-column prop="description" label="描述" show-overflow-tooltip>
            <template #default="{ row }">{{ row.description || '-' }}</template>
          </el-table-column>
          <el-table-column label="操作" width="160" fixed="right">
            <template #default="{ row }">
              <el-button type="primary" link size="small" :icon="Edit" @click="openGroupModal(row)">编辑</el-button>
              <el-button type="primary" link size="small" @click="goToGroupTags(row)">点位列表</el-button>
              <el-button type="danger" link size="small" :icon="Delete" @click="deleteGroup(row.id)">删除</el-button>
            </template>
          </el-table-column>
        </el-table>
        <el-empty v-else description="暂无点位组，请先创建。" />
      </el-tab-pane>
      <el-tab-pane name="tags">
        <template #label>标签列表 <el-tag size="small" type="info">{{ tags.length }}</el-tag></template>
        <div class="tab-header">
          <h3>数据标签</h3>
          <div class="header-actions">
            <el-button size="small" :icon="RefreshRight" :loading="readingTags" :disabled="!tags.length" @click="readAllTags">{{ readingTags ? '读取中...' : '读取全部' }}</el-button>
            <el-button size="small" :disabled="!groups.length" @click="openBatchTagModal()">批量添加</el-button>
            <el-button type="primary" size="small" :icon="Plus" :disabled="!groups.length" @click="openTagModal()">添加点位</el-button>
          </div>
        </div>
        <el-alert v-if="!groups.length" type="warning" title="请先创建点位组，再添加标签。" show-icon class="mb-2" />
        <el-table v-else-if="tags.length" :data="tags" size="small" stripe>
          <el-table-column prop="name" label="名称" min-width="100" />
          <el-table-column prop="address" label="地址" width="120">
            <template #default="{ row }"><span class="font-mono">{{ row.address || '-' }}</span></template>
          </el-table-column>
          <el-table-column label="类型" width="90">
            <template #default="{ row }"><el-tag size="small" type="info">{{ row.data_type || '-' }}</el-tag></template>
          </el-table-column>
          <el-table-column label="属性" width="80">
            <template #default="{ row }">{{ row.attr === 'write' ? 'Write' : row.attr === 'readwrite' ? 'ReadWrite' : 'Read' }}</template>
          </el-table-column>
          <el-table-column label="乘系数" width="80">
            <template #default>-</template>
          </el-table-column>
          <el-table-column label="偏移量" width="80">
            <template #default>-</template>
          </el-table-column>
          <el-table-column label="精度" width="70">
            <template #default>-</template>
          </el-table-column>
          <el-table-column label="当前值" min-width="100">
            <template #default="{ row }">
              <span v-if="tagValues[row.id] !== undefined">{{ typeof tagValues[row.id] === 'object' ? JSON.stringify(tagValues[row.id]) : tagValues[row.id] }}</span>
              <span v-else class="text-muted">-</span>
            </template>
          </el-table-column>
          <el-table-column prop="description" label="描述" min-width="80" show-overflow-tooltip>
            <template #default="{ row }">{{ row.description || '-' }}</template>
          </el-table-column>
          <el-table-column label="操作" width="120" fixed="right">
            <template #default="{ row }">
              <el-button type="primary" link size="small" :icon="Edit" @click="openTagModal(row)">编辑</el-button>
              <el-button type="danger" link size="small" :icon="Delete" @click="deleteTag(row.id)">删除</el-button>
            </template>
          </el-table-column>
        </el-table>
        <el-empty v-else description="暂无标签，请添加。" />
      </el-tab-pane>
      <el-tab-pane v-if="isNorth" name="subs">
        <template #label>订阅管理 <el-tag size="small" type="info">{{ subscriptions.length }}</el-tag></template>
        <div class="tab-header">
          <h3>数据订阅</h3>
          <el-button type="primary" size="small" :icon="Plus" :disabled="!southNodes.length" @click="openSubModal()">添加订阅</el-button>
        </div>
        <el-alert v-if="!southNodes.length" type="warning" title="暂无可订阅的南向设备，请先创建南向设备。" show-icon class="mb-2" />
        <el-table v-else-if="subscriptions.length" :data="subscriptions" size="small" stripe>
          <el-table-column label="南向设备" min-width="120">
            <template #default="{ row }">{{ getSouthNodeName(row.south_node_id) }}</template>
          </el-table-column>
          <el-table-column label="订阅组" min-width="120">
            <template #default="{ row }">{{ getSouthGroupName(row.south_node_id, row.group_id) }}</template>
          </el-table-column>
          <el-table-column label="操作" width="100" fixed="right">
            <template #default="scope">
              <el-button type="danger" link size="small" :icon="Delete" @click="removeSub(scope.$index)">取消订阅</el-button>
            </template>
          </el-table-column>
        </el-table>
        <el-empty v-else description="暂无订阅，北向应用需要订阅南向设备的数据组才能获取数据。" />
      </el-tab-pane>
    </el-tabs>

    <el-dialog v-model="showGroupModal" :title="editingGroup ? '编辑组' : '添加组'" width="440px" destroy-on-close>
      <el-form :model="groupForm" label-width="100px" label-position="top">
        <el-form-item label="组名称" required>
          <el-input v-model="groupForm.name" placeholder="例如：default" clearable />
        </el-form-item>
        <el-form-item label="采集间隔 (ms)">
          <el-input-number v-model="groupForm.interval_ms" :min="100" :step="100" style="width: 100%" />
        </el-form-item>
        <el-form-item label="描述">
          <el-input v-model="groupForm.description" placeholder="可选" clearable />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="showGroupModal = false">取消</el-button>
        <el-button type="primary" :disabled="!groupForm.name" @click="saveGroup">保存</el-button>
      </template>
    </el-dialog>

    <el-dialog v-model="showTagModal" :title="editingTag ? '编辑标签' : '添加点位'" width="440px" destroy-on-close>
      <el-form :model="tagForm" label-width="100px" label-position="top">
        <el-form-item label="名称" required>
          <el-input v-model="tagForm.name" placeholder="例如：temperature" clearable />
        </el-form-item>
        <el-form-item label="所属组" required>
          <el-select v-model="tagForm.group_id" placeholder="请选择" style="width: 100%">
            <el-option v-for="g in groups" :key="g.id" :label="g.name" :value="g.id" />
          </el-select>
        </el-form-item>
        <el-form-item label="类型">
          <el-select v-model="tagForm.data_type" placeholder="请选择" style="width: 100%">
            <el-option v-for="dt in dataTypes" :key="dt" :label="dt" :value="dt" />
          </el-select>
        </el-form-item>
        <el-form-item label="地址" required>
          <el-input v-model="tagForm.address" placeholder="根据驱动协议填写" clearable class="font-mono" />
        </el-form-item>
        <el-form-item label="属性">
          <el-select v-model="tagForm.attr" placeholder="请选择" style="width: 100%">
            <el-option v-for="a in attrOptions" :key="a.value" :label="a.label" :value="a.value" />
          </el-select>
        </el-form-item>
        <el-form-item label="描述">
          <el-input v-model="tagForm.description" placeholder="可选" clearable />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="showTagModal = false">取消</el-button>
        <el-button type="primary" :disabled="!tagForm.name || !tagForm.group_id || !tagForm.address" @click="saveTag">保存</el-button>
      </template>
    </el-dialog>

    <el-dialog v-model="showBatchTagModal" title="批量添加点位" width="800px" destroy-on-close>
      <el-form label-position="top">
        <el-form-item label="选择组" required>
          <el-select v-model="batchTagGroupId" placeholder="请选择" style="width: 100%">
            <el-option v-for="g in groups" :key="g.id" :label="g.name" :value="g.id" />
          </el-select>
        </el-form-item>
        <el-table :data="batchTagRows" size="small" border>
          <el-table-column label="*名称" width="120">
            <template #default="{ row, $index }">
              <el-input v-model="row.name" placeholder="名称" size="small" />
            </template>
          </el-table-column>
          <el-table-column label="*类型" width="100">
            <template #default="{ row }">
              <el-select v-model="row.data_type" placeholder="类型" size="small" style="width: 100%">
                <el-option v-for="dt in dataTypes" :key="dt" :label="dt" :value="dt" />
              </el-select>
            </template>
          </el-table-column>
          <el-table-column label="*地址" width="120">
            <template #default="{ row }">
              <el-input v-model="row.address" placeholder="地址" size="small" class="font-mono" />
            </template>
          </el-table-column>
          <el-table-column label="*属性" width="100">
            <template #default="{ row }">
              <el-select v-model="row.attr" size="small" style="width: 100%">
                <el-option v-for="a in attrOptions" :key="a.value" :label="a.label" :value="a.value" />
              </el-select>
            </template>
          </el-table-column>
          <el-table-column label="描述" min-width="100">
            <template #default="{ row }">
              <el-input v-model="row.description" placeholder="可选" size="small" />
            </template>
          </el-table-column>
          <el-table-column label="操作" width="90" fixed="right">
            <template #default="{ $index }">
              <el-button type="danger" link size="small" :disabled="batchTagRows.length <= 1" @click="removeBatchTagRow($index)">删除</el-button>
            </template>
          </el-table-column>
        </el-table>
        <el-button type="primary" text :icon="Plus" class="mt-1" @click="addBatchTagRow">+ 添加</el-button>
      </el-form>
      <template #footer>
        <el-button @click="showBatchTagModal = false">取消</el-button>
        <el-button type="primary" @click="saveBatchTags">创建</el-button>
      </template>
    </el-dialog>

    <el-dialog v-model="showSubModal" title="添加订阅" width="440px" destroy-on-close>
      <el-form :model="subForm" label-width="100px" label-position="top">
        <el-form-item label="南向设备" required>
          <el-select v-model="subForm.south_node_id" placeholder="请选择" style="width: 100%">
            <el-option v-for="n in southNodes" :key="n.id" :label="n.name" :value="n.id" />
          </el-select>
        </el-form-item>
        <el-form-item label="数据组" required>
          <el-select v-model="subForm.group_id" placeholder="请选择" style="width: 100%" :disabled="!southGroups.length">
            <el-option v-for="g in southGroups" :key="g.id" :label="g.name" :value="g.id" />
          </el-select>
          <div v-if="!southGroups.length && subForm.south_node_id" class="form-hint">该设备暂无数据组</div>
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="showSubModal = false">取消</el-button>
        <el-button type="primary" :disabled="!subForm.south_node_id || !subForm.group_id" @click="addSubscription">订阅</el-button>
      </template>
    </el-dialog>

    <el-dialog v-model="showSettingModal" title="节点配置" width="600px" destroy-on-close>
      <el-radio-group v-model="settingEditMode" size="small" class="setting-mode-group">
        <el-radio-button value="form">表单</el-radio-button>
        <el-radio-button value="json">JSON</el-radio-button>
      </el-radio-group>
      <template v-if="settingEditMode === 'form' && node">
        <NodeConfigForm
          :plugin-name="node.plugin_name"
          :kind="isNorth ? 'north' : 'south'"
          :model-value="settingConfigFromForm"
          @update:model-value="onSettingConfigFromForm"
        />
      </template>
      <el-input
        v-else
        v-model="settingForm"
        type="textarea"
        :rows="14"
        class="font-mono setting-json-input"
      />
      <template #footer>
        <el-button @click="showSettingModal = false">取消</el-button>
        <el-button type="primary" @click="saveSetting">保存配置</el-button>
      </template>
    </el-dialog>

    <el-dialog v-model="showNameModal" title="编辑节点名称" width="400px" destroy-on-close>
      <el-form label-position="top">
        <el-form-item label="名称">
          <el-input v-model="editNameValue" placeholder="输入节点名称" clearable />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="showNameModal = false">取消</el-button>
        <el-button type="primary" :disabled="!editNameValue?.trim()" @click="saveNodeName">保存</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<style scoped>
.mb-2 { margin-bottom: 1rem; }
.mt-1 { margin-top: 0.5rem; }
.detail-header { display: flex; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 1rem; margin-bottom: 1rem; }
.detail-header .header-left { display: flex; align-items: center; gap: 1rem; }
.detail-header .header-right { display: flex; gap: 0.5rem; }
.node-info { display: flex; align-items: center; gap: 0.75rem; flex-wrap: wrap; }
.node-name { margin: 0; font-size: 1.25rem; }
.tab-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 1rem; }
.header-actions { display: flex; gap: 0.5rem; }
.form-hint { font-size: 0.8rem; color: var(--text-muted); margin-top: 0.25rem; }
.font-mono { font-family: var(--font-mono); }
.text-muted { color: var(--text-muted); }
.detail-tabs { margin-top: 0.5rem; }
.setting-mode-group { margin-bottom: 1rem; }
.setting-json-input { margin-top: 0.5rem; }
</style>
