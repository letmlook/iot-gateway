<script setup>
import { ref, onMounted, computed, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Close, Setting, VideoPlay, VideoPause, Plus, Edit, Delete, RefreshRight, EditPen } from '@element-plus/icons-vue'
import { api } from '../api.js'
import NodeConfigForm from './NodeConfigForm.vue'

const { t } = useI18n()
const props = defineProps({
  nodeId: { type: String, required: true },
  kind: { type: String, required: true } // 'south' | 'north'
})
const emit = defineEmits(['close', 'refresh'])

const isNorth = computed(() => props.kind === 'north')

const node = ref(null)
const groups = ref([])
const tags = ref([])
const subscriptions = ref([])
const southNodes = ref([])
const loading = ref(false)
const error = ref('')
const activeTab = ref('groups')

const showGroupForm = ref(false)
const showTagForm = ref(false)
const showSubForm = ref(false)
const showSettingPanel = ref(false)
const showNameInline = ref(false)
const showBatchTagPanel = ref(false)
const editingGroup = ref(null)
const editingTag = ref(null)
const editNameValue = ref('')

const groupForm = ref({ name: '', interval_ms: 1000, description: '' })
const tagForm = ref({ name: '', group_id: '', data_type: 'Float64', address: '', attr: 'read', description: '' })
const attrOptions = [
  { value: 'read', label: 'Read' },
  { value: 'write', label: 'Write' },
  { value: 'readwrite', label: 'ReadWrite' }
]
const batchTagGroupId = ref('')
const batchTagRows = ref([{ name: '', data_type: 'Float64', address: '', attr: 'read', description: '' }])
const settingForm = ref('')
const settingConfigFromForm = ref({})
const settingEditMode = ref('json')
const subForm = ref({ south_node_id: '', group_id: '' })
const southGroups = ref([])
const groupsBySouth = ref({})

const dataTypes = ['Bool', 'Int8', 'Int16', 'Int32', 'Int64', 'UInt8', 'UInt16', 'UInt32', 'UInt64', 'Float32', 'Float64', 'String', 'Bytes']

async function loadNode() {
  loading.value = true
  error.value = ''
  try {
    node.value = await api.node(props.nodeId)
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
    groups.value = await api.groups(props.nodeId)
  } catch (e) {
    console.error('加载组失败', e)
  }
}

async function loadTags() {
  try {
    tags.value = await api.tags(props.nodeId)
  } catch (e) {
    console.error('加载标签失败', e)
  }
}

async function loadSubscriptions() {
  try {
    subscriptions.value = await api.subscriptions(props.nodeId)
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
    southGroups.value = []
  }
}

function openGroupForm(group = null) {
  editingGroup.value = group
  if (group) {
    groupForm.value = { name: group.name, interval_ms: group.interval_ms || 1000, description: group.description || '' }
  } else {
    groupForm.value = { name: '', interval_ms: 1000, description: '' }
  }
  showGroupForm.value = true
}

async function saveGroup() {
  try {
    if (editingGroup.value) {
      await api.updateGroup(props.nodeId, editingGroup.value.id, groupForm.value)
    } else {
      await api.createGroup(props.nodeId, groupForm.value)
    }
    showGroupForm.value = false
    await loadGroups()
    ElMessage.success('保存成功')
  } catch (e) {
    error.value = '保存组失败: ' + e.message
  }
}

async function deleteGroup(gid) {
  try {
    await ElMessageBox.confirm('确定删除该组？组内标签也会被删除。', '确认删除', { type: 'warning', confirmButtonText: '删除', cancelButtonText: '取消' })
    await api.deleteGroup(props.nodeId, gid)
    await loadGroups()
    await loadTags()
    ElMessage.success('已删除')
  } catch (e) {
    if (e !== 'cancel') error.value = '删除组失败: ' + e.message
  }
}

function openTagForm(tag = null) {
  editingTag.value = tag
  if (tag) {
    tagForm.value = {
      name: tag.name, group_id: tag.group_id, data_type: tag.data_type || 'Float64',
      address: tag.address || '', attr: (tag.attr || 'read').toLowerCase(), description: tag.description || ''
    }
  } else {
    tagForm.value = { name: '', group_id: groups.value[0]?.id || '', data_type: 'Float64', address: '', attr: 'read', description: '' }
  }
  showTagForm.value = true
}

async function saveTag() {
  try {
    const payload = { ...tagForm.value, attr: tagForm.value.attr }
    if (editingTag.value) {
      await api.updateTag(props.nodeId, editingTag.value.id, payload)
    } else {
      await api.createTag(props.nodeId, payload)
    }
    showTagForm.value = false
    await loadTags()
    ElMessage.success('保存成功')
  } catch (e) {
    error.value = '保存标签失败: ' + e.message
  }
}

async function deleteTag(tid) {
  try {
    await ElMessageBox.confirm('确定删除该标签？', '确认删除', { type: 'warning', confirmButtonText: '删除', cancelButtonText: '取消' })
    await api.deleteTag(props.nodeId, tid)
    await loadTags()
    ElMessage.success('已删除')
  } catch (e) {
    if (e !== 'cancel') error.value = '删除标签失败: ' + e.message
  }
}

const tagValues = ref({})
const readingTags = ref(false)
async function readAllTags() {
  if (!tags.value.length) return
  readingTags.value = true
  try {
    const result = await api.readTags(props.nodeId, tags.value.map(t => t.id))
    tagValues.value = {}
    result.forEach(([tid, val]) => { tagValues.value[tid] = val })
  } catch (e) {
    error.value = '读取标签失败: ' + e.message
  } finally {
    readingTags.value = false
  }
}

function openSubForm() {
  subForm.value = { south_node_id: southNodes.value[0]?.id || '', group_id: '' }
  if (subForm.value.south_node_id) loadSouthGroups(subForm.value.south_node_id)
  showSubForm.value = true
}
watch(() => subForm.value.south_node_id, (v) => {
  loadSouthGroups(v)
  subForm.value.group_id = ''
})

async function addSubscription() {
  if (!subForm.value.south_node_id || !subForm.value.group_id) return
  try {
    const newSubs = [...subscriptions.value, { south_node_id: subForm.value.south_node_id, group_id: subForm.value.group_id }]
    await api.setSubscriptions(props.nodeId, newSubs)
    showSubForm.value = false
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
    await api.setSubscriptions(props.nodeId, newSubs)
    await loadSubscriptions()
    ElMessage.success('已取消订阅')
  } catch (e) {
    if (e !== 'cancel') error.value = '取消订阅失败: ' + e.message
  }
}

async function openSettingPanel() {
  if (!node.value) return
  try {
    const setting = await api.nodeSetting(props.nodeId)
    const configObj = setting?.config || {}
    settingForm.value = JSON.stringify(setting, null, 2)
    settingConfigFromForm.value = { ...configObj }
    try {
      const schema = props.kind === 'south' ? await api.pluginSouthSchema(node.value.plugin_name) : await api.pluginNorthSchema(node.value.plugin_name)
      settingEditMode.value = (schema && schema.params && schema.params.length > 0) ? 'form' : 'json'
    } catch {
      settingEditMode.value = 'json'
    }
    showSettingPanel.value = true
  } catch (e) {
    error.value = t('nodeDetail.loadConfigFailed') + e.message
  }
}
function onSettingConfigFromForm(v) {
  settingConfigFromForm.value = v
}
async function saveSetting() {
  try {
    const body = settingEditMode.value === 'form' ? { config: { ...settingConfigFromForm.value } } : JSON.parse(settingForm.value)
    await api.updateNodeSetting(props.nodeId, body)
    showSettingPanel.value = false
    await loadNode()
    emit('refresh')
    ElMessage.success(t('nodeDetail.saveSuccess'))
  } catch (e) {
    error.value = t('nodeDetail.saveFailed') + e.message
  }
}

async function toggleNode() {
  try {
    if (node.value.state === 'running') {
      await api.stopNode(props.nodeId)
      ElMessage.success('已停止')
    } else {
      await api.startNode(props.nodeId)
      ElMessage.success('已启动')
    }
    await loadNode()
    emit('refresh')
  } catch (e) {
    error.value = '操作失败: ' + e.message
  }
}

function openNameInline() {
  editNameValue.value = node.value?.name || ''
  showNameInline.value = true
}
async function saveNodeName() {
  const name = (editNameValue.value || '').trim()
  if (!name) { ElMessage.warning('名称不能为空'); return }
  try {
    await api.updateNode(props.nodeId, { name })
    showNameInline.value = false
    await loadNode()
    emit('refresh')
    ElMessage.success('名称已更新')
  } catch (e) {
    error.value = '更新名称失败: ' + e.message
  }
}

function getGroupName(gid) {
  return groups.value.find(g => g.id === gid)?.name || gid?.slice(0, 8)
}
function goToGroupTags() {
  activeTab.value = 'tags'
}

function openBatchTagPanel() {
  batchTagGroupId.value = groups.value[0]?.id || ''
  batchTagRows.value = [{ name: '', data_type: 'Float64', address: '', attr: 'read', description: '' }]
  showBatchTagPanel.value = true
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
  if (!valid.length) { ElMessage.warning('请至少填写一行名称与地址'); return }
  if (!batchTagGroupId.value) { ElMessage.warning('请选择组'); return }
  try {
    const payload = valid.map(r => ({
      name: r.name, address: r.address, group_id: batchTagGroupId.value,
      data_type: r.data_type || 'Float64', attr: r.attr || 'read', description: r.description || undefined
    }))
    await api.batchCreateTags(props.nodeId, payload)
    showBatchTagPanel.value = false
    await loadTags()
    ElMessage.success(`已创建 ${payload.length} 个点位`)
  } catch (e) {
    error.value = '批量创建失败: ' + e.message
  }
}

function getSouthNodeName(sid) {
  return southNodes.value.find(n => n.id === sid)?.name || sid?.slice(0, 8)
}
function getSouthGroupName(sid, gid) {
  const gs = groupsBySouth.value[sid] || []
  const g = gs.find(x => x.id === gid)
  return g?.name || (typeof gid === 'string' ? gid.slice(0, 8) : '-')
}

onMounted(loadNode)
watch(() => props.nodeId, loadNode)
</script>

<template>
  <div class="node-detail-panel">
    <div class="panel-header-bar">
      <div class="header-left">
        <el-button text :icon="Close" @click="emit('close')">关闭</el-button>
        <div v-if="node" class="node-info">
          <span v-if="!showNameInline" class="node-name">{{ node.name }}</span>
          <template v-else>
            <el-input v-model="editNameValue" size="small" placeholder="名称" style="width: 140px" />
            <el-button size="small" type="primary" @click="saveNodeName">保存</el-button>
            <el-button size="small" @click="showNameInline = false">取消</el-button>
          </template>
          <el-button v-if="!showNameInline" type="primary" link size="small" :icon="EditPen" @click="openNameInline">编辑</el-button>
          <el-tag size="small" type="info">{{ node.plugin_name }}</el-tag>
          <el-tag :type="node.state === 'running' ? 'success' : node.state === 'error' ? 'danger' : 'info'" size="small" effect="light">
            {{ node.state === 'running' ? '运行中' : node.state === 'error' ? '错误' : '已停止' }}
          </el-tag>
        </div>
      </div>
      <div v-if="node" class="header-actions">
        <el-button size="small" :icon="Setting" @click="openSettingPanel">配置</el-button>
        <el-button
          :type="node.state === 'running' ? 'warning' : 'success'"
          size="small"
          :icon="node.state === 'running' ? VideoPause : VideoPlay"
          @click="toggleNode"
        >
          {{ node.state === 'running' ? '停止' : '启动' }}
        </el-button>
      </div>
    </div>

    <el-alert v-if="error" type="error" :title="error" closable show-icon @close="error = ''" class="mb-2" />

    <el-skeleton v-if="loading" :rows="4" animated />

    <template v-else-if="node">
      <!-- 配置：右侧内联 -->
      <div v-if="showSettingPanel" class="inline-section setting-section">
        <div class="inline-section-header">
          <span>{{ t('nodeDetail.nodeConfig') }}</span>
          <el-button text size="small" @click="showSettingPanel = false">收起</el-button>
        </div>
        <el-radio-group v-model="settingEditMode" size="small" class="mb-2">
          <el-radio-button value="form">{{ t('schema.form') }}</el-radio-button>
          <el-radio-button value="json">JSON</el-radio-button>
        </el-radio-group>
        <template v-if="settingEditMode === 'form'">
          <NodeConfigForm
            :plugin-name="node.plugin_name"
            :kind="kind"
            :model-value="settingConfigFromForm"
            @update:model-value="onSettingConfigFromForm"
          />
        </template>
        <el-input v-else v-model="settingForm" type="textarea" :rows="10" class="font-mono" />
        <div class="inline-section-footer">
          <el-button size="small" @click="showSettingPanel = false">{{ t('common.cancel') }}</el-button>
          <el-button size="small" type="primary" @click="saveSetting">{{ t('nodeDetail.saveConfig') }}</el-button>
        </div>
      </div>

      <el-tabs v-else v-model="activeTab" class="detail-tabs">
        <el-tab-pane name="groups">
          <template #label>组 <el-tag size="small" type="info">{{ groups.length }}</el-tag></template>
          <div class="tab-toolbar">
            <el-button type="primary" size="small" :icon="Plus" @click="openGroupForm()">添加组</el-button>
          </div>
          <div v-if="showGroupForm" class="inline-section">
            <el-form :model="groupForm" label-position="top" size="small">
              <el-form-item label="组名称" required>
                <el-input v-model="groupForm.name" placeholder="例如：default" clearable />
              </el-form-item>
              <el-form-item label="采集间隔 (ms)">
                <el-input-number v-model="groupForm.interval_ms" :min="100" :step="100" style="width: 100%" />
              </el-form-item>
              <el-form-item label="描述">
                <el-input v-model="groupForm.description" :placeholder="t('nodeDetail.optional')" clearable />
              </el-form-item>
              <el-form-item>
                <el-button @click="showGroupForm = false">取消</el-button>
                <el-button type="primary" :disabled="!groupForm.name" @click="saveGroup">保存</el-button>
              </el-form-item>
            </el-form>
          </div>
          <el-table v-if="groups.length" :data="groups" size="small" stripe max-height="240">
            <el-table-column prop="name" label="组名称" min-width="100" />
            <el-table-column label="点数" width="60">
              <template #default="{ row }">{{ tags.filter(t => t.group_id === row.id).length }}</template>
            </el-table-column>
            <el-table-column label="间隔" width="70">
              <template #default="{ row }">{{ row.interval_ms || 1000 }}</template>
            </el-table-column>
            <el-table-column label="操作" width="120" fixed="right">
              <template #default="{ row }">
                <el-button type="primary" link size="small" :icon="Edit" @click="openGroupForm(row)">编辑</el-button>
                <el-button type="danger" link size="small" :icon="Delete" @click="deleteGroup(row.id)">删除</el-button>
              </template>
            </el-table-column>
          </el-table>
          <el-empty v-else description="暂无组" :image-size="48" />
        </el-tab-pane>

        <el-tab-pane name="tags">
          <template #label>标签 <el-tag size="small" type="info">{{ tags.length }}</el-tag></template>
          <div class="tab-toolbar">
            <el-button size="small" :icon="RefreshRight" :loading="readingTags" :disabled="!tags.length" @click="readAllTags">读取</el-button>
            <el-button size="small" :disabled="!groups.length" @click="openBatchTagPanel">批量添加</el-button>
            <el-button type="primary" size="small" :icon="Plus" :disabled="!groups.length" @click="openTagForm()">添加</el-button>
          </div>
          <div v-if="showTagForm" class="inline-section">
            <el-form :model="tagForm" label-position="top" size="small">
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
                <el-select v-model="tagForm.attr" style="width: 100%">
                  <el-option v-for="a in attrOptions" :key="a.value" :label="a.label" :value="a.value" />
                </el-select>
              </el-form-item>
              <el-form-item>
                <el-button @click="showTagForm = false">取消</el-button>
                <el-button type="primary" :disabled="!tagForm.name || !tagForm.group_id || !tagForm.address" @click="saveTag">保存</el-button>
              </el-form-item>
            </el-form>
          </div>
          <div v-if="showBatchTagPanel" class="inline-section">
            <el-form label-position="top" size="small">
              <el-form-item label="选择组" required>
                <el-select v-model="batchTagGroupId" placeholder="请选择" style="width: 100%">
                  <el-option v-for="g in groups" :key="g.id" :label="g.name" :value="g.id" />
                </el-select>
              </el-form-item>
              <el-table :data="batchTagRows" size="small" border max-height="160">
                <el-table-column label="名称" width="90">
                  <template #default="{ row }"><el-input v-model="row.name" size="small" placeholder="名称" /></template>
                </el-table-column>
                <el-table-column label="类型" width="85">
                  <template #default="{ row }">
                    <el-select v-model="row.data_type" size="small" style="width: 100%">
                      <el-option v-for="dt in dataTypes" :key="dt" :label="dt" :value="dt" />
                    </el-select>
                  </template>
                </el-table-column>
                <el-table-column label="地址" width="90">
                  <template #default="{ row }"><el-input v-model="row.address" size="small" class="font-mono" /></template>
                </el-table-column>
                <el-table-column label="操作" width="60">
                  <template #default="{ $index }">
                    <el-button type="danger" link size="small" :disabled="batchTagRows.length <= 1" @click="removeBatchTagRow($index)">删</el-button>
                  </template>
                </el-table-column>
              </el-table>
              <el-button type="primary" text size="small" :icon="Plus" @click="addBatchTagRow">+ 行</el-button>
            </el-form>
            <div class="inline-section-footer">
              <el-button size="small" @click="showBatchTagPanel = false">取消</el-button>
              <el-button size="small" type="primary" @click="saveBatchTags">创建</el-button>
            </div>
          </div>
          <el-alert v-if="!groups.length" type="warning" title="请先创建组" show-icon class="mb-2" />
          <el-table v-else-if="tags.length" :data="tags" size="small" stripe max-height="220">
            <el-table-column prop="name" label="名称" min-width="80" />
            <el-table-column prop="address" label="地址" width="80" show-overflow-tooltip>
              <template #default="{ row }"><span class="font-mono">{{ row.address || '-' }}</span></template>
            </el-table-column>
            <el-table-column label="类型" width="72">
              <template #default="{ row }"><el-tag size="small" type="info">{{ row.data_type || '-' }}</el-tag></template>
            </el-table-column>
            <el-table-column label="值" width="70">
              <template #default="{ row }">
                <span v-if="tagValues[row.id] !== undefined">{{ typeof tagValues[row.id] === 'object' ? JSON.stringify(tagValues[row.id]) : tagValues[row.id] }}</span>
                <span v-else class="text-muted">-</span>
              </template>
            </el-table-column>
            <el-table-column label="操作" width="90" fixed="right">
              <template #default="{ row }">
                <el-button type="primary" link size="small" :icon="Edit" @click="openTagForm(row)">编辑</el-button>
                <el-button type="danger" link size="small" :icon="Delete" @click="deleteTag(row.id)">删</el-button>
              </template>
            </el-table-column>
          </el-table>
          <el-empty v-else description="暂无标签" :image-size="48" />
        </el-tab-pane>

        <el-tab-pane v-if="isNorth" name="subs">
          <template #label>订阅 <el-tag size="small" type="info">{{ subscriptions.length }}</el-tag></template>
          <div class="tab-toolbar">
            <el-button type="primary" size="small" :icon="Plus" :disabled="!southNodes.length" @click="openSubForm">添加订阅</el-button>
          </div>
          <div v-if="showSubForm" class="inline-section">
            <el-form :model="subForm" label-position="top" size="small">
              <el-form-item label="南向设备" required>
                <el-select v-model="subForm.south_node_id" placeholder="请选择" style="width: 100%">
                  <el-option v-for="n in southNodes" :key="n.id" :label="n.name" :value="n.id" />
                </el-select>
              </el-form-item>
              <el-form-item label="数据组" required>
                <el-select v-model="subForm.group_id" placeholder="请选择" style="width: 100%" :disabled="!southGroups.length">
                  <el-option v-for="g in southGroups" :key="g.id" :label="g.name" :value="g.id" />
                </el-select>
              </el-form-item>
              <el-form-item>
                <el-button @click="showSubForm = false">取消</el-button>
                <el-button type="primary" :disabled="!subForm.south_node_id || !subForm.group_id" @click="addSubscription">订阅</el-button>
              </el-form-item>
            </el-form>
          </div>
          <el-alert v-if="!southNodes.length" type="warning" title="暂无可订阅的南向设备" show-icon class="mb-2" />
          <el-table v-else-if="subscriptions.length" :data="subscriptions" size="small" stripe max-height="240">
            <el-table-column label="南向设备" min-width="100">
              <template #default="{ row }">{{ getSouthNodeName(row.south_node_id) }}</template>
            </el-table-column>
            <el-table-column label="组" min-width="80">
              <template #default="{ row }">{{ getSouthGroupName(row.south_node_id, row.group_id) }}</template>
            </el-table-column>
            <el-table-column label="操作" width="90" fixed="right">
              <template #default="scope">
                <el-button type="danger" link size="small" :icon="Delete" @click="removeSub(scope.$index)">取消</el-button>
              </template>
            </el-table-column>
          </el-table>
          <el-empty v-else description="暂无订阅" :image-size="48" />
        </el-tab-pane>
      </el-tabs>
    </template>
  </div>
</template>

<style scoped>
.node-detail-panel { display: flex; flex-direction: column; height: 100%; overflow: hidden; }
.panel-header-bar { display: flex; justify-content: space-between; align-items: center; flex-wrap: wrap; gap: 0.5rem; padding: 0.75rem 1rem; border-bottom: 1px solid var(--el-border-color); }
.header-left { display: flex; align-items: center; gap: 0.5rem; flex-wrap: wrap; }
.node-info { display: flex; align-items: center; gap: 0.5rem; flex-wrap: wrap; }
.node-name { font-weight: 600; font-size: 0.95rem; }
.header-actions { display: flex; gap: 0.5rem; }
.mb-2 { margin-bottom: 0.5rem; }
.inline-section { padding: 0.75rem 1rem; margin-bottom: 0.75rem; background: var(--el-fill-color-light); border-radius: var(--el-border-radius-base); border: 1px solid var(--el-border-color-lighter); }
.inline-section-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 0.5rem; font-weight: 500; }
.inline-section-footer { margin-top: 0.5rem; }
.tab-toolbar { display: flex; gap: 0.5rem; margin-bottom: 0.75rem; }
.detail-tabs { flex: 1; overflow: hidden; display: flex; flex-direction: column; }
.detail-tabs :deep(.el-tabs__content) { flex: 1; overflow: auto; }
.detail-tabs :deep(.el-tabs__item) { font-size: 0.85rem; }
.font-mono { font-family: var(--font-mono); }
.text-muted { color: var(--text-muted); }
</style>
