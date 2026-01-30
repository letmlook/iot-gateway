<script setup>
import { ref, inject, onMounted, computed } from 'vue'
import { useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Plus, VideoPlay, VideoPause, Edit, Delete, MoreFilled, Upload, Download, Grid, List } from '@element-plus/icons-vue'
import { api } from '../api.js'

const router = useRouter()
const { t } = useI18n()
const southPlugins = inject('southPlugins', ref([]))

const nodes = ref([])
const loading = ref(false)
const error = ref('')
const pluginFilter = ref('')
const keywordSearch = ref('')
const viewMode = ref('list') // list | grid

// API 返回 { name, name_zh?, name_en?, description?, description_zh?, description_en?, version }
const pluginOptions = computed(() => southPlugins.value || [])

const filteredNodes = computed(() => {
  let list = nodes.value
  if (pluginFilter.value) {
    list = list.filter(n => n.plugin_name === pluginFilter.value)
  }
  if (keywordSearch.value.trim()) {
    const k = keywordSearch.value.trim().toLowerCase()
    list = list.filter(n =>
      (n.name && n.name.toLowerCase().includes(k)) ||
      (n.plugin_name && n.plugin_name.toLowerCase().includes(k))
    )
  }
  return list
})

async function loadNodes() {
  loading.value = true
  error.value = ''
  try {
    const all = await api.nodes()
    nodes.value = all.filter(n => n.kind === 'south')
  } catch (e) {
    error.value = t('south.loadFailed') + e.message
  } finally {
    loading.value = false
  }
}

function goToCreate() {
  router.push('/south/new')
}

async function startNode(id) {
  try {
    await api.startNode(id)
    await loadNodes()
    ElMessage.success(t('south.startSuccess'))
  } catch (e) {
    ElMessage.error(t('south.startFailed') + e.message)
  }
}

async function stopNode(id) {
  try {
    await api.stopNode(id)
    await loadNodes()
    ElMessage.success(t('south.stopSuccess'))
  } catch (e) {
    ElMessage.error(t('south.stopFailed') + e.message)
  }
}

async function toggleNode(node) {
  if (node.state === 'running') await stopNode(node.id)
  else await startNode(node.id)
}

async function deleteNode(id) {
  try {
    await ElMessageBox.confirm(t('south.deleteConfirm'), t('common.confirmDelete'), {
      type: 'warning',
      confirmButtonText: t('common.delete'),
      cancelButtonText: t('common.cancel')
    })
    await api.deleteNode(id)
    await loadNodes()
    ElMessage.success(t('south.deleteSuccess'))
  } catch (e) {
    if (e !== 'cancel') ElMessage.error(t('south.deleteFailed') + e.message)
  }
}

function goToDetail(node) {
  router.push(`/south/${node.id}`)
}

function goToMonitor(node) {
  router.push({ path: '/monitor', query: { nodeId: node.id } })
}

function getStateType(state) {
  if (state === 'running') return 'success'
  if (state === 'error') return 'danger'
  return 'info'
}

function getStateText(state) {
  if (state === 'running') return t('common.running')
  if (state === 'error') return t('common.error')
  return t('common.stopped')
}

function handleExport() {
  ElMessage.info('备份功能：请使用系统管理中的「备份」')
}

function handleImport() {
  ElMessage.info('导入功能：请使用系统管理中的「导入配置」')
}

function copyNode(node) {
  router.push({ path: '/south/new', query: { copyFrom: node.id } })
}

onMounted(loadNodes)
</script>

<template>
  <div class="page-container">
    <div class="page-header south-header">
      <h2 class="page-title">{{ t('south.title') }}</h2>
      <div class="header-toolbar">
        <el-select v-model="pluginFilter" :placeholder="t('south.selectPlugin')" clearable style="width: 160px" class="mr-1">
          <el-option v-for="p in pluginOptions" :key="p.name" :label="p.name" :value="p.name" />
        </el-select>
        <el-input v-model="keywordSearch" :placeholder="t('south.keywordSearch')" clearable style="width: 180px" class="mr-1" />
        <div class="toolbar-btns">
          <el-button :icon="Upload" text :title="t('south.import')" @click="handleImport" />
          <el-button :icon="Download" text :title="t('south.export')" @click="handleExport" />
          <el-button :icon="Grid" text :type="viewMode === 'grid' ? 'primary' : ''" :title="t('south.gridView')" @click="viewMode = 'grid'" />
          <el-button :icon="List" text :type="viewMode === 'list' ? 'primary' : ''" :title="t('south.listView')" @click="viewMode = 'list'" />
        </div>
        <el-button type="primary" :icon="Plus" @click="goToCreate">{{ t('south.addDevice') }}</el-button>
      </div>
    </div>

    <el-alert v-if="error" type="error" :title="error" closable show-icon @close="error = ''" class="mb-2" />

    <el-skeleton v-if="loading" :rows="6" animated />

    <template v-else>
      <!-- 列表视图 -->
      <el-table
        v-if="viewMode === 'list'"
        :data="filteredNodes"
        size="default"
        stripe
        style="width: 100%"
        :header-cell-style="{ background: 'var(--el-fill-color-light)' }"
      >
        <el-table-column type="selection" width="48" />
        <el-table-column prop="name" :label="t('common.name')" min-width="140">
          <template #default="{ row }">
            <el-link type="primary" @click="goToDetail(row)">{{ row.name }}</el-link>
          </template>
        </el-table-column>
        <el-table-column :label="t('south.workState')" width="100">
          <template #default="{ row }">
            <el-tag :type="getStateType(row.state)" size="small" effect="light">
              {{ getStateText(row.state) }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="t('south.connState')" width="100">
          <template #default="{ row }">
            <span :class="{ 'text-success': row.state === 'running' }">
              {{ row.state === 'running' ? t('south.connected') : t('south.disconnected') }}
            </span>
          </template>
        </el-table-column>
        <el-table-column label="Delay(ms)" width="110">
          <template #default> - </template>
        </el-table-column>
        <el-table-column prop="plugin_name" :label="t('south.pluginType')" width="120" />
        <el-table-column :label="t('common.operation')" width="140" fixed="right">
          <template #default="{ row }">
            <el-switch
              :model-value="row.state === 'running'"
              @change="toggleNode(row)"
            />
            <el-dropdown trigger="click" @command="(cmd) => { if (cmd === 'edit') goToDetail(row); else if (cmd === 'stats') goToMonitor(row); else if (cmd === 'setting') goToDetail(row); else if (cmd === 'copy') copyNode(row); else if (cmd === 'delete') deleteNode(row.id) }">
              <el-button type="primary" link :icon="MoreFilled" class="ml-1" />
              <template #dropdown>
                <el-dropdown-menu>
                  <el-dropdown-item command="edit">
                    <el-icon><Edit /></el-icon>
                    {{ t('south.editDevice') }}
                  </el-dropdown-item>
                  <el-dropdown-item command="stats">{{ t('south.dataMonitor') }}</el-dropdown-item>
                  <el-dropdown-item command="setting">{{ t('south.deviceConfig') }}</el-dropdown-item>
                  <el-dropdown-item command="copy">{{ t('common.copy') }}</el-dropdown-item>
                  <el-dropdown-item command="delete" divided>
                    <span style="color: var(--el-color-danger)">{{ t('common.delete') }}</span>
                  </el-dropdown-item>
                </el-dropdown-menu>
              </template>
            </el-dropdown>
          </template>
        </el-table-column>
      </el-table>

      <!-- 网格视图（保留原卡片） -->
      <div v-else class="device-grid">
        <el-card
          v-for="node in filteredNodes"
          :key="node.id"
          class="device-card"
          :class="'state-' + (node.state || 'stopped')"
          shadow="hover"
        >
          <template #header>
            <div class="card-header">
              <div class="device-info">
                <span class="device-name" @click="goToDetail(node)" style="cursor: pointer">{{ node.name }}</span>
                <el-tag size="small" type="info" class="ml-1">{{ node.plugin_name }}</el-tag>
              </div>
              <el-tag :type="getStateType(node.state)" size="small" effect="light">
                {{ getStateText(node.state) }}
              </el-tag>
            </div>
          </template>
          <div class="card-body">
            <div class="info-row">
              <span class="info-label">{{ t('south.connState') }}</span>
              <span class="info-value">{{ node.state === 'running' ? t('south.connected') : t('south.disconnected') }}</span>
            </div>
          </div>
          <template #footer>
            <div class="card-actions">
              <el-switch
                :model-value="node.state === 'running'"
                @change="toggleNode(node)"
              />
              <el-button type="primary" size="small" :icon="Edit" @click="goToDetail(node)">{{ t('nodeDetail.config') }}</el-button>
              <el-button type="danger" size="small" :icon="Delete" @click="deleteNode(node.id)">{{ t('common.delete') }}</el-button>
            </div>
          </template>
        </el-card>
      </div>

      <div v-if="viewMode === 'list'" class="pagination-wrap">
        <span class="total-hint">共 {{ filteredNodes.length }} 条</span>
      </div>

      <el-empty v-if="!filteredNodes.length" :description="t('south.noNodes')" class="empty-block">
        <template #description>
          <p v-if="nodes.length">{{ t('south.noMatch') }}</p>
          <p v-else>{{ t('south.createFirst') }}</p>
        </template>
      </el-empty>
    </template>
  </div>
</template>

<style scoped>
.mb-2 { margin-bottom: 1rem; }
.ml-1 { margin-left: 0.25rem; }
.mr-1 { margin-right: 0.5rem; }
.page-title { margin: 0 0 1rem; font-size: 1.25rem; }
.south-header { display: flex; flex-wrap: wrap; align-items: center; gap: 1rem; }
.header-toolbar { display: flex; flex-wrap: wrap; align-items: center; gap: 0.5rem; margin-left: auto; }
.toolbar-btns { display: inline-flex; align-items: center; }
.device-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(280px, 1fr)); gap: 1rem; }
.device-card.state-running { border-left: 3px solid var(--el-color-success); }
.device-card.state-stopped { border-left: 3px solid var(--el-color-info); }
.device-card.state-error { border-left: 3px solid var(--el-color-danger); }
.card-header { display: flex; justify-content: space-between; align-items: flex-start; }
.device-info { display: flex; align-items: center; flex-wrap: wrap; gap: 0.25rem; }
.device-name { font-weight: 600; font-size: 1rem; }
.card-body { padding: 0.5rem 0; }
.info-row { display: flex; justify-content: space-between; font-size: 0.85rem; }
.info-label { color: var(--text-muted); }
.card-actions { display: flex; gap: 0.5rem; flex-wrap: wrap; align-items: center; }
.form-hint { font-size: 0.8rem; color: var(--text-muted); margin-top: 0.25rem; }
.config-fallback-hint { font-size: 0.8rem; color: var(--el-text-color-secondary); margin-bottom: 0.5rem; }
.mt-1 { margin-top: 0.5rem; }
.font-mono { font-family: var(--font-mono); }
.empty-block { padding: 3rem; }
.pagination-wrap { margin-top: 1rem; font-size: 0.9rem; color: var(--text-muted); }
.text-success { color: var(--el-color-success); }
</style>
