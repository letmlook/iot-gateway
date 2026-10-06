//! API v1 DTO 层：所有 REST 响应类型 + OpenAPI schema definitions。
//!
//! 刻意放在 gateway-server 而非 gateway-sdk/gateway-core：插件生态不应为文档买单。
//! utoipa 派生在此层使用，不污染核心域。

use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

// ---------------------------------------------------------------------------
// 分页
// ---------------------------------------------------------------------------

/// 通用分页查询参数。page 默认 1，page_size 默认 50 clamp [1, 1000]。
#[derive(Debug, Deserialize, ToSchema)]
pub struct PageParams {
    #[serde(default = "default_page")]
    pub page: u32,
    #[serde(default = "default_page_size")]
    pub page_size: u32,
    /// name 子串过滤（大小写不敏感），仅 nodes/groups/tags
    #[serde(default)]
    pub q: Option<String>,
    /// 节点类型过滤，仅 nodes
    #[serde(default)]
    pub kind: Option<String>,
    /// 组 ID 过滤，仅 tags
    #[serde(default)]
    pub group_id: Option<String>,
}

fn default_page() -> u32 {
    1
}
fn default_page_size() -> u32 {
    50
}

impl PageParams {
    /// page clamp 下界（< 1 → 400）
    pub fn validate_page(&self) -> Result<(), crate::api::ApiError> {
        if self.page < 1 {
            return Err(crate::api::ApiError::bad_request("page must be >= 1"));
        }
        Ok(())
    }

    /// page_size clamp 到 [1, 1000]，返回实际使用的值
    pub fn clamped_page_size(&self) -> u32 {
        self.page_size.clamp(1, 1000)
    }

    /// 0-based offset
    pub fn offset(&self) -> usize {
        ((self.page - 1) * self.page_size) as usize
    }
}

/// 分页信封，v1 列表端点专用。
#[derive(Debug, Serialize, ToSchema)]
#[serde(bound = "")]
pub struct Page<T: ToSchema + Serialize> {
    pub items: Vec<T>,
    pub total: u64,
    pub page: u32,
    #[serde(rename = "pageSize")]
    pub page_size: u32,
}

impl<T: ToSchema + Serialize> Page<T> {
    pub fn new(items: Vec<T>, total: u64, page: u32, page_size: u32) -> Self {
        Self {
            items,
            total,
            page,
            page_size,
        }
    }
}

// ---------------------------------------------------------------------------
// NodeDto
// ---------------------------------------------------------------------------

/// v1 节点响应：脱敏 config + connection_status。
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct NodeDto {
    pub id: String,
    pub name: String,
    pub kind: String,
    #[serde(rename = "pluginName")]
    pub plugin_name: String,
    pub state: String,
    pub config: serde_json::Value,
    #[serde(rename = "connectionStatus", skip_serializing_if = "Option::is_none")]
    pub connection_status: Option<serde_json::Value>,
}

impl NodeDto {
    /// 从已序列化和富化的 JSON 构建：json 必须已含 connection_status 并已脱敏 config。
    pub fn from_enriched_json(node: &gateway_core::Node, json: serde_json::Value) -> Self {
        use gateway_sdk::{NodeKind, NodeState};
        let obj = json;
        let connection_status = obj.get("connection_status").cloned();
        let config = obj.get("config").cloned().unwrap_or_default();
        let kind = if node.kind() == NodeKind::North {
            "north".to_string()
        } else {
            "south".to_string()
        };
        let state_str = match node.state {
            NodeState::Running => "running",
            NodeState::Stopped => "stopped",
            _ => "unknown",
        }
        .to_string();
        Self {
            id: node.id().0.to_string(),
            name: node.config.name.clone(),
            kind,
            plugin_name: node.config.plugin_name.clone(),
            state: state_str,
            config,
            connection_status,
        }
    }
}

// ---------------------------------------------------------------------------
// GroupDto
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct GroupDto {
    pub id: String,
    pub name: String,
    #[serde(rename = "intervalMs")]
    pub interval_ms: u64,
    pub description: Option<String>,
}

impl From<&gateway_sdk::Group> for GroupDto {
    fn from(g: &gateway_sdk::Group) -> Self {
        Self {
            id: g.id.0.to_string(),
            name: g.name.clone(),
            interval_ms: g.interval_ms,
            description: g.description.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// TagDto
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct TagDto {
    pub id: String,
    #[serde(rename = "groupId")]
    pub group_id: String,
    pub name: String,
    pub address: String,
    pub attr: String,
    #[serde(rename = "dataType", skip_serializing_if = "Option::is_none")]
    pub data_type: Option<String>,
    pub description: Option<String>,
}

impl From<&gateway_sdk::Tag> for TagDto {
    fn from(t: &gateway_sdk::Tag) -> Self {
        Self {
            id: t.id.0.to_string(),
            group_id: t.group_id.0.to_string(),
            name: t.name.clone(),
            address: t.address.clone(),
            attr: format!("{:?}", t.attr),
            data_type: t.data_type.clone(),
            description: t.description.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// RuleDto
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RuleDto {
    pub id: String,
    pub name: String,
    pub enabled: bool,
    /// RuleSource 结构体如实序列化为 JSON
    #[schema(value_type = Object)]
    pub source: serde_json::Value,
    #[schema(value_type = Object)]
    pub condition: serde_json::Value,
    #[serde(rename = "forMs")]
    pub for_ms: u64,
    #[serde(rename = "clearMs")]
    pub clear_ms: u64,
    /// RuleAction 枚举如实序列化为 JSON
    #[schema(value_type = Object)]
    pub action: serde_json::Value,
    pub runtime: RuleRuntime,
}

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct RuleRuntime {
    pub fired: bool,
    #[serde(rename = "lastValue", skip_serializing_if = "Option::is_none")]
    pub last_value: Option<serde_json::Value>,
    #[serde(rename = "fireCount")]
    pub fire_count: u64,
}

impl RuleDto {
    pub fn from_view(view: &gateway_core::RuleView) -> Self {
        Self {
            id: view.rule.id.clone(),
            name: view.rule.name.clone(),
            enabled: view.rule.enabled,
            source: serde_json::to_value(&view.rule.source).unwrap_or_default(),
            condition: serde_json::to_value(&view.rule.condition).unwrap_or_default(),
            for_ms: view.rule.for_ms,
            clear_ms: view.rule.clear_ms,
            action: serde_json::to_value(&view.rule.action).unwrap_or_default(),
            runtime: RuleRuntime {
                fired: view.runtime.fired,
                last_value: view.runtime.last_value.map(serde_json::Value::from),
                fire_count: view.runtime.fire_count,
            },
        }
    }
}

// ---------------------------------------------------------------------------
// UserDto
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct UserDto {
    pub id: String,
    pub username: String,
    pub role: String,
    #[serde(rename = "createdAt")]
    pub created_at: String,
    #[serde(rename = "updatedAt")]
    pub updated_at: String,
}

impl From<&crate::users::User> for UserDto {
    fn from(u: &crate::users::User) -> Self {
        Self {
            id: u.id.clone(),
            username: u.username.clone(),
            role: u.role.as_str().to_string(),
            created_at: u.created_at.clone(),
            updated_at: u.updated_at.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// PluginDto
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
pub struct PluginDto {
    pub name: String,
    pub description: String,
    pub version: String,
    pub name_zh: Option<String>,
    pub name_en: Option<String>,
    pub description_zh: Option<String>,
    pub description_en: Option<String>,
    pub licensed: bool,
    #[serde(rename = "isFree")]
    pub is_free: bool,
}

// ---------------------------------------------------------------------------
// ValueDto
// ---------------------------------------------------------------------------

/// /nodes/:id/values 响应中的单个 tag 值。
#[derive(Debug, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct ValueDto {
    #[serde(rename = "tagId")]
    pub tag_id: String,
    pub name: String,
    #[serde(rename = "groupId")]
    pub group_id: String,
    pub value: serde_json::Value,
    pub ts: Option<i64>,
    pub available: bool,
}

// ---------------------------------------------------------------------------
// Error
// ---------------------------------------------------------------------------

/// OpenAPI 全局错误组件。
#[derive(Debug, Serialize, ToSchema)]
pub struct Error {
    pub code: String,
    pub message: String,
}

impl From<&crate::api::error::ApiErrorBody> for Error {
    fn from(e: &crate::api::error::ApiErrorBody) -> Self {
        Self {
            code: e.code.clone(),
            message: e.message.clone(),
        }
    }
}

// ---------------------------------------------------------------------------
// WS 帧 DTO（OpenAPI 中声明，与 ws.rs 共用同一类型定义）
// ---------------------------------------------------------------------------

/// WS 客户端 → 服务器帧。
#[derive(Debug, Deserialize, ToSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
#[allow(dead_code)]
pub enum WsClientFrame {
    Auth {
        token: Option<String>,
    },
    Ping,
    Pong,
    Subscribe {
        topics: Vec<String>,
        #[serde(rename = "nodeIds", skip_serializing_if = "Option::is_none")]
        node_ids: Option<Vec<String>>,
        #[serde(rename = "groupIds", skip_serializing_if = "Option::is_none")]
        group_ids: Option<Vec<String>>,
    },
    Unsubscribe {
        topics: Vec<String>,
    },
}

/// values 帧数据体。
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WsValuesData {
    #[serde(rename = "nodeId")]
    pub node_id: String,
    #[serde(rename = "nodeName", skip_serializing_if = "Option::is_none")]
    pub node_name: Option<String>,
    #[serde(rename = "groupId")]
    pub group_id: String,
    #[serde(rename = "groupName", skip_serializing_if = "Option::is_none")]
    pub group_name: Option<String>,
    pub values: Vec<WsTagValue>,
}

/// 单个 tag 的值（DataValue 编码与 REST 完全一致）。
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WsTagValue {
    #[serde(rename = "tagId")]
    pub tag_id: String,
    #[serde(rename = "tagName", skip_serializing_if = "Option::is_none")]
    pub tag_name: Option<String>,
    pub value: serde_json::Value,
}

/// nodes 快照帧中的节点。
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(rename_all = "camelCase")]
pub struct WsNodeSnapshot {
    pub id: String,
    pub name: String,
    pub kind: String,
    #[serde(rename = "pluginName")]
    pub plugin_name: String,
    pub state: String,
    #[serde(rename = "connectionStatus", skip_serializing_if = "Option::is_none")]
    pub connection_status: Option<serde_json::Value>,
}

/// WS 服务器 → 客户端帧。
#[derive(Debug, Clone, Serialize, ToSchema)]
#[serde(tag = "type", rename_all = "camelCase")]
#[allow(dead_code)]
pub enum WsServerFrame {
    Hello {
        version: String,
        build_date: String,
        features: Vec<String>,
    },
    Auth {
        success: bool,
        message: Option<String>,
    },
    Ping,
    Pong,
    Values {
        data: WsValuesData,
    },
    Nodes {
        nodes: Vec<WsNodeSnapshot>,
    },
    Error {
        message: String,
    },
}
