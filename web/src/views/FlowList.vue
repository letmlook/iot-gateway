<template>
  <div class="flow-list">
    <div class="page-header">
      <h2>{{ $t('flows.title') || '数据流编排' }}</h2>
      <el-button type="primary" @click="$router.push('/flows/new')">
        {{ $t('flows.new') || '新建数据流' }}
      </el-button>
    </div>

    <el-table :data="flows" v-loading="loading" stripe>
      <el-table-column prop="name" label="名称" min-width="150" />
      <el-table-column prop="status" label="状态" width="100">
        <template #default="{ row }">
          <el-tag :type="statusType(row.status)" size="small">{{ statusLabel(row.status) }}</el-tag>
        </template>
      </el-table-column>
      <el-table-column label="节点数" width="80">
        <template #default="{ row }">{{ row.nodes?.length || 0 }}</template>
      </el-table-column>
      <el-table-column prop="version" label="版本" width="70" />
      <el-table-column prop="updated_at" label="更新时间" width="160">
        <template #default="{ row }">{{ formatTime(row.updatedAt) }}</template>
      </el-table-column>
      <el-table-column label="操作" width="280" fixed="right">
        <template #default="{ row }">
          <el-button size="small" @click="$router.push(`/flows/${row.id}`)">编辑</el-button>
          <el-button v-if="row.status === 'draft'" size="small" type="primary" @click="handleDeploy(row.id)">部署</el-button>
          <el-button v-if="row.status === 'deployed'" size="small" type="success" @click="handleStart(row.id)">启动</el-button>
          <el-button v-if="row.status === 'running'" size="small" type="warning" @click="handlePause(row.id)">暂停</el-button>
          <el-button v-if="row.status === 'paused'" size="small" type="success" @click="handleStart(row.id)">恢复</el-button>
          <el-button v-if="row.status === 'running' || row.status === 'paused'" size="small" type="danger" @click="handleStop(row.id)">停止</el-button>
          <el-button size="small" type="danger" @click="handleDelete(row.id)">删除</el-button>
        </template>
      </el-table-column>
    </el-table>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { api } from '../api.js'
import { ElMessage, ElMessageBox } from 'element-plus'

const flows = ref([])
const loading = ref(false)

async function loadFlows() {
  loading.value = true
  try {
    const data = await api.flows()
    flows.value = data.flows || []
  } catch (e) {
    ElMessage.error('加载数据流失败: ' + e.message)
  } finally {
    loading.value = false
  }
}

async function handleDeploy(id) {
  try {
    await api.deployFlow(id)
    ElMessage.success('部署成功')
    loadFlows()
  } catch (e) { ElMessage.error('部署失败: ' + e.message) }
}

async function handleStart(id) {
  try {
    await api.startFlow(id)
    ElMessage.success('启动成功')
    loadFlows()
  } catch (e) { ElMessage.error('启动失败: ' + e.message) }
}

async function handlePause(id) {
  try {
    await api.pauseFlow(id)
    ElMessage.success('暂停成功')
    loadFlows()
  } catch (e) { ElMessage.error('暂停失败: ' + e.message) }
}

async function handleStop(id) {
  try {
    await api.stopFlow(id)
    ElMessage.success('停止成功')
    loadFlows()
  } catch (e) { ElMessage.error('停止失败: ' + e.message) }
}

async function handleDelete(id) {
  try {
    await ElMessageBox.confirm('确定删除该数据流？', '确认', { type: 'warning' })
    await api.deleteFlow(id)
    ElMessage.success('删除成功')
    loadFlows()
  } catch (e) {
    if (e !== 'cancel') ElMessage.error('删除失败: ' + e.message)
  }
}

function statusType(s) {
  return { draft: '', deployed: 'info', running: 'success', paused: 'warning', stopped: 'info', error: 'danger' }[s] || ''
}

function statusLabel(s) {
  return { draft: '草稿', deployed: '已部署', running: '运行中', paused: '已暂停', stopped: '已停止', error: '错误' }[s] || s
}

function formatTime(ts) {
  if (!ts) return '-'
  return new Date(ts).toLocaleString('zh-CN')
}

onMounted(loadFlows)
</script>

<style scoped>
.flow-list { padding: 20px; }
.page-header { display: flex; justify-content: space-between; align-items: center; margin-bottom: 16px; }
.page-header h2 { margin: 0; }
</style>
