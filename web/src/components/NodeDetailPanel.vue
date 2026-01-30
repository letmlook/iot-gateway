<script setup>
import { ref, onMounted, computed, watch } from 'vue'
import { useI18n } from 'vue-i18n'
import { getErrorMessage } from '../i18n'
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
const activeTab = ref(props.kind === 'north' ? 'subs' : 'groups')
// 北向主题（来自节点配置 topic_template）
const DEFAULT_TOPIC_TEMPLATE = 'gateway/data/${node_id}/${group_id}'
const nodeSettingRef = ref(null)
const topicTemplate = ref('')
const topicSaving = ref(false)

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
  }
}

async function loadTopicFromSetting() {
  try {
    const setting = await api.nodeSetting(props.nodeId)
    nodeSettingRef.value = setting
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
    const cfg = { ...(nodeSettingRef.value?.config || {}), topic_template: v }
    await api.updateNodeSetting(props.nodeId, { config: cfg })
    nodeSettingRef.value = nodeSettingRef.value ? { ...nodeSettingRef.value, config: cfg } : { config: cfg }
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
    ElMessage.success(t('nodeDetail.saveGroupSuccess'))
  } catch (e) {
    error.value = t('nodeDetail.saveGroupFailed') + getErrorMessage(t, e)
  }
}

async function deleteGroup(gid) {
  try {
    await ElMessageBox.confirm(t('nodeDetail.deleteGroupConfirm'), t('common.confirmDelete'), { type: 'warning', confirmButtonText: t('common.delete'), cancelButtonText: t('common.cancel') })
    await api.deleteGroup(props.nodeId, gid)
    await loadGroups()
    await loadTags()
    ElMessage.success(t('nodeDetail.deleteGroupSuccess'))
  } catch (e) {
    if (e !== 'cancel') error.value = t('nodeDetail.deleteGroupFailed') + getErrorMessage(t, e)
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
    ElMessage.success(t('nodeDetail.saveGroupSuccess'))
  } catch (e) {
    error.value = t('nodeDetail.saveTagFailed') + getErrorMessage(t, e)
  }
}

async function deleteTag(tid) {
  try {
    await ElMessageBox.confirm(t('nodeDetail.deleteTagConfirm'), t('common.confirmDelete'), { type: 'warning', confirmButtonText: t('common.delete'), cancelButtonText: t('common.cancel') })
    await api.deleteTag(props.nodeId, tid)
    await loadTags()
    ElMessage.success(t('nodeDetail.deleteTagSuccess'))
  } catch (e) {
    if (e !== 'cancel') error.value = t('nodeDetail.deleteTagFailed') + getErrorMessage(t, e)
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
    error.value = t('nodeDetail.readTagsFailed') + getErrorMessage(t, e)
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
    ElMessage.success(t('nodeDetail.addSubSuccess'))
  } catch (e) {
    error.value = t('nodeDetail.addSubFailed') + getErrorMessage(t, e)
  }
}

async function removeSub(index) {
  try {
    await ElMessageBox.confirm(t('nodeDetail.unsubscribeConfirm'), t('common.confirm'), { type: 'warning', confirmButtonText: t('nodeDetail.unsubscribeConfirmBtn'), cancelButtonText: t('nodeDetail.cancelBack') })
    const newSubs = subscriptions.value.filter((_, i) => i !== index)
    await api.setSubscriptions(props.nodeId, newSubs)
    await loadSubscriptions()
    ElMessage.success(t('nodeDetail.unsubscribeSuccess'))
  } catch (e) {
    if (e !== 'cancel') error.value = t('nodeDetail.unsubscribeFailed') + getErrorMessage(t, e)
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
    error.value = t('nodeDetail.loadConfigFailed') + getErrorMessage(t, e)
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
    error.value = t('nodeDetail.saveFailed') + getErrorMessage(t, e)
  }
}

async function toggleNode() {
  try {
    if (node.value.state === 'running') {
      await api.stopNode(props.nodeId)
      ElMessage.success(t('nodeDetail.stopSuccess'))
    } else {
      await api.startNode(props.nodeId)
      ElMessage.success(t('nodeDetail.startSuccess'))
    }
    await loadNode()
    emit('refresh')
  } catch (e) {
    error.value = t('nodeDetail.opFailed') + getErrorMessage(t, e)
  }
}

function openNameInline() {
  editNameValue.value = node.value?.name || ''
  showNameInline.value = true
}
async function saveNodeName() {
  const name = (editNameValue.value || '').trim()
  if (!name) { ElMessage.warning(t('nodeDetail.nameRequired')); return }
  try {
    await api.updateNode(props.nodeId, { name })
    showNameInline.value = false
    await loadNode()
    emit('refresh')
    ElMessage.success(t('nodeDetail.nameUpdated'))
  } catch (e) {
    error.value = t('nodeDetail.updateNameFailed') + getErrorMessage(t, e)
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
  if (!valid.length) { ElMessage.warning(t('nodeDetail.batchNameAddressRequired')); return }
  if (!batchTagGroupId.value) { ElMessage.warning(t('nodeDetail.selectGroupRequired')); return }
  try {
    const payload = valid.map(r => ({
      name: r.name, address: r.address, group_id: batchTagGroupId.value,
      data_type: r.data_type || 'Float64', attr: r.attr || 'read', description: r.description || undefined
    }))
    await api.batchCreateTags(props.nodeId, payload)
    showBatchTagPanel.value = false
    await loadTags()
    ElMessage.success(t('nodeDetail.batchCreated', { n: payload.length }))
  } catch (e) {
    error.value = t('nodeDetail.batchCreateFailed') + getErrorMessage(t, e)
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
        <el-button text :icon="Close" @click="emit('close')">{{ t('nodeDetail.close') }}</el-button>
        <div v-if="node" class="node-info">
          <span v-if="!showNameInline" class="node-name">{{ node.name }}</span>
          <template v-else>
            <el-input v-model="editNameValue" size="small" :placeholder="t('common.name')" style="width: 140px" />
            <el-button size="small" type="primary" @click="saveNodeName">{{ t('nodeDetail.save') }}</el-button>
            <el-button size="small" @click="showNameInline = false">{{ t('nodeDetail.cancel') }}</el-button>
          </template>
          <el-button v-if="!showNameInline" type="primary" link size="small" :icon="EditPen" @click="openNameInline">{{ t('nodeDetail.edit') }}</el-button>
          <el-tag size="small" type="info">{{ node.plugin_name }}</el-tag>
          <el-tag :type="node.state === 'running' ? 'success' : node.state === 'error' ? 'danger' : 'info'" size="small" effect="light">
            {{ node.state === 'running' ? t('common.running') : node.state === 'error' ? t('common.error') : t('common.stopped') }}
          </el-tag>
        </div>
      </div>
      <div v-if="node" class="header-actions">
        <el-button size="small" :icon="Setting" @click="openSettingPanel">{{ t('nodeDetail.config') }}</el-button>
        <el-button
          :type="node.state === 'running' ? 'warning' : 'success'"
          size="small"
          :icon="node.state === 'running' ? VideoPause : VideoPlay"
          @click="toggleNode"
        >
          {{ node.state === 'running' ? t('common.stop') : t('common.start') }}
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
          <el-button text size="small" @click="showSettingPanel = false">{{ t('nodeDetail.collapse') }}</el-button>
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
        <el-tab-pane v-if="!isNorth" name="groups">
          <template #label>{{ t('nodeDetail.groups') }} <el-tag size="small" type="info">{{ groups.length }}</el-tag></template>
          <div class="tab-toolbar">
            <el-button type="primary" size="small" :icon="Plus" @click="openGroupForm()">{{ t('nodeDetail.addGroup') }}</el-button>
          </div>
          <div v-if="showGroupForm" class="inline-section">
            <el-form :model="groupForm" label-position="top" size="small">
              <el-form-item :label="t('nodeDetail.groupName')" required>
                <el-input v-model="groupForm.name" :placeholder="t('nodeDetail.exampleDefault')" clearable />
              </el-form-item>
              <el-form-item :label="t('nodeDetail.collectIntervalMs')">
                <el-input-number v-model="groupForm.interval_ms" :min="100" :step="100" style="width: 100%" />
              </el-form-item>
              <el-form-item :label="t('nodeDetail.description')">
                <el-input v-model="groupForm.description" :placeholder="t('nodeDetail.optional')" clearable />
              </el-form-item>
              <el-form-item>
                <el-button @click="showGroupForm = false">{{ t('nodeDetail.cancel') }}</el-button>
                <el-button type="primary" :disabled="!groupForm.name" @click="saveGroup">{{ t('nodeDetail.save') }}</el-button>
              </el-form-item>
            </el-form>
          </div>
          <el-table v-if="groups.length" :data="groups" size="small" stripe max-height="240">
            <el-table-column prop="name" :label="t('nodeDetail.groupNameLabel')" min-width="80" />
            <el-table-column :label="t('nodeDetail.tagCount')" width="50">
              <template #default="{ row }">{{ tags.filter(t => t.group_id === row.id).length }}</template>
            </el-table-column>
            <el-table-column :label="t('nodeDetail.intervalLabel')" width="60">
              <template #default="{ row }">{{ row.interval_ms || 1000 }}</template>
            </el-table-column>
            <el-table-column :label="t('nodeDetail.opLabel')" width="100" fixed="right">
              <template #default="{ row }">
                <el-button type="primary" link size="small" :icon="Edit" @click="openGroupForm(row)">{{ t('nodeDetail.edit') }}</el-button>
                <el-button type="danger" link size="small" :icon="Delete" @click="deleteGroup(row.id)">{{ t('common.delete') }}</el-button>
              </template>
            </el-table-column>
          </el-table>
          <el-empty v-else :description="t('nodeDetail.noGroups')" :image-size="48" />
        </el-tab-pane>

        <el-tab-pane v-if="!isNorth" name="tags">
          <template #label>{{ t('nodeDetail.tags') }} <el-tag size="small" type="info">{{ tags.length }}</el-tag></template>
          <div class="tab-toolbar">
            <el-button size="small" :icon="RefreshRight" :loading="readingTags" :disabled="!tags.length" @click="readAllTags">{{ t('nodeDetail.read') }}</el-button>
            <el-button size="small" :disabled="!groups.length" @click="openBatchTagPanel">{{ t('nodeDetail.batchAdd') }}</el-button>
            <el-button type="primary" size="small" :icon="Plus" :disabled="!groups.length" @click="openTagForm()">{{ t('nodeDetail.add') }}</el-button>
          </div>
          <div v-if="showTagForm" class="inline-section">
            <el-form :model="tagForm" label-position="top" size="small">
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
                <el-select v-model="tagForm.attr" style="width: 100%">
                  <el-option v-for="a in attrOptions" :key="a.value" :label="a.label" :value="a.value" />
                </el-select>
              </el-form-item>
              <el-form-item>
                <el-button @click="showTagForm = false">{{ t('nodeDetail.cancel') }}</el-button>
                <el-button type="primary" :disabled="!tagForm.name || !tagForm.group_id || !tagForm.address" @click="saveTag">{{ t('nodeDetail.save') }}</el-button>
              </el-form-item>
            </el-form>
          </div>
          <div v-if="showBatchTagPanel" class="inline-section">
            <el-form label-position="top" size="small">
              <el-form-item :label="t('nodeDetail.selectGroup')" required>
                <el-select v-model="batchTagGroupId" :placeholder="t('nodeDetail.selectPlaceholder')" style="width: 100%">
                  <el-option v-for="g in groups" :key="g.id" :label="g.name" :value="g.id" />
                </el-select>
              </el-form-item>
              <el-table :data="batchTagRows" size="small" border max-height="160">
                <el-table-column :label="t('common.name')" min-width="80">
                  <template #default="{ row }"><el-input v-model="row.name" size="small" :placeholder="t('common.name')" /></template>
                </el-table-column>
                <el-table-column :label="t('nodeDetail.typeLabel')" width="80">
                  <template #default="{ row }">
                    <el-select v-model="row.data_type" size="small" style="width: 100%">
                      <el-option v-for="dt in dataTypes" :key="dt" :label="dt" :value="dt" />
                    </el-select>
                  </template>
                </el-table-column>
                <el-table-column :label="t('nodeDetail.addressLabel')" min-width="80">
                  <template #default="{ row }"><el-input v-model="row.address" size="small" class="font-mono" /></template>
                </el-table-column>
                <el-table-column :label="t('nodeDetail.opLabel')" width="50">
                  <template #default="{ $index }">
                    <el-button type="danger" link size="small" :disabled="batchTagRows.length <= 1" @click="removeBatchTagRow($index)">{{ t('nodeDetail.deleteShort') }}</el-button>
                  </template>
                </el-table-column>
              </el-table>
              <el-button type="primary" text size="small" :icon="Plus" @click="addBatchTagRow">+ {{ t('common.items') }}</el-button>
            </el-form>
            <div class="inline-section-footer">
              <el-button size="small" @click="showBatchTagPanel = false">{{ t('nodeDetail.cancel') }}</el-button>
              <el-button size="small" type="primary" @click="saveBatchTags">{{ t('nodeDetail.create') }}</el-button>
            </div>
          </div>
          <el-alert v-if="!groups.length" type="warning" :title="t('nodeDetail.noGroupsHint')" show-icon class="mb-2" />
          <el-table v-else-if="tags.length" :data="tags" size="small" stripe max-height="220">
            <el-table-column prop="name" :label="t('common.name')" min-width="70" />
            <el-table-column prop="address" :label="t('nodeDetail.addressLabel')" min-width="70" show-overflow-tooltip>
              <template #default="{ row }"><span class="font-mono">{{ row.address || '-' }}</span></template>
            </el-table-column>
            <el-table-column :label="t('nodeDetail.typeLabel')" width="65">
              <template #default="{ row }"><el-tag size="small" type="info">{{ row.data_type || '-' }}</el-tag></template>
            </el-table-column>
            <el-table-column :label="t('nodeDetail.valueLabel')" min-width="60">
              <template #default="{ row }">
                <span v-if="tagValues[row.id] !== undefined">{{ typeof tagValues[row.id] === 'object' ? JSON.stringify(tagValues[row.id]) : tagValues[row.id] }}</span>
                <span v-else class="text-muted">-</span>
              </template>
            </el-table-column>
            <el-table-column :label="t('nodeDetail.opLabel')" width="80" fixed="right">
              <template #default="{ row }">
                <el-button type="primary" link size="small" :icon="Edit" @click="openTagForm(row)">{{ t('nodeDetail.edit') }}</el-button>
                <el-button type="danger" link size="small" :icon="Delete" @click="deleteTag(row.id)">{{ t('nodeDetail.deleteShort') }}</el-button>
              </template>
            </el-table-column>
          </el-table>
          <el-empty v-else :description="t('nodeDetail.noTags')" :image-size="48" />
        </el-tab-pane>

        <el-tab-pane v-if="isNorth" name="subs">
          <template #label>{{ t('nodeDetail.subManage') }} <el-tag size="small" type="info">{{ subscriptions.length }}</el-tag></template>
          <div v-if="isNorth" class="topic-section">
            <label class="topic-label">{{ t('nodeDetail.topicTemplate') }}</label>
            <div class="topic-row">
              <el-input
                v-model="topicTemplate"
                :placeholder="t('nodeDetail.topicPlaceholder')"
                size="small"
                class="topic-input font-mono"
                clearable
              />
              <el-button type="primary" size="small" :loading="topicSaving" @click="saveTopic">{{ t('nodeDetail.saveTopic') }}</el-button>
            </div>
            <div class="topic-hint">{{ t('nodeDetail.topicHint') }}</div>
          </div>
          <div class="tab-toolbar">
            <el-button type="primary" size="small" :icon="Plus" :disabled="!southNodes.length" @click="openSubForm">{{ t('nodeDetail.addSub') }}</el-button>
          </div>
          <div v-if="showSubForm" class="inline-section">
            <el-form :model="subForm" label-position="top" size="small">
              <el-form-item :label="t('nodeDetail.southDevice')" required>
                <el-select v-model="subForm.south_node_id" :placeholder="t('nodeDetail.selectPlaceholder')" style="width: 100%">
                  <el-option v-for="n in southNodes" :key="n.id" :label="n.name" :value="n.id" />
                </el-select>
              </el-form-item>
              <el-form-item :label="t('nodeDetail.dataGroup')" required>
                <el-select v-model="subForm.group_id" :placeholder="t('nodeDetail.selectPlaceholder')" style="width: 100%" :disabled="!southGroups.length">
                  <el-option v-for="g in southGroups" :key="g.id" :label="g.name" :value="g.id" />
                </el-select>
              </el-form-item>
              <el-form-item>
                <el-button @click="showSubForm = false">{{ t('nodeDetail.cancel') }}</el-button>
                <el-button type="primary" :disabled="!subForm.south_node_id || !subForm.group_id" @click="addSubscription">{{ t('nodeDetail.subscribe') }}</el-button>
              </el-form-item>
            </el-form>
          </div>
          <el-alert v-if="!southNodes.length" type="warning" :title="t('nodeDetail.noSouthToSub')" show-icon class="mb-2" />
          <el-table v-else-if="subscriptions.length" :data="subscriptions" size="small" stripe max-height="240">
            <el-table-column :label="t('nodeDetail.southDevice')" min-width="80">
              <template #default="{ row }">{{ getSouthNodeName(row.south_node_id) }}</template>
            </el-table-column>
            <el-table-column :label="t('nodeDetail.groups')" min-width="70">
              <template #default="{ row }">{{ getSouthGroupName(row.south_node_id, row.group_id) }}</template>
            </el-table-column>
            <el-table-column :label="t('nodeDetail.opLabel')" width="70" fixed="right">
              <template #default="scope">
                <el-button type="danger" link size="small" :icon="Delete" @click="removeSub(scope.$index)">{{ t('nodeDetail.unsubscribeShort') }}</el-button>
              </template>
            </el-table-column>
          </el-table>
          <el-empty v-else :description="t('nodeDetail.noSubs')" :image-size="48" />
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
.topic-section { padding: 0.5rem 0; margin-bottom: 0.5rem; border-bottom: 1px solid var(--el-border-color-lighter); }
.topic-label { display: block; font-weight: 500; margin-bottom: 0.25rem; font-size: 0.85rem; }
.topic-row { display: flex; gap: 0.5rem; align-items: center; flex-wrap: wrap; }
.topic-input { flex: 1; min-width: 160px; }
.topic-hint { font-size: 0.75rem; color: var(--text-muted); margin-top: 0.2rem; }
</style>
