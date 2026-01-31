//! 总线消息类型。南向 -> Core -> 北向。

use crate::types::{DataValue, GroupId, NodeId, TagId};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// 一组 Tag 的采集结果（南向 -> 路由核心）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GroupData {
    pub node_id: NodeId,
    pub group_id: GroupId,
    pub ts: DateTime<Utc>,
    /// (tag_id, value)
    pub values: Vec<(TagId, DataValue)>,
    /// 南向节点名称，用于北向 topic/上报字段（不把 id 放到 topic 和字段中）
    #[serde(default)]
    pub node_name: Option<String>,
    /// 南向组名称
    #[serde(default)]
    pub group_name: Option<String>,
    /// tag_id -> 点位名称，用于北向 values/tags 的 key
    #[serde(default)]
    pub tag_names: Option<std::collections::HashMap<TagId, String>>,
}

/// 北向对南向某 Group 的订阅
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GroupSubscription {
    pub south_node_id: NodeId,
    pub group_id: GroupId,
}

/// 读 Tag 请求（北向/API -> 南向 或 内部下发）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagRead {
    pub tag_ids: Vec<TagId>,
}

/// 写 Tag 请求
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TagWrite {
    pub values: Vec<(TagId, DataValue)>,
}
