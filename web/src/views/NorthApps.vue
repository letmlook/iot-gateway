<script setup>
import { ref, inject, onMounted, computed } from 'vue'
import { useRouter } from 'vue-router'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Plus, VideoPlay, VideoPause, Edit, Delete } from '@element-plus/icons-vue'
import { api } from '../api.js'

const router = useRouter()
const northPlugins = inject('northPlugins', ref([]))

const nodes = ref([])
const loading = ref(false)
const error = ref('')
const showCreateModal = ref(false)

const createForm = ref({
  name: '',
  plugin_name: '',
  config: '{}'
})

const pluginOptions = computed(() => {
  return northPlugins.value.map(p => ({
    name: p[0],
    description: p[1],
    version: p[2]
  }))
})

async function loadNodes() {
  loading.value = true
  error.value = ''
  try {
    const all = await api.nodes()
    nodes.value = all.filter(n => n.kind === 'north')
  } catch (e) {
    error.value = '加载节点失败: ' + e.message
  } finally {
    loading.value = false
  }
}

async function createNode() {
  let config = {}
  try {
    config = JSON.parse(createForm.value.config || '{}')
  } catch {
    ElMessage.error('config 必须是合法 JSON')
    return
  }
  try {
    await api.createNode({
      name: createForm.value.name,
      kind: 'north',
      plugin_name: createForm.value.plugin_name,
      config
    })
    showCreateModal.value = false
    createForm.value = { name: '', plugin_name: '', config: '{}' }
    await loadNodes()
    ElMessage.success('创建成功')
  } catch (e) {
    ElMessage.error('创建失败: ' + e.message)
  }
}

async function startNode(id) {
  try {
    await api.startNode(id)
    await loadNodes()
    ElMessage.success('已启动')
  } catch (e) {
    ElMessage.error('启动失败: ' + e.message)
  }
}

async function stopNode(id) {
  try {
    await api.stopNode(id)
    await loadNodes()
    ElMessage.success('已停止')
  } catch (e) {
    ElMessage.error('停止失败: ' + e.message)
  }
}

async function deleteNode(id) {
  try {
    await ElMessageBox.confirm('确定删除该北向应用？相关配置将被清除。', '确认删除', {
      type: 'warning',
      confirmButtonText: '删除',
      cancelButtonText: '取消'
    })
    await api.deleteNode(id)
    await loadNodes()
    ElMessage.success('已删除')
  } catch (e) {
    if (e !== 'cancel') ElMessage.error('删除失败: ' + e.message)
  }
}

function goToDetail(node) {
  router.push(`/north/${node.id}`)
}

function openCreateModal() {
  createForm.value.plugin_name = pluginOptions.value[0]?.name || ''
  showCreateModal.value = true
}

function resetCreateForm() {
  createForm.value = { name: '', plugin_name: '', config: '{}' }
}

function getStateType(state) {
  if (state === 'running') return 'success'
  if (state === 'error') return 'danger'
  return 'info'
}

function getStateText(state) {
  if (state === 'running') return '运行中'
  if (state === 'error') return '错误'
  return '已停止'
}

onMounted(loadNodes)
</script>

<template>
  <div class="page-container">
    <div class="page-header">
      <div class="header-info">
        <p class="header-desc">管理北向数据应用，将采集的数据上报到云端或应用系统。</p>
      </div>
      <el-button type="primary" :icon="Plus" @click="openCreateModal">添加应用</el-button>
    </div>

    <el-alert v-if="error" type="error" :title="error" closable show-icon @close="error = ''" class="mb-2" />

    <el-skeleton v-if="loading" :rows="6" animated />

    <div v-else class="device-grid">
      <el-card
        v-for="node in nodes"
        :key="node.id"
        class="device-card north-card"
        :class="'state-' + (node.state || 'stopped')"
        shadow="hover"
      >
        <template #header>
          <div class="card-header">
            <div class="device-info">
              <span class="device-name">{{ node.name }}</span>
              <el-tag size="small" type="info" class="ml-1">{{ node.plugin_name }}</el-tag>
            </div>
            <el-tag :type="getStateType(node.state)" size="small" effect="light">
              {{ getStateText(node.state) }}
            </el-tag>
          </div>
        </template>
        <div class="card-body">
          <div class="info-row">
            <span class="info-label">节点 ID</span>
            <span class="info-value id-value">{{ node.id.slice(0, 8) }}...</span>
          </div>
        </div>
        <template #footer>
          <div class="card-actions">
            <el-button
              :type="node.state === 'running' ? 'warning' : 'success'"
              size="small"
              :icon="node.state === 'running' ? VideoPause : VideoPlay"
              @click="node.state === 'running' ? stopNode(node.id) : startNode(node.id)"
            >
              {{ node.state === 'running' ? '停止' : '启动' }}
            </el-button>
            <el-button type="primary" size="small" :icon="Edit" @click="goToDetail(node)">配置</el-button>
            <el-button type="danger" size="small" :icon="Delete" @click="deleteNode(node.id)">删除</el-button>
          </div>
        </template>
      </el-card>

      <el-empty v-if="!nodes.length" description="暂无北向应用" class="empty-block">
        <template #image>
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5" class="empty-icon">
            <path d="M12 2L2 7l10 5 10-5-10-5z"/>
            <path d="M2 17l10 5 10-5"/>
            <path d="M2 12l10 5 10-5"/>
          </svg>
        </template>
        <p class="empty-hint">点击上方「添加应用」创建第一个北向数据应用。</p>
      </el-empty>
    </div>

    <el-dialog
      v-model="showCreateModal"
      title="添加北向应用"
      width="480px"
      destroy-on-close
      @closed="resetCreateForm"
    >
      <el-form :model="createForm" label-width="100px" label-position="top">
        <el-form-item label="应用名称" required>
          <el-input v-model="createForm.name" placeholder="例如：mqtt-cloud-1" clearable />
        </el-form-item>
        <el-form-item label="应用插件" required>
          <el-select v-model="createForm.plugin_name" placeholder="请选择" style="width: 100%">
            <el-option
              v-for="p in pluginOptions"
              :key="p.name"
              :label="p.name + (p.version ? ` (v${p.version})` : '')"
              :value="p.name"
            />
          </el-select>
          <div v-if="pluginOptions.find(p => p.name === createForm.plugin_name)?.description" class="form-hint">
            {{ pluginOptions.find(p => p.name === createForm.plugin_name)?.description }}
          </div>
        </el-form-item>
        <el-form-item label="配置参数 (JSON)">
          <el-input
            v-model="createForm.config"
            type="textarea"
            :rows="4"
            placeholder='{"host":"broker.emqx.io","port":1883,"topic_prefix":"iot"}'
            class="font-mono"
          />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="showCreateModal = false">取消</el-button>
        <el-button type="primary" :disabled="!createForm.name || !createForm.plugin_name" @click="createNode">
          创建
        </el-button>
      </template>
    </el-dialog>
  </div>
</template>

<style scoped>
.mb-2 { margin-bottom: 1rem; }
.ml-1 { margin-left: 0.25rem; }
.device-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(320px, 1fr)); gap: 1rem; }
.device-card.state-running { border-left: 3px solid var(--el-color-success); }
.device-card.state-stopped { border-left: 3px solid var(--el-color-info); }
.device-card.state-error { border-left: 3px solid var(--el-color-danger); }
.north-card :deep(.el-card__header) { background: linear-gradient(135deg, var(--el-fill-color-light) 0%, rgba(124,58,237,0.06) 100%); }
.card-header { display: flex; justify-content: space-between; align-items: flex-start; }
.device-info { display: flex; align-items: center; flex-wrap: wrap; gap: 0.25rem; }
.device-name { font-weight: 600; font-size: 1rem; }
.card-body { padding: 0.5rem 0; }
.info-row { display: flex; justify-content: space-between; font-size: 0.85rem; }
.info-label { color: var(--text-muted); }
.info-value.id-value { font-family: var(--font-mono); font-size: 0.8rem; }
.card-actions { display: flex; gap: 0.5rem; flex-wrap: wrap; }
.form-hint { font-size: 0.8rem; color: var(--text-muted); margin-top: 0.25rem; }
.font-mono { font-family: var(--font-mono); }
.empty-block { grid-column: 1 / -1; padding: 3rem; }
.empty-icon { width: 80px; height: 80px; color: var(--el-color-info); opacity: 0.6; }
.empty-hint { color: var(--text-muted); font-size: 0.9rem; margin-top: 0.5rem; }
</style>
