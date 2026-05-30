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
      <el-button type="info" plain style="margin-left: auto" @click="$router.push('/monitor/live')">📊 Live</el-button>
    </div>

    <div class="editor-body">
      <!-- Node Palette -->
      <div class="node-palette">
        <el-collapse v-model="paletteOpen">
          <!-- Canvas nodes tree with status badges -->
          <el-collapse-item title="画布节点 🌳" name="canvas-nodes">
            <div v-if="nodes.length === 0" class="canvas-nodes-empty">拖拽或添加节点到画布</div>
            <div
              v-for="node in nodes"
              :key="node.id"
              class="canvas-node-item"
              :class="{ 'canvas-node-selected': selectedNode && selectedNode.id === node.id }"
              @click="selectCanvasNode(node)"
            >
              <span class="node-type-icon">
                {{ node.type === 'south' ? '🔌' : node.type === 'north' ? '📤' : '⚙️' }}
              </span>
              <span class="node-tree-label">{{ node.label }}</span>
              <span
                class="node-status-dot"
                :style="{ backgroundColor: STATUS_COLORS[nodeRuntimeStatus[node.id] || 'unknown'] }"
                :title="'状态: ' + statusLabel(nodeRuntimeStatus[node.id])"
              ></span>
            </div>
            <div v-if="nodes.length > 0" class="canvas-nodes-legend">
              <span class="legend-item"><span class="legend-dot" style="background:#52c41a"></span>运行</span>
              <span class="legend-item"><span class="legend-dot" style="background:#ff4d4f"></span>错误</span>
              <span class="legend-item"><span class="legend-dot" style="background:#999"></span>停止</span>
            </div>
          </el-collapse-item>
          <el-collapse-item title="南向设备 🔌" name="south">
            <div
              v-for="plugin in southPlugins"
              :key="plugin.name"
              class="palette-item south"
              draggable="true"
              @dragstart="onDragStart($event, 'south', plugin.name)"
            >
              <el-tooltip :content="plugin.description_zh || plugin.description || ''" placement="right" :open-delay="300">
                <span>{{ plugin.name_zh || plugin.name }}</span>
              </el-tooltip>
            </div>
          </el-collapse-item>
          <el-collapse-item title="算子 ⚙️" name="operator">
            <div
              v-for="op in operatorList"
              :key="op.name"
              class="palette-item operator"
              draggable="true"
              @dragstart="onDragStart($event, 'operator', op.name)"
            >
              <el-tooltip :content="op.description_zh || op.description || ''" placement="right" :open-delay="300">
                <span>{{ op.name_zh || op.name }}</span>
              </el-tooltip>
            </div>
          </el-collapse-item>
          <el-collapse-item title="北向应用 📤" name="north">
            <div
              v-for="plugin in northPlugins"
              :key="plugin.name"
              class="palette-item north"
              draggable="true"
              @dragstart="onDragStart($event, 'north', plugin.name)"
            >
              <el-tooltip :content="plugin.description_zh || plugin.description || ''" placement="right" :open-delay="300">
                <span>{{ plugin.name_zh || plugin.name }}</span>
              </el-tooltip>
            </div>
          </el-collapse-item>
        </el-collapse>
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

          <!-- South Node -->
          <template v-if="selectedNode.type === 'south'">
            <el-tabs>
              <el-tab-pane label="配置">
                <el-form-item label="名称">
                  <el-input v-model="selectedNode.data.name" />
                </el-form-item>
                <el-form-item label="类型">
                  <el-tag>{{ selectedNode.type }}</el-tag>
                </el-form-item>
                <template v-if="southSchema">
                  <div v-for="param in schemaParams(southSchema)" :key="param.name">
                    <el-form-item :label="param.name_zh || param.title || param.name">
                      <el-select v-if="param.options && param.options.length" v-model="selectedNode.data.config[param.name]">
                        <el-option v-for="option in param.options" :key="optionValue(option)" :label="optionLabel(option)" :value="optionValue(option)" />
                      </el-select>
                      <el-input v-else v-model="selectedNode.data.config[param.name]" />
                    </el-form-item>
                  </div>
                </template>
              </el-tab-pane>
              <el-tab-pane label="点位">
                <el-button size="small" @click="loadPointsData" :loading="pointsLoading">刷新</el-button>
                <el-table :data="groupsData" size="small" style="margin-top: 8px" v-if="groupsData.length">
                  <el-table-column prop="name" label="分组" />
                  <el-table-column prop="interval_ms" label="周期(ms)" width="100" />
                </el-table>
                <el-table :data="tagsData" size="small" style="margin-top: 8px">
                  <el-table-column prop="name" label="名称" />
                  <el-table-column prop="address" label="地址" />
                  <el-table-column prop="type" label="类型" width="60" />
                  <el-table-column prop="access" label="访问" width="70" />
                </el-table>
              </el-tab-pane>
            </el-tabs>
          </template>

          <!-- Operator Node -->
          <template v-else-if="selectedNode.type === 'operator'">
            <el-tabs>
              <el-tab-pane label="配置">
                <el-form-item label="名称">
                  <el-input v-model="selectedNode.data.name" />
                </el-form-item>
                <el-form-item label="算子">
                  <el-select v-model="selectedNode.data.operatorName" @change="onOperatorTypeChange">
                    <el-option v-for="op in operatorList" :key="op.name" :label="op.name_zh || op.name" :value="op.name" />
                  </el-select>
                </el-form-item>
                <template v-if="selectedNode.data.operatorName">
                  <el-divider />
                  <!-- filter -->
                  <template v-if="selectedNode.data.operatorName === 'filter'">
                    <el-form-item label="条件">
                      <el-input type="textarea" v-model="selectedNode.data.operatorConfig.condition" :rows="3" placeholder="如: temperature > 100" />
                    </el-form-item>
                    <el-form-item label="通过">
                      <el-switch v-model="selectedNode.data.operatorConfig.pass" />
                    </el-form-item>
                  </template>
                  <!-- transform -->
                  <template v-else-if="selectedNode.data.operatorName === 'transform'">
                    <el-form-item label="操作">
                      <el-input type="textarea" v-model="selectedNode.data.operatorConfig.operations" :rows="4" placeholder='[{"op": "add", "field": "value", "const": 10}]' />
                    </el-form-item>
                  </template>
                  <!-- aggregate -->
                  <template v-else-if="selectedNode.data.operatorName === 'aggregate'">
                    <el-form-item label="窗口类型">
                      <el-select v-model="selectedNode.data.operatorConfig.window_type">
                        <el-option label="tumble" value="tumble" />
                        <el-option label="hop" value="hop" />
                        <el-option label="session" value="session" />
                      </el-select>
                    </el-form-item>
                    <el-form-item label="窗口大小">
                      <el-input-number v-model="selectedNode.data.operatorConfig.window_size" :min="1" />
                    </el-form-item>
                    <el-form-item label="聚合">
                      <el-input type="textarea" v-model="selectedNode.data.operatorConfig.aggregations" :rows="3" placeholder='[{"field": "temp", "func": "avg"}]' />
                    </el-form-item>
                  </template>
                  <!-- router -->
                  <template v-else-if="selectedNode.data.operatorName === 'router'">
                    <el-form-item label="默认端口">
                      <el-input v-model="selectedNode.data.operatorConfig.default_port" />
                    </el-form-item>
                    <el-form-item label="路由">
                      <el-input type="textarea" v-model="selectedNode.data.operatorConfig.routes" :rows="4" placeholder='[{"condition": "temp > 50", "port": "high"}]' />
                    </el-form-item>
                  </template>
                  <!-- buffer -->
                  <template v-else-if="selectedNode.data.operatorName === 'buffer'">
                    <el-form-item label="大小限制">
                      <el-input-number v-model="selectedNode.data.operatorConfig.size_limit" :min="1" />
                    </el-form-item>
                    <el-form-item label="时间限制(ms)">
                      <el-input-number v-model="selectedNode.data.operatorConfig.time_limit" :min="1" />
                    </el-form-item>
                    <el-form-item label="最大条数">
                      <el-input-number v-model="selectedNode.data.operatorConfig.max_items" :min="1" />
                    </el-form-item>
                  </template>
                  <!-- alarm -->
                  <template v-else-if="selectedNode.data.operatorName === 'alarm'">
                    <el-form-item label="规则">
                      <el-input type="textarea" v-model="selectedNode.data.operatorConfig.rules" :rows="5" placeholder='[{"type": "high", "field": "temperature", "threshold": 100, "severity": "critical"}]' />
                    </el-form-item>
                  </template>
                  <!-- json-path -->
                  <template v-else-if="selectedNode.data.operatorName === 'json-path'">
                    <el-form-item label="源字段">
                      <el-input v-model="selectedNode.data.operatorConfig.source_field" />
                    </el-form-item>
                    <el-form-item label="表达式">
                      <el-input type="textarea" v-model="selectedNode.data.operatorConfig.expressions" :rows="3" placeholder='["$.data.temp", "$.data.hum"]' />
                    </el-form-item>
                  </template>
                  <!-- deadband -->
                  <template v-else-if="selectedNode.data.operatorName === 'deadband'">
                    <el-form-item label="字段">
                      <el-input v-model="selectedNode.data.operatorConfig.field" />
                    </el-form-item>
                    <el-form-item label="类型">
                      <el-select v-model="selectedNode.data.operatorConfig.type">
                        <el-option label="absolute" value="absolute" />
                        <el-option label="percent" value="percent" />
                      </el-select>
                    </el-form-item>
                    <el-form-item label="值">
                      <el-input-number v-model="selectedNode.data.operatorConfig.value" />
                    </el-form-item>
                  </template>
                  <!-- formula -->
                  <template v-else-if="selectedNode.data.operatorName === 'formula'">
                    <el-form-item label="表达式">
                      <el-input type="textarea" v-model="selectedNode.data.operatorConfig.expression" :rows="4" placeholder="temperature * 1.8 + 32" />
                    </el-form-item>
                  </template>
                  <!-- clamp -->
                  <template v-else-if="selectedNode.data.operatorName === 'clamp'">
                    <el-form-item label="字段">
                      <el-input v-model="selectedNode.data.operatorConfig.field" />
                    </el-form-item>
                    <el-form-item label="最小值">
                      <el-input-number v-model="selectedNode.data.operatorConfig.min" />
                    </el-form-item>
                    <el-form-item label="最大值">
                      <el-input-number v-model="selectedNode.data.operatorConfig.max" />
                    </el-form-item>
                  </template>
                  <!-- round -->
                  <template v-else-if="selectedNode.data.operatorName === 'round'">
                    <el-form-item label="字段">
                      <el-input v-model="selectedNode.data.operatorConfig.field" />
                    </el-form-item>
                    <el-form-item label="小数位">
                      <el-input-number v-model="selectedNode.data.operatorConfig.decimals" :min="0" :max="10" />
                    </el-form-item>
                  </template>
                  <!-- change -->
                  <template v-else-if="selectedNode.data.operatorName === 'change'">
                    <el-form-item label="字段">
                      <el-input v-model="selectedNode.data.operatorConfig.field" />
                    </el-form-item>
                  </template>
                  <!-- range -->
                  <template v-else-if="selectedNode.data.operatorName === 'range'">
                    <el-form-item label="字段">
                      <el-input v-model="selectedNode.data.operatorConfig.field" />
                    </el-form-item>
                    <el-form-item label="输入最小">
                      <el-input-number v-model="selectedNode.data.operatorConfig.in_min" />
                    </el-form-item>
                    <el-form-item label="输入最大">
                      <el-input-number v-model="selectedNode.data.operatorConfig.in_max" />
                    </el-form-item>
                    <el-form-item label="输出最小">
                      <el-input-number v-model="selectedNode.data.operatorConfig.out_min" />
                    </el-form-item>
                    <el-form-item label="输出最大">
                      <el-input-number v-model="selectedNode.data.operatorConfig.out_max" />
                    </el-form-item>
                  </template>
                  <!-- batch -->
                  <template v-else-if="selectedNode.data.operatorName === 'batch'">
                    <el-form-item label="批次大小">
                      <el-input-number v-model="selectedNode.data.operatorConfig.size" :min="1" />
                    </el-form-item>
                    <el-form-item label="超时(ms)">
                      <el-input-number v-model="selectedNode.data.operatorConfig.timeout_ms" :min="1" />
                    </el-form-item>
                  </template>
                  <!-- split -->
                  <template v-else-if="selectedNode.data.operatorName === 'split'">
                    <el-form-item label="字段">
                      <el-input v-model="selectedNode.data.operatorConfig.field" />
                    </el-form-item>
                    <el-form-item label="分隔符">
                      <el-input v-model="selectedNode.data.operatorConfig.delimiter" />
                    </el-form-item>
                  </template>
                  <!-- join -->
                  <template v-else-if="selectedNode.data.operatorName === 'join'">
                    <el-form-item label="字段">
                      <el-input v-model="selectedNode.data.operatorConfig.field" />
                    </el-form-item>
                    <el-form-item label="分隔符">
                      <el-input v-model="selectedNode.data.operatorConfig.delimiter" />
                    </el-form-item>
                  </template>
                  <!-- dedup -->
                  <template v-else-if="selectedNode.data.operatorName === 'dedup'">
                    <el-form-item label="字段">
                      <el-input v-model="selectedNode.data.operatorConfig.field" />
                    </el-form-item>
                  </template>
                  <!-- script -->
                  <template v-else-if="selectedNode.data.operatorName === 'script'">
                    <el-form-item label="脚本">
                      <el-input type="textarea" v-model="selectedNode.data.operatorConfig.script" :rows="6" placeholder="// rhai script" />
                    </el-form-item>
                  </template>
                  <!-- throttle -->
                  <template v-else-if="selectedNode.data.operatorName === 'throttle'">
                    <el-form-item label="最大速率">
                      <el-input-number v-model="selectedNode.data.operatorConfig.max_rate" :min="1" />
                    </el-form-item>
                    <el-form-item label="窗口(ms)">
                      <el-input-number v-model="selectedNode.data.operatorConfig.window_ms" :min="1" />
                    </el-form-item>
                  </template>
                  <!-- convert -->
                  <template v-else-if="selectedNode.data.operatorName === 'convert'">
                    <el-form-item label="字段">
                      <el-input v-model="selectedNode.data.operatorConfig.field" />
                    </el-form-item>
                    <el-form-item label="目标类型">
                      <el-select v-model="selectedNode.data.operatorConfig.to_type">
                        <el-option label="int" value="int" />
                        <el-option label="float" value="float" />
                        <el-option label="string" value="string" />
                        <el-option label="bool" value="bool" />
                      </el-select>
                    </el-form-item>
                  </template>
                  <!-- log -->
                  <template v-else-if="selectedNode.data.operatorName === 'log'">
                    <el-form-item label="级别">
                      <el-select v-model="selectedNode.data.operatorConfig.level">
                        <el-option label="debug" value="debug" />
                        <el-option label="info" value="info" />
                        <el-option label="warn" value="warn" />
                        <el-option label="error" value="error" />
                      </el-select>
                    </el-form-item>
                    <el-form-item label="模板">
                      <el-input v-model="selectedNode.data.operatorConfig.template" placeholder="如: temperature={}" />
                    </el-form-item>
                  </template>
                </template>
              </el-tab-pane>
            </el-tabs>
          </template>

          <!-- North Node -->
          <template v-else-if="selectedNode.type === 'north'">
            <el-tabs>
              <el-tab-pane label="配置">
                <el-form-item label="名称">
                  <el-input v-model="selectedNode.data.name" />
                </el-form-item>
                <el-form-item label="类型">
                  <el-tag>{{ selectedNode.type }}</el-tag>
                </el-form-item>
                <template v-if="northSchema">
                  <div v-for="param in schemaParams(northSchema)" :key="param.name">
                    <el-form-item :label="param.name_zh || param.title || param.name">
                      <el-select v-if="param.options && param.options.length" v-model="selectedNode.data.config[param.name]">
                        <el-option v-for="option in param.options" :key="optionValue(option)" :label="optionLabel(option)" :value="optionValue(option)" />
                      </el-select>
                      <el-input v-else v-model="selectedNode.data.config[param.name]" />
                    </el-form-item>
                  </div>
                </template>
              </el-tab-pane>
              <el-tab-pane label="订阅">
                <p style="font-size:12px;color:#666;margin-bottom:8px">选择此北向节点订阅的数据源：</p>
                <el-checkbox-group v-model="selectedNode.data.subscriptions">
                  <div v-for="node in canvasSourceNodes" :key="node.id" style="margin-bottom:4px">
                    <el-checkbox :label="node.id">
                      {{ node.label }} <span style="color:#999;font-size:11px">({{ node.type }})</span>
                    </el-checkbox>
                  </div>
                </el-checkbox-group>
                <el-button size="small" type="primary" style="margin-top:8px" @click="saveSubscriptions">保存订阅</el-button>
              </el-tab-pane>
            </el-tabs>
          </template>

          <template v-else>
            <el-form-item label="名称">
              <el-input v-model="selectedNode.data.name" />
            </el-form-item>
            <el-form-item label="类型">
              <el-tag>{{ selectedNode.type }}</el-tag>
            </el-form-item>
          </template>

          <el-divider />
          <el-button type="danger" size="small" @click="removeNode(selectedNode.id)">删除节点</el-button>
        </el-form>
      </div>
    </div>

    <!-- Data Preview Panel -->
    <div class="preview-panel" v-if="previewOpen">
      <el-collapse v-model="previewActiveNames">
        <el-collapse-item title="数据预览" name="preview">
          <div v-if="selectedNode">
            <el-tabs size="small">
              <el-tab-pane label="输入数据">
                <pre class="data-preview">{{ previewInput }}</pre>
              </el-tab-pane>
              <el-tab-pane label="输出数据">
                <pre class="data-preview">{{ previewOutput }}</pre>
              </el-tab-pane>
              <el-tab-pane label="最近消息">
                <div v-for="(msg, i) in recentMessages" :key="i" class="recent-msg">
                  <span class="msg-time">{{ msg.time }}</span>
                  <span :class="'msg-' + msg.dir">{{ msg.dir === 'in' ? '←' : '→' }}</span>
                  <span class="msg-data">{{ msg.data }}</span>
                </div>
              </el-tab-pane>
            </el-tabs>
          </div>
          <div v-else style="color:#999;text-align:center;padding:20px">请先选择节点</div>
        </el-collapse-item>
      </el-collapse>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, onMounted, onUnmounted, watch } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import { VueFlow, useVueFlow, MarkerType, Position } from '@vue-flow/core'
import { Background } from '@vue-flow/background'
import { Controls } from '@vue-flow/controls'
import { MiniMap } from '@vue-flow/minimap'
import { ElMessage } from 'element-plus'
import { api } from '../api.js'

// Node status constants
const NODE_STATUS = {
  RUNNING: 'running',
  STOPPED: 'stopped',
  ERROR: 'error',
  UNKNOWN: 'unknown'
}

const STATUS_COLORS = {
  running: '#52c41a',
  stopped: '#999',
  error: '#ff4d4f',
  unknown: '#999'
}

function statusLabel(s) {
  return { running: '运行中', stopped: '已停止', error: '错误', unknown: '未知' }[s] || s || '未知'
}

function normalizePluginList(response) {
  return Array.isArray(response) ? response : (response?.plugins || [])
}

function schemaParams(schema) {
  if (!schema) return []
  if (Array.isArray(schema.params)) return schema.params
  return Object.entries(schema.properties || {}).map(([name, prop]) => ({ name, ...prop }))
}

function optionValue(option) {
  return option && typeof option === 'object' ? (option.value ?? option.name ?? option.label) : option
}

function optionLabel(option) {
  return option && typeof option === 'object' ? (option.label ?? option.name ?? option.value) : option
}

function buildPreviewSampleInput() {
  return {
    input: {
      payload: {
        temperature: { type: 'Float64', value: 25.6 },
        humidity: { type: 'Float64', value: 60 }
      },
      metadata: {
        source: 'flow-editor-preview'
      }
    }
  }
}

function previewResultSummary(result) {
  const outputCount = Array.isArray(result?.output) ? result.output.length : 0
  const alarmCount = Array.isArray(result?.alarm_events) ? result.alarm_events.length : 0
  const errorCount = Array.isArray(result?.errors) ? result.errors.length : 0
  return `preview ok: output=${outputCount}, alarms=${alarmCount}, errors=${errorCount}`
}

function previewErrorPayload(error) {
  return {
    error: error?.message || String(error),
    source: 'backend-preview'
  }
}

function addRecentPreviewMessage(dir, data) {
  recentMessages.value = [
    { time: new Date().toLocaleTimeString(), dir, data },
    ...recentMessages.value.slice(0, 3)
  ]
}

function dataPort(id, name, required) {
  return { id, name, port_type: 'data', required }
}

function buildInputPorts(node) {
  if (node.data.kind === 'operator') return [dataPort('in', '输入', true)]
  if (node.data.kind === 'north') return [dataPort('in', '输入', true)]
  return []
}

function buildOutputPorts(node) {
  if (node.data.kind === 'south') return [dataPort('out', '输出', false)]
  if (node.data.kind === 'operator') return [dataPort('out', '输出', false)]
  return []
}

function nodePosition(node) {
  return {
    x: node.position?.x || 0,
    y: node.position?.y || 0
  }
}

function parseJsonArray(value) {
  if (Array.isArray(value)) return value
  if (typeof value !== 'string') return []
  try {
    const parsed = JSON.parse(value)
    return Array.isArray(parsed) ? parsed : []
  } catch (_) {
    return []
  }
}

function operatorRuntimeConfig(node) {
  const config = { ...(node.data.operatorConfig || {}) }
  switch (node.data.operatorName) {
    case 'range':
      if (config.field !== undefined && config.tag === undefined) {
        config.tag = config.field
        delete config.field
      }
      return config
    case 'transform':
      if (config.rules === undefined && config.operations !== undefined) {
        config.rules = config.operations
      }
      return config
    case 'alarm':
      config.rules = parseJsonArray(config.rules)
      return config
    case 'aggregate':
      if (config.specs === undefined && config.aggregations !== undefined) {
        config.specs = config.aggregations
      }
      config.specs = parseJsonArray(config.specs)
      return config
    default:
      return config
  }
}

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
const flowCreatedAt = ref(null)
const flowDescription = ref(null)
const nodes = ref([])
const edges = ref([])
const selectedNode = ref(null)
const southPlugins = ref([])
const northPlugins = ref([])
const operatorList = ref([])

// Node runtime status: { [nodeId]: 'running' | 'stopped' | 'error' | 'unknown' }
const nodeRuntimeStatus = ref({})

// Palette state
const paletteOpen = ref(['south', 'operator', 'north'])

// Properties panel state
const southSchema = ref(null)
const northSchema = ref(null)
const groupsData = ref([])
const tagsData = ref([])
const pointsLoading = ref(false)

// Subscription state for north nodes
const canvasSourceNodes = computed(() =>
  nodes.value.filter(n => n.type === 'south' || n.type === 'operator')
)

// Preview panel state
const previewOpen = ref(false)
const previewActiveNames = ref(['preview'])
const previewInput = ref('暂无数据')
const previewOutput = ref('暂无数据')
const recentMessages = ref([])
const currentFlowBindings = ref([])

// Status polling
let statusPollTimer = null
let statusPollCount = 0

// Node status polling
let nodeStatusTimer = null

function startNodeStatusPoll() {
  stopNodeStatusPoll()
  fetchNodeStatus()
  nodeStatusTimer = setInterval(fetchNodeStatus, 5000)
}

function stopNodeStatusPoll() {
  if (nodeStatusTimer) {
    clearInterval(nodeStatusTimer)
    nodeStatusTimer = null
  }
}

async function fetchNodeStatus() {
  if (!nodes.value.length) return
  try {
    const allNodes = await api.nodes()
    // Map status by node id
    const statusMap = {}
    allNodes.forEach(n => {
      statusMap[n.id] = n.state || n.status || 'unknown'
    })
    // Also check flow-specific node statuses
    try {
      const flowData = await api.flow(flowId.value)
      const flowNodes = flowData.flow?.nodes || []
      flowNodes.forEach(fn => {
        statusMap[fn.id] = fn.state || fn.status || statusMap[fn.id] || 'unknown'
      })
    } catch (_) {}
    nodeRuntimeStatus.value = statusMap
  } catch (e) {
    console.error('Failed to fetch node status', e)
  }
}

onMounted(async () => {
  // Load operators from API
  try {
    const opsData = await api.operators()
    operatorList.value = opsData.operators || []
  } catch (e) {
    console.error('Failed to load operators:', e)
    operatorList.value = []
  }

  // Load plugins
  try {
    const south = await api.pluginsSouth()
    southPlugins.value = normalizePluginList(south)
    const north = await api.pluginsNorth()
    northPlugins.value = normalizePluginList(north)
  } catch (e) {
    console.error(e)
    southPlugins.value = []
    northPlugins.value = []
  }

  if (flowId.value) {
    try {
      const data = await api.flow(flowId.value)
      const flow = data.flow
      flowName.value = flow.name
      flowStatus.value = flow.status
      flowCreatedAt.value = flow.created_at || null
      flowDescription.value = flow.description || null
      currentFlowBindings.value = flow.bindings || []
      nodes.value = (flow.nodes || []).map(n => ({
        id: n.id,
        type: n.kind,
        label: n.name,
        position: n.position || { x: 100, y: 100 },
        data: {
          name: n.name,
          kind: n.kind,
          pluginName: n.operator_name || n.plugin_name || n.name,
          operatorName: n.operator_name,
          config: n.config || {},
          operatorConfig: n.config || {},
          subscriptions: []
        }
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

onUnmounted(() => {
  if (statusPollTimer) clearInterval(statusPollTimer)
  stopNodeStatusPoll()
  stopPreviewRefresh()
})

// Watch selected node to load schema/points data
watch(selectedNode, async (node) => {
  if (!node) return

  // Reset data
  southSchema.value = null
  northSchema.value = null
  groupsData.value = []
  tagsData.value = []

  if (node.type === 'south') {
    const pluginName = node.data.pluginName || node.data.name
    try {
      southSchema.value = await api.pluginSouthSchema(pluginName)
    } catch (e) { console.error(e) }
    loadPointsData()
  } else if (node.type === 'north') {
    const pluginName = node.data.pluginName || node.data.name
    try {
      northSchema.value = await api.pluginNorthSchema(pluginName)
    } catch (e) { console.error(e) }
    // Load existing subscriptions
    try {
      const subsData = await api.subscriptions(node.id)
      node.data.subscriptions = (subsData.subscriptions || []).map(s => s.south_node_id)
    } catch (e) { console.error(e) }
  }
}, { immediate: true })

// Start node status polling once nodes are loaded
watch(nodes, (newNodes) => {
  if (newNodes && newNodes.length > 0) {
    startNodeStatusPoll()
  }
}, { immediate: true })

async function loadPointsData() {
  if (!selectedNode.value || selectedNode.value.type !== 'south') return
  pointsLoading.value = true
  try {
    const nodeId = selectedNode.value.id
    const [groupsRes, tagsRes] = await Promise.all([
      api.groups(nodeId),
      api.tags(nodeId)
    ])
    groupsData.value = groupsRes.groups || []
    tagsData.value = tagsRes.tags || []
  } catch (e) {
    ElMessage.error('加载点位数据失败: ' + e.message)
  } finally {
    pointsLoading.value = false
  }
}

async function saveSubscriptions() {
  if (!selectedNode.value) return
  try {
    const subs = selectedNode.value.data.subscriptions.map(id => ({ south_node_id: id }))
    await api.setSubscriptions(selectedNode.value.id, subs)
    ElMessage.success('订阅保存成功')
  } catch (e) {
    ElMessage.error('保存订阅失败: ' + e.message)
  }
}

function onOperatorTypeChange(opName) {
  // Initialize operator config with defaults
  const defaults = {
    filter: { condition: '', pass: true },
    transform: { operations: '[]' },
    aggregate: { window_type: 'tumble', window_size: 60, aggregations: '[]' },
    router: { default_port: 'default', routes: '[]' },
    buffer: { size_limit: 1000, time_limit: 5000, max_items: 100 },
    alarm: { rules: '[]' },
    'json-path': { source_field: '', expressions: '[]' },
    deadband: { field: '', type: 'absolute', value: 0 },
    formula: { expression: '' },
    clamp: { field: '', min: 0, max: 100 },
    round: { field: '', decimals: 2 },
    change: { field: '' },
    range: { field: '', in_min: 0, in_max: 100, out_min: 0, out_max: 100 },
    batch: { size: 10, timeout_ms: 1000 },
    split: { field: '', delimiter: ',' },
    join: { field: '', delimiter: ',' },
    dedup: { field: '' },
    script: { script: '' },
    throttle: { max_rate: 100, window_ms: 1000 },
    convert: { field: '', to_type: 'int' },
    log: { level: 'info', template: '' },
  }
  selectedNode.value.data.operatorConfig = defaults[opName] || {}
}

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
    : operatorList.value.find(o => o.name === name)?.name_zh || name

  const position = project({ x: event.clientX - 250, y: event.clientY - 60 })

  const nodeData = {
    name: label,
    kind,
    pluginName: name,
    operatorName: kind === 'operator' ? name : undefined,
    config: {},
    operatorConfig: kind === 'operator' ? getOperatorDefaultConfig(name) : {},
    subscriptions: []
  }

  nodes.value.push({
    id,
    type: kind,
    label,
    position,
    data: nodeData
  })
}

function getOperatorDefaultConfig(opName) {
  const defaults = {
    filter: { condition: '', pass: true },
    transform: { operations: '[]' },
    aggregate: { window_type: 'tumble', window_size: 60, aggregations: '[]' },
    router: { default_port: 'default', routes: '[]' },
    buffer: { size_limit: 1000, time_limit: 5000, max_items: 100 },
    alarm: { rules: '[]' },
    'json-path': { source_field: '', expressions: '[]' },
    deadband: { field: '', type: 'absolute', value: 0 },
    formula: { expression: '' },
    clamp: { field: '', min: 0, max: 100 },
    round: { field: '', decimals: 2 },
    change: { field: '' },
    range: { field: '', in_min: 0, in_max: 100, out_min: 0, out_max: 100 },
    batch: { size: 10, timeout_ms: 1000 },
    split: { field: '', delimiter: ',' },
    join: { field: '', delimiter: ',' },
    dedup: { field: '' },
    script: { script: '' },
    throttle: { max_rate: 100, window_ms: 1000 },
    convert: { field: '', to_type: 'int' },
    log: { level: 'info', template: '' },
  }
  return defaults[opName] || {}
}

function onNodeClick({ node }) {
  selectedNode.value = node
  // Open preview panel
  previewOpen.value = true
  previewActiveNames.value = ['preview']
}

function selectCanvasNode(node) {
  // Find the vue-flow node and select it
  const vueFlowNode = nodes.value.find(n => n.id === node.id)
  if (vueFlowNode) {
    selectedNode.value = vueFlowNode
    previewOpen.value = true
    previewActiveNames.value = ['preview']
  }
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
    const savedFlowId = flowId.value || crypto.randomUUID()
    const now = new Date().toISOString()
    const flowNodes = nodes.value.map(n => ({
      id: n.id,
      name: n.data.name,
      kind: n.data.kind,
      operator_name: n.data.operatorName,
      config: n.data.kind === 'operator'
        ? operatorRuntimeConfig(n)
        : (n.data.config || {}),
      input_ports: buildInputPorts(n),
      output_ports: buildOutputPorts(n),
      position: nodePosition(n),
    }))
    const flowEdges = edges.value.map(e => ({
      source_node_id: e.source,
      source_port: e.sourceHandle || 'out',
      target_node_id: e.target,
      target_port: e.targetHandle || 'in',
    }))
    const body = {
      id: savedFlowId,
      name: flowName.value || '未命名数据流',
      description: flowDescription.value,
      status: flowStatus.value || 'draft',
      nodes: flowNodes,
      edges: flowEdges,
      bindings: currentFlowBindings.value || [],
      version: 1,
      created_at: flowCreatedAt.value || now,
      updated_at: now,
    }
    if (flowId.value) {
      await api.updateFlow(flowId.value, body)
      ElMessage.success('保存成功')
    } else {
      const created = await api.createFlow(body)
      const createdFlowId = created?.flow?.id || savedFlowId
      ElMessage.success('保存成功')
      await router.replace(`/flows/${createdFlowId}`)
    }
  } catch (e) {
    ElMessage.error('保存失败: ' + e.message)
  }
}

async function handleValidate() {
  const hasSouth = nodes.value.some(n => n.type === 'south')
  const hasNorth = nodes.value.some(n => n.type === 'north')
  if (!hasSouth) { ElMessage.warning('请添加南向设备节点'); return }
  if (!hasNorth) { ElMessage.warning('请添加北向应用节点'); return }
  ElMessage.success('验证通过')
}

function startStatusPoll() {
  stopStatusPoll()
  statusPollCount = 0
  statusPollTimer = setInterval(async () => {
    if (statusPollCount >= 5) { // 5 * 3s = 15s
      stopStatusPoll()
      return
    }
    statusPollCount++
    try {
      const data = await api.flow(flowId.value)
      flowStatus.value = data.flow.status
    } catch (e) { console.error(e) }
  }, 3000)
}

function stopStatusPoll() {
  if (statusPollTimer) {
    clearInterval(statusPollTimer)
    statusPollTimer = null
  }
}

async function handleDeploy() {
  if (!flowId.value) { ElMessage.warning('请先保存数据流'); return }
  try {
    await api.deployFlow(flowId.value)
    ElMessage.success('部署成功')
    startStatusPoll()
  } catch (e) { ElMessage.error('部署失败: ' + e.message) }
}

async function handleStart() {
  try {
    await api.startFlow(flowId.value)
    ElMessage.success('启动成功')
    startStatusPoll()
  } catch (e) { ElMessage.error('启动失败: ' + e.message) }
}

async function handlePause() {
  try {
    await api.pauseFlow(flowId.value)
    ElMessage.success('暂停成功')
    startStatusPoll()
  } catch (e) { ElMessage.error('暂停失败: ' + e.message) }
}

async function handleStop() {
  try {
    await api.stopFlow(flowId.value)
    ElMessage.success('停止成功')
    startStatusPoll()
  } catch (e) { ElMessage.error('停止失败: ' + e.message) }
}

// Data preview refresh
let previewTimer = null

watch([previewOpen, selectedNode], ([isOpen, node]) => {
  if (isOpen && node) {
    startPreviewRefresh()
  } else {
    stopPreviewRefresh()
  }
})

function startPreviewRefresh() {
  stopPreviewRefresh()
  refreshPreview()
  previewTimer = setInterval(refreshPreview, 2000)
}

function stopPreviewRefresh() {
  if (previewTimer) {
    clearInterval(previewTimer)
    previewTimer = null
  }
}

async function refreshPreview() {
  if (!selectedNode.value) return

  const sampleInput = buildPreviewSampleInput()
  previewInput.value = JSON.stringify(sampleInput, null, 2)

  if (!flowId.value) {
    const message = { message: '请先保存数据流后再预览' }
    previewOutput.value = JSON.stringify(message, null, 2)
    addRecentPreviewMessage('out', message.message)
    return
  }

  try {
    const result = await api.previewFlow(flowId.value, sampleInput)
    previewOutput.value = JSON.stringify(result, null, 2)
    addRecentPreviewMessage('out', previewResultSummary(result))
  } catch (e) {
    const errorPayload = previewErrorPayload(e)
    previewOutput.value = JSON.stringify(errorPayload, null, 2)
    addRecentPreviewMessage('out', `preview error: ${errorPayload.error}`)
  }
}
</script>

<style scoped>
.flow-editor { display: flex; flex-direction: column; height: 100vh; }
.editor-toolbar { display: flex; align-items: center; gap: 8px; padding: 8px 16px; border-bottom: 1px solid #ddd; background: #fff; }
.editor-body { display: flex; flex: 1; overflow: hidden; position: relative; }
.node-palette { width: 180px; padding: 12px; background: #f5f5f5; border-right: 1px solid #ddd; overflow-y: auto; }
.node-palette h4 { margin: 0 0 4px; font-size: 12px; color: #666; }
.palette-item { padding: 6px 10px; margin-bottom: 4px; border-radius: 4px; font-size: 12px; cursor: grab; }
.palette-item.south { background: #e3f2fd; border: 1px solid #90caf9; }
.palette-item.operator { background: #fff3e0; border: 1px solid #ffcc80; }
.palette-item.north { background: #e8f5e9; border: 1px solid #a5d6a7; }

/* Canvas node tree */
.canvas-node-item {
  display: flex;
  align-items: center;
  gap: 6px;
  padding: 5px 8px;
  margin-bottom: 3px;
  border-radius: 4px;
  cursor: pointer;
  font-size: 12px;
  transition: background 0.15s;
  border: 1px solid transparent;
}
.canvas-node-item:hover {
  background: #e8e8e8;
}
.canvas-node-item.canvas-node-selected {
  background: #e6f7ff;
  border-color: #91d5ff;
}
.node-type-icon {
  font-size: 12px;
  flex-shrink: 0;
}
.node-tree-label {
  flex: 1;
  overflow: hidden;
  text-overflow: ellipsis;
  white-space: nowrap;
}
.node-status-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  flex-shrink: 0;
  cursor: help;
}
.canvas-nodes-empty {
  font-size: 11px;
  color: #999;
  text-align: center;
  padding: 8px 0;
}
.canvas-nodes-legend {
  display: flex;
  gap: 8px;
  margin-top: 6px;
  padding-top: 6px;
  border-top: 1px solid #e0e0e0;
}
.legend-item {
  display: flex;
  align-items: center;
  gap: 3px;
  font-size: 10px;
  color: #888;
}
.legend-dot {
  width: 6px;
  height: 6px;
  border-radius: 50%;
}
.flow-canvas { flex: 1; position: relative; }
.properties-panel { width: 280px; padding: 12px; background: #fff; border-left: 1px solid #ddd; overflow-y: auto; }
.properties-panel h4 { margin: 0 0 8px; }
.preview-panel { position: absolute; bottom: 0; left: 180px; right: 280px; background: #fff; border-top: 1px solid #ddd; z-index: 10; }
.preview-panel :deep(.el-collapse) { border: none; }
.preview-panel :deep(.el-collapse-item__header) { padding: 0 12px; font-size: 12px; color: #666; }
.preview-panel :deep(.el-collapse-item__content) { padding: 8px 12px; }
.data-preview { font-size: 11px; background: #f5f5f5; padding: 6px; border-radius: 4px; overflow: auto; max-height: 100px; margin: 4px 0; }
.recent-msg { font-size: 11px; padding: 2px 0; display: flex; gap: 6px; }
.msg-time { color: #999; }
.msg-in { color: #409eff; }
.msg-out { color: #67c23a; }
.msg-data { color: #333; }
</style>
