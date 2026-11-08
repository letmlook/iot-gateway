<script setup>
// 流程编辑器（v2.0 T1-③前端）。
// 映射纪律（P3 对策）：VueFlow ↔ FlowGraph 的全部转换只经 utils/graphMapping.js；
// 保存契约：整图保存（P2：草稿允许不完整，publish 由后端校验兜底）。
import { ref, computed, onMounted, onUnmounted } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import { DocumentChecked, VideoPlay, VideoPause, Delete } from '@element-plus/icons-vue'
import { VueFlow, Position, Handle, applyNodeChanges, applyEdgeChanges } from '@vue-flow/core'
import '@vue-flow/core/dist/style.css'
import '@vue-flow/core/dist/theme-default.css'
import { api } from '../api.js'
import { getErrorMessage } from '../i18n'
import {
  buildPortsMap,
  graphToVueFlow,
  vueFlowToGraph,
  edgeId,
  portsFor,
  defaultConfig,
} from '../utils/graphMapping.js'

const { t } = useI18n()
const route = useRoute()
const router = useRouter()
const flowId = route.params.id

const flowName = ref('')
const flowDescription = ref('')
const flowStatus = ref('draft')
const flowVersion = ref(0)
const vfNodes = ref([])
const vfEdges = ref([])
const portsMap = ref({})
const operators = ref([])
const selectedId = ref(null)
const saving = ref(false)
const error = ref('')

let addOffset = 0

const selectedNode = computed(() => vfNodes.value.find((n) => n.id === selectedId.value) || null)

function kindLabel(data) {
  if (data.kind === 'source') return t('flowsEditor.kindSource')
  if (data.kind === 'sink') return t(`flowsEditor.op_${data.operator}`)
  return data.operator ? t(`flowsEditor.op_${data.operator}`) : t('flowsEditor.kindOperator')
}

// ---------- 加载 ----------

async function load() {
  try {
    const [flow, ops, nodes] = await Promise.all([api.flow(flowId), api.flowOperators(), api.nodes()])
    flowName.value = flow.name
    flowDescription.value = flow.description || ''
    flowStatus.value = flow.status
    flowVersion.value = flow.version
    operators.value = ops.operators || []
    portsMap.value = buildPortsMap(operators.value)
    const vf = graphToVueFlow(flow.graph, portsMap.value)
    vfNodes.value = vf.nodes
    vfEdges.value = vf.edges
    allNodes.value = nodes
  } catch (e) {
    error.value = t('flows.loadFailed') + getErrorMessage(t, e)
  }
}

// ---------- 节点/边维护 ----------

function onNodesChange(changes) {
  vfNodes.value = applyNodeChanges(changes, vfNodes.value)
}
function onEdgesChange(changes) {
  vfEdges.value = applyEdgeChanges(changes, vfEdges.value)
}
function onConnect(params) {
  if (!params.source || params.source === params.target) return
  const id = edgeId(params.source, params.sourceHandle || 'out', params.target)
  if (vfEdges.value.some((e) => e.id === id)) return
  vfEdges.value.push({
    id,
    source: params.source,
    sourceHandle: params.sourceHandle || 'out',
    target: params.target,
    targetHandle: 'in',
  })
}
function onNodeClick({ node }) {
  selectedId.value = node.id
  const n = selectedNode.value
  if (n && n.data.kind === 'source' && n.data.config.south_node_id) loadGroups(n.data.config.south_node_id)
}
function addNode(kind, operator = null) {
  const id = crypto.randomUUID()
  const ports = portsFor({ kind, operator }, portsMap.value)
  const label =
    kind === 'source'
      ? t('flowsEditor.newSource')
      : t(`flowsEditor.op_${operator}`)
  vfNodes.value.push({
    id,
    type: 'flow-node',
    position: { x: 60 + addOffset * 36, y: 60 + addOffset * 28 },
    data: {
      label,
      kind,
      operator,
      config: defaultConfig(operator),
      inputs: ports.inputs,
      outputs: ports.outputs,
    },
  })
  addOffset += 1
  selectedId.value = id
}
async function removeSelected() {
  const n = selectedNode.value
  if (!n) return
  try {
    await ElMessageBox.confirm(t('flowsEditor.removeNodeConfirm', { name: n.data.label }), {
      type: 'warning',
    })
  } catch {
    return
  }
  vfNodes.value = vfNodes.value.filter((x) => x.id !== n.id)
  vfEdges.value = vfEdges.value.filter(
    (e) => e.source !== n.id && e.target !== n.id,
  )
  selectedId.value = null
}

// ---------- 配置面板数据 ----------

const allNodes = ref([])
const southNodes = computed(() => allNodes.value.filter((n) => n.kind === 'south'))
const northNodes = computed(() => allNodes.value.filter((n) => n.kind === 'north'))
const sourceGroups = ref([])

async function loadGroups(southNodeId) {
  try {
    const groups = await api.groups(southNodeId)
    sourceGroups.value = groups || []
  } catch {
    sourceGroups.value = []
  }
}
function onSourceNodeChange(nodeId) {
  selectedNode.value.data.config.group_id = ''
  if (nodeId) loadGroups(nodeId)
}

const OPS_WITH_FORM = ['threshold_alarm', 'state_change_alarm', 'route', 'sink_north']
const panelMode = computed(() => {
  const n = selectedNode.value
  if (!n) return null
  if (n.data.kind === 'source') return 'source'
  if (n.data.kind === 'sink' && n.data.operator === 'sink_north') return 'sink_north'
  if (n.data.kind === 'operator' && OPS_WITH_FORM.includes(n.data.operator)) return n.data.operator
  return 'plain'
})

// ---------- 保存与发布 ----------

async function save() {
  saving.value = true
  error.value = ''
  try {
    const graph = vueFlowToGraph(vfNodes.value, vfEdges.value)
    await api.updateFlow(flowId, {
      name: flowName.value,
      description: flowDescription.value || null,
      nodes: graph.nodes,
      edges: graph.edges,
    })
    ElMessage.success(t('flowsEditor.saved'))
  } catch (e) {
    error.value = getErrorMessage(t, e)
    ElMessage.error(error.value)
  } finally {
    saving.value = false
  }
}

async function publish() {
  await save()
  try {
    const r = await api.publishFlow(flowId)
    flowStatus.value = r.status
    flowVersion.value = r.version
    ElMessage.success(t('flows.publishOk'))
  } catch (e) {
    // publish 校验失败信息完整呈现（后端逐条列出）
    ElMessage.error(getErrorMessage(t, e))
  }
}

async function unpublish() {
  try {
    const r = await api.unpublishFlow(flowId)
    flowStatus.value = r.status
    ElMessage.success(t('flows.unpublishOk'))
  } catch (e) {
    ElMessage.error(getErrorMessage(t, e))
  }
}

function back() {
  router.push('/flows')
}

onMounted(load)
onUnmounted(() => {})
</script>

<template>
  <div class="editor-page">
    <div class="editor-toolbar">
      <el-button size="small" @click="back">{{ t('common.back') }}</el-button>
      <el-input v-model="flowName" size="small" class="editor-name" maxlength="64" />
      <el-tag :type="flowStatus === 'published' ? 'success' : 'info'" size="small">
        {{ t(`flows.status_${flowStatus}`) }} · v{{ flowVersion }}
      </el-tag>
      <span class="editor-spacer" />
      <el-button size="small" type="danger" :icon="Delete" :disabled="!selectedNode" @click="removeSelected">
        {{ t('flowsEditor.removeNode') }}
      </el-button>
      <el-button
        v-if="flowStatus === 'published'"
        size="small"
        type="warning"
        :icon="VideoPause"
        @click="unpublish"
      >{{ t('flows.unpublish') }}</el-button>
      <el-button v-else size="small" type="success" :icon="VideoPlay" @click="publish">
        {{ t('flows.publish') }}
      </el-button>
      <el-button size="small" type="primary" :icon="DocumentChecked" :loading="saving" @click="save">
        {{ t('flowsEditor.save') }}
      </el-button>
    </div>
    <div v-if="error" class="editor-error">{{ error }}</div>

    <div class="editor-body">
      <!-- 算子面板：来自 /flows/operators（P6 对策，前端不硬编码算子知识；source 为结构性节点） -->
      <div class="editor-palette">
        <div class="palette-title">{{ t('flowsEditor.palette') }}</div>
        <el-button class="palette-item" @click="addNode('source')">
          {{ t('flowsEditor.kindSource') }}
        </el-button>
        <el-button
          v-for="op in operators"
          :key="op.name"
          class="palette-item"
          @click="addNode('operator', op.name)"
        >{{ t(`flowsEditor.op_${op.name}`) }}</el-button>
      </div>

      <div class="editor-canvas">
        <VueFlow
          :nodes="vfNodes"
          :edges="vfEdges"
          :default-edge-options="{ type: 'smoothstep' }"
          fit-view-on-init
          @nodes-change="onNodesChange"
          @edges-change="onEdgesChange"
          @connect="onConnect"
          @node-click="onNodeClick"
          @pane-click="selectedId = null"
        >
          <template #node-flow-node="{ data, selected }">
            <div class="flow-node" :class="[`kind-${data.kind}`, { selected }]">
              <Handle
                v-for="(p, i) in data.inputs"
                :key="'in-' + p"
                type="target"
                :id="p"
                :position="Position.Top"
                :style="{ left: `${((i + 1) / (data.inputs.length + 1)) * 100}%` }"
              />
              <div class="flow-node-title">{{ data.label }}</div>
              <div class="flow-node-sub">{{ kindLabel(data) }}</div>
              <Handle
                v-for="(p, i) in data.outputs"
                :key="'out-' + p"
                type="source"
                :id="p"
                :position="Position.Bottom"
                :style="{ left: `${((i + 1) / (data.outputs.length + 1)) * 100}%` }"
              />
            </div>
          </template>
        </VueFlow>
      </div>

      <!-- 配置面板 -->
      <div class="editor-panel">
        <div class="palette-title">{{ t('flowsEditor.config') }}</div>
        <template v-if="selectedNode">
          <el-form label-position="top" size="small" class="editor-form">
            <el-form-item :label="t('flowsEditor.nodeName')">
              <el-input v-model="selectedNode.data.label" maxlength="64" />
            </el-form-item>

            <!-- source：南向节点 + 组 -->
            <template v-if="panelMode === 'source'">
              <el-form-item :label="t('flowsEditor.southNode')" required>
                <el-select
                  :model-value="selectedNode.data.config.south_node_id"
                  filterable
                  @update:model-value="(v) => { selectedNode.data.config.south_node_id = v; onSourceNodeChange(v) }"
                >
                  <el-option
                    v-for="n in southNodes"
                    :key="n.id"
                    :value="n.id"
                    :label="n.name"
                  />
                </el-select>
              </el-form-item>
              <el-form-item :label="t('flowsEditor.southGroup')" required>
                <el-select v-model="selectedNode.data.config.group_id" filterable>
                  <el-option
                    v-for="g in sourceGroups"
                    :key="g.id"
                    :value="g.id"
                    :label="g.name"
                  />
                </el-select>
              </el-form-item>
            </template>

            <!-- threshold_alarm -->
            <template v-else-if="panelMode === 'threshold_alarm'">
              <el-form-item :label="t('flowsEditor.tagName')" required>
                <el-input v-model="selectedNode.data.config.tag_name" />
              </el-form-item>
              <el-form-item :label="t('flowsEditor.highLimit')">
                <el-input-number v-model="selectedNode.data.config.high" :controls="false" class="w-full" />
              </el-form-item>
              <el-form-item :label="t('flowsEditor.lowLimit')">
                <el-input-number v-model="selectedNode.data.config.low" :controls="false" class="w-full" />
              </el-form-item>
              <el-form-item :label="t('flowsEditor.level')">
                <el-select v-model="selectedNode.data.config.level">
                  <el-option value="critical" :label="t('alarms.level_critical')" />
                  <el-option value="high" :label="t('alarms.level_high')" />
                  <el-option value="medium" :label="t('alarms.level_medium')" />
                  <el-option value="low" :label="t('alarms.level_low')" />
                  <el-option value="info" :label="t('alarms.level_info')" />
                </el-select>
              </el-form-item>
              <el-form-item :label="t('flowsEditor.message')">
                <el-input v-model="selectedNode.data.config.message" />
              </el-form-item>
            </template>

            <!-- state_change_alarm -->
            <template v-else-if="panelMode === 'state_change_alarm'">
              <el-form-item :label="t('flowsEditor.tagName')" required>
                <el-input v-model="selectedNode.data.config.tag_name" />
              </el-form-item>
              <el-form-item :label="t('flowsEditor.fromState')">
                <el-input v-model="selectedNode.data.config.from" placeholder="*" />
              </el-form-item>
              <el-form-item :label="t('flowsEditor.toState')" required>
                <el-input v-model="selectedNode.data.config.to" />
              </el-form-item>
              <el-form-item :label="t('flowsEditor.level')">
                <el-select v-model="selectedNode.data.config.level">
                  <el-option value="critical" :label="t('alarms.level_critical')" />
                  <el-option value="high" :label="t('alarms.level_high')" />
                  <el-option value="medium" :label="t('alarms.level_medium')" />
                  <el-option value="low" :label="t('alarms.level_low')" />
                  <el-option value="info" :label="t('alarms.level_info')" />
                </el-select>
              </el-form-item>
            </template>

            <!-- route -->
            <template v-else-if="panelMode === 'route'">
              <el-form-item :label="t('flowsEditor.tagName')" required>
                <el-input v-model="selectedNode.data.config.tag_name" />
              </el-form-item>
              <el-form-item :label="t('flowsEditor.compareOp')" required>
                <el-select v-model="selectedNode.data.config.op">
                  <el-option value="gt" label="> (gt)" />
                  <el-option value="lt" label="< (lt)" />
                  <el-option value="ge" label="≥ (ge)" />
                  <el-option value="le" label="≤ (le)" />
                  <el-option value="eq" label="= (eq)" />
                  <el-option value="ne" label="≠ (ne)" />
                </el-select>
              </el-form-item>
              <el-form-item :label="t('flowsEditor.compareValue')" required>
                <el-input-number v-model="selectedNode.data.config.value" :controls="false" class="w-full" />
              </el-form-item>
            </template>

            <!-- sink_north -->
            <template v-else-if="panelMode === 'sink_north'">
              <el-form-item :label="t('flowsEditor.northTarget')" required>
                <el-select v-model="selectedNode.data.config.node_id" filterable>
                  <el-option
                    v-for="n in northNodes"
                    :key="n.id"
                    :value="n.id"
                    :label="n.name"
                  />
                </el-select>
              </el-form-item>
            </template>

            <template v-else-if="panelMode === 'plain'">
              <div class="editor-hint">{{ t('flowsEditor.noConfig') }}</div>
            </template>
          </el-form>
        </template>
        <div v-else class="editor-hint">{{ t('flowsEditor.selectHint') }}</div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.editor-page {
  display: flex;
  flex-direction: column;
  gap: 10px;
  height: 100%;
}
.editor-toolbar {
  display: flex;
  align-items: center;
  gap: 10px;
}
.editor-name {
  width: 220px;
}
.editor-spacer {
  flex: 1;
}
.editor-error {
  color: var(--el-color-danger);
  font-size: 13px;
}
.editor-body {
  display: flex;
  gap: 10px;
  flex: 1;
  min-height: 480px;
}
.editor-palette,
.editor-panel {
  width: 230px;
  flex-shrink: 0;
  border: 1px solid var(--el-border-color);
  border-radius: 8px;
  padding: 10px;
  overflow: auto;
  background: var(--el-bg-color);
}
.palette-title {
  font-weight: 600;
  font-size: 13px;
  margin-bottom: 10px;
  opacity: 0.75;
}
.palette-item {
  display: block;
  width: 100%;
  margin: 0 0 8px;
}
.editor-canvas {
  flex: 1;
  border: 1px solid var(--el-border-color);
  border-radius: 8px;
  overflow: hidden;
  min-height: 480px;
}
.flow-node {
  min-width: 130px;
  padding: 8px 12px;
  border: 1.5px solid var(--el-border-color);
  border-radius: 8px;
  background: var(--el-bg-color);
  text-align: center;
}
.flow-node.selected,
.flow-node.kind-source.selected {
  border-color: var(--el-color-primary);
}
.flow-node.kind-source {
  border-color: var(--el-color-success);
}
.flow-node.kind-sink {
  border-color: var(--el-color-warning);
}
.flow-node-title {
  font-weight: 600;
  font-size: 13px;
}
.flow-node-sub {
  font-size: 11px;
  opacity: 0.6;
}
.editor-form {
  padding-right: 4px;
}
.editor-hint {
  opacity: 0.6;
  font-size: 12px;
}
.w-full {
  width: 100%;
}
</style>
