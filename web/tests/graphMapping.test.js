//! graphMapping 往返测试(redesign 踩坑 P3 的对策锁)。
//! 运行:npm run test:unit(node --test,无新增依赖)。

import { test } from 'node:test'
import assert from 'node:assert/strict'

import {
  buildPortsMap,
  graphToVueFlow,
  vueFlowToGraph,
  edgeId,
  portsFor,
  defaultConfig,
  SOURCE_PORTS,
} from '../src/utils/graphMapping.js'

// /flows/operators 的真实返回形状(5 个 MVP 算子)
const OPERATORS = [
  { name: 'threshold_alarm', inputs: ['in'], outputs: ['alarm', 'normal'] },
  { name: 'state_change_alarm', inputs: ['in'], outputs: ['alarm', 'normal'] },
  { name: 'route', inputs: ['in'], outputs: ['true', 'false'] },
  { name: 'sink_north', inputs: ['in'], outputs: [] },
  { name: 'sink_alarm', inputs: ['in'], outputs: [] },
]

function sampleGraph() {
  const mk = () => crypto.randomUUID()
  const src = mk()
  const th = mk()
  const sa = mk()
  const sn = mk()
  return {
    nodes: [
      {
        id: src,
        name: '数据源',
        kind: 'source',
        config: { south_node_id: mk(), group_id: mk() },
        position: { x: 10, y: 20 },
      },
      {
        id: th,
        name: '超温告警',
        kind: 'operator',
        operator: 'threshold_alarm',
        config: { tag_name: 'temperature', high: 80, low: null, level: 'high', message: '' },
        position: { x: 110, y: 20 },
      },
      {
        id: sa,
        name: '告警输出',
        kind: 'sink',
        operator: 'sink_alarm',
        config: {},
        position: { x: 210, y: -20 },
      },
      {
        id: sn,
        name: '北向输出',
        kind: 'sink',
        operator: 'sink_north',
        config: { node_id: mk() },
        position: { x: 210, y: 60 },
      },
    ],
    edges: [
      { source_node_id: src, source_port: 'out', target_node_id: th },
      { source_node_id: th, source_port: 'alarm', target_node_id: sa },
      { source_node_id: th, source_port: 'normal', target_node_id: sn },
    ],
  }
}

test('往返:graph → VueFlow → graph 全字段还原(P3 锁)', () => {
  const graph = sampleGraph()
  const portsMap = buildPortsMap(OPERATORS)
  const vf = graphToVueFlow(graph, portsMap)

  assert.equal(vf.nodes.length, 4)
  assert.equal(vf.edges.length, 3)

  const back = vueFlowToGraph(vf.nodes, vf.edges)
  assert.deepEqual(back, graph, '往返必须逐字段还原(含 config/position/ports)')
})

test('往返幂等:映射两次结果逐字节一致', () => {
  const graph = sampleGraph()
  const portsMap = buildPortsMap(OPERATORS)
  const once = vueFlowToGraph(
    graphToVueFlow(graph, portsMap).nodes,
    graphToVueFlow(graph, portsMap).edges,
  )
  const twice = vueFlowToGraph(
    graphToVueFlow(once, portsMap).nodes,
    graphToVueFlow(once, portsMap).edges,
  )
  assert.deepEqual(twice, once)
  assert.deepEqual(JSON.stringify(twice), JSON.stringify(once))
})

test('边的端口语义保留:alarm 端口边映射后 sourceHandle=alarm', () => {
  const graph = sampleGraph()
  const portsMap = buildPortsMap(OPERATORS)
  const vf = graphToVueFlow(graph, portsMap)
  const alarmEdge = vf.edges.find((e) => e.sourceHandle === 'alarm')
  assert.ok(alarmEdge, 'alarm 端口边必须存在')
  const target = graph.nodes.find((n) => n.id === alarmEdge.target)
  assert.equal(target.operator, 'sink_alarm')
})

test('确定性边 id:同一条逻辑边 id 稳定(VueFlow 选择/删除依赖)', () => {
  const a = crypto.randomUUID()
  const b = crypto.randomUUID()
  assert.equal(edgeId(a, 'alarm', b), edgeId(a, 'alarm', b))
  assert.notEqual(edgeId(a, 'alarm', b), edgeId(a, 'normal', b))
})

test('端口解析:source 用内置端口,sink/算子用 API 映射,未知算子有兜底', () => {
  const portsMap = buildPortsMap(OPERATORS)
  assert.deepEqual(portsFor({ kind: 'source' }, portsMap), SOURCE_PORTS)
  assert.deepEqual(portsFor({ kind: 'operator', operator: 'route' }, portsMap).outputs, [
    'true',
    'false',
  ])
  assert.deepEqual(portsFor({ kind: 'sink', operator: 'sink_alarm' }, portsMap).inputs, ['in'])
  // 未知算子兜底(不崩编辑器)
  assert.deepEqual(portsFor({ kind: 'operator', operator: 'nope' }, portsMap), {
    inputs: ['in'],
    outputs: [],
  })
})

test('缺 position/config 的节点映射不抛错(草稿图允许不完整,P2 语义)', () => {
  const partial = {
    nodes: [{ id: crypto.randomUUID(), name: 'x', kind: 'source', config: {} }],
    edges: [],
  }
  const vf = graphToVueFlow(partial, buildPortsMap(OPERATORS))
  assert.equal(vf.nodes[0].position.x, 0)
  const back = vueFlowToGraph(vf.nodes, vf.edges)
  assert.equal(back.nodes[0].name, 'x')
})

test('defaultConfig 覆盖全部需要表单的算子', () => {
  for (const op of ['threshold_alarm', 'state_change_alarm', 'route', 'sink_north']) {
    const cfg = defaultConfig(op)
    assert.equal(typeof cfg, 'object')
  }
  assert.deepEqual(defaultConfig('sink_alarm'), {})
  assert.deepEqual(defaultConfig(null), {})
})
