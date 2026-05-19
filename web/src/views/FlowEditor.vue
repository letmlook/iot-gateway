<template>
  <div class="flow-editor">
    <!-- Toolbar -->
    <div class="editor-toolbar">
      <el-button @click="$router.push('/flows')">← 返回</el-button>
      <el-input v-model="flowName" placeholder="数据流名称" style="width: 200px" />
      <el-button type="primary" @click="handleSave">保存</el-button>
      <el-button @click="handleValidate">验证</el-button>
      <el-button v-if="flowStatus === 'draft' || flowStatus === 'stopped'" type="success" @click="handleDeploy">部署</el-button>
      <el-button v-if="flowStatus === 'deployed'" type="success" @click="handleStart">启动</el-button>
      <el-button v-if="flowStatus === 'running'" type="warning" @click="handlePause">暂停</el-button>
      <el-button v-if="flowStatus === 'running' || flowStatus === 'paused'" type="danger" @click="handleStop">停止</el-button>
      <el-tag v-if="flowStatus" style="margin-left: 8px">{{ statusLabel(flowStatus) }}</el-tag>
    </div>

    <div class="editor-body">
      <!-- Node Palette -->
      <div class="node-palette">
        <h4>南向设备</h4>
        <div
          v-for="plugin in southPlugins"
          :key="plugin.name"
          class="palette-item south"
          draggable="true"
          @dragstart="onDragStart($event, 'south', plugin.name)"
        >
          {{ plugin.name_zh || plugin.name }}
        </div>
        <h4 style="margin-top: 12px">算子</h4>
        <div
          v-for="op in operators"
          :key="op.name"
          class="palette-item operator"
          draggable="true"
          @dragstart="onDragStart($event, 'operator', op.name)"
        >
          {{ op.name_zh || op.name }}
        </div>
        <h4 style="margin-top: 12px">北向应用</h4>
        <div
          v-for="plugin in northPlugins"
          :key="plugin.name"
          class="palette-item north"
          draggable="true"
          @dragstart="onDragStart($event, 'north', plugin.name)"
        >
          {{ plugin.name_zh || plugin.name }}
        </div>
      </div>

      <!-- VueFlow Canvas -->
      <div class="flow-canvas" @drop="handleDrop" @dragover.prevent>
        <VueFlow
          v-model:nodes="nodes"
          v-model:edges="edges"
          :default-edge-options="{ type: 'smoothstep' }"
          fit-view-on-init
          @node-click="onNodeClick"
          @pane-context-menu="onPaneContextMenu"
        >
          <Background pattern-color="#aaa" :gap="16" />
          <Controls />
          <MiniMap />
        </VueFlow>
      </div>

      <!-- Properties Panel -->
      <div class="properties-panel" v-if="selectedNode">
        <h4>节点属性</h4>
        <p><strong>{{ selectedNode.label }}</strong></p>
        <el-form label-width="80px" size="small">
          <el-form-item label="名称">
            <el-input v-model="selectedNode.data.name" />
          </el-form-item>
          <el-form-item label="类型">
            <el-tag>{{ selectedNode.type }}</el-tag>
          </el-form-item>
          <template v-if="selectedNode.type === 'operator'">
            <el-form-item label="算子">
              <el-select v-model="selectedNode.data.operatorName">
                <el-option v-for="op in operators" :key="op.name" :label="op.name_zh || op.name" :value="op.name" />
              </el-select>
            </el-form-item>
          </template>
          <el-divider />
          <el-button type="danger" size="small" @click="removeNode(selectedNode.id)">删除节点</el-button>
        </el-form>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, onMounted } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { VueFlow, useVueFlow, MarkerType } from '@vue-flow/core'
import { Background } from '@vue-flow/background'
import { Controls } from '@vue-flow/controls'
import { MiniMap } from '@vue-flow/minimap'
import { ElMessage } from 'element-plus'
import { api } from '../api.js'

import '@vue-flow/core/dist/style.css'
import '@vue-flow/core/dist/theme-default.css'
import '@vue-flow/controls/dist/style.css'
import '@vue-flow/minimap/dist/style.css'

const route = useRoute()
const router = useRouter()
const { project } = useVueFlow()

const flowId = computed(() => route.params.id)
const flowName = ref('')
const flowStatus = ref('')
const nodes = ref([])
const edges = ref([])
const selectedNode = ref(null)
const southPlugins = ref([])
const northPlugins = ref([])
const operators = ref([
  { name: 'filter', name_zh: '过滤器' },
  { name: 'transform', name_zh: '转换器' },
  { name: 'aggregate', name_zh: '聚合器' },
  { name: 'router', name_zh: '路由器' },
  { name: 'buffer', name_zh: '缓冲器' },
])

onMounted(async () => {
  // Load plugins
  try {
    const south = await api.pluginsSouth()
    southPlugins.value = south.plugins || []
    const north = await api.pluginsNorth()
    northPlugins.value = north.plugins || []
  } catch (e) { console.error(e) }

  if (flowId.value) {
    // Load existing flow
    try {
      const data = await api.flow(flowId.value)
      const flow = data.flow
      flowName.value = flow.name
      flowStatus.value = flow.status
      // Convert flow nodes to VueFlow nodes
      nodes.value = (flow.nodes || []).map(n => ({
        id: n.id,
        type: n.kind,
        label: n.name,
        position: { x: 100, y: 100 },
        data: { name: n.name, kind: n.kind, operatorName: n.operator_name, config: n.config }
      }))
      edges.value = (flow.edges || []).map((e, i) => ({
        id: `e${i}`,
        source: e.source_node_id,
        target: e.target_node_id,
        sourceHandle: e.source_port,
        targetHandle: e.target_port,
        markerEnd: MarkerType.ArrowClosed,
      }))
    } catch (e) {
      ElMessage.error('加载数据流失败: ' + e.message)
    }
  }
})

// Palette drag
function onDragStart(event, kind, name) {
  event.dataTransfer.setData('kind', kind)
  event.dataTransfer.setData('name', name)
}

function handleDrop(event) {
  const kind = event.dataTransfer.getData('kind')
  const name = event.dataTransfer.getData('name')
  if (!kind || !name) return

  const id = crypto.randomUUID()
  const label = kind === 'south'
    ? southPlugins.value.find(p => p.name === name)?.name_zh || name
    : kind === 'north'
    ? northPlugins.value.find(p => p.name === name)?.name_zh || name
    : operators.value.find(o => o.name === name)?.name_zh || name

  const position = project({ x: event.clientX - 250, y: event.clientY - 60 })

  nodes.value.push({
    id,
    type: kind,
    label,
    position,
    data: { name: label, kind, operatorName: kind === 'operator' ? name : undefined }
  })
}

function onNodeClick({ node }) {
  selectedNode.value = node
}

function onPaneContextMenu() {
  selectedNode.value = null
}

function removeNode(id) {
  nodes.value = nodes.value.filter(n => n.id !== id)
  edges.value = edges.value.filter(e => e.source !== id && e.target !== id)
  selectedNode.value = null
}

async function handleSave() {
  try {
    const flowNodes = nodes.value.map(n => ({
      id: n.id,
      name: n.data.name,
      kind: n.data.kind,
      operator_name: n.data.operatorName,
      config: {},
      input_ports: n.data.kind === 'operator' ? [{ id: 'in', name: '输入', port_type: 'data', required: true }] : [],
      output_ports: n.data.kind === 'operator' ? [{ id: 'out', name: '输出', port_type: 'data', required: false }] : [],
    }))
    const flowEdges = edges.value.map(e => ({
      source_node_id: e.source,
      source_port: e.sourceHandle || 'out',
      target_node_id: e.target,
      target_port: e.targetHandle || 'in',
    }))
    const body = {
      id: flowId.value || '00000000-0000-0000-0000-000000000000',
      name: flowName.value || '未命名数据流',
      description: null,
      status: 'draft',
      nodes: flowNodes,
      edges: flowEdges,
      version: 1,
      created_at: new Date().toISOString(),
      updated_at: new Date().toISOString(),
    }
    if (flowId.value) {
      await api.updateFlow(flowId.value, body)
    } else {
      await api.createFlow(body)
    }
    ElMessage.success('保存成功')
  } catch (e) {
    ElMessage.error('保存失败: ' + e.message)
  }
}

async function handleValidate() {
  // Basic validation: at least one south and one north
  const hasSouth = nodes.value.some(n => n.type === 'south')
  const hasNorth = nodes.value.some(n => n.type === 'north')
  if (!hasSouth) { ElMessage.warning('请添加南向设备节点'); return }
  if (!hasNorth) { ElMessage.warning('请添加北向应用节点'); return }
  ElMessage.success('验证通过')
}

async function handleDeploy() {
  if (!flowId.value) { ElMessage.warning('请先保存数据流'); return }
  try {
    await api.deployFlow(flowId.value)
    ElMessage.success('部署成功')
    flowStatus.value = 'deployed'
  } catch (e) { ElMessage.error('部署失败: ' + e.message) }
}

async function handleStart() {
  try {
    await api.startFlow(flowId.value)
    ElMessage.success('启动成功')
    flowStatus.value = 'running'
  } catch (e) { ElMessage.error('启动失败: ' + e.message) }
}

async function handlePause() {
  try {
    await api.pauseFlow(flowId.value)
    ElMessage.success('暂停成功')
    flowStatus.value = 'paused'
  } catch (e) { ElMessage.error('暂停失败: ' + e.message) }
}

async function handleStop() {
  try {
    await api.stopFlow(flowId.value)
    ElMessage.success('停止成功')
    flowStatus.value = 'stopped'
  } catch (e) { ElMessage.error('停止失败: ' + e.message) }
}

function statusLabel(s) {
  return { draft: '草稿', deployed: '已部署', running: '运行中', paused: '已暂停', stopped: '已停止', error: '错误' }[s] || s
}
</script>

<style scoped>
.flow-editor { display: flex; flex-direction: column; height: 100vh; }
.editor-toolbar { display: flex; align-items: center; gap: 8px; padding: 8px 16px; border-bottom: 1px solid #ddd; background: #fff; }
.editor-body { display: flex; flex: 1; overflow: hidden; }
.node-palette { width: 160px; padding: 12px; background: #f5f5f5; border-right: 1px solid #ddd; overflow-y: auto; }
.node-palette h4 { margin: 0 0 4px; font-size: 12px; color: #666; }
.palette-item { padding: 6px 10px; margin-bottom: 4px; border-radius: 4px; font-size: 12px; cursor: grab; }
.palette-item.south { background: #e3f2fd; border: 1px solid #90caf9; }
.palette-item.operator { background: #fff3e0; border: 1px solid #ffcc80; }
.palette-item.north { background: #e8f5e9; border: 1px solid #a5d6a7; }
.flow-canvas { flex: 1; position: relative; }
.properties-panel { width: 240px; padding: 12px; background: #fff; border-left: 1px solid #ddd; overflow-y: auto; }
.properties-panel h4 { margin: 0 0 8px; }
</style>
