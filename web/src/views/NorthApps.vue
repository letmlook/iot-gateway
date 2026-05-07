<script setup>
import { ref, inject, onMounted, computed } from 'vue'
import { useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { getErrorMessage } from '../i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Plus, Grid, List, Search, Refresh } from '@element-plus/icons-vue'
import { api } from '../api.js'
import PageHeader from '../components/PageHeader.vue'
import StatusIndicator from '../components/StatusIndicator.vue'
import DeviceCard from '../components/DeviceCard.vue'

const router = useRouter()
const { t } = useI18n()
const northPlugins = inject('northPlugins', ref([]))

const nodes = ref([])
const loading = ref(false)
const error = ref('')
const pluginFilter = ref('')
const keywordSearch = ref('')
const viewMode = ref('grid')

const pluginOptions = computed(() => northPlugins.value || [])

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

// 每个北向节点的订阅数
const subscriptionCounts = ref({})
async function loadSubscriptionCount(nodeId) {
  try {
    const subs = await api.subscriptions(nodeId)
    subscriptionCounts.value[nodeId] = subs?.length ?? 0
  } catch {
    subscriptionCounts.value[nodeId] = 0
  }
}

function getSubCount(nodeId) {
  return subscriptionCounts.value[nodeId] ?? 0
}

// MQTT 等北向节点：使用实际连接状态；connected 为 null/未返回且运行中时视为已连接，避免误显示断开
function getConnStatusClass(row) {
  if (row.plugin_name === 'mqtt' && row.connection_status && row.state === 'running') {
    const connected = row.connection_status.connected
    return connected === false ? 'offline error' : 'online'
  }
  return row.state === 'running' ? 'online' : 'offline'
}

function getConnStatusText(row) {
  if (row.plugin_name === 'mqtt' && row.connection_status && row.state === 'running') {
    const connected = row.connection_status.connected
    return connected === false ? t('south.disconnected') : t('south.connected')
  }
  return row.state === 'running' ? t('south.connected') : t('south.disconnected')
}

async function loadNodes() {
  loading.value = true
  error.value = ''
  try {
    const all = await api.nodes()
    nodes.value = all.filter(n => n.kind === 'north')
    for (const n of nodes.value) {
      loadSubscriptionCount(n.id)
    }
  } catch (e) {
    error.value = t('north.loadFailed') + getErrorMessage(t, e)
  } finally {
    loading.value = false
  }
}

function goToCreate() {
  router.push('/north/new')
}

async function startNode(id) {
  try {
    await api.startNode(id)
    await loadNodes()
    ElMessage.success(t('north.startSuccess'))
  } catch (e) {
    ElMessage.error(t('south.startFailed') + getErrorMessage(t, e))
  }
}

async function stopNode(id) {
  try {
    await api.stopNode(id)
    await loadNodes()
    ElMessage.success(t('north.stopSuccess'))
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
    await ElMessageBox.confirm(t('north.deleteConfirm'), t('common.confirmDelete'), {
      type: 'warning',
      confirmButtonText: t('common.delete'),
      cancelButtonText: t('common.cancel')
    })
    await api.deleteNode(id)
    await loadNodes()
    ElMessage.success(t('north.deleteSuccess'))
  } catch (e) {
    if (e !== 'cancel') ElMessage.error(t('north.deleteFailed') + getErrorMessage(t, e))
  }
}

function goToDetail(node) {
  router.push(`/north/${node.id}`)
}

function copyNode(node) {
  router.push({ path: '/north/new', query: { copyFrom: node.id } })
}

onMounted(loadNodes)
</script>

<template>
  <div class="north-page">
    <PageHeader :title="t('north.title')">
      <template #actions>
        <el-button type="primary" :icon="Plus" @click="goToCreate">
          {{ t('north.addApp') }}
        </el-button>
      </template>
    </PageHeader>
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

    <!-- 工具栏 -->
    <div class="toolbar">
      <div class="toolbar-left">
        <el-input
          v-model="keywordSearch"
          :placeholder="t('north.keywordSearch')"
          :prefix-icon="Search"
          clearable
          class="search-input"
        />
        <el-select
          v-model="pluginFilter"
          :placeholder="t('north.selectPlugin')"
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
          :device="{ ...node, groups_count: getSubCount(node.id) }"
          type="north"
          @click="goToDetail(node)"
          @toggle="toggleNode(node)"
          @edit="goToDetail(node)"
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
          <el-table-column :label="t('south.connState')" width="120">
            <template #default="{ row }">
              <el-tooltip
                v-if="row.connection_status?.last_error"
                :content="row.connection_status.last_error"
                placement="top"
              >
                <span :class="['conn-status', getConnStatusClass(row)]">
                  {{ getConnStatusText(row) }}
                </span>
              </el-tooltip>
              <span v-else :class="['conn-status', getConnStatusClass(row)]">
                {{ getConnStatusText(row) }}
              </span>
            </template>
          </el-table-column>
          <el-table-column :label="t('north.subscriptions')" width="100">
            <template #default="{ row }">
              <span class="sub-count">{{ getSubCount(row.id) }}</span>
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
      <el-empty v-if="!filteredNodes.length" :description="nodes.length ? t('north.noMatch') : t('north.noNodes')" class="empty-state">
        <el-button v-if="!nodes.length" type="primary" @click="goToCreate">
          {{ t('north.addApp') }}
        </el-button>
      </el-empty>

      <!-- 分页/统计 -->
      <div v-if="filteredNodes.length" class="footer-info">
        <span>{{ filteredNodes.length }} {{ t('common.items') }}</span>
      </div>
    </template>
  </div>
</template>

<style scoped>
.north-page {
  max-width: 1400px;
}

.mb-4 { margin-bottom: 1.5rem; }

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

.conn-status.offline.error {
  color: var(--danger);
}

.sub-count {
  font-family: var(--font-mono);
  font-weight: 600;
  color: var(--accent-purple);
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
