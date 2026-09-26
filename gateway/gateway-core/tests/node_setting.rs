//! `node_setting`（热改配置）的校验与失败恢复测试。
//!
//! 背景：`node_create` 一直按插件 Schema 校验配置，但热改路径曾绕过校验——
//! 运行期反而成了往库里写非法配置的后门。这里验证三条：
//! 1. 非法配置被拒绝，且**节点原配置保持不变**；
//! 2. 合法配置正常生效；
//! 3. 没有声明 Schema 的插件行为不变（不能因为校验把无 Schema 插件锁死）。

use async_trait::async_trait;
use gateway_core::{Manager, DEFAULT_MAX_CONCURRENT_POLLS};
use gateway_sdk::schema::{ConfigSchema, ParamAttribute, ParamSchema, ParamType, ParamValid};
use gateway_sdk::{
    DataValue, Group, GroupId, NodeId, NodeKind, PluginConfig, PluginError, PluginMeta,
    PluginResult, SouthPlugin, Tag, TagId,
};
use std::sync::Arc;
use tokio::sync::RwLock;

/// 可配置 Schema 的假南向插件：记录最近一次收到的 setting
struct SchemaFake {
    schema: Option<ConfigSchema>,
    last_setting: RwLock<Option<PluginConfig>>,
    calls: std::sync::atomic::AtomicU64,
}

impl SchemaFake {
    fn new(schema: Option<ConfigSchema>) -> Arc<Self> {
        Arc::new(Self {
            schema,
            last_setting: RwLock::new(None),
            calls: std::sync::atomic::AtomicU64::new(0),
        })
    }

    /// 最近一次收到的 setting 配置（用于断言配置确实到达了插件）
    async fn last_setting(&self) -> Option<PluginConfig> {
        self.last_setting.read().await.clone()
    }
}

fn host_schema() -> ConfigSchema {
    ConfigSchema::new()
        .param(ParamSchema {
            name: "host".to_string(),
            name_zh: Some("地址".to_string()),
            name_en: Some("Host".to_string()),
            description: None,
            description_zh: None,
            description_en: None,
            attribute: ParamAttribute::Required,
            ty: ParamType::String,
            default: None,
            valid: Some(ParamValid {
                min: None,
                max: None,
                regex: None,
                length: Some(64),
            }),
            ..Default::default()
        })
        .param(ParamSchema {
            name: "port".to_string(),
            name_zh: Some("端口".to_string()),
            name_en: Some("Port".to_string()),
            description: None,
            description_zh: None,
            description_en: None,
            attribute: ParamAttribute::Optional,
            ty: ParamType::Int,
            default: Some(serde_json::json!(502)),
            valid: Some(ParamValid {
                min: Some(1),
                max: Some(65535),
                regex: None,
                length: None,
            }),
            ..Default::default()
        })
}

#[async_trait]
impl SouthPlugin for SchemaFake {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "schemafake",
            kind: gateway_sdk::PluginKind::South,
            description: None,
            version: "0.0.0",
            name_zh: None,
            name_en: None,
            description_zh: None,
            description_en: None,
        }
    }

    fn config_schema(&self) -> Option<ConfigSchema> {
        self.schema.clone()
    }

    async fn open(&self, _node_id: NodeId, _config: PluginConfig) -> PluginResult<()> {
        Ok(())
    }

    async fn close(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }

    async fn setting(&self, _node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        *self.last_setting.write().await = Some(config);
        self.calls
            .fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        Ok(())
    }

    async fn poll_group(
        &self,
        _node_id: NodeId,
        _group_id: GroupId,
        tags: &[Tag],
    ) -> PluginResult<Vec<(TagId, DataValue)>> {
        Ok(tags.iter().map(|t| (t.id, DataValue::Int32(0))).collect())
    }

    async fn write_tags(&self, _node_id: NodeId, _values: &[(Tag, DataValue)]) -> PluginResult<()> {
        Err(PluginError::not_supported("write not supported"))
    }

    async fn list_groups(&self, _node_id: NodeId) -> PluginResult<Vec<Group>> {
        Ok(Vec::new())
    }

    async fn list_tags(&self, _node_id: NodeId, _group_id: GroupId) -> PluginResult<Vec<Tag>> {
        Ok(Vec::new())
    }
}

fn cfg(pairs: &[(&str, serde_json::Value)]) -> PluginConfig {
    let mut c = PluginConfig::new();
    for (k, v) in pairs {
        c.insert(k.to_string(), v.clone());
    }
    c
}

/// 建 Manager + 已创建节点，返回 (mgr, 插件句柄, node_id)
async fn setup(schema: Option<ConfigSchema>) -> (Manager, Arc<SchemaFake>, NodeId) {
    let plugin = SchemaFake::new(schema);
    let handle = plugin.clone();
    let mut mgr = Manager::with_limits(256, DEFAULT_MAX_CONCURRENT_POLLS);
    mgr.register_south("schemafake", plugin);
    let node = mgr
        .node_create(
            "n1".to_string(),
            NodeKind::South,
            "schemafake".to_string(),
            cfg(&[("host", serde_json::json!("old-host"))]),
        )
        .await
        .expect("create node");
    (mgr, handle, node.id())
}

#[tokio::test]
async fn invalid_setting_is_rejected_and_config_kept() {
    let (mgr, _plugin, nid) = setup(Some(host_schema())).await;

    // 缺少必填项
    let err = mgr
        .node_setting(nid, cfg(&[("port", serde_json::json!(1234))]))
        .await
        .expect_err("missing required field must be rejected");
    assert!(err.contains("host"), "错误应指出缺失字段，实际: {}", err);

    // 原配置必须原样保留（不能被半途写入）
    let node = mgr.node_get(nid).expect("node exists");
    assert_eq!(
        node.config.config.get("host").and_then(|v| v.as_str()),
        Some("old-host"),
        "校验失败后节点配置必须保持原状"
    );

    // 类型错误同样被拒
    let err = mgr
        .node_setting(
            nid,
            cfg(&[
                ("host", serde_json::json!("h")),
                ("port", serde_json::json!("not-a-number")),
            ]),
        )
        .await
        .expect_err("wrong type must be rejected");
    assert!(err.contains("port"), "实际: {}", err);

    // 越界被拒
    let err = mgr
        .node_setting(
            nid,
            cfg(&[
                ("host", serde_json::json!("h")),
                ("port", serde_json::json!(0)),
            ]),
        )
        .await
        .expect_err("out-of-range must be rejected");
    assert!(err.contains("min"), "实际: {}", err);
}

#[tokio::test]
async fn valid_setting_updates_config() {
    let (mgr, plugin, nid) = setup(Some(host_schema())).await;
    mgr.node_setting(
        nid,
        cfg(&[
            ("host", serde_json::json!("new-host")),
            ("port", serde_json::json!(1502)),
        ]),
    )
    .await
    .expect("valid setting must pass");

    let node = mgr.node_get(nid).expect("node exists");
    assert_eq!(
        node.config.config.get("host").and_then(|v| v.as_str()),
        Some("new-host")
    );
    assert_eq!(
        node.config.config.get("port").and_then(|v| v.as_i64()),
        Some(1502)
    );
    // 配置必须真的到达插件（而不只是写进了 Store）
    let last = plugin
        .last_setting()
        .await
        .expect("setting should reach the plugin");
    assert_eq!(last.get("host").and_then(|v| v.as_str()), Some("new-host"));
}

#[tokio::test]
async fn plugin_without_schema_keeps_working() {
    // 无 Schema 的插件：任何配置都放行（回归保护，避免校验把这类插件锁死）
    let (mgr, plugin, nid) = setup(None).await;
    mgr.node_setting(nid, cfg(&[("anything", serde_json::json!(1))]))
        .await
        .expect("plugin without schema must accept arbitrary config");
    let node = mgr.node_get(nid).expect("node exists");
    assert_eq!(
        node.config.config.get("anything").and_then(|v| v.as_i64()),
        Some(1)
    );
    // 配置必须真的到达插件
    let last = plugin
        .last_setting()
        .await
        .expect("setting should reach the plugin");
    assert_eq!(last.get("anything").and_then(|v| v.as_i64()), Some(1));
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn failed_setting_does_not_leave_a_running_node_stopped() {
    // setting 阶段失败的插件：节点原本在运行，失败后应被恢复运行，而不是停在 Stopped
    struct FailOnSetting {
        schema: Option<ConfigSchema>,
    }
    #[async_trait]
    impl SouthPlugin for FailOnSetting {
        fn meta(&self) -> PluginMeta {
            PluginMeta {
                name: "failschema",
                kind: gateway_sdk::PluginKind::South,
                description: None,
                version: "0.0.0",
                name_zh: None,
                name_en: None,
                description_zh: None,
                description_en: None,
            }
        }
        fn config_schema(&self) -> Option<ConfigSchema> {
            self.schema.clone()
        }
        async fn open(&self, _: NodeId, _: PluginConfig) -> PluginResult<()> {
            Ok(())
        }
        async fn close(&self, _: NodeId) -> PluginResult<()> {
            Ok(())
        }
        async fn setting(&self, _: NodeId, _: PluginConfig) -> PluginResult<()> {
            Err(PluginError::msg("device busy"))
        }
        async fn poll_group(
            &self,
            _: NodeId,
            _: GroupId,
            tags: &[Tag],
        ) -> PluginResult<Vec<(TagId, DataValue)>> {
            Ok(tags.iter().map(|t| (t.id, DataValue::Int32(0))).collect())
        }
        async fn list_groups(&self, _: NodeId) -> PluginResult<Vec<Group>> {
            Ok(Vec::new())
        }
        async fn list_tags(&self, _: NodeId, _: GroupId) -> PluginResult<Vec<Tag>> {
            Ok(Vec::new())
        }
    }

    let mut mgr = Manager::with_limits(256, DEFAULT_MAX_CONCURRENT_POLLS);
    mgr.register_south(
        "failschema",
        Arc::new(FailOnSetting {
            schema: Some(host_schema()),
        }),
    );
    let node = mgr
        .node_create(
            "n".to_string(),
            NodeKind::South,
            "failschema".to_string(),
            cfg(&[("host", serde_json::json!("h"))]),
        )
        .await
        .expect("create node");
    let nid = node.id();
    mgr.node_start(nid).await.expect("start");

    let err = mgr
        .node_setting(nid, cfg(&[("host", serde_json::json!("h2"))]))
        .await
        .expect_err("setting failure must surface");
    assert!(err.contains("device busy"), "实际: {}", err);

    let node = mgr.node_get(nid).expect("node exists");
    assert_eq!(
        node.state,
        gateway_sdk::NodeState::Running,
        "setting 失败后节点必须恢复运行，而不是停在 Stopped"
    );
}
