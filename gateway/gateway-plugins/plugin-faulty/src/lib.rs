//! 故障注入插件（**测试夹具，不对应任何真实设备**）。
//!
//! 用途：验证宿主对插件异常的隔离能力。历史上 FFI 调用在 tokio worker 线程上直接执行，
//! 插件一旦 panic（或内部 `block_on`）会直接 abort 整个网关；本夹具用于把这类回归钉死在测试里。
//!
//! 触发方式：
//! - 节点配置 `panic_on`：`open` | `start` | `poll` | `write` | `abort`，其余值表示正常运行；
//!   `abort` 会用 `std::process::abort()` 直接杀掉进程——用来验证**进程级隔离**是否真的生效
//!   （进程内模式下它会杀掉整个网关，这正是需要隔离的理由）；
//! - 环境变量 `FAULTY_PLUGIN_PANIC_META=1`：让加载期的 `meta` 调用 panic（该阶段拿不到节点配置）。

#[cfg(feature = "ffi")]
mod ffi;

use async_trait::async_trait;
use gateway_sdk::types::{DataValue, PluginKind};
use gateway_sdk::{
    Group, GroupId, NodeId, PluginConfig, PluginMeta, PluginResult, SouthPlugin, Tag, TagId,
};
use std::collections::HashMap;
use std::sync::RwLock;

/// 需要注入 panic 的生命周期阶段
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum PanicOn {
    #[default]
    None,
    Open,
    Start,
    Poll,
    Write,
    /// 直接终止进程（不是 panic 能捕获的那类故障）
    Abort,
}

impl PanicOn {
    /// 从节点配置解析；无法识别的值一律按「正常运行」处理，便于把夹具当普通插件用
    pub fn from_config(config: &PluginConfig) -> Self {
        match config.get("panic_on").and_then(|v| v.as_str()) {
            Some("open") => PanicOn::Open,
            Some("start") => PanicOn::Start,
            Some("poll") => PanicOn::Poll,
            Some("write") => PanicOn::Write,
            Some("abort") => PanicOn::Abort,
            _ => PanicOn::None,
        }
    }
}

/// 故障注入插件
#[derive(Default)]
pub struct FaultyPlugin {
    /// 每个节点注入的阶段
    modes: RwLock<HashMap<NodeId, PanicOn>>,
}

impl FaultyPlugin {
    pub fn new() -> Self {
        Self::default()
    }

    /// 读取该节点的注入阶段。
    ///
    /// 特意「先复制再返回」，让锁在本函数返回时就释放：panic 发生时不会留下
    /// 一个处于 poison 状态的守卫，这样夹具可以真实模拟「插件内部越界/unwrap 失败」
    /// 之后仍能被继续调用的场景。
    fn mode_of(&self, node_id: NodeId) -> PanicOn {
        self.modes
            .read()
            .map(|m| m.get(&node_id).copied().unwrap_or_default())
            .unwrap_or_default()
    }
}

/// 注入 panic 的统一入口：消息里带上阶段名，便于在宿主的错误信息里定位
fn inject(stage: &str) -> ! {
    panic!("injected plugin panic at '{}'", stage);
}

#[async_trait]
impl SouthPlugin for FaultyPlugin {
    fn meta(&self) -> PluginMeta {
        // 加载期故障注入：meta 在拿到节点配置之前就被调用，只能用环境变量触发
        if std::env::var("FAULTY_PLUGIN_PANIC_META")
            .map(|v| v == "1")
            .unwrap_or(false)
        {
            panic!("injected plugin panic at 'meta'");
        }
        PluginMeta {
            name: "faulty",
            kind: PluginKind::South,
            description: Some("故障注入夹具：按配置在指定阶段 panic"),
            version: "0.1.0",
            name_zh: Some("故障注入夹具"),
            name_en: Some("Fault Injector"),
            description_zh: Some("测试夹具：在指定生命周期阶段 panic，验证宿主隔离"),
            description_en: Some("Test fixture that panics on demand to verify host isolation"),
        }
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let mode = PanicOn::from_config(&config);
        // 先注入再登记：模拟「open 失败 → 插件内状态未建立」，宿主应能回滚节点创建
        if mode == PanicOn::Open {
            inject("open");
        }
        if let Ok(mut m) = self.modes.write() {
            m.insert(node_id, mode);
        }
        Ok(())
    }

    async fn close(&self, node_id: NodeId) -> PluginResult<()> {
        if let Ok(mut m) = self.modes.write() {
            m.remove(&node_id);
        }
        Ok(())
    }

    async fn start(&self, node_id: NodeId) -> PluginResult<()> {
        if self.mode_of(node_id) == PanicOn::Start {
            inject("start");
        }
        Ok(())
    }

    async fn poll_group(
        &self,
        node_id: NodeId,
        group_id: GroupId,
        tags: &[Tag],
    ) -> PluginResult<Vec<(TagId, DataValue)>> {
        let _ = group_id;
        match self.mode_of(node_id) {
            PanicOn::Poll => inject("poll_group"),
            // 硬崩溃：模拟段错误/abort 这类进程内无法挽回的故障
            PanicOn::Abort => {
                eprintln!("faulty plugin: aborting process on purpose (poll_group)");
                std::process::abort();
            }
            _ => {}
        }
        Ok(tags.iter().map(|t| (t.id, DataValue::Int32(42))).collect())
    }

    async fn write_tags(&self, node_id: NodeId, values: &[(Tag, DataValue)]) -> PluginResult<()> {
        if self.mode_of(node_id) == PanicOn::Write {
            inject("write_tags");
        }
        let _ = values;
        Ok(())
    }

    async fn list_groups(&self, node_id: NodeId) -> PluginResult<Vec<Group>> {
        let _ = node_id;
        Ok(Vec::new())
    }

    async fn list_tags(&self, node_id: NodeId, group_id: GroupId) -> PluginResult<Vec<Tag>> {
        let _ = (node_id, group_id);
        Ok(Vec::new())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn cfg(value: serde_json::Value) -> PluginConfig {
        let mut m = PluginConfig::new();
        if let serde_json::Value::Object(obj) = value {
            m.extend(obj);
        }
        m
    }

    #[test]
    fn panic_stage_is_parsed_from_config() {
        assert_eq!(PanicOn::from_config(&cfg(json!({}))), PanicOn::None);
        assert_eq!(
            PanicOn::from_config(&cfg(json!({ "panic_on": "poll" }))),
            PanicOn::Poll
        );
        assert_eq!(
            PanicOn::from_config(&cfg(json!({ "panic_on": "write" }))),
            PanicOn::Write
        );
        assert_eq!(
            PanicOn::from_config(&cfg(json!({ "panic_on": "open" }))),
            PanicOn::Open
        );
        assert_eq!(
            PanicOn::from_config(&cfg(json!({ "panic_on": "start" }))),
            PanicOn::Start
        );
    }

    #[test]
    fn abort_stage_is_parsed() {
        assert_eq!(
            PanicOn::from_config(&cfg(json!({ "panic_on": "abort" }))),
            PanicOn::Abort
        );
    }

    #[test]
    fn unknown_stage_falls_back_to_no_injection() {
        assert_eq!(
            PanicOn::from_config(&cfg(json!({ "panic_on": "whatever" }))),
            PanicOn::None
        );
        // 类型不符也不应误触发
        assert_eq!(
            PanicOn::from_config(&cfg(json!({ "panic_on": 7 }))),
            PanicOn::None
        );
    }

    #[tokio::test]
    async fn poll_returns_value_for_every_tag() {
        let p = FaultyPlugin::new();
        let node = NodeId::new();
        p.open(node, PluginConfig::new()).await.unwrap();
        let gid = GroupId::new();
        let tags = vec![Tag::new("t1", "dummy", gid), Tag::new("t2", "dummy", gid)];
        let out = p.poll_group(node, gid, &tags).await.unwrap();
        assert_eq!(out.len(), 2);
    }
}
