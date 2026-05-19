//! 本系统核心类型：Tag、Group、Node、DataValue 等。

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use uuid::Uuid;

/// 插件种类：南向设备驱动 / 北向应用
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PluginKind {
    South,
    North,
}

/// 统一数据值类型
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value")]
pub enum DataValue {
    Bool(bool),
    Int8(i8),
    Int16(i16),
    Int32(i32),
    Int64(i64),
    UInt8(u8),
    UInt16(u16),
    UInt32(u32),
    UInt64(u64),
    Float32(f32),
    Float64(f64),
    String(String),
    Bytes(Vec<u8>),
}

/// 数据类型标识（用于 Schema、校验、UI）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum DataType {
    Bool,
    Int8,
    Int16,
    Int32,
    Int64,
    UInt8,
    UInt16,
    UInt32,
    UInt64,
    Float32,
    Float64,
    String,
    Bytes,
}

impl DataType {
    pub const fn as_str(&self) -> &'static str {
        match self {
            DataType::Bool => "bool",
            DataType::Int8 => "int8",
            DataType::Int16 => "int16",
            DataType::Int32 => "int32",
            DataType::Int64 => "int64",
            DataType::UInt8 => "uint8",
            DataType::UInt16 => "uint16",
            DataType::UInt32 => "uint32",
            DataType::UInt64 => "uint64",
            DataType::Float32 => "float32",
            DataType::Float64 => "float64",
            DataType::String => "string",
            DataType::Bytes => "bytes",
        }
    }
}

impl std::fmt::Display for DataType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.as_str())
    }
}

impl DataValue {
    pub fn as_f64(&self) -> Option<f64> {
        match self {
            DataValue::Float64(v) => Some(*v),
            DataValue::Float32(v) => Some(*v as f64),
            DataValue::Int64(v) => Some(*v as f64),
            DataValue::UInt64(v) => Some(*v as f64),
            _ => None,
        }
    }

    pub fn as_string(&self) -> Option<String> {
        match self {
            DataValue::String(s) => Some(s.clone()),
            _ => None,
        }
    }

    pub fn as_i64(&self) -> Option<i64> {
        match self {
            DataValue::Int64(v) => Some(*v),
            DataValue::Int32(v) => Some(*v as i64),
            DataValue::Int16(v) => Some(*v as i64),
            DataValue::Int8(v) => Some(*v as i64),
            DataValue::UInt64(v) => Some(*v as i64),
            DataValue::UInt32(v) => Some(*v as i64),
            DataValue::UInt16(v) => Some(*v as i64),
            DataValue::UInt8(v) => Some(*v as i64),
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self {
            DataValue::UInt64(v) => Some(*v),
            DataValue::UInt32(v) => Some(*v as u64),
            DataValue::UInt16(v) => Some(*v as u64),
            DataValue::UInt8(v) => Some(*v as u64),
            DataValue::Int64(v) if *v >= 0 => Some(*v as u64),
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            DataValue::Bool(v) => Some(*v),
            _ => None,
        }
    }

    /// 对应 DataType
    pub fn data_type(&self) -> DataType {
        match self {
            DataValue::Bool(_) => DataType::Bool,
            DataValue::Int8(_) => DataType::Int8,
            DataValue::Int16(_) => DataType::Int16,
            DataValue::Int32(_) => DataType::Int32,
            DataValue::Int64(_) => DataType::Int64,
            DataValue::UInt8(_) => DataType::UInt8,
            DataValue::UInt16(_) => DataType::UInt16,
            DataValue::UInt32(_) => DataType::UInt32,
            DataValue::UInt64(_) => DataType::UInt64,
            DataValue::Float32(_) => DataType::Float32,
            DataValue::Float64(_) => DataType::Float64,
            DataValue::String(_) => DataType::String,
            DataValue::Bytes(_) => DataType::Bytes,
        }
    }
}

/// 点位（Tag）唯一标识
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TagId(pub Uuid);

impl TagId {
    pub fn new() -> Self {
        TagId(Uuid::new_v4())
    }
}

impl Default for TagId {
    fn default() -> Self {
        Self::new()
    }
}

/// 组（Group）唯一标识
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct GroupId(pub Uuid);

impl GroupId {
    pub fn new() -> Self {
        GroupId(Uuid::new_v4())
    }
}

impl Default for GroupId {
    fn default() -> Self {
        Self::new()
    }
}

/// 节点（Node）唯一标识。节点 = 适配器 + 插件实例
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct NodeId(pub Uuid);

impl NodeId {
    pub fn new() -> Self {
        NodeId(Uuid::new_v4())
    }
}

impl Default for NodeId {
    fn default() -> Self {
        Self::new()
    }
}

/// 读写属性
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum TagAttr {
    Read,
    Write,
    ReadWrite,
}

/// 点位（Tag）配置
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Tag {
    pub id: TagId,
    pub name: String,
    /// 协议侧地址，如 Modbus 寄存器地址、OPC UA 节点等
    pub address: String,
    pub attr: TagAttr,
    /// 数据类型提示（可选，插件可据此解析）
    pub data_type: Option<String>,
    pub description: Option<String>,
    /// 所属 Group
    pub group_id: GroupId,
}

impl Tag {
    pub fn new(name: impl Into<String>, address: impl Into<String>, group_id: GroupId) -> Self {
        Self {
            id: TagId::new(),
            name: name.into(),
            address: address.into(),
            attr: TagAttr::Read,
            data_type: None,
            description: None,
            group_id,
        }
    }
}

/// 组（Group）配置。采集与订阅的基本单元。
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Group {
    pub id: GroupId,
    pub name: String,
    /// 轮询间隔（毫秒）。南向驱动按此间隔采集该组下 Tag
    pub interval_ms: u64,
    pub description: Option<String>,
}

impl Group {
    pub fn new(name: impl Into<String>, interval_ms: u64) -> Self {
        Self {
            id: GroupId::new(),
            name: name.into(),
            interval_ms,
            description: None,
        }
    }
}

/// 节点状态
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum NodeState {
    Stopped,
    Running,
    Error,
}

/// 节点类型（南向 / 北向）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum NodeKind {
    South,
    North,
}

/// 插件配置（JSON）。各插件自行解析。
pub type PluginConfig = HashMap<String, serde_json::Value>;

/// Pipeline 数据单元，流经 South → Operator → North。
/// node_id: 源节点（South 或 Operator）
/// payload: 字段名 → 值 的映射（灵活，Operator 可添加/转换字段）
/// metadata: 元数据键值对（如 "source_group" → "group_temp", "quality" → "good"）
/// ts: 事件/采集时间戳
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineData {
    pub node_id: NodeId,
    pub payload: HashMap<String, DataValue>,
    pub metadata: HashMap<String, String>,
    pub ts: DateTime<Utc>,
}

impl PipelineData {
    pub fn new(node_id: NodeId) -> Self {
        Self {
            node_id,
            payload: HashMap::new(),
            metadata: HashMap::new(),
            ts: Utc::now(),
        }
    }

    pub fn with_payload(mut self, payload: HashMap<String, DataValue>) -> Self {
        self.payload = payload;
        self
    }

    pub fn with_metadata(mut self, key: impl Into<String>, value: impl Into<String>) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }
}

