<script setup>
// 流程列表（v2.0 T1-③）：CRUD 入口 + 发布/下线/删除（Admin 动作，403 由后端裁决并提示）。
import { ref, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { useRouter } from 'vue-router'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Plus, Edit, Delete, VideoPlay, VideoPause, Refresh } from '@element-plus/icons-vue'
import { api } from '../api.js'
import { getErrorMessage } from '../i18n'
import PageHeader from '../components/PageHeader.vue'

const { t } = useI18n()
const router = useRouter()

const flows = ref([])
const loading = ref(false)
const error = ref('')
const createVisible = ref(false)
const creating = ref(false)
const draft = ref({ name: '', description: '' })

const statusTagType = (s) => ({ published: 'success', draft: 'info' }[s] || 'info')

async function load() {
  loading.value = true
  error.value = ''
  try {
    const r = await api.flows()
    flows.value = r.flows || []
  } catch (e) {
    error.value = t('flows.loadFailed') + getErrorMessage(t, e)
  } finally {
    loading.value = false
  }
}

function fmtTime(ms) {
  if (!ms) return '-'
  const d = new Date(ms)
  const p = (x) => String(x).padStart(2, '0')
  return `${d.getMonth() + 1}-${p(d.getDate())} ${p(d.getHours())}:${p(d.getMinutes())}`
}

function openEditor(row) {
  router.push(`/flows/${row.id}/edit`)
}

async function create() {
  if (!draft.value.name.trim()) {
    ElMessage.warning(t('flows.nameRequired'))
    return
  }
  creating.value = true
  try {
    const created = await api.createFlow({
      name: draft.value.name,
      description: draft.value.description || null,
      nodes: [],
      edges: [],
    })
    createVisible.value = false
    draft.value = { name: '', description: '' }
    ElMessage.success(t('flows.created'))
    router.push(`/flows/${created.id}/edit`)
  } catch (e) {
    ElMessage.error(getErrorMessage(t, e))
  } finally {
    creating.value = false
  }
}

async function publish(row) {
  try {
    await api.publishFlow(row.id)
    ElMessage.success(t('flows.publishOk'))
    await load()
  } catch (e) {
    // 校验失败信息直接呈现（后端返回完整校验详情）
    ElMessage.error(getErrorMessage(t, e))
  }
}

async function unpublish(row) {
  try {
    await api.unpublishFlow(row.id)
    ElMessage.success(t('flows.unpublishOk'))
    await load()
  } catch (e) {
    ElMessage.error(getErrorMessage(t, e))
  }
}

async function remove(row) {
  try {
    await ElMessageBox.confirm(t('flows.deleteConfirm', { name: row.name }), {
      type: 'warning',
    })
  } catch {
    return
  }
  try {
    await api.deleteFlow(row.id)
    ElMessage.success(t('flows.deleteOk'))
    await load()
  } catch (e) {
    ElMessage.error(getErrorMessage(t, e))
  }
}

onMounted(load)
</script>

<template>
  <div class="flows-page">
    <PageHeader :title="t('flows.title')" :subtitle="t('flows.subtitle')">
      <template #actions>
        <el-button :icon="Refresh" :loading="loading" @click="load" />
        <el-button type="primary" :icon="Plus" @click="createVisible = true">
          {{ t('flows.new') }}
        </el-button>
      </template>
    </PageHeader>

    <el-card shadow="never">
      <div v-if="error" class="flows-error">{{ error }}</div>
      <el-table :data="flows" v-loading="loading">
        <el-table-column prop="name" :label="t('flows.name')" min-width="180" />
        <el-table-column prop="description" :label="t('flows.description')" min-width="200" show-overflow-tooltip />
        <el-table-column :label="t('flows.status')" width="110">
          <template #default="{ row }">
            <el-tag :type="statusTagType(row.status)">
              {{ t(`flows.status_${row.status}`) }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column prop="version" :label="t('flows.version')" width="90" />
        <el-table-column :label="t('flows.updatedAt')" width="150">
          <template #default="{ row }">{{ fmtTime(row.updated_at) }}</template>
        </el-table-column>
        <el-table-column :label="t('flows.actions')" width="280" fixed="right">
          <template #default="{ row }">
            <el-button size="small" type="primary" :icon="Edit" @click="openEditor(row)">
              {{ t('flows.edit') }}
            </el-button>
            <el-button
              v-if="row.status !== 'published'"
              size="small"
              type="success"
              :icon="VideoPlay"
              @click="publish(row)"
            >{{ t('flows.publish') }}</el-button>
            <el-button
              v-else
              size="small"
              type="warning"
              :icon="VideoPause"
              @click="unpublish(row)"
            >{{ t('flows.unpublish') }}</el-button>
            <el-button size="small" type="danger" :icon="Delete" @click="remove(row)" />
          </template>
        </el-table-column>
        <template #empty>
          <el-empty :description="t('flows.empty')" />
        </template>
      </el-table>
    </el-card>

    <el-dialog v-model="createVisible" :title="t('flows.new')" width="480">
      <el-form label-width="90">
        <el-form-item :label="t('flows.name')" required>
          <el-input v-model="draft.name" maxlength="64" />
        </el-form-item>
        <el-form-item :label="t('flows.description')">
          <el-input v-model="draft.description" type="textarea" :rows="2" maxlength="256" />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="createVisible = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="creating" @click="create">
          {{ t('common.create') }}
        </el-button>
      </template>
    </el-dialog>
  </div>
</template>

<style scoped>
.flows-page {
  display: flex;
  flex-direction: column;
  gap: 16px;
}
.flows-error {
  color: var(--el-color-danger);
  font-size: 13px;
  margin-bottom: 8px;
}
</style>
