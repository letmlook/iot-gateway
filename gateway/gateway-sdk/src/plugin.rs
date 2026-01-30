//! 南/北向插件 trait 定义，对标 Neuron neu_plugin_intf_funs_t、驱动/应用生命周期。

use crate::error::{PluginError, PluginResult};
use crate::messages::{GroupData, GroupSubscription};
use crate::schema::{ConfigSchema, TagSchema};
use crate::types::{Group, GroupId, NodeId, Tag, TagId};
use crate::types::PluginConfig;
use async_trait::async_trait;
use serde::Serialize;
use std::sync::Arc;

/// 插件元信息（对标 neu_plugin_module_t：version、module_name、module_descr、kind）
/// 支持中英文名称与描述，前端按语言选用。
#[derive(Debug, Clone)]
pub struct PluginMeta {
    pub name: &'static str,
    pub kind: crate::types::PluginKind,
    pub description: Option<&'static str>,
    pub version: &'static str,
    /// 中文名称（前端 zh 时显示）
    pub name_zh: Option<&'static str>,
    /// 英文名称（前端 en 时显示）
    pub name_en: Option<&'static str>,
    /// 中文描述（前端 zh 时显示）
    pub description_zh: Option<&'static str>,
    /// 英文描述（前端 en 时显示）
    pub description_en: Option<&'static str>,
}

/// 插件列表项（API 返回用），含中英文名称与描述。
#[derive(Debug, Clone, Serialize)]
pub struct PluginInfo {
    pub name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name_zh: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub name_en: Option<String>,
    pub description: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description_zh: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub description_en: Option<String>,
    pub version: String,
}

impl PluginInfo {
    /// 从插件键名与 PluginMeta 构建 API 用 PluginInfo。
    pub fn from_meta(name: &str, m: &PluginMeta) -> Self {
        Self {
            name: name.to_string(),
            name_zh: m.name_zh.map(String::from),
            name_en: m.name_en.map(String::from),
            description: m.description.map(String::from),
            description_zh: m.description_zh.map(String::from),
            description_en: m.description_en.map(String::from),
            version: m.version.to_string(),
        }
    }
}

/// 南向插件：连接设备、按 Group 轮询、读写、校验。对标 Neuron 南向驱动。
#[async_trait]
pub trait SouthPlugin: Send + Sync {
    fn meta(&self) -> PluginMeta;

    /// 配置 Schema（可选）。用于 UI 表单、校验。对标 modbus-tcp.json。
    fn config_schema(&self) -> Option<ConfigSchema> {
        None
    }

    /// 点位 Schema（可选）。用于地址校验、UI 提示。
    fn tag_schema(&self) -> Option<TagSchema> {
        None
    }

    // ---------- 生命周期（对标 open/close/init/uninit/start/stop/setting） ----------

    /// 创建 node 时首先调用，创建/分配 per-node 状态。
    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()>;

    /// 删除 node 时最后调用，释放 open 分配的资源。
    async fn close(&self, node_id: NodeId) -> PluginResult<()>;

    /// open 之后调用，初始化插件内资源（如全局锁、线程池）。
    async fn init(&self, node_id: NodeId) -> PluginResult<()> {
        let _ = node_id;
        Ok(())
    }

    /// 删除 node 时首先调用，释放 init 中申请的资源。
    async fn uninit(&self, node_id: NodeId) -> PluginResult<()> {
        let _ = node_id;
        Ok(())
    }

    /// 用户点击「启动」时调用，连接设备，准备好 poll_group / write_tags。
    async fn start(&self, node_id: NodeId) -> PluginResult<()> {
        let _ = node_id;
        Ok(())
    }

    /// 用户点击「停止」时调用，断开连接，不再调用 poll_group。
    async fn stop(&self, node_id: NodeId) -> PluginResult<()> {
        let _ = node_id;
        Ok(())
    }

    /// 用户修改插件配置时调用（JSON）。不删除 node。
    async fn setting(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let _ = (node_id, config);
        Ok(())
    }

    // ---------- 点位校验与采集 ----------

    /// 添加/更新 tag 时调用，校验地址、类型等。返回 Err 则拒绝。对标 driver.validate_tag。
    async fn validate_tag(&self, node_id: NodeId, tag: &Tag) -> PluginResult<()> {
        let _ = (node_id, tag);
        Ok(())
    }

    /// 按 Group 定时采集。对标 driver.group_timer。
    async fn poll_group(
        &self,
        node_id: NodeId,
        group_id: GroupId,
        tags: &[Tag],
    ) -> PluginResult<Vec<(TagId, crate::types::DataValue)>>;

    /// 写 Tag。对标 driver.write_tag。values 为 (Tag, DataValue) 以便插件从 Tag.address 解析写地址。默认不支持。
    async fn write_tags(
        &self,
        node_id: NodeId,
        values: &[(Tag, crate::types::DataValue)],
    ) -> PluginResult<()> {
        let _ = (node_id, values);
        Err(PluginError::not_supported("write not supported"))
    }

    /// 列出该节点下的 Group。
    async fn list_groups(&self, node_id: NodeId) -> PluginResult<Vec<Group>>;

    /// 列出某 Group 下的 Tag。
    async fn list_tags(&self, node_id: NodeId, group_id: GroupId) -> PluginResult<Vec<Tag>>;
}

/// 北向插件：订阅南向 Group，接收 GroupData，转发到云/应用。对标 Neuron 北向应用。
#[async_trait]
pub trait NorthPlugin: Send + Sync {
    fn meta(&self) -> PluginMeta;

    /// 配置 Schema（可选）。
    fn config_schema(&self) -> Option<ConfigSchema> {
        None
    }

    // ---------- 生命周期 ----------

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()>;

    async fn close(&self, node_id: NodeId) -> PluginResult<()>;

    async fn init(&self, node_id: NodeId) -> PluginResult<()> {
        let _ = node_id;
        Ok(())
    }

    async fn uninit(&self, node_id: NodeId) -> PluginResult<()> {
        let _ = node_id;
        Ok(())
    }

    async fn start(&self, node_id: NodeId) -> PluginResult<()> {
        let _ = node_id;
        Ok(())
    }

    async fn stop(&self, node_id: NodeId) -> PluginResult<()> {
        let _ = node_id;
        Ok(())
    }

    async fn setting(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let _ = (node_id, config);
        Ok(())
    }

    // ---------- 订阅与数据 ----------

    /// 设置订阅：(south_node_id, group_id) 列表。
    async fn set_subscriptions(
        &self,
        node_id: NodeId,
        subscriptions: &[GroupSubscription],
    ) -> PluginResult<()>;

    /// 核心推送 GroupData（来自已订阅的南向 Group）。
    async fn on_group_data(&self, node_id: NodeId, data: Arc<GroupData>) -> PluginResult<()>;
}
