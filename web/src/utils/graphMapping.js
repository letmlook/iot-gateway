//! VueFlow ↔ 领域模型(FlowGraph)的单一映射模块。
//!
//! redesign/v1 踩坑 P3:图保存映射不完整/丢字段,连修 4 次。v2 对策:所有转换只在本模块,
//! 并配往返测试(tests/graphMapping.test.js,`npm run test:unit`)。本文件必须保持纯 JS
//! (不 import Vue/VueFlow),使 node --test 可直接运行。

/**
 * 编辑器内置的结构性端口(Source 节点不是算子,不在 /flows/operators 列表中)。
 */
export const SOURCE_PORTS = { inputs: [], outputs: ['out'] }
export const DEFAULT_SINK_PORTS = { inputs: ['in'], outputs: [] }

/**
 * 由 /flows/operators 返回构建「算子名 → 端口规范」映射。
 * @param {Array<{name:string, inputs:string[], outputs:string[]}>} operators
 * @returns {Record<string, {inputs:string[], outputs:string[]}>}
 */
export function buildPortsMap(operators) {
  const map = {}
  for (const op of operators || []) {
    map[op.name] = { inputs: op.inputs || [], outputs: op.outputs || [] }
  }
  return map
}

/**
 * 节点渲染所需的端口规范。
 * @param {{kind:string, operator?:string|null}} node
 * @param {Record<string, {inputs:string[], outputs:string[]}>} portsMap
 */
export function portsFor(node, portsMap) {
  if (node.kind === 'source') return SOURCE_PORTS
  if (node.kind === 'sink') {
    const op = node.operator ? portsMap[node.operator] : null
    return op || DEFAULT_SINK_PORTS
  }
  const op = node.operator ? portsMap[node.operator] : null
  return op || { inputs: ['in'], outputs: [] }
}

/**
 * FlowGraph → VueFlow 元素。节点 id 与流程节点 id 一一对应(字符串 UUID),
 * 边的 sourceHandle/sourcePort 显式携带端口名。
 * @param {{nodes:Array, edges:Array}} graph
 * @param {Record<string, {inputs:string[], outputs:string[]}>} portsMap
 */
export function graphToVueFlow(graph, portsMap) {
  const vfNodes = (graph.nodes || []).map((n) => {
    const ports = portsFor(n, portsMap)
    return {
      id: String(n.id),
      type: 'flow-node',
      position: { x: n.position?.x ?? 0, y: n.position?.y ?? 0 },
      data: {
        label: n.name,
        kind: n.kind,
        operator: n.operator ?? null,
        config: structuredClone(n.config ?? {}),
        inputs: ports.inputs,
        outputs: ports.outputs,
      },
    }
  })
  const vfEdges = (graph.edges || []).map((e) => ({
    id: edgeId(e.source_node_id, e.source_port, e.target_node_id),
    source: String(e.source_node_id),
    sourceHandle: e.source_port,
    target: String(e.target_node_id),
    targetHandle: 'in',
  }))
  return { nodes: vfNodes, edges: vfEdges }
}

/**
 * VueFlow 元素 → FlowGraph。严格按 id 还原,字段丢失即测试失败(P3 对策)。
 * @param {Array} vfNodes VueFlow nodes(data 携带 kind/operator/config)
 * @param {Array} vfEdges VueFlow edges(source/sourceHandle/target)
 */
export function vueFlowToGraph(vfNodes, vfEdges) {
  const nodes = (vfNodes || []).map((n) => ({
    id: n.id,
    name: n.data?.label ?? '',
    kind: n.data?.kind,
    ...(n.data?.operator ? { operator: n.data.operator } : {}),
    config: structuredClone(n.data?.config ?? {}),
    position: { x: n.position?.x ?? 0, y: n.position?.y ?? 0 },
  }))
  const edges = (vfEdges || []).map((e) => ({
    source_node_id: e.source,
    source_port: e.sourceHandle ?? 'out',
    target_node_id: e.target,
  }))
  return { nodes, edges }
}

/**
 * 确定性边 id(同一条逻辑边无论映射多少次 id 不变——VueFlow 的边选择/删除依赖它)。
 */
export function edgeId(sourceId, sourcePort, targetId) {
  return `e-${sourceId}-${sourcePort}-${targetId}`
}

/**
 * 按算子返回新建节点的默认配置(编辑器面板的初值;后端 publish 校验兜底)。
 * @param {string|null} operator
 */
export function defaultConfig(operator) {
  switch (operator) {
    case 'threshold_alarm':
      return { tag_name: '', high: null, low: null, level: 'high', message: '' }
    case 'state_change_alarm':
      return { tag_name: '', from: '*', to: '', level: 'medium', message: '' }
    case 'route':
      return { tag_name: '', op: 'gt', value: 0 }
    case 'sink_north':
      return { node_id: '' }
    default:
      return {}
  }
}
