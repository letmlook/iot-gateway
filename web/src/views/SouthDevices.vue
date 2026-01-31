<script setup>
import { ref, inject, onMounted, computed } from 'vue'
import { useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { getErrorMessage } from '../i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Plus, Grid, List, Search, Refresh } from '@element-plus/icons-vue'
import { api } from '../api.js'
import StatusIndicator from '../components/StatusIndicator.vue'
import DeviceCard from '../components/DeviceCard.vue'

const router = useRouter()
const { t } = useI18n()
const southPlugins = inject('southPlugins', ref([]))

const nodes = ref([])
const loading = ref(false)
const error = ref('')
const pluginFilter = ref('')
const keywordSearch = ref('')
const viewMode = ref('grid') // list | grid

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

// 统计
const runningCount = computed(() => nodes.value.filter(n => n.state === 'running').length)
const errorCount = computed(() => nodes.value.filter(n => n.state === 'error').length)

async function loadNodes() {
  loading.value = true
  error.value = ''
  try {
    const all = await api.nodes()
    nodes.value = all.filter(n => n.kind === 'south')
  } catch (e) {
    error.value = t('south.loadFailed') + getErrorMessage(t, e)
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
    ElMessage.error(t('south.startFailed') + getErrorMessage(t, e))
  }
}

async function stopNode(id) {
  try {
    await api.stopNode(id)
    await loadNodes()
    ElMessage.success(t('south.stopSuccess'))
  } catch (e) {
    ElMessage.error(t('south.stopFailed') + getErrorMessage(t, e))
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
    if (e !== 'cancel') ElMessage.error(t('south.deleteFailed') + getErrorMessage(t, e))
  }
}

function goToDetail(node) {
  router.push(`/south/${node.id}`)
}

function goToMonitor(node) {
  router.push({ path: '/monitor', query: { nodeId: node.id } })
}

function copyNode(node) {
  router.push({ path: '/south/new', query: { copyFrom: node.id } })
}

function handleCardAction(action, node) {
  switch (action) {
    case 'edit':
      goToDetail(node)
      break
    case 'monitor':
      goToMonitor(node)
      break
    case 'copy':
      copyNode(node)
      break
    case 'delete':
      deleteNode(node.id)
      break
  }
}

onMounted(loadNodes)
</script>

<template>
  <div class="south-page">
    <!-- 页面头部 -->
    <div class="page-header">
      <div class="header-content">
        <h1 class="page-title">{{ t('south.title') }}</h1>
        <div class="header-stats">
          <span class="stat-chip">
            <span class="stat-dot running"></span>
            {{ runningCount }} {{ t('common.running') }}
          </span>
          <span v-if="errorCount > 0" class="stat-chip error">
            <span class="stat-dot error"></span>
            {{ errorCount }} {{ t('common.error') }}
          </span>
        </div>
      </div>
      <div class="header-actions">
        <el-button type="primary" :icon="Plus" @click="goToCreate">
          {{ t('south.addDevice') }}
        </el-button>
      </div>
    </div>

    <!-- 工具栏 -->
    <div class="toolbar">
      <div class="toolbar-left">
        <el-input
          v-model="keywordSearch"
          :placeholder="t('south.keywordSearch')"
          :prefix-icon="Search"
          clearable
          class="search-input"
        />
        <el-select
          v-model="pluginFilter"
          :placeholder="t('south.selectPlugin')"
          clearable
          class="filter-select"
        >
          <el-option v-for="p in pluginOptions" :key="p.name" :label="p.name" :value="p.name" />
        </el-select>
      </div>
      <div class="toolbar-right">
        <el-button-group class="view-toggle">
          <el-button
            :type="viewMode === 'grid' ? 'primary' : ''"
            :icon="Grid"
            @click="viewMode = 'grid'"
          />
          <el-button
            :type="viewMode === 'list' ? 'primary' : ''"
            :icon="List"
            @click="viewMode = 'list'"
          />
        </el-button-group>
        <el-button :icon="Refresh" @click="loadNodes" :loading="loading">
          {{ t('common.refresh') }}
        </el-button>
      </div>
    </div>

    <el-alert v-if="error" type="error" :title="error" closable show-icon @close="error = ''" class="mb-4" />

    <el-skeleton v-if="loading" :rows="6" animated />

    <template v-else>
      <!-- 卡片视图 -->
      <div v-if="viewMode === 'grid'" class="devices-grid">
        <DeviceCard
          v-for="node in filteredNodes"
          :key="node.id"
          :device="node"
          type="south"
          @click="goToDetail(node)"
          @toggle="toggleNode(node)"
          @edit="goToDetail(node)"
          @monitor="goToMonitor(node)"
          @copy="copyNode(node)"
          @delete="deleteNode(node.id)"
        />
      </div>

      <!-- 列表视图 -->
      <div v-else class="devices-table">
        <el-table
          :data="filteredNodes"
          size="default"
          stripe
          :header-cell-style="{ background: 'var(--bg-elevated)', color: 'var(--text-secondary)' }"
        >
          <el-table-column prop="name" :label="t('common.name')" min-width="150">
            <template #default="{ row }">
              <div class="name-cell">
                <el-link type="primary" @click="goToDetail(row)">{{ row.name }}</el-link>
              </div>
            </template>
          </el-table-column>
          <el-table-column :label="t('south.workState')" width="120">
            <template #default="{ row }">
              <StatusIndicator :status="row.state" size="small" />
            </template>
          </el-table-column>
          <el-table-column :label="t('south.connState')" width="100">
            <template #default="{ row }">
              <el-tooltip v-if="row.connection_status && row.state === 'running' && row.connection_status.last_error" :content="row.connection_status.last_error" placement="top">
                <span :class="['conn-status', row.connection_status.connected === false ? 'offline' : 'online']">
                  {{ row.connection_status.connected === false ? t('south.disconnected') : t('south.connected') }}
                </span>
              </el-tooltip>
              <span v-else :class="['conn-status', row.state === 'running' ? 'online' : 'offline']">
                {{ row.state === 'running' ? t('south.connected') : t('south.disconnected') }}
              </span>
            </template>
          </el-table-column>
          <el-table-column prop="plugin_name" :label="t('south.pluginType')" min-width="120">
            <template #default="{ row }">
              <span class="plugin-tag">{{ row.plugin_name }}</span>
            </template>
          </el-table-column>
          <el-table-column :label="t('common.operation')" width="160" fixed="right">
            <template #default="{ row }">
              <div class="table-actions">
                <el-switch
                  :model-value="row.state === 'running'"
                  size="small"
                  @change="toggleNode(row)"
                />
                <el-button type="primary" link size="small" @click="goToDetail(row)">
                  {{ t('nodeDetail.config') }}
                </el-button>
                <el-button type="danger" link size="small" @click="deleteNode(row.id)">
                  {{ t('common.delete') }}
                </el-button>
              </div>
            </template>
          </el-table-column>
        </el-table>
      </div>

      <!-- 空状态 -->
      <el-empty v-if="!filteredNodes.length" :description="nodes.length ? t('south.noMatch') : t('south.noNodes')" class="empty-state">
        <el-button v-if="!nodes.length" type="primary" @click="goToCreate">
          {{ t('south.addDevice') }}
        </el-button>
      </el-empty>

      <!-- 分页/统计 -->
      <div v-if="filteredNodes.length" class="footer-info">
        <span>{{ t('common.items').replace('{n}', filteredNodes.length) }} {{ filteredNodes.length }} {{ t('common.items') }}</span>
      </div>
    </template>
  </div>
</template>

<style scoped>
.south-page {
  max-width: 1400px;
}

.mb-4 { margin-bottom: 1.5rem; }

/* 页面头部 */
.page-header {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  margin-bottom: 1.25rem;
  gap: 1rem;
  flex-wrap: wrap;
}

.header-content {
  display: flex;
  flex-direction: column;
  gap: 0.5rem;
}

.page-title {
  font-size: 1.5rem;
  font-weight: 700;
  color: var(--text-primary);
  margin: 0;
}

.header-stats {
  display: flex;
  gap: 0.75rem;
}

.stat-chip {
  display: inline-flex;
  align-items: center;
  gap: 0.35rem;
  padding: 0.25rem 0.6rem;
  background: var(--bg-inset);
  border-radius: 100px;
  font-size: 0.8rem;
  color: var(--text-secondary);
}

.stat-chip.error {
  background: rgba(229, 62, 62, 0.1);
  color: var(--danger);
}

.stat-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--text-muted);
}

.stat-dot.running {
  background: var(--success);
}

.stat-dot.error {
  background: var(--danger);
}

/* 工具栏 */
.toolbar {
  display: flex;
  justify-content: space-between;
  align-items: center;
  gap: 1rem;
  margin-bottom: 1rem;
  flex-wrap: wrap;
}

.toolbar-left {
  display: flex;
  gap: 0.75rem;
  flex-wrap: wrap;
}

.search-input {
  width: 220px;
}

.filter-select {
  width: 160px;
}

.toolbar-right {
  display: flex;
  gap: 0.75rem;
  align-items: center;
}

.view-toggle {
  margin-right: 0.5rem;
}

/* 卡片网格 */
.devices-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
  gap: 1rem;
}

/* 表格 */
.devices-table {
  background: var(--bg-surface);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-lg);
  overflow: hidden;
}

.name-cell {
  font-weight: 500;
}

.conn-status {
  font-size: 0.85rem;
}

.conn-status.online {
  color: var(--success);
}

.conn-status.offline {
  color: var(--text-muted);
}

.plugin-tag {
  font-family: var(--font-mono);
  font-size: 0.8rem;
  background: var(--bg-inset);
  padding: 0.15rem 0.5rem;
  border-radius: var(--radius-xs);
  color: var(--text-secondary);
}

.table-actions {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

/* 空状态 */
.empty-state {
  padding: 3rem;
}

/* 底部信息 */
.footer-info {
  margin-top: 1rem;
  padding-top: 0.75rem;
  border-top: 1px solid var(--border-subtle);
  font-size: 0.85rem;
  color: var(--text-muted);
}

@media (max-width: 768px) {
  .toolbar {
    flex-direction: column;
    align-items: stretch;
  }
  
  .toolbar-left,
  .toolbar-right {
    width: 100%;
  }
  
  .search-input,
  .filter-select {
    width: 100%;
  }
}
</style>
