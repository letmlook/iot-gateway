<script setup>
import { ref, computed, onMounted } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { getErrorMessage } from '../i18n'
import { ElMessage } from 'element-plus'
import { ArrowLeft } from '@element-plus/icons-vue'
import { api } from '../api.js'
import PageHeader from '../components/PageHeader.vue'
import NodeConfigForm from '../components/NodeConfigForm.vue'

const route = useRoute()
const router = useRouter()
const { t } = useI18n()

const nodeId = computed(() => route.params.id)
const isNorth = computed(() => route.path.startsWith('/north'))
const node = ref(null)
const configForm = ref({})
const loading = ref(true)
const error = ref('')
const saving = ref(false)

async function loadNodeAndSetting() {
  if (!nodeId.value) return
  loading.value = true
  error.value = ''
  try {
    node.value = await api.node(nodeId.value)
    const setting = await api.nodeSetting(nodeId.value)
    configForm.value = { ...(setting?.config || {}) }
  } catch (e) {
    error.value = t('nodeDetail.loadConfigFailed') + getErrorMessage(t, e)
  } finally {
    loading.value = false
  }
}

function goBack() {
  router.push(isNorth.value ? `/north/${nodeId.value}` : `/south/${nodeId.value}`)
}

function onConfigUpdate(v) {
  configForm.value = { ...v }
}

async function save() {
  if (!node.value) return
  saving.value = true
  error.value = ''
  try {
    await api.updateNodeSetting(nodeId.value, { config: { ...configForm.value } })
    ElMessage.success(t('nodeDetail.saveSuccess'))
    goBack()
  } catch (e) {
    error.value = t('nodeDetail.saveFailed') + getErrorMessage(t, e)
  } finally {
    saving.value = false
  }
}

onMounted(loadNodeAndSetting)
</script>

<template>
  <div class="page-container config-page-fill">
    <PageHeader :title="t('nodeDetail.nodeConfig')" :subtitle="node?.name || nodeId" />
    <div class="detail-meta">
      <el-button :icon="ArrowLeft" @click="goBack">{{ t('createNode.back') }}</el-button>
    </div>
    <el-alert v-if="error" type="error" :title="error" closable show-icon @close="error = ''" class="mb-2" />
    <el-card v-if="node" class="config-card-fill">
      <NodeConfigForm
        v-if="node"
        :plugin-name="node.plugin_name"
        :kind="isNorth ? 'north' : 'south'"
        :model-value="configForm"
        @update:model-value="onConfigUpdate"
      />
      <template v-if="node">
        <div class="config-actions">
          <el-button @click="goBack">{{ t('common.cancel') }}</el-button>
          <el-button type="primary" :loading="saving" @click="save">{{ t('nodeDetail.saveConfig') }}</el-button>
        </div>
      </template>
    </el-card>
    <el-skeleton v-else-if="loading" :rows="6" animated />
  </div>
</template>

<style scoped>
.config-page-fill { flex: 1; min-height: 0; display: flex; flex-direction: column; }
.detail-meta { display: flex; align-items: center; gap: 1rem; margin-bottom: 1rem; flex-shrink: 0; }
.config-card-fill { flex: 1; min-height: 0; display: flex; flex-direction: column; padding: 1.5rem; overflow-y: auto; }
.config-card-fill :deep(.el-card__body) { display: flex; flex-direction: column; gap: 1rem; }
.config-actions { display: flex; gap: 0.75rem; flex-shrink: 0; padding-top: 0.5rem; }
.mb-2 { margin-bottom: 1rem; }
</style>
