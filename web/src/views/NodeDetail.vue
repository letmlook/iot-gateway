<script setup>
import { ref, onMounted, computed, watch, onUnmounted } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { getErrorMessage } from '../i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Back, Setting, VideoPlay, VideoPause, Plus, Edit, Delete, RefreshRight, EditPen } from '@element-plus/icons-vue'
import { api } from '../api.js'
const route = useRoute()
const router = useRouter()
const { t } = useI18n()

const nodeId = computed(() => route.params.id)
const isNorth = computed(() => route.path.startsWith('/north'))
// 北向 MQTT 连接状态：connected === true 为已连接，connected === false 为断开，connected 为 null/未返回时若节点运行中则视为已连接，避免误显示断开
const connectionStatusConnected = computed(() => {
  const cs = node.value?.connection_status
  if (cs?.connected === false) return false
  if (cs?.connected === true) return true
  return node.value?.state === 'running'
})

const node = ref(null)
const groups = ref([])
const tags = ref([])
const subscriptions = ref([])
const southNodes = ref([])
const loading = ref(false)
const error = ref('')
const activeTab = ref(route.path.startsWith('/north') ? 'subs' : 'groups')
// 北向主题（来自节点配置 topic_template）
const DEFAULT_TOPIC_TEMPLATE = 'gateway/data/${node_id}/${group_id}'
const nodeSetting = ref(null)
const topicTemplate = ref('')
const topicSaving = ref(false)

// 模态框状态
const showGroupModal = ref(false)
const showTagModal = ref(false)
const showSubModal = ref(false)
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
const subForm = ref({ south_node_id: '', group_id: '' })
const southGroups = ref([])
const groupsBySouth = ref({})

const dataTypes = ['Bool', 'Int8', 'Int16', 'Int32', 'Int64', 'UInt8', 'UInt16', 'UInt32', 'UInt64', 'Float32', 'Float64', 'String', 'Bytes']

let connStatusTimer = null
function startConnStatusPoll() {
  if (connStatusTimer) return
  if (!node.value || node.value.state !== 'running') return
  connStatusTimer = setInterval(async () => {
    if (!node.value) return
    try {
      const status = await api.nodeConnectionStatus(nodeId.value)
      if (node.value) node.value.connection_status = status
    } catch { /* ignore */ }
  }, 5000)
}
function stopConnStatusPoll() {
  if (connStatusTimer) {
    clearInterval(connStatusTimer)
    connStatusTimer = null
  }
}

async function loadNode() {
  loading.value = true
  error.value = ''
  try {
    node.value = await api.node(nodeId.value)
    if (isNorth.value) {
      activeTab.value = 'subs'
      await loadSubscriptions()
      await loadSouthNodes()
      await loadTopicFromSetting()
    } else {
      await loadGroups()
      await loadTags()
    }
  } catch (e) {
    error.value = t('nodeDetail.loadNodeFailed') + getErrorMessage(t, e)
  } finally {
    loading.value = false
    if (node.value && node.value.state === 'running') {
      startConnStatusPoll()
    }
  }
}

async function loadTopicFromSetting() {
  try {
    const setting = await api.nodeSetting(nodeId.value)
    nodeSetting.value = setting
    const cfg = setting?.config || {}
    topicTemplate.value = (cfg.topic_template && typeof cfg.topic_template === 'string')
      ? cfg.topic_template
      : DEFAULT_TOPIC_TEMPLATE
  } catch {
    topicTemplate.value = DEFAULT_TOPIC_TEMPLATE
  }
}

async function saveTopic() {
  const v = (topicTemplate.value || '').trim() || DEFAULT_TOPIC_TEMPLATE
  topicSaving.value = true
  try {
    const cfg = { ...(nodeSetting.value?.config || {}), topic_template: v }
    await api.updateNodeSetting(nodeId.value, { config: cfg })
    nodeSetting.value = nodeSetting.value ? { ...nodeSetting.value, config: cfg } : { config: cfg }
    topicTemplate.value = v
    ElMessage.success(t('nodeDetail.topicSaved'))
  } catch (e) {
    error.value = t('nodeDetail.topicSaveFailed') + getErrorMessage(t, e)
  } finally {
    topicSaving.value = false
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
    ElMessage.success(t('nodeDetail.saveGroupSuccess'))
  } catch (e) {
    error.value = t('nodeDetail.saveGroupFailed') + getErrorMessage(t, e)
  }
}

async function deleteGroup(gid) {
  try {
    await ElMessageBox.confirm(t('nodeDetail.deleteGroupConfirm'), t('common.confirmDelete'), { type: 'warning', confirmButtonText: t('common.delete'), cancelButtonText: t('common.cancel') })
    await api.deleteGroup(nodeId.value, gid)
    await loadGroups()
    await loadTags()
    ElMessage.success(t('nodeDetail.deleteGroupSuccess'))
  } catch (e) {
    if (e !== 'cancel') error.value = t('nodeDetail.deleteGroupFailed') + getErrorMessage(t, e)
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
    ElMessage.success(t('nodeDetail.saveGroupSuccess'))
  } catch (e) {
    error.value = t('nodeDetail.saveTagFailed') + getErrorMessage(t, e)
  }
}

async function deleteTag(tid) {
  try {
    await ElMessageBox.confirm(t('nodeDetail.deleteTagConfirm'), t('common.confirmDelete'), { type: 'warning', confirmButtonText: t('common.delete'), cancelButtonText: t('common.cancel') })
    await api.deleteTag(nodeId.value, tid)
    await loadTags()
    ElMessage.success(t('nodeDetail.deleteTagSuccess'))
  } catch (e) {
    if (e !== 'cancel') error.value = t('nodeDetail.deleteTagFailed') + getErrorMessage(t, e)
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
    error.value = t('nodeDetail.readTagsFailed') + getErrorMessage(t, e)
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
    ElMessage.success(t('nodeDetail.addSubSuccess'))
  } catch (e) {
    error.value = t('nodeDetail.addSubFailed') + getErrorMessage(t, e)
  }
}

async function removeSub(index) {
  try {
    await ElMessageBox.confirm(t('nodeDetail.unsubscribeConfirm'), t('common.confirm'), { type: 'warning', confirmButtonText: t('nodeDetail.unsubscribeConfirmBtn'), cancelButtonText: t('nodeDetail.cancelBack') })
    const newSubs = subscriptions.value.filter((_, i) => i !== index)
    await api.setSubscriptions(nodeId.value, newSubs)
    await loadSubscriptions()
    ElMessage.success(t('nodeDetail.unsubscribeSuccess'))
  } catch (e) {
    if (e !== 'cancel') error.value = t('nodeDetail.unsubscribeFailed') + getErrorMessage(t, e)
  }
}

async function toggleNode() {
  try {
    if (node.value.state === 'running') {
      await api.stopNode(nodeId.value)
      ElMessage.success(t('nodeDetail.stopSuccess'))
    } else {
      await api.startNode(nodeId.value)
      ElMessage.success(t('nodeDetail.startSuccess'))
    }
    await loadNode()
  } catch (e) {
    error.value = t('nodeDetail.opFailed') + getErrorMessage(t, e)
  }
}

// 编辑节点名称
function openNameModal() {
  editNameValue.value = node.value?.name || ''
  showNameModal.value = true
}

async function saveNodeName() {
  const name = (editNameValue.value || '').trim()
  if (!name) {
    ElMessage.warning(t('nodeDetail.nameRequired'))
    return
  }
  try {
    await api.updateNode(nodeId.value, { name })
    showNameModal.value = false
    await loadNode()
    ElMessage.success(t('nodeDetail.nameUpdated'))
  } catch (e) {
    error.value = t('nodeDetail.updateNameFailed') + getErrorMessage(t, e)
  }
}

function getGroupName(gid) {
  return groups.value.find(g => g.id === gid)?.name || gid.slice(0, 8)
}

function goToGroupTags(group) {
  router.push(`/south/${nodeId.value}/group/${group.id}`)
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
    ElMessage.warning(t('nodeDetail.batchNameAddressRequired'))
    return
  }
  if (!batchTagGroupId.value) {
    ElMessage.warning(t('nodeDetail.selectGroupRequired'))
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
    ElMessage.success(t('nodeDetail.batchCreated', { n: payload.length }))
  } catch (e) {
    error.value = t('nodeDetail.batchCreateFailed') + getErrorMessage(t, e)
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
watch(nodeId, () => { stopConnStatusPoll(); loadNode() })
watch(node, (n) => {
  stopConnStatusPoll()
  if (n && isNorth.value && n.plugin_name === 'mqtt' && n.state === 'running') startConnStatusPoll()
}, { deep: true })
onUnmounted(stopConnStatusPoll)
</script>

<template>
  <div class="page-container">
    <div class="detail-header" v-if="node">
      <div class="header-left">
        <el-button :icon="Back" @click="router.push(isNorth ? '/north' : '/south')">{{ t('nodeDetail.cancelBack') }}</el-button>
        <div class="node-info">
          <h2 class="node-name">{{ node.name }}</h2>
          <el-button type="primary" link size="small" :icon="EditPen" @click="openNameModal">{{ t('nodeDetail.editName') }}</el-button>
          <el-tag size="small" type="info">{{ node.plugin_name }}</el-tag>
          <el-tag :type="node.state === 'running' ? 'success' : node.state === 'error' ? 'danger' : 'info'" size="small" effect="light">
            {{ node.state === 'running' ? t('common.running') : node.state === 'error' ? t('common.error') : t('common.stopped') }}
          </el-tag>
          <el-tooltip
            v-if="node.connection_status && node.state === 'running' && node.connection_status.last_error"
            :content="node.connection_status.last_error"
            placement="bottom"
          >
            <el-tag
              :type="connectionStatusConnected ? 'success' : 'danger'"
              size="small"
              effect="plain"
            >
              {{ connectionStatusConnected ? t('south.connected') : t('south.disconnected') }}
            </el-tag>
          </el-tooltip>
          <el-tag
            v-else-if="node.connection_status && node.state === 'running'"
            :type="connectionStatusConnected ? 'success' : 'danger'"
            size="small"
            effect="plain"
          >
            {{ connectionStatusConnected ? t('south.connected') : t('south.disconnected') }}
          </el-tag>
        </div>
      </div>
      <div class="header-right">
        <el-button :icon="Setting" @click="router.push(isNorth ? `/north/${nodeId}/config` : `/south/${nodeId}/config`)">{{ t('nodeDetail.config') }}</el-button>
        <el-button
          :type="node.state === 'running' ? 'warning' : 'success'"
          :icon="node.state === 'running' ? VideoPause : VideoPlay"
          @click="toggleNode"
        >
          {{ node.state === 'running' ? t('common.stop') : t('common.start') }}
        </el-button>
      </div>
    </div>

    <el-alert v-if="error" type="error" :title="error" closable show-icon @close="error = ''" class="mb-2" />

    <el-skeleton v-if="loading" :rows="6" animated />

    <el-tabs v-else-if="node" v-model="activeTab" class="detail-tabs">
      <el-tab-pane v-if="!isNorth" name="groups">
        <template #label>{{ t('nodeDetail.groupList') }} <el-tag size="small" type="info">{{ groups.length }}</el-tag></template>
        <div class="tab-header">
          <h3>{{ t('nodeDetail.pointGroup') }}</h3>
          <el-button type="primary" size="small" :icon="Plus" @click="openGroupModal()">{{ t('nodeDetail.addGroup') }}</el-button>
        </div>
        <el-table v-if="groups.length" :data="groups" size="small" stripe class="groups-table-clickable" @row-click="(row) => goToGroupTags(row)">
          <el-table-column :label="t('nodeDetail.groupNameLabel')" min-width="120">
            <template #default="{ row }">
              <span class="group-name-link">{{ row.name }}</span>
            </template>
          </el-table-column>
          <el-table-column :label="t('nodeDetail.pointCount')" width="80">
            <template #default="{ row }">{{ tags.filter(t => t.group_id === row.id).length }}</template>
          </el-table-column>
          <el-table-column :label="t('nodeDetail.intervalMs')" width="80">
            <template #default="{ row }">{{ row.interval_ms || 1000 }}</template>
          </el-table-column>
          <el-table-column prop="description" :label="t('nodeDetail.description')" min-width="100" show-overflow-tooltip>
            <template #default="{ row }">{{ row.description || '-' }}</template>
          </el-table-column>
          <el-table-column :label="t('nodeDetail.opLabel')" width="160" fixed="right">
            <template #default="{ row }">
              <el-button type="primary" link size="small" :icon="Edit" @click.stop="openGroupModal(row)">{{ t('nodeDetail.edit') }}</el-button>
              <el-button type="primary" link size="small" @click.stop="goToGroupTags(row)">{{ t('nodeDetail.tagList') }}</el-button>
              <el-button type="danger" link size="small" :icon="Delete" @click.stop="deleteGroup(row.id)">{{ t('common.delete') }}</el-button>
            </template>
          </el-table-column>
        </el-table>
        <el-empty v-else :description="t('nodeDetail.noPointGroupHint')" />
      </el-tab-pane>
      <el-tab-pane v-if="isNorth" name="subs">
        <template #label>{{ t('nodeDetail.subManage') }} <el-tag size="small" type="info">{{ subscriptions.length }}</el-tag></template>
        <div class="topic-section mb-2">
          <label class="topic-label">{{ t('nodeDetail.topicTemplate') }}</label>
          <div class="topic-row">
            <el-input
              v-model="topicTemplate"
              :placeholder="t('nodeDetail.topicPlaceholder')"
              class="topic-input font-mono"
              clearable
            />
            <el-button type="primary" size="small" :loading="topicSaving" @click="saveTopic">{{ t('nodeDetail.saveTopic') }}</el-button>
          </div>
          <div class="topic-hint">{{ t('nodeDetail.topicHint') }}</div>
        </div>
        <div class="tab-header">
          <h3>{{ t('nodeDetail.dataSubs') }}</h3>
          <el-button type="primary" size="small" :icon="Plus" :disabled="!southNodes.length" @click="openSubModal()">{{ t('nodeDetail.addSub') }}</el-button>
        </div>
        <el-alert v-if="!southNodes.length" type="warning" :title="t('nodeDetail.noSouthCreateFirst')" show-icon class="mb-2" />
        <el-table v-else-if="subscriptions.length" :data="subscriptions" size="small" stripe>
          <el-table-column :label="t('nodeDetail.southDevice')" min-width="100">
            <template #default="{ row }">{{ getSouthNodeName(row.south_node_id) }}</template>
          </el-table-column>
          <el-table-column :label="t('nodeDetail.subGroup')" min-width="100">
            <template #default="{ row }">{{ getSouthGroupName(row.south_node_id, row.group_id) }}</template>
          </el-table-column>
          <el-table-column :label="t('nodeDetail.opLabel')" width="90" fixed="right">
            <template #default="scope">
              <el-button type="danger" link size="small" :icon="Delete" @click="removeSub(scope.$index)">{{ t('nodeDetail.unsubscribeConfirmBtn') }}</el-button>
            </template>
          </el-table-column>
        </el-table>
        <el-empty v-else :description="t('nodeDetail.noSubsHint')" />
      </el-tab-pane>
    </el-tabs>

    <el-dialog v-model="showGroupModal" :title="editingGroup ? t('nodeDetail.editGroup') : t('nodeDetail.addGroup')" width="440px" destroy-on-close>
      <el-form :model="groupForm" label-width="100px" label-position="top">
        <el-form-item :label="t('nodeDetail.groupName')" required>
          <el-input v-model="groupForm.name" :placeholder="t('nodeDetail.exampleDefault')" clearable />
        </el-form-item>
        <el-form-item :label="t('nodeDetail.collectIntervalMs')">
          <el-input-number v-model="groupForm.interval_ms" :min="100" :step="100" style="width: 100%" />
        </el-form-item>
        <el-form-item :label="t('nodeDetail.description')">
          <el-input v-model="groupForm.description" :placeholder="t('nodeDetail.optional')" clearable />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="showGroupModal = false">{{ t('nodeDetail.cancel') }}</el-button>
        <el-button type="primary" :disabled="!groupForm.name" @click="saveGroup">{{ t('nodeDetail.save') }}</el-button>
      </template>
    </el-dialog>

    <el-dialog v-model="showTagModal" :title="editingTag ? t('nodeDetail.editTag') : t('nodeDetail.addTag')" width="440px" destroy-on-close>
      <el-form :model="tagForm" label-width="100px" label-position="top">
        <el-form-item :label="t('common.name')" required>
          <el-input v-model="tagForm.name" :placeholder="t('nodeDetail.exampleTemperature')" clearable />
        </el-form-item>
        <el-form-item :label="t('nodeDetail.dataGroup')" required>
          <el-select v-model="tagForm.group_id" :placeholder="t('nodeDetail.selectPlaceholder')" style="width: 100%">
            <el-option v-for="g in groups" :key="g.id" :label="g.name" :value="g.id" />
          </el-select>
        </el-form-item>
        <el-form-item :label="t('nodeDetail.typeLabel')">
          <el-select v-model="tagForm.data_type" :placeholder="t('nodeDetail.selectPlaceholder')" style="width: 100%">
            <el-option v-for="dt in dataTypes" :key="dt" :label="dt" :value="dt" />
          </el-select>
        </el-form-item>
        <el-form-item :label="t('nodeDetail.addressLabel')" required>
          <el-input v-model="tagForm.address" :placeholder="t('nodeDetail.addressPlaceholder')" clearable class="font-mono" />
        </el-form-item>
        <el-form-item :label="t('nodeDetail.attrLabel')">
          <el-select v-model="tagForm.attr" :placeholder="t('nodeDetail.selectPlaceholder')" style="width: 100%">
            <el-option v-for="a in attrOptions" :key="a.value" :label="a.label" :value="a.value" />
          </el-select>
        </el-form-item>
        <el-form-item :label="t('nodeDetail.description')">
          <el-input v-model="tagForm.description" :placeholder="t('nodeDetail.optional')" clearable />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="showTagModal = false">{{ t('nodeDetail.cancel') }}</el-button>
        <el-button type="primary" :disabled="!tagForm.name || !tagForm.group_id || !tagForm.address" @click="saveTag">{{ t('nodeDetail.save') }}</el-button>
      </template>
    </el-dialog>

    <el-dialog v-model="showBatchTagModal" :title="t('nodeDetail.batchAddPoints')" width="800px" destroy-on-close>
      <el-form label-position="top">
        <el-form-item :label="t('nodeDetail.selectGroup')" required>
          <el-select v-model="batchTagGroupId" :placeholder="t('nodeDetail.selectPlaceholder')" style="width: 100%">
            <el-option v-for="g in groups" :key="g.id" :label="g.name" :value="g.id" />
          </el-select>
        </el-form-item>
        <el-table :data="batchTagRows" size="small" border>
          <el-table-column :label="t('common.name')" min-width="100">
            <template #default="{ row, $index }">
              <el-input v-model="row.name" :placeholder="t('common.name')" size="small" />
            </template>
          </el-table-column>
          <el-table-column :label="t('nodeDetail.typeLabel')" width="90">
            <template #default="{ row }">
              <el-select v-model="row.data_type" :placeholder="t('nodeDetail.typeLabel')" size="small" style="width: 100%">
                <el-option v-for="dt in dataTypes" :key="dt" :label="dt" :value="dt" />
              </el-select>
            </template>
          </el-table-column>
          <el-table-column :label="t('nodeDetail.addressLabel')" min-width="100">
            <template #default="{ row }">
              <el-input v-model="row.address" :placeholder="t('nodeDetail.addressLabel')" size="small" class="font-mono" />
            </template>
          </el-table-column>
          <el-table-column :label="t('nodeDetail.attrLabel')" width="90">
            <template #default="{ row }">
              <el-select v-model="row.attr" size="small" style="width: 100%">
                <el-option v-for="a in attrOptions" :key="a.value" :label="a.label" :value="a.value" />
              </el-select>
            </template>
          </el-table-column>
          <el-table-column :label="t('nodeDetail.description')" min-width="80">
            <template #default="{ row }">
              <el-input v-model="row.description" :placeholder="t('nodeDetail.optional')" size="small" />
            </template>
          </el-table-column>
          <el-table-column :label="t('nodeDetail.opLabel')" width="70" fixed="right">
            <template #default="{ $index }">
              <el-button type="danger" link size="small" :disabled="batchTagRows.length <= 1" @click="removeBatchTagRow($index)">{{ t('common.delete') }}</el-button>
            </template>
          </el-table-column>
        </el-table>
        <el-button type="primary" text :icon="Plus" class="mt-1" @click="addBatchTagRow">+ {{ t('nodeDetail.add') }}</el-button>
      </el-form>
      <template #footer>
        <el-button @click="showBatchTagModal = false">{{ t('nodeDetail.cancel') }}</el-button>
        <el-button type="primary" @click="saveBatchTags">{{ t('nodeDetail.create') }}</el-button>
      </template>
    </el-dialog>

    <el-dialog v-model="showSubModal" :title="t('nodeDetail.addSub')" width="440px" destroy-on-close>
      <el-form :model="subForm" label-width="100px" label-position="top">
        <el-form-item :label="t('nodeDetail.southDevice')" required>
          <el-select v-model="subForm.south_node_id" :placeholder="t('nodeDetail.selectPlaceholder')" style="width: 100%">
            <el-option v-for="n in southNodes" :key="n.id" :label="n.name" :value="n.id" />
          </el-select>
        </el-form-item>
        <el-form-item :label="t('nodeDetail.dataGroup')" required>
          <el-select v-model="subForm.group_id" :placeholder="t('nodeDetail.selectPlaceholder')" style="width: 100%" :disabled="!southGroups.length">
            <el-option v-for="g in southGroups" :key="g.id" :label="g.name" :value="g.id" />
          </el-select>
          <div v-if="!southGroups.length && subForm.south_node_id" class="form-hint">{{ t('nodeDetail.noDataGroupHint') }}</div>
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="showSubModal = false">{{ t('nodeDetail.cancel') }}</el-button>
        <el-button type="primary" :disabled="!subForm.south_node_id || !subForm.group_id" @click="addSubscription">{{ t('nodeDetail.subscribe') }}</el-button>
      </template>
    </el-dialog>

    <el-dialog v-model="showNameModal" :title="t('nodeDetail.editName')" width="400px" destroy-on-close>
      <el-form label-position="top">
        <el-form-item :label="t('common.name')">
          <el-input v-model="editNameValue" :placeholder="t('nodeDetail.nodeNamePlaceholder')" clearable />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="showNameModal = false">{{ t('nodeDetail.cancel') }}</el-button>
        <el-button type="primary" :disabled="!editNameValue?.trim()" @click="saveNodeName">{{ t('nodeDetail.save') }}</el-button>
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
.groups-table-clickable :deep(.el-table__row) { cursor: pointer; }
.group-name-link { color: var(--el-color-primary); font-weight: 500; }
.topic-section { padding: 0.75rem 0; border-bottom: 1px solid var(--el-border-color-lighter); }
.topic-label { display: block; font-weight: 500; margin-bottom: 0.35rem; font-size: 0.9rem; }
.topic-row { display: flex; gap: 0.5rem; align-items: center; flex-wrap: wrap; }
.topic-input { flex: 1; min-width: 200px; }
.topic-hint { font-size: 0.8rem; color: var(--text-muted); margin-top: 0.25rem; }
</style>
