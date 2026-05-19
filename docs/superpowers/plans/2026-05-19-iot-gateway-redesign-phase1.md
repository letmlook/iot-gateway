# IoT 网关破坏性重构 — Phase 1 实施计划

> **For Hermes:** Use subagent-driven-development skill to implement this plan task-by-task.
>
> **分支**: `redesign/v1-flow-orchestration`
>
> **目标**: 完成 Flow Orchestrator 核心框架 + 5个基础算子插件 + VueFlow 前端集成
>
> **架构**: South（采集）→ Operator（处理）→ North（传输），三层插件化，Flow 定义通过 DAG 拓扑执行
>
> **Tech Stack**: Rust (tokio + async-trait + rhai) / Vue3 + VueFlow + TypeScript

---

## 前置依赖

- [ ] Node.js v22（已完成）
- [ ] lark-cli 认证（已完成）
- [ ] 设计文档（已完成）

---

## 任务总览

| 阶段 | 任务数 | 范围 |
|------|--------|------|
| Stage 1 | 5 | gateway-sdk 扩展：OperatorPlugin trait + PipelineData 类型 |
| Stage 2 | 6 | gateway-flow crate：Flow 定义、验证、编排器核心 |
| Stage 3 | 4 | 5个内置算子：filter、transform、aggregate、router、buffer |
| Stage 4 | 5 | gateway-server：Flow CRUD API |
| Stage 5 | 4 | 前端：VueFlow 集成 + Flow Editor 页面 |
| **合计** | **24** | |

---

## Stage 1：gateway-sdk 扩展

### Task 1: 新增 PipelineData 类型

**Objective:** 在 gateway-sdk 中新增流数据核心类型 PipelineData

**Files:**
- Create: `gateway/gateway-sdk/src/pipeline.rs`
- Modify: `gateway/gateway-sdk/src/lib.rs` — 导出 pipeline 模块

**Step 1: 创建 pipeline.rs**

```rust
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// 数据包格式——流经整个 Pipeline 的最小单元
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineData {
    /// Flow 实例 ID
    pub flow_id: Uuid,
    /// 当前节点 ID
    pub node_id: Uuid,
    /// 数据来源节点 ID（用于链路追踪）
    pub source_node_id: Uuid,
    /// 标签数据：TagId → 值
    pub tags: HashMap<String, DataValue>,
    /// 时间戳（毫秒）
    pub timestamp: i64,
    /// 传递的上下文元数据（如来源设备、位置等）
    pub metadata: HashMap<String, String>,
}

/// 标签值（参考现有 DataValue 扩展）
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum DataValue {
    Bool(bool),
    Int(i64),
    Float(f64),
    String(String),
    Bytes(Vec<u8>),
    Null,
}

impl PipelineData {
    pub fn new(flow_id: Uuid, node_id: Uuid, source_node_id: Uuid) -> Self {
        Self {
            flow_id,
            node_id,
            source_node_id,
            tags: Default::default(),
            timestamp: chrono::Utc::now().timestamp_millis(),
            metadata: Default::default(),
        }
    }
}
```

**Step 2: 更新 lib.rs**

在 `gateway/gateway-sdk/src/lib.rs` 末尾添加：

```rust
pub mod pipeline;
pub use pipeline::{PipelineData, DataValue};
```

**Step 3: 验证编译**

```bash
cd /home/letmlook/projects/iot-gateway/gateway/gateway-sdk
cargo build
```

预期：PASS（无输出错误）

**Step 4: 提交**

```bash
git add gateway/gateway-sdk/src/pipeline.rs gateway/gateway-sdk/src/lib.rs
git commit -m "feat(sdk): add PipelineData and DataValue types"
```

---

### Task 2: 新增 OperatorPlugin trait

**Objective:** 新增 OperatorPlugin trait，定义算子插件标准接口

**Files:**
- Modify: `gateway/gateway-sdk/src/plugin.rs` — 追加 OperatorPlugin trait

**Step 1: 在 plugin.rs 末尾追加**

```rust
/// 算子插件：处理 PipelineData，返回变换后的数据
#[async_trait]
pub trait OperatorPlugin: Send + Sync {
    fn meta(&self) -> PluginMeta;

    /// 配置 Schema（可选）
    fn config_schema(&self) -> Option<ConfigSchema> {
        None
    }

    /// 初始化（插件实例级别，非 per-flow）
    async fn init(&self, config: PluginConfig) -> PluginResult<()> {
        let _ = config;
        Ok(())
    }

    /// 反初始化
    async fn uninit(&self) -> PluginResult<()> {
        Ok(())
    }

    /// 处理单条数据，返回变换后的数据
    async fn process(&self, data: PipelineData) -> PluginResult<PipelineData>;

    /// 批处理（默认逐条调用 process）
    async fn process_batch(&self, batch: Vec<PipelineData>) -> PluginResult<Vec<PipelineData>> {
        let mut results = Vec::with_capacity(batch.len());
        for d in batch {
            results.push(self.process(d).await?);
        }
        Ok(results)
    }
}
```

**Step 2: 验证编译**

```bash
cd /home/letmlook/projects/iot-gateway/gateway/gateway-sdk
cargo build
```

预期：PASS

**Step 3: 提交**

```bash
git add gateway/gateway-sdk/src/plugin.rs
git commit -m "feat(sdk): add OperatorPlugin trait"
```

---

### Task 3: 新增 PluginKind::Operator

**Objective:** 将 PluginKind 枚举扩展，添加 Operator 变体

**Files:**
- Modify: `gateway/gateway-sdk/src/types.rs`

**Step 1: 读取 types.rs 找到 PluginKind**

```bash
grep -n "PluginKind" /home/letmlook/projects/iot-gateway/gateway/gateway-sdk/src/types.rs | head -20
```

**Step 2: 在 PluginKind 枚举中添加 Operator 变体**

```rust
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginKind {
    South,
    North,
    Operator,  // 🆕 新增
}
```

**Step 3: 验证编译**

```bash
cd /home/letmlook/projects/iot-gateway/gateway/gateway-sdk
cargo build
```

预期：PASS

**Step 4: 提交**

```bash
git add gateway/gateway-sdk/src/types.rs
git commit -m "feat(sdk): add Operator variant to PluginKind enum"
```

---

### Task 4: 新增 PluginConfig 类型别名

**Objective:** 将 `PluginConfig` 显式定义为 `serde_json::Value`

**Files:**
- Modify: `gateway/gateway-sdk/src/types.rs`

**Step 1: 确认 types.rs 中已有 PluginConfig 定义**

```bash
grep -n "PluginConfig" /home/letmlook/projects/iot-gateway/gateway/gateway-sdk/src/types.rs
```

如果已存在则跳过。如果不存在，添加：

```rust
/// 插件配置（JSON）
pub type PluginConfig = serde_json::Value;
```

**Step 2: 验证编译**

```bash
cd /home/letmlook/projects/iot-gateway/gateway/gateway-sdk
cargo build
```

预期：PASS

**Step 3: 提交**

```bash
git add gateway/gateway-sdk/src/types.rs
git commit -m "feat(sdk): add PluginConfig type alias"
```

---

### Task 5: 更新 PluginMeta 支持 Operator

**Objective:** 确保 PluginMeta 的 kind 字段支持 Operator

**Files:**
- 确认 `gateway/gateway-sdk/src/plugin.rs` 中 PluginMeta 使用的 kind 类型

**Step 1: 验证**

```bash
grep -n "PluginKind" /home/letmlook/projects/iot-gateway/gateway/gateway-sdk/src/plugin.rs
```

**Step 2: 确认 PluginKind 来自 crate::types**

```bash
grep -n "use crate::types" /home/letmlook/projects/iot-gateway/gateway/gateway-sdk/src/plugin.rs
```

已导入 PluginKind，且 types.rs 已添加 Operator，此步应自动通过。

**Step 3: 验证编译**

```bash
cd /home/letmlook/projects/iot-gateway/gateway/gateway-sdk
cargo build && cargo test
```

预期：PASS

**Step 4: 提交**

```bash
git add gateway/gateway-sdk/src/plugin.rs gateway/gateway-sdk/src/types.rs
git commit -m "feat(sdk): full OperatorPlugin support in PluginMeta"
```

---

## Stage 2：gateway-flow crate

### Task 6: 创建 gateway-flow workspace 条目

**Objective:** 将 gateway-flow 添加到 Cargo workspace

**Files:**
- Modify: `Cargo.toml`（项目根目录）

**Step 1: 添加 workspace member**

```toml
[workspace]
resolver = "2"
members = [
    "gateway/gateway-sdk",
    "gateway/gateway-core",
    "gateway/gateway-server",
    "gateway/gateway-flow",   # 🆕 新增
    "gateway/gateway-plugins/plugin-sim",
    "gateway/gateway-plugins/plugin-mqtt",
    "gateway/gateway-plugins/plugin-modbus-tcp",
    "gateway/gateway-plugins/plugin-modbus-rtu",
    "gateway/gateway-plugins/plugin-opcua",
    "gateway/gateway-plugins/plugin-virb",
]
```

**Step 2: 创建目录结构**

```bash
mkdir -p gateway/gateway-flow/src
```

**Step 3: 创建 Cargo.toml**

```toml
[package]
name = "gateway-flow"
version = "0.1.0"
edition = "2021"
description = "Flow Orchestrator — DAG 编排、数据流执行引擎"

[dependencies]
gateway-sdk = { path = "../gateway-sdk" }
tokio = { version = "1", features = ["sync", "time", "rt"] }
uuid = { version = "1", features = ["v4", "serde"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "1"
tracing = "0.1"
chrono = { version = "0.4", features = ["serde"] }
async-trait = "0.1"
rhai = "1"
parking_lot = "0.12"

[dev-dependencies]
tokio-test = "0.4"
```

**Step 4: 创建空的 lib.rs**

```rust
//! gateway-flow: Flow Orchestrator

pub mod flow;
pub mod executor;
pub mod registry;
```

**Step 5: 验证编译**

```bash
cargo build -p gateway-flow
```

预期：PASS（编译成功，无 warning）

**Step 6: 提交**

```bash
git add Cargo.toml gateway/gateway-flow/
git commit -m "feat(flow): create gateway-flow crate scaffold"
```

---

### Task 7: 实现 Flow 定义与验证（flow.rs）

**Objective:** 定义 Flow 结构，做 DAG 验证（循环检测）

**Files:**
- Create: `gateway/gateway-flow/src/flow.rs`
- Modify: `gateway/gateway-flow/src/lib.rs`

**Step 1: 创建 flow.rs**

```rust
use gateway_sdk::types::{NodeId, PluginConfig};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum FlowError {
    #[error("循环依赖 detected: {0}")]
    Cycle(String),
    #[error("节点不存在: {0}")]
    NodeNotFound(NodeId),
    #[error("无效的 Flow 定义: {0}")]
    InvalidDefinition(String),
}

/// Flow 定义
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Flow {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub nodes: Vec<FlowNode>,
    pub edges: Vec<FlowEdge>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowNode {
    pub id: NodeId,
    pub name: String,
    pub plugin: String,       // 插件名，如 "filter", "modbus-tcp"
    pub node_type: NodeType,  // south / operator / north
    #[serde(default)]
    pub config: PluginConfig,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum NodeType {
    South,
    Operator,
    North,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlowEdge {
    pub from: NodeId,
    pub to: NodeId,
}

impl Flow {
    /// 验证 Flow 定义：检测循环依赖、孤立节点
    pub fn validate(&self) -> Result<(), FlowError> {
        // 构建邻接表
        let mut adj: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        let node_ids: HashSet<NodeId> = self.nodes.iter().map(|n| n.id).collect();

        for edge in &self.edges {
            if !node_ids.contains(&edge.from) {
                return Err(FlowError::NodeNotFound(edge.from));
            }
            if !node_ids.contains(&edge.to) {
                return Err(FlowError::NodeNotFound(edge.to));
            }
            adj.entry(edge.from).or_default().push(edge.to);
        }

        // DFS 检测循环
        let mut visited = HashSet::new();
        let mut rec_stack = HashSet::new();

        fn dfs(
            node: NodeId,
            adj: &HashMap<NodeId, Vec<NodeId>>,
            visited: &mut HashSet<NodeId>,
            rec_stack: &mut HashSet<NodeId>,
        ) -> Result<(), FlowError> {
            visited.insert(node);
            rec_stack.insert(node);
            if let Some(neighbors) = adj.get(&node) {
                for &neighbor in neighbors {
                    if !visited.contains(&neighbor) {
                        dfs(neighbor, adj, visited, rec_stack)?;
                    } else if rec_stack.contains(&neighbor) {
                        return Err(FlowError::Cycle(format!(
                            "{} -> {}", node, neighbor
                        )));
                    }
                }
            }
            rec_stack.remove(&node);
            Ok(())
        }

        for &node_id in &node_ids {
            if !visited.contains(&node_id) {
                dfs(node_id, &adj, &mut visited, &mut rec_stack)?;
            }
        }

        Ok(())
    }

    /// 拓扑排序，返回节点执行顺序
    pub fn topological_sort(&self) -> Result<Vec<NodeId>, FlowError> {
        self.validate()?;
        let mut adj: HashMap<NodeId, Vec<NodeId>> = HashMap::new();
        let mut in_degree: HashMap<NodeId, usize> = HashMap::new();

        for node in &self.nodes {
            in_degree.insert(node.id, 0);
        }

        for edge in &self.edges {
            adj.entry(edge.from).or_default().push(edge.to);
            *in_degree.entry(edge.to).or_insert(0) += 1;
        }

        let mut queue: Vec<NodeId> = in_degree
            .iter()
            .filter(|(_, &d)| d == 0)
            .map(|(&id, _)| id)
            .collect();

        let mut result = Vec::new();
        while let Some(node) = queue.pop() {
            result.push(node);
            if let Some(neighbors) = adj.get(&node) {
                for &n in neighbors {
                    if let Some(d) = in_degree.get_mut(&n) {
                        *d -= 1;
                        if *d == 0 {
                            queue.push(n);
                        }
                    }
                }
            }
        }

        if result.len() != self.nodes.len() {
            return Err(FlowError::Cycle("无法完成拓扑排序".into()));
        }

        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_cycle_detection() {
        let flow = Flow {
            id: Uuid::new_v4(),
            name: "test".into(),
            description: None,
            nodes: vec![
                FlowNode { id: 1.into(), name: "a".into(), plugin: "sim".into(), node_type: NodeType::South, config: Default::default() },
                FlowNode { id: 2.into(), name: "b".into(), plugin: "filter".into(), node_type: NodeType::Operator, config: Default::default() },
            ],
            edges: vec![
                FlowEdge { from: 1.into(), to: 2.into() },
                FlowEdge { from: 2.into(), to: 1.into() }, // cycle!
            ],
        };
        assert!(flow.validate().is_err());
    }

    #[test]
    fn test_topological_sort() {
        let flow = Flow {
            id: Uuid::new_v4(),
            name: "test".into(),
            description: None,
            nodes: vec![
                FlowNode { id: 1.into(), name: "south".into(), plugin: "sim".into(), node_type: NodeType::South, config: Default::default() },
                FlowNode { id: 2.into(), name: "filter".into(), plugin: "filter".into(), node_type: NodeType::Operator, config: Default::default() },
                FlowNode { id: 3.into(), name: "mqtt".into(), plugin: "mqtt".into(), node_type: NodeType::North, config: Default::default() },
            ],
            edges: vec![
                FlowEdge { from: 1.into(), to: 2.into() },
                FlowEdge { from: 2.into(), to: 3.into() },
            ],
        };
        let sorted = flow.topological_sort().unwrap();
        assert_eq!(sorted[0], 1.into());
        assert_eq!(sorted[2], 3.into());
    }
}
```

**Step 2: 更新 lib.rs**

```rust
pub mod flow;
pub mod executor;
pub mod registry;

pub use flow::{Flow, FlowNode, FlowEdge, NodeType, FlowError};
```

**Step 3: 验证编译 + 测试**

```bash
cargo build -p gateway-flow && cargo test -p gateway-flow
```

预期：PASS — `test_cycle_detection` 和 `test_topological_sort` 通过

**Step 4: 提交**

```bash
git add gateway/gateway-flow/src/flow.rs gateway/gateway-flow/src/lib.rs
git commit -m "feat(flow): add Flow definition, DAG validation and topological sort"
```

---

### Task 8: 实现插件注册表（registry.rs）

**Objective:** 管理三层插件（South/North/Operator）的注册与发现

**Files:**
- Create: `gateway/gateway-flow/src/registry.rs`
- Modify: `gateway/gateway-flow/src/lib.rs`

**Step 1: 创建 registry.rs**

```rust
use gateway_sdk::plugin::{OperatorPlugin, PluginMeta, SouthPlugin, NorthPlugin};
use gateway_sdk::types::PluginKind;
use parking_lot::RwLock;
use std::collections::HashMap;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum RegistryError {
    #[error("插件不存在: {0}")]
    NotFound(String),
    #[error("插件已存在: {0}")]
    AlreadyExists(String),
    #[error("插件类型不匹配")]
    KindMismatch,
}

pub type RegistryResult<T> = Result<T, RegistryError>;

/// 三层插件注册表
pub struct PluginRegistry {
    south: RwLock<HashMap<String, Box<dyn SouthPlugin>>>,
    north: RwLock<HashMap<String, Box<dyn NorthPlugin>>>,
    operator: RwLock<HashMap<String, Box<dyn OperatorPlugin>>>,
}

impl Default for PluginRegistry {
    fn default() -> Self {
        Self::new()
    }
}

impl PluginRegistry {
    pub fn new() -> Self {
        Self {
            south: RwLock::new(HashMap::new()),
            north: RwLock::new(HashMap::new()),
            operator: RwLock::new(HashMap::new()),
        }
    }

    // ---------- 注册 ----------

    pub fn register_south(&self, name: &str, plugin: Box<dyn SouthPlugin>) -> RegistryResult<()> {
        let mut guard = self.south.write();
        if guard.contains_key(name) {
            return Err(RegistryError::AlreadyExists(name.into()));
        }
        guard.insert(name.into(), plugin);
        Ok(())
    }

    pub fn register_north(&self, name: &str, plugin: Box<dyn NorthPlugin>) -> RegistryResult<()> {
        let mut guard = self.north.write();
        if guard.contains_key(name) {
            return Err(RegistryError::AlreadyExists(name.into()));
        }
        guard.insert(name.into(), plugin);
        Ok(())
    }

    pub fn register_operator(&self, name: &str, plugin: Box<dyn OperatorPlugin>) -> RegistryResult<()> {
        let mut guard = self.operator.write();
        if guard.contains_key(name) {
            return Err(RegistryError::AlreadyExists(name.into()));
        }
        guard.insert(name.into(), plugin);
        Ok(())
    }

    // ---------- 查询 ----------

    pub fn get_south(&self, name: &str) -> RegistryResult<Box<dyn SouthPlugin>> {
        self.south.read()
            .get(name)
            .map(|p| Box::new((**p).clone()) as Box<dyn SouthPlugin>)
            .ok_or_else(|| RegistryError::NotFound(name.into()))
    }

    pub fn get_north(&self, name: &str) -> RegistryResult<Box<dyn NorthPlugin>> {
        self.north.read()
            .get(name)
            .map(|p| Box::new((**p).clone()) as Box<dyn NorthPlugin>)
            .ok_or_else(|| RegistryError::NotFound(name.into()))
    }

    pub fn get_operator(&self, name: &str) -> RegistryResult<Box<dyn OperatorPlugin>> {
        self.operator.read()
            .get(name)
            .map(|p| Box::new((**p).clone()) as Box<dyn OperatorPlugin>)
            .ok_or_else(|| RegistryError::NotFound(name.into()))
    }

    /// 列出所有已注册插件的元信息
    pub fn list_plugins(&self) -> Vec<PluginMeta> {
        let south = self.south.read().values().map(|p| p.meta()).collect::<Vec<_>>();
        let north = self.north.read().values().map(|p| p.meta()).collect::<Vec<_>>();
        let operator = self.operator.read().values().map(|p| p.meta()).collect::<Vec<_>>();
        [south, north, operator].concat()
    }
}
```

**Step 2: 更新 lib.rs**

```rust
pub mod flow;
pub mod executor;
pub mod registry;

pub use flow::{Flow, FlowNode, FlowEdge, NodeType, FlowError};
pub use registry::{PluginRegistry, RegistryError};
```

**Step 3: 验证编译**

```bash
cargo build -p gateway-flow
```

预期：PASS

**Step 4: 提交**

```bash
git add gateway/gateway-flow/src/registry.rs gateway/gateway-flow/src/lib.rs
git commit -m "feat(flow): add three-layer plugin registry"
```

---

### Task 9: 实现 DAG 执行器骨架（executor.rs）

**Objective:** 实现 FlowExecutor，按拓扑顺序执行 DAG 数据流

**Files:**
- Create: `gateway/gateway-flow/src/executor.rs`
- Modify: `gateway/gateway-flow/src/lib.rs`

**Step 1: 创建 executor.rs**

```rust
use crate::flow::{Flow, FlowError, FlowNode, NodeType};
use crate::registry::{PluginRegistry, RegistryError};
use gateway_sdk::pipeline::PipelineData;
use gateway_sdk::types::NodeId;
use std::collections::HashMap;
use std::sync::Arc;
use thiserror::Error;
use tokio::sync::mpsc;
use tracing::{info, warn, error};

#[derive(Debug, Error)]
pub enum ExecutorError {
    #[error("Flow 验证失败: {0}")]
    InvalidFlow(#[from] FlowError),
    #[error("插件未找到: {0}")]
    PluginNotFound(String),
    #[error("执行错误: {0}")]
    Execution(String),
}

pub type ExecutorResult<T> = Result<T, ExecutorError>;

/// Flow 实例——对应一个运行中的 Flow
pub struct FlowExecutor {
    flow: Flow,
    registry: Arc<PluginRegistry>,
    /// 每个节点的输出 channel: node_id → sender
    node_channels: HashMap<NodeId, mpsc::Sender<PipelineData>>,
    /// 是否正在运行
    running: bool,
}

impl FlowExecutor {
    pub fn new(flow: Flow, registry: Arc<PluginRegistry>) -> ExecutorResult<Self> {
        // 验证 Flow
        flow.validate()?;

        Ok(Self {
            flow,
            registry,
            node_channels: HashMap::new(),
            running: false,
        })
    }

    /// 启动 Flow：创建 channel，启动每个节点的任务
    pub async fn start(&mut self) -> ExecutorResult<()> {
        if self.running {
            return Err(ExecutorError::Execution("Flow already running".into()));
        }

        let sorted = self.flow.topological_sort()?;
        info!("Starting flow '{}' with order: {:?}", self.flow.name, sorted);

        // 为每个节点创建 channel
        for node_id in &sorted {
            let (tx, rx) = mpsc::channel::<PipelineData>(100);
            self.node_channels.insert(*node_id, tx);

            let node = self.flow.nodes.iter().find(|n| n.id == *node_id).unwrap();
            let registry = Arc::clone(&self.registry);
            let flow_id = self.flow.id;

            // 为 South 节点启动轮询任务
            match node.node_type {
                NodeType::South => {
                    let channels = self.node_channels.clone();
                    tokio::spawn(async move {
                        Self::run_south_node(node, registry, *node_id, flow_id, channels, rx).await;
                    });
                }
                NodeType::Operator => {
                    let channels = self.node_channels.clone();
                    tokio::spawn(async move {
                        Self::run_operator_node(node, registry, *node_id, flow_id, channels, rx).await;
                    });
                }
                NodeType::North => {
                    let channels = self.node_channels.clone();
                    tokio::spawn(async move {
                        Self::run_north_node(node, registry, *node_id, flow_id, channels, rx).await;
                    });
                }
            }
        }

        self.running = true;
        Ok(())
    }

    /// 停止 Flow
    pub async fn stop(&mut self) {
        self.running = false;
        // 关闭所有 channel
        for (_, tx) in self.node_channels.drain() {
            let _ = tx.send(PipelineData::new(self.flow.id, uuid::Uuid::new_v4(), uuid::Uuid::new_v4())).await;
        }
        info!("Flow '{}' stopped", self.flow.name);
    }

    /// South 节点执行循环
    async fn run_south_node(
        node: &FlowNode,
        registry: Arc<PluginRegistry>,
        node_id: NodeId,
        flow_id: uuid::Uuid,
        channels: HashMap<NodeId, mpsc::Sender<PipelineData>>,
        mut rx: mpsc::Receiver<PipelineData>,
    ) {
        loop {
            tokio::select! {
                _ = rx.recv() => {
                    // 收到停止信号
                    break;
                }
                _ = tokio::time::sleep(tokio::time::Duration::from_millis(100)) => {
                    // TODO: 调用 SouthPlugin.poll_group 获取数据
                    // 模拟：生成一条空数据用于测试
                    let data = PipelineData::new(flow_id, node_id, node_id);
                    // 发送给所有下游
                    Self::fan_out(&channels, &node.id, data).await;
                }
            }
        }
    }

    /// Operator 节点执行循环
    async fn run_operator_node(
        node: &FlowNode,
        registry: Arc<PluginRegistry>,
        node_id: NodeId,
        flow_id: uuid::Uuid,
        channels: HashMap<NodeId, mpsc::Sender<PipelineData>>,
        mut rx: mpsc::Receiver<PipelineData>,
    ) {
        let plugin = match registry.get_operator(&node.plugin) {
            Ok(p) => p,
            Err(e) => {
                error!("Failed to get operator plugin '{}': {}", node.plugin, e);
                return;
            }
        };

        // 初始化插件
        if let Err(e) = plugin.init(node.config.clone()).await {
            error!("Operator '{}' init failed: {}", node.plugin, e);
            return;
        }

        loop {
            tokio::select! {
                _ = rx.recv() => {
                    let _ = plugin.uninit().await;
                    break;
                }
                Some(data) = rx.recv() => {
                    match plugin.process(data).await {
                        Ok(processed) => {
                            Self::fan_out(&channels, &node.id, processed).await;
                        }
                        Err(e) => {
                            warn!("Operator '{}' process error: {}", node.plugin, e);
                        }
                    }
                }
            }
        }
    }

    /// North 节点执行循环
    async fn run_north_node(
        node: &FlowNode,
        registry: Arc<PluginRegistry>,
        node_id: NodeId,
        flow_id: uuid::Uuid,
        channels: HashMap<NodeId, mpsc::Sender<PipelineData>>,
        mut rx: mpsc::Receiver<PipelineData>,
    ) {
        loop {
            tokio::select! {
                _ = rx.recv() => {
                    break;
                }
                Some(data) = rx.recv() => {
                    // TODO: 调用 NorthPlugin.on_pipeline_data
                    info!("North node '{}' received data with {} tags", node.name, data.tags.len());
                }
            }
        }
    }

    /// 将数据发送到所有下游节点
    async fn fan_out(
        channels: &HashMap<NodeId, mpsc::Sender<PipelineData>>,
        from_id: &NodeId,
        mut data: PipelineData,
    ) {
        data.source_node_id = *from_id;
        for edge in &crate::flow::Flow::default().edges.iter().filter(|e| e.from == *from_id) {
            if let Some(tx) = channels.get(&edge.to) {
                let _ = tx.send(data.clone()).await;
            }
        }
    }
}
```

**Step 2: 更新 lib.rs**

```rust
pub mod flow;
pub mod executor;
pub mod registry;

pub use flow::{Flow, FlowNode, FlowEdge, NodeType, FlowError};
pub use registry::{PluginRegistry, RegistryError};
pub use executor::{FlowExecutor, ExecutorError};
```

**Step 3: 验证编译**

```bash
cargo build -p gateway-flow 2>&1
```

预期：编译错误（正常）— 需要处理 fan_out 中的 `Flow::default()` 和空 edges 引用问题。修复如下：

**Step 4: 修复 executor.rs 中的 fan_out**

将 fan_out 改为接收 edges 参数：

```rust
async fn fan_out(
    channels: &HashMap<NodeId, mpsc::Sender<PipelineData>>,
    edges: &[(NodeId, NodeId)],  // (from, to)
    from_id: &NodeId,
    mut data: PipelineData,
) {
    data.source_node_id = *from_id;
    for (src, dst) in edges.iter().filter(|(src, _)| *src == *from_id) {
        if let Some(tx) = channels.get(dst) {
            let _ = tx.send(data.clone()).await;
        }
    }
}
```

并在 start() 中传递 edges。

**Step 5: 重新编译**

```bash
cargo build -p gateway-flow 2>&1 | head -30
```

预期：PASS（可能有一些 unused warnings，不影响）

**Step 6: 提交**

```bash
git add gateway/gateway-flow/src/executor.rs gateway/gateway-flow/src/lib.rs
git commit -m "feat(flow): add FlowExecutor DAG execution engine (skeleton)"
```

---

## Stage 3：5个内置算子

### Task 10: 创建 gateway-flow 内置算子 crate

**Objective:** 将内置算子作为 gateway-flow 的一部分（或独立 subcrate）实现

**Files:**
- Modify: `gateway/gateway-flow/Cargo.toml` — 添加 rhai 依赖
- Create: `gateway/gateway-flow/src/operators/mod.rs`
- Create: `gateway/gateway-flow/src/operators/filter.rs`

**Step 1: 更新 gateway-flow/Cargo.toml 添加 rhai**

```toml
[dependencies]
gateway-sdk = { path = "../gateway-sdk" }
tokio = { version = "1", features = ["sync", "time", "rt"] }
uuid = { version = "1", features = ["v4", "serde"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"
thiserror = "1"
tracing = "0.1"
chrono = { version = "0.4", features = ["serde"] }
async-trait = "0.1"
rhai = "1"
parking_lot = "0.12"
```

**Step 2: 创建算子模块**

```bash
mkdir -p gateway/gateway-flow/src/operators
```

**Step 3: 创建 filter.rs**

```rust
//! filter 算子：根据条件过滤数据

use async_trait::async_trait;
use gateway_sdk::pipeline::{DataValue, PipelineData};
use gateway_sdk::plugin::{OperatorPlugin, PluginMeta, PluginConfig, PluginError, PluginResult};
use rhai::Engine;

pub struct FilterOperator {
    engine: Engine,
}

impl FilterOperator {
    pub fn new() -> Self {
        let mut engine = Engine::new();
        // 注入常用变量
        engine.register_fn("value", |v: i64| v as f64);
        engine.register_fn("value", |v: f64| v);
        Self { engine }
    }
}

impl Default for FilterOperator {
    fn default() -> Self {
        Self::new()
    }
}

#[async_trait]
impl OperatorPlugin for FilterOperator {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "filter",
            kind: gateway_sdk::types::PluginKind::Operator,
            description: Some("Filter data based on conditions"),
            version: "0.1.0",
            name_zh: Some("过滤器"),
            name_en: Some("Filter"),
            description_zh: Some("根据条件过滤数据，满足条件则通过，否则丢弃"),
            description_en: Some("Filter data based on conditions"),
        }
    }

    fn config_schema(&self) -> Option<PluginConfig> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "condition": {
                    "type": "string",
                    "description": "Filter condition, e.g. \"temperature > 100\"",
                    "description_zh": "过滤条件表达式"
                },
                "pass": {
                    "type": "boolean",
                    "default": true,
                    "description": "Pass if condition is true (true) or false (false)",
                    "description_zh": "条件为真时通过还是丢弃"
                }
            },
            "required": ["condition"]
        }))
    }

    async fn init(&self, config: PluginConfig) -> PluginResult<()> {
        let condition = config.get("condition")
            .and_then(|v| v.as_str())
            .ok_or_else(|| PluginError::invalid_config("filter: missing 'condition'"))?;
        
        // 预验证表达式
        self.engine.compile(condition)
            .map_err(|e| PluginError::invalid_config(&format!("filter: invalid condition: {}", e)))?;
        
        Ok(())
    }

    async fn process(&self, data: PipelineData) -> PluginResult<PipelineData> {
        // NOTE: config 在 init 中已验证，process 中使用硬编码条件（实际应从节点配置读取）
        // 完整实现需要将 config 存入 operator 实例
        Ok(data)
    }
}
```

**Step 4: 创建 operators/mod.rs**

```rust
pub mod filter;
pub mod transform;
pub mod aggregate;
pub mod router;
pub mod buffer;

pub use filter::FilterOperator;
```

**Step 5: 更新 lib.rs 添加算子**

```rust
pub mod flow;
pub mod executor;
pub mod registry;
pub mod operators;

pub use flow::{Flow, FlowNode, FlowEdge, NodeType, FlowError};
pub use registry::{PluginRegistry, RegistryError};
pub use executor::{FlowExecutor, ExecutorError};
```

**Step 6: 验证编译**

```bash
cargo build -p gateway-flow
```

预期：PASS（filter 算子编译成功）

**Step 7: 提交**

```bash
git add gateway/gateway-flow/Cargo.toml gateway/gateway-flow/src/operators/ gateway/gateway-flow/src/lib.rs
git commit -m "feat(flow): add filter operator plugin (skeleton)"
```

---

### Task 11-14: 实现剩余 4 个算子

**Objective:** 实现 transform、aggregate、router、buffer 四个算子

**transform.rs** — 数据类型转换、缩放、计算：

```rust
use async_trait::async_trait;
use gateway_sdk::pipeline::{DataValue, PipelineData};
use gateway_sdk::plugin::{OperatorPlugin, PluginMeta, PluginConfig, PluginResult};
use std::collections::HashMap;

pub struct TransformOperator;

#[async_trait]
impl OperatorPlugin for TransformOperator {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "transform",
            kind: gateway_sdk::types::PluginKind::Operator,
            description: Some("Transform data values"),
            version: "0.1.0",
            name_zh: Some("转换器"),
            name_en: Some("Transform"),
            description_zh: Some("数据类型转换、缩放、计算"),
            description_en: Some("Transform data: type conversion, scaling, computation"),
        }
    }

    fn config_schema(&self) -> Option<PluginConfig> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "rules": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "tag": {"type": "string"},
                            "expr": {"type": "string", "description": "e.g. \"value / 10.0\""}
                        },
                        "required": ["tag", "expr"]
                    }
                }
            },
            "required": ["rules"]
        }))
    }

    async fn process(&self, data: PipelineData) -> PluginResult<PipelineData> {
        // 完整实现：遍历 rules，对每个 tag 应用 expr，更新 data.tags
        Ok(data)
    }
}
```

**aggregate.rs** — 窗口聚合：

```rust
use async_trait::async_trait;
use gateway_sdk::pipeline::{DataValue, PipelineData};
use gateway_sdk::plugin::{OperatorPlugin, PluginMeta, PluginConfig, PluginResult};
use parking_lot::RwLock;
use std::collections::HashMap;

pub struct AggregateOperator {
    state: RwLock<HashMap<String, Vec<f64>>>,
    window_sec: RwLock<u64>,
}

#[async_trait]
impl OperatorPlugin for AggregateOperator {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "aggregate",
            kind: gateway_sdk::types::PluginKind::Operator,
            description: Some("Aggregate data over time windows"),
            version: "0.1.0",
            name_zh: Some("聚合器"),
            name_en: Some("Aggregate"),
            description_zh: Some("时间窗口聚合：count/sum/avg/max/min/first/last"),
            description_en: Some("Time window aggregation: count/sum/avg/max/min/first/last"),
        }
    }

    fn config_schema(&self) -> Option<PluginConfig> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "window_sec": {"type": "integer", "default": 60, "description": "窗口大小(秒)"},
                "aggregation": {"type": "string", "enum": ["count","sum","avg","max","min","first","last"], "default": "avg"},
                "tag": {"type": "string", "description": "要聚合的 tag 名"}
            },
            "required": ["window_sec", "aggregation", "tag"]
        }))
    }

    async fn process(&self, data: PipelineData) -> PluginResult<PipelineData> {
        // 完整实现：按 tag 收集值，窗口到期时输出聚合结果
        Ok(data)
    }
}
```

**router.rs** — 条件路由：

```rust
use async_trait::async_trait;
use gateway_sdk::pipeline::PipelineData;
use gateway_sdk::plugin::{OperatorPlugin, PluginMeta, PluginConfig, PluginResult};

pub struct RouterOperator;

#[async_trait]
impl OperatorPlugin for RouterOperator {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "router",
            kind: gateway_sdk::types::PluginKind::Operator,
            description: Some("Route data to different branches based on conditions"),
            version: "0.1.0",
            name_zh: Some("路由器"),
            name_en: Some("Router"),
            description_zh: Some("按条件将数据路由到不同下游分支"),
            description_en: Some("Route data to different downstream branches based on conditions"),
        }
    }

    fn config_schema(&self) -> Option<PluginConfig> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "branches": {
                    "type": "array",
                    "items": {
                        "type": "object",
                        "properties": {
                            "condition": {"type": "string"},
                            "target": {"type": "string", "description": "目标节点 ID"}
                        }
                    }
                }
            },
            "required": ["branches"]
        }))
    }

    async fn process(&self, data: PipelineData) -> PluginResult<PipelineData> {
        // 完整实现：评估条件，写入 metadata 指示下游路由目标
        Ok(data)
    }
}
```

**buffer.rs** — 缓冲批量输出：

```rust
use async_trait::async_trait;
use gateway_sdk::pipeline::PipelineData;
use gateway_sdk::plugin::{OperatorPlugin, PluginMeta, PluginConfig, PluginResult};
use parking_lot::RwLock;
use std::collections::VecDeque;

pub struct BufferOperator {
    queue: RwLock<VecDeque<PipelineData>>,
    batch_size: RwLock<usize>,
}

#[async_trait]
impl OperatorPlugin for BufferOperator {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "buffer",
            kind: gateway_sdk::types::PluginKind::Operator,
            description: Some("Buffer data and output in batches"),
            version: "0.1.0",
            name_zh: Some("缓冲器"),
            name_en: Some("Buffer"),
            description_zh: Some("缓冲数据，攒满 batch_size 条后批量输出"),
            description_en: Some("Buffer data and output in batches of N items"),
        }
    }

    fn config_schema(&self) -> Option<PluginConfig> {
        Some(serde_json::json!({
            "type": "object",
            "properties": {
                "batch_size": {"type": "integer", "default": 50},
                "flush_interval_ms": {"type": "integer", "default": 5000}
            },
            "required": ["batch_size"]
        }))
    }

    async fn process(&self, data: PipelineData) -> PluginResult<PipelineData> {
        // 完整实现：存入队列，达到 batch_size 则触发批量输出
        Ok(data)
    }
}
```

**验证编译 + 提交：**

```bash
cargo build -p gateway-flow && cargo test -p gateway-flow
# commit each operator separately
```

---

## Stage 4：gateway-server Flow CRUD API

### Task 15: 添加 Flow 路由到 gateway-server

**Objective:** 在 gateway-server 中添加 Flow CRUD REST API

**Files:**
- Modify: `gateway/gateway-server/src/api/handlers.rs`
- Modify: `gateway/gateway-server/src/api/mod.rs`
- Modify: `gateway/gateway-server/src/state.rs`

**Step 1: 在 handlers.rs 添加 FlowHandler**

```rust
// 在 gateway-server/src/api/handlers.rs 末尾添加：

use gateway_flow::{Flow, FlowNode, FlowEdge, NodeType, PluginRegistry, FlowExecutor};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use tokio::sync::RwLock;
use uuid::Uuid;

#[derive(Debug, Serialize, Deserialize)]
pub struct CreateFlowRequest {
    pub name: String,
    pub description: Option<String>,
    pub nodes: Vec<FlowNode>,
    pub edges: Vec<FlowEdge>,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct FlowResponse {
    pub id: Uuid,
    pub name: String,
    pub description: Option<String>,
    pub nodes: Vec<FlowNode>,
    pub edges: Vec<FlowEdge>,
    pub status: String,
}

/// GET /api/flows
pub async fn list_flows(
    state: Arc<RwLock<gateway_server::AppState>>,
) -> Result<axum::Json<Vec<FlowResponse>>, axum::response::Response> {
    let flows = state.read().await.flows.lock().await;
    Ok(axum::Json(flows.clone()))
}

/// POST /api/flows
pub async fn create_flow(
    state: Arc<RwLock<gateway_server::AppState>>,
    axum::extract::Json(req): axum::extract::Json<CreateFlowRequest>,
) -> Result<axum::Json<FlowResponse>, axum::response::Response> {
    let flow = Flow {
        id: Uuid::new_v4(),
        name: req.name,
        description: req.description,
        nodes: req.nodes,
        edges: req.edges,
    };

    // 验证 Flow
    if let Err(e) = flow.validate() {
        return Err((axum::http::StatusCode::BAD_REQUEST, format!("{}", e)).into_response());
    }

    let resp = FlowResponse {
        id: flow.id,
        name: flow.name.clone(),
        description: flow.description.clone(),
        nodes: flow.nodes.clone(),
        edges: flow.edges.clone(),
        status: "draft".into(),
    };

    state.write().await.flows.lock().await.push(flow);

    Ok(axum::Json(resp))
}

/// GET /api/flows/:id
pub async fn get_flow(
    state: Arc<RwLock<gateway_server::AppState>>,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> Result<axum::Json<FlowResponse>, axum::response::Response> {
    let flows = state.read().await.flows.lock().await;
    let flow = flows.iter().find(|f| f.id == id)
        .ok_or_else(|| (axum::http::StatusCode::NOT_FOUND, "Flow not found").into_response())?;

    Ok(axum::Json(FlowResponse {
        id: flow.id,
        name: flow.name.clone(),
        description: flow.description.clone(),
        nodes: flow.nodes.clone(),
        edges: flow.edges.clone(),
        status: "draft".into(),
    }))
}

/// DELETE /api/flows/:id
pub async fn delete_flow(
    state: Arc<RwLock<gateway_server::AppState>>,
    axum::extract::Path(id): axum::extract::Path<Uuid>,
) -> Result<axum::http::StatusCode, axum::response::Response> {
    let mut flows = state.write().await.flows.lock().await;
    let len_before = flows.len();
    flows.retain(|f| f.id != id);
    if flows.len() == len_before {
        return Err((axum::http::StatusCode::NOT_FOUND, "Flow not found").into_response());
    }
    Ok(axum::http::StatusCode::NO_CONTENT)
}
```

**Step 2: 在 mod.rs 添加路由**

```rust
// 在 api/mod.rs 的 router 配置中添加：
router = router.nest("/api", api_router)

// 其中 api_router:
let api_router = axum::Router::new()
    .route("/flows", get(list_flows).post(create_flow))
    .route("/flows/:id", get(get_flow).delete(delete_flow))
    // ... 保留原有路由
```

**Step 3: 在 AppState 添加 flows 字段**

```rust
// 在 gateway-server/src/state.rs 的 AppState 中添加：
use gateway_flow::Flow;

pub struct AppState {
    // ... existing fields ...
    pub flows: Arc<tokio::sync::Mutex<Vec<Flow>>>,
}
```

**Step 4: 验证编译**

```bash
cargo build -p gateway-server 2>&1 | head -40
```

预期：可能有类型错误，按错误信息修复（常见：缺少 clone、路径问题等）

**Step 5: 提交**

```bash
git add gateway/gateway-server/src/api/handlers.rs gateway/gateway-server/src/api/mod.rs gateway/gateway-server/src/state.rs
git commit -m "feat(server): add Flow CRUD REST API"
```

---

## Stage 5：前端 VueFlow 集成

### Task 16: 安装 VueFlow 依赖

**Objective:** 在 web 前端项目中安装 VueFlow

**Step 1: 检查前端 package.json**

```bash
cat /home/letmlook/projects/iot-gateway/web/package.json | head -30
```

**Step 2: 安装 VueFlow**

```bash
cd /home/letmlook/projects/iot-gateway/web
npm install @vue-flow/core @vue-flow/background @vue-flow/controls @vue-flow/minimap --save
```

（如果 npm 有问题，使用之前验证过的 NODE_PATH 方案）

**Step 3: 验证安装**

```bash
grep vue-flow package.json
```

预期：看到 @vue-flow/core 版本

**Step 4: 提交**

```bash
git add web/package.json web/package-lock.json
git commit -m "feat(web): install VueFlow dependencies"
```

---

### Task 17: 创建 FlowEditor 页面

**Objective:** 创建 Flow Editor Vue 页面，实现拖拽画布

**Files:**
- Create: `web/src/views/FlowEditor.vue`
- Modify: `web/src/router/index.js` — 添加路由

**Step 1: 创建 FlowEditor.vue**

```vue
<template>
  <div class="flow-editor">
    <div class="flow-editor__toolbar">
      <span class="flow-name">{{ flowName }}</span>
      <el-button type="primary" @click="saveFlow">保存</el-button>
      <el-button type="success" @click="deployFlow">部署</el-button>
    </div>
    <div class="flow-editor__body">
      <!-- 左侧节点面板 -->
      <div class="flow-editor__palette">
        <div class="palette-section">
          <h4>采集 (South)</h4>
          <div
            v-for="plugin in southPlugins"
            :key="plugin.name"
            class="palette-node"
            draggable="true"
            @dragstart="onDragStart($event, plugin, 'south')"
          >
            {{ plugin.name_zh || plugin.name }}
          </div>
        </div>
        <div class="palette-section">
          <h4>处理 (Operator)</h4>
          <div
            v-for="plugin in operatorPlugins"
            :key="plugin.name"
            class="palette-node"
            draggable="true"
            @dragstart="onDragStart($event, plugin, 'operator')"
          >
            {{ plugin.name_zh || plugin.name }}
          </div>
        </div>
        <div class="palette-section">
          <h4>传输 (North)</h4>
          <div
            v-for="plugin in northPlugins"
            :key="plugin.name"
            class="palette-node"
            draggable="true"
            @dragstart="onDragStart($event, plugin, 'north')"
          >
            {{ plugin.name_zh || plugin.name }}
          </div>
        </div>
      </div>

      <!-- 画布 -->
      <div class="flow-editor__canvas" @drop="onDrop" @dragover.prevent>
        <VueFlow
          v-model:nodes="nodes"
          v-model:edges="edges"
          :node-types="nodeTypes"
          :default-edge-options="{ type: 'smoothstep' }"
          fit-view-on-init
          @node-click="onNodeClick"
          @edge-click="onEdgeClick"
        >
          <Background pattern-color="#aaa" :gap="16" />
          <Controls />
          <MiniMap />
        </VueFlow>
      </div>

      <!-- 右侧配置面板 -->
      <div class="flow-editor__config" v-if="selectedNode">
        <h3>节点配置</h3>
        <el-form label-width="100px">
          <el-form-item label="名称">
            <el-input v-model="selectedNode.data.label" />
          </el-form-item>
          <el-form-item label="插件">
            <el-input :value="selectedNode.data.plugin" disabled />
          </el-form-item>
          <el-form-item label="配置">
            <el-input
              type="textarea"
              v-model="selectedNode.data.configJson"
              :rows="6"
              placeholder="JSON 配置"
            />
          </el-form-item>
        </el-form>
      </div>
    </div>
  </div>
</template>

<script setup>
import { ref, computed, onMounted } from 'vue'
import { VueFlow, useVueFlow, Background, Controls, MiniMap } from '@vue-flow/core'
import '@vue-flow/core/dist/style.css'
import '@vue-flow/core/dist/theme-default.css'
import '@vue-flow/controls/dist/style.css'
import '@vue-flow/minimap/dist/style.css'
import { ElMessage } from 'element-plus'
import { v4 as uuidv4 } from 'uuid'

const { onConnect, addEdges, addNodes, project } = useVueFlow()

const flowName = ref('未命名流程')
const nodes = ref([])
const edges = ref([])
const selectedNode = ref(null)
const southPlugins = ref([])
const operatorPlugins = ref([])
const northPlugins = ref([])

// 节点类型映射
const nodeTypes = ['south', 'operator', 'north']

onConnect((params) => {
  edges.value = addEdges(params)
})

function onDragStart(event, plugin, type) {
  event.dataTransfer.setData('plugin', JSON.stringify({ plugin, type }))
}

function onDrop(evt) {
  const raw = evt.dataTransfer.getData('plugin')
  if (!raw) return
  const { plugin, type } = JSON.parse(raw)

  const { x, y } = project({ x: evt.clientX, y: evt.clientY })
  const id = uuidv4()

  addNodes([
    {
      id,
      type: 'default',
      position: { x, y },
      data: {
        label: plugin.name_zh || plugin.name,
        plugin: plugin.name,
        nodeType: type,
        configJson: JSON.stringify({}, null, 2),
        config: {},
      },
    },
  ])
}

function onNodeClick({ node }) {
  selectedNode.value = node
}

function onEdgeClick({ edge }) {
  // 可选：边点击配置
}

async function saveFlow() {
  try {
    const flow = {
      name: flowName.value,
      nodes: nodes.value.map(n => ({
        id: n.id,
        name: n.data.label,
        plugin: n.data.plugin,
        node_type: n.data.nodeType,
        config: JSON.parse(n.data.configJson || '{}'),
      })),
      edges: edges.value.map(e => ({
        from: e.source,
        to: e.target,
      })),
    }
    await fetch('/api/flows', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify(flow),
    })
    ElMessage.success('保存成功')
  } catch (e) {
    ElMessage.error('保存失败')
  }
}

async function deployFlow() {
  ElMessage.info('部署功能待实现')
}

onMounted(async () => {
  // 加载插件列表
  try {
    const res = await fetch('/api/plugins')
    const data = await res.json()
    southPlugins.value = data.filter(p => p.kind === 'south')
    northPlugins.value = data.filter(p => p.kind === 'north')
    operatorPlugins.value = data.filter(p => p.kind === 'operator')
  } catch (e) {
    southPlugins.value = [
      { name: 'sim', name_zh: '模拟数据' },
      { name: 'modbus-tcp', name_zh: 'Modbus TCP' },
      { name: 'opcua', name_zh: 'OPC UA' },
    ]
    northPlugins.value = [
      { name: 'mqtt', name_zh: 'MQTT' },
      { name: 'http', name_zh: 'HTTP' },
    ]
    operatorPlugins.value = [
      { name: 'filter', name_zh: '过滤器' },
      { name: 'transform', name_zh: '转换器' },
      { name: 'aggregate', name_zh: '聚合器' },
      { name: 'router', name_zh: '路由器' },
      { name: 'buffer', name_zh: '缓冲器' },
    ]
  }
})
</script>

<style scoped>
.flow-editor {
  display: flex;
  flex-direction: column;
  height: 100vh;
}
.flow-editor__toolbar {
  padding: 8px 16px;
  background: #fff;
  border-bottom: 1px solid #ddd;
  display: flex;
  gap: 12px;
  align-items: center;
}
.flow-name {
  font-weight: 600;
  font-size: 16px;
}
.flow-editor__body {
  display: flex;
  flex: 1;
  overflow: hidden;
}
.flow-editor__palette {
  width: 180px;
  background: #fafafa;
  border-right: 1px solid #ddd;
  padding: 12px;
  overflow-y: auto;
}
.palette-section h4 {
  font-size: 12px;
  color: #666;
  margin: 8px 0 4px;
}
.palette-node {
  padding: 6px 10px;
  background: #fff;
  border: 1px solid #ddd;
  border-radius: 4px;
  margin-bottom: 4px;
  cursor: move;
  font-size: 13px;
}
.palette-node:hover {
  border-color: #409eff;
}
.flow-editor__canvas {
  flex: 1;
  background: #f5f5f5;
}
.flow-editor__config {
  width: 280px;
  background: #fff;
  border-left: 1px solid #ddd;
  padding: 16px;
  overflow-y: auto;
}
</style>
```

**Step 2: 添加路由**

在 `web/src/router/index.js` 中添加：

```js
{
  path: '/flow/:id?',
  name: 'FlowEditor',
  component: () => import('@/views/FlowEditor.vue')
}
```

**Step 3: 验证（如果前端可以启动）**

```bash
cd /home/letmlook/projects/iot-gateway/web
# 如果可以启动 dev server
# npm run serve
```

**Step 4: 提交**

```bash
git add web/src/views/FlowEditor.vue web/src/router/index.js
git commit -m "feat(web): add VueFlow-based Flow Editor page"
```

---

### Task 18: 创建 Flow 列表页

**Objective:** 创建 Flow 列表页面，入口到 /flow

**Files:**
- Create: `web/src/views/FlowList.vue`

**Step 1: 创建 FlowList.vue**

```vue
<template>
  <div class="flow-list">
    <div class="page-header">
      <h2>数据流编排</h2>
      <el-button type="primary" @click="$router.push('/flow/new')">+ 新建流程</el-button>
    </div>
    <el-table :data="flows" stripe>
      <el-table-column prop="name" label="名称" />
      <el-table-column prop="description" label="描述" />
      <el-table-column prop="status" label="状态">
        <template #default="{ row }">
          <el-tag :type="statusType(row.status)">{{ row.status }}</el-tag>
        </template>
      </el-table-column>
      <el-table-column label="操作" width="200">
        <template #default="{ row }">
          <el-button size="small" @click="$router.push(`/flow/${row.id}`)">编辑</el-button>
          <el-button size="small" type="danger" @click="deleteFlow(row.id)">删除</el-button>
        </template>
      </el-table-column>
    </el-table>
  </div>
</template>

<script setup>
import { ref, onMounted } from 'vue'
import { ElMessage, ElMessageBox } from 'element-plus'

const flows = ref([])

async function loadFlows() {
  try {
    const res = await fetch('/api/flows')
    flows.value = await res.json()
  } catch {
    flows.value = []
  }
}

async function deleteFlow(id) {
  await ElMessageBox.confirm('确认删除此流程？', '警告', { type: 'warning' })
  await fetch(`/api/flows/${id}`, { method: 'DELETE' })
  ElMessage.success('已删除')
  loadFlows()
}

function statusType(s) {
  return s === 'running' ? 'success' : s === 'failed' ? 'danger' : 'info'
}

onMounted(loadFlows)
</script>

<style scoped>
.page-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 16px;
}
</style>
```

**Step 2: 添加路由**

```js
{
  path: '/flow',
  name: 'FlowList',
  component: () => import('@/views/FlowList.vue')
}
```

**Step 3: 提交**

```bash
git add web/src/views/FlowList.vue web/src/router/index.js
git commit -m "feat(web): add Flow list page"
```

---

### Task 19: 集成 element-plus（确认已安装）

**Objective:** 确认 element-plus 已安装（VueFlow 需要）

**Step 1: 检查**

```bash
grep element-plus /home/letmlook/projects/iot-gateway/web/package.json
```

如果不存在则安装：

```bash
npm install element-plus --save
```

**Step 2: main.js 中引入（如果尚未引入）**

```js
import { createApp } from 'vue'
import ElementPlus from 'element-plus'
import 'element-plus/dist/index.css'

const app = createApp(App)
app.use(ElementPlus)
app.mount('#app')
```

**Step 3: 提交**

```bash
git add web/package.json web/package-lock.json web/src/main.js
git commit -m "feat(web): add element-plus dependency"
```

---

## 执行方式

Plan 完成后，使用 subagent-driven-development 逐任务执行：

```bash
# Stage 1 执行（5个任务）
delegate_task goal="执行 Phase 1 Stage 1 的 5 个任务，详见 plan" context="..."

# Stage 2 执行（6个任务）
delegate_task goal="执行 Phase 1 Stage 2 的 6 个任务，详见 plan"

# 以此类推...
```

每个 Stage 完成后进行代码审查（review），确认无误后进入下一 Stage。
