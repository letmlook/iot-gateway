<script setup>
import { ref, inject, onMounted, computed } from 'vue'
import { useRouter } from 'vue-router'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Plus, VideoPlay, VideoPause, Edit, Delete, MoreFilled, Upload, Download, Grid, List } from '@element-plus/icons-vue'
import { api } from '../api.js'

const router = useRouter()
const southPlugins = inject('southPlugins', ref([]))

const nodes = ref([])
const loading = ref(false)
const error = ref('')
const pluginFilter = ref('')
const keywordSearch = ref('')
const viewMode = ref('list') // list | grid

const pluginOptions = computed(() => {
  return southPlugins.value.map(p => ({
    name: p[0],
    description: p[1],
    version: p[2]
  }))
})

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
    error.value = '加载节点失败: ' + e.message
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

async function toggleNode(node) {
  if (node.state === 'running') await stopNode(node.id)
  else await startNode(node.id)
}

async function deleteNode(id) {
  try {
    await ElMessageBox.confirm('确定删除该南向设备？相关配置将被清除。', '确认删除', {
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
  if (state === 'running') return '运行中'
  if (state === 'error') return '错误'
  return '已停止'
}

function handleExport() {
  ElMessage.info('导出功能：请使用系统管理中的「导出配置」')
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
      <h2 class="page-title">南向设备</h2>
      <div class="header-toolbar">
        <el-select v-model="pluginFilter" placeholder="请选择插件类型" clearable style="width: 160px" class="mr-1">
          <el-option v-for="p in pluginOptions" :key="p.name" :label="p.name" :value="p.name" />
        </el-select>
        <el-input v-model="keywordSearch" placeholder="输入关键字搜索" clearable style="width: 180px" class="mr-1" />
        <div class="toolbar-btns">
          <el-button :icon="Upload" text title="导入" @click="handleImport" />
          <el-button :icon="Download" text title="导出" @click="handleExport" />
          <el-button :icon="Grid" text :type="viewMode === 'grid' ? 'primary' : ''" title="网格视图" @click="viewMode = 'grid'" />
          <el-button :icon="List" text :type="viewMode === 'list' ? 'primary' : ''" title="列表视图" @click="viewMode = 'list'" />
        </div>
        <el-button type="primary" :icon="Plus" @click="goToCreate">添加设备</el-button>
      </div>
    </div>

    <el-alert v-if="error" type="error" :title="error" closable show-icon @close="error = ''" class="mb-2" />

    <el-skeleton v-if="loading" :rows="6" animated />

    <template v-else>
      <!-- 列表视图（对标 Neuron 表格） -->
      <el-table
        v-if="viewMode === 'list'"
        :data="filteredNodes"
        size="default"
        stripe
        style="width: 100%"
        :header-cell-style="{ background: 'var(--el-fill-color-light)' }"
      >
        <el-table-column type="selection" width="48" />
        <el-table-column prop="name" label="名称" min-width="140">
          <template #default="{ row }">
            <el-link type="primary" @click="goToDetail(row)">{{ row.name }}</el-link>
          </template>
        </el-table-column>
        <el-table-column label="工作状态" width="100">
          <template #default="{ row }">
            <el-tag :type="getStateType(row.state)" size="small" effect="light">
              {{ getStateText(row.state) }}
            </el-tag>
          </template>
        </el-table-column>
        <el-table-column label="连接状态" width="100">
          <template #default="{ row }">
            <span :class="{ 'text-success': row.state === 'running' }">
              {{ row.state === 'running' ? '已连接' : '断开' }}
            </span>
          </template>
        </el-table-column>
        <el-table-column label="延时(毫秒)" width="110">
          <template #default> - </template>
        </el-table-column>
        <el-table-column prop="plugin_name" label="插件" width="120" />
        <el-table-column label="操作" width="140" fixed="right">
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
                    编辑设备
                  </el-dropdown-item>
                  <el-dropdown-item command="stats">数据统计</el-dropdown-item>
                  <el-dropdown-item command="setting">设备配置</el-dropdown-item>
                  <el-dropdown-item command="copy">复制</el-dropdown-item>
                  <el-dropdown-item command="delete" divided>
                    <span style="color: var(--el-color-danger)">删除</span>
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
              <span class="info-label">连接状态</span>
              <span class="info-value">{{ node.state === 'running' ? '已连接' : '断开' }}</span>
            </div>
          </div>
          <template #footer>
            <div class="card-actions">
              <el-switch
                :model-value="node.state === 'running'"
                @change="toggleNode(node)"
              />
              <el-button type="primary" size="small" :icon="Edit" @click="goToDetail(node)">配置</el-button>
              <el-button type="danger" size="small" :icon="Delete" @click="deleteNode(node.id)">删除</el-button>
            </div>
          </template>
        </el-card>
      </div>

      <div v-if="viewMode === 'list'" class="pagination-wrap">
        <span class="total-hint">共 {{ filteredNodes.length }} 条</span>
      </div>

      <el-empty v-if="!filteredNodes.length" description="暂无南向设备" class="empty-block">
        <template #description>
          <p v-if="nodes.length">没有匹配的设备，可调整筛选条件。</p>
          <p v-else>点击「添加设备」创建第一个南向设备驱动。</p>
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
