<script setup>
import { ref, computed, onMounted } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { getErrorMessage } from '../i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Back, Plus, Edit, Delete, RefreshRight } from '@element-plus/icons-vue'
import { api } from '../api.js'

const route = useRoute()
const router = useRouter()
const { t } = useI18n()

const nodeId = computed(() => route.params.id)
const groupId = computed(() => route.params.groupId)

const node = ref(null)
const group = ref(null)
const tags = ref([])
const loading = ref(false)
const error = ref('')
const showTagModal = ref(false)
const showBatchTagModal = ref(false)
const editingTag = ref(null)
const tagForm = ref({ name: '', group_id: '', data_type: 'Float64', address: '', attr: 'read', description: '' })
const batchTagRows = ref([{ name: '', data_type: 'Float64', address: '', attr: 'read', description: '' }])
const tagValues = ref({})
const readingTags = ref(false)

const attrOptions = [
  { value: 'read', label: 'Read' },
  { value: 'write', label: 'Write' },
  { value: 'readwrite', label: 'ReadWrite' }
]
const dataTypes = ['Bool', 'Int8', 'Int16', 'Int32', 'Int64', 'UInt8', 'UInt16', 'UInt32', 'UInt64', 'Float32', 'Float64', 'String', 'Bytes']

async function load() {
  loading.value = true
  error.value = ''
  try {
    node.value = await api.node(nodeId.value)
    const groups = await api.groups(nodeId.value)
    group.value = groups.find(g => g.id === groupId.value) || null
    const allTags = await api.tags(nodeId.value)
    tags.value = allTags.filter(tag => tag.group_id === groupId.value)
  } catch (e) {
    error.value = getErrorMessage(t, e)
  } finally {
    loading.value = false
  }
}

function goBack() {
  router.push(`/south/${nodeId.value}`)
}

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
      group_id: groupId.value,
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
    await load()
    ElMessage.success(t('nodeDetail.saveGroupSuccess'))
  } catch (e) {
    error.value = t('nodeDetail.saveTagFailed') + getErrorMessage(t, e)
  }
}

async function deleteTag(tid) {
  try {
    await ElMessageBox.confirm(t('nodeDetail.deleteTagConfirm'), t('common.confirmDelete'), { type: 'warning', confirmButtonText: t('common.delete'), cancelButtonText: t('common.cancel') })
    await api.deleteTag(nodeId.value, tid)
    await load()
    ElMessage.success(t('nodeDetail.deleteTagSuccess'))
  } catch (e) {
    if (e !== 'cancel') error.value = t('nodeDetail.deleteTagFailed') + getErrorMessage(t, e)
  }
}

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

function openBatchTagModal() {
  batchTagRows.value = [{ name: '', data_type: 'Float64', address: '', attr: 'read', description: '' }]
  showBatchTagModal.value = true
}

function addBatchTagRow() {
  batchTagRows.value.push({ name: '', data_type: 'Float64', address: '', attr: 'read', description: '' })
}

function removeBatchTagRow(index) {
  batchTagRows.value.splice(index, 1)
}

async function saveBatchTags() {
  const payload = {
    tags: batchTagRows.value
      .filter(r => (r.name || '').trim() && (r.address || '').trim())
      .map(r => ({ name: r.name.trim(), address: r.address.trim(), group_id: groupId.value, data_type: r.data_type || 'Float64', attr: r.attr || 'read', description: r.description || '' }))
  }
  if (!payload.tags.length) {
    ElMessage.warning(t('nodeDetail.batchNameAddressRequired'))
    return
  }
  try {
    await api.batchCreateTags(nodeId.value, payload.tags)
    showBatchTagModal.value = false
    await load()
    ElMessage.success(t('nodeDetail.batchCreated', { n: payload.tags.length }))
  } catch (e) {
    error.value = t('nodeDetail.batchCreateFailed') + getErrorMessage(t, e)
  }
}

onMounted(load)
</script>

<template>
  <div class="group-detail">
    <div class="page-header">
      <div class="header-left">
        <el-button :icon="Back" @click="goBack">{{ t('nodeDetail.cancelBack') }}</el-button>
        <el-breadcrumb separator="/" class="breadcrumb">
          <el-breadcrumb-item :to="{ path: '/south' }">{{ t('south.title') }}</el-breadcrumb-item>
          <el-breadcrumb-item :to="{ path: `/south/${nodeId}` }">{{ node?.name || nodeId }}</el-breadcrumb-item>
          <el-breadcrumb-item>{{ group?.name || groupId }}</el-breadcrumb-item>
        </el-breadcrumb>
      </div>
    </div>

    <el-alert v-if="error" type="error" :title="error" closable show-icon @close="error = ''" class="mb-2" />

    <el-skeleton v-if="loading" :rows="6" animated />

    <template v-else-if="group">
      <div class="group-info card mb-3">
        <h3>{{ group.name }}</h3>
        <div class="group-meta">
          <span>{{ t('nodeDetail.collectIntervalMs') }}: {{ group.interval_ms || 1000 }} ms</span>
          <span v-if="group.description">{{ t('nodeDetail.description') }}: {{ group.description }}</span>
        </div>
      </div>

      <div class="tab-header mb-2">
        <h3>{{ t('nodeDetail.dataTags') }}</h3>
        <div class="header-actions">
          <el-button size="small" :icon="RefreshRight" :loading="readingTags" :disabled="!tags.length" @click="readAllTags">
            {{ readingTags ? t('nodeDetail.readAllReading') : t('nodeDetail.readAll') }}
          </el-button>
          <el-button size="small" @click="openBatchTagModal()">{{ t('nodeDetail.batchAdd') }}</el-button>
          <el-button type="primary" size="small" :icon="Plus" @click="openTagModal()">{{ t('nodeDetail.addPoint') }}</el-button>
        </div>
      </div>

      <el-table v-if="tags.length" :data="tags" size="small" stripe>
        <el-table-column prop="name" :label="t('common.name')" min-width="80" />
        <el-table-column prop="address" :label="t('nodeDetail.addressLabel')" min-width="80">
          <template #default="{ row }"><span class="font-mono">{{ row.address || '-' }}</span></template>
        </el-table-column>
        <el-table-column :label="t('nodeDetail.typeLabel')" width="70">
          <template #default="{ row }"><el-tag size="small" type="info">{{ row.data_type || '-' }}</el-tag></template>
        </el-table-column>
        <el-table-column :label="t('nodeDetail.attrLabel')" width="70">
          <template #default="{ row }">{{ row.attr === 'write' ? 'Write' : row.attr === 'readwrite' ? 'RW' : 'Read' }}</template>
        </el-table-column>
        <el-table-column :label="t('nodeDetail.currentValue')" min-width="80">
          <template #default="{ row }">
            <span v-if="tagValues[row.id] !== undefined">{{ typeof tagValues[row.id] === 'object' ? JSON.stringify(tagValues[row.id]) : tagValues[row.id] }}</span>
            <span v-else class="text-muted">-</span>
          </template>
        </el-table-column>
        <el-table-column prop="description" :label="t('nodeDetail.description')" min-width="60" show-overflow-tooltip>
          <template #default="{ row }">{{ row.description || '-' }}</template>
        </el-table-column>
        <el-table-column :label="t('nodeDetail.opLabel')" width="100" fixed="right">
          <template #default="{ row }">
            <el-button type="primary" link size="small" :icon="Edit" @click="openTagModal(row)">{{ t('nodeDetail.edit') }}</el-button>
            <el-button type="danger" link size="small" :icon="Delete" @click="deleteTag(row.id)">{{ t('common.delete') }}</el-button>
          </template>
        </el-table-column>
      </el-table>
      <el-empty v-else :description="t('nodeDetail.noTagsHint')" />
    </template>

    <el-empty v-else :description="t('nodeDetail.noDataGroupHint')" />

    <el-dialog v-model="showTagModal" :title="editingTag ? t('nodeDetail.editTag') : t('nodeDetail.addTag')" width="440px" destroy-on-close>
      <el-form :model="tagForm" label-width="100px" label-position="top">
        <el-form-item :label="t('common.name')" required>
          <el-input v-model="tagForm.name" :placeholder="t('nodeDetail.exampleTemperature')" clearable />
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
        <el-button type="primary" :disabled="!tagForm.name || !tagForm.address" @click="saveTag">{{ t('nodeDetail.save') }}</el-button>
      </template>
    </el-dialog>

    <el-dialog v-model="showBatchTagModal" :title="t('nodeDetail.batchAddPoints')" width="800px" destroy-on-close>
      <el-form label-position="top">
        <el-table :data="batchTagRows" size="small" border>
          <el-table-column :label="t('common.name')" min-width="100">
            <template #default="{ row }">
              <el-input v-model="row.name" :placeholder="t('common.name')" size="small" />
            </template>
          </el-table-column>
          <el-table-column :label="t('nodeDetail.typeLabel')" width="90">
            <template #default="{ row }">
              <el-select v-model="row.data_type" size="small" style="width: 100%">
                <el-option v-for="dt in dataTypes" :key="dt" :label="dt" :value="dt" />
              </el-select>
            </template>
          </el-table-column>
          <el-table-column :label="t('nodeDetail.addressLabel')" min-width="100">
            <template #default="{ row }">
              <el-input v-model="row.address" size="small" class="font-mono" />
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
              <el-input v-model="row.description" size="small" />
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
  </div>
</template>

<style scoped>
.group-detail {
  padding: 16px;
}
.page-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  margin-bottom: 16px;
}
.header-left {
  display: flex;
  align-items: center;
  gap: 12px;
}
.breadcrumb {
  font-size: 14px;
}
.group-info.card {
  padding: 16px;
  border-radius: 8px;
  background: var(--el-fill-color-light);
}
.group-info h3 {
  margin: 0 0 8px 0;
  font-size: 18px;
}
.group-meta {
  display: flex;
  gap: 16px;
  color: var(--el-text-color-secondary);
  font-size: 13px;
}
.tab-header {
  display: flex;
  align-items: center;
  justify-content: space-between;
  flex-wrap: wrap;
  gap: 8px;
}
.tab-header h3 {
  margin: 0;
  font-size: 16px;
}
.header-actions {
  display: flex;
  gap: 8px;
}
.font-mono {
  font-family: monospace;
}
.text-muted {
  color: var(--el-text-color-secondary);
}
.mb-2 { margin-bottom: 8px; }
.mb-3 { margin-bottom: 16px; }
.mt-1 { margin-top: 8px; }
</style>
