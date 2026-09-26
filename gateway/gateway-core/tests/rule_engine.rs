//! 规则引擎端到端测试：真实采集路径 → 条件判断 → 动作执行。
//!
//! 与 `rules.rs` 的单元测试互补：那里测状态机语义（给定时间序列），
//! 这里测**接进采集链路之后**的行为——该触发时触发、不该触发时不动手、动作失败不影响采集。

use async_trait::async_trait;
use gateway_core::{
    CompareOp, Manager, Rule, RuleAction, RuleCondition, RuleSource, DEFAULT_MAX_CONCURRENT_POLLS,
};
use gateway_sdk::{
    DataValue, Group, GroupId, NodeId, NodeKind, PluginConfig, PluginError, PluginMeta,
    PluginResult, SouthPlugin, Tag, TagId,
};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{Mutex, RwLock};

/// 假南向插件：返回值可控，并记录收到的写值调用
struct RuleFake {
    value: RwLock<f64>,
    writes: Mutex<Vec<(String, f64)>>,
    polls: AtomicU64,
}

impl RuleFake {
    fn new(value: f64) -> Arc<Self> {
        Arc::new(Self {
            value: RwLock::new(value),
            writes: Mutex::new(Vec::new()),
            polls: AtomicU64::new(0),
        })
    }

    async fn set_value(&self, v: f64) {
        *self.value.write().await = v;
    }

    async fn writes(&self) -> Vec<(String, f64)> {
        self.writes.lock().await.clone()
    }

    fn write_count(&self) -> usize {
        self.writes.try_lock().map(|w| w.len()).unwrap_or(0)
    }

    fn polls(&self) -> u64 {
        self.polls.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl SouthPlugin for RuleFake {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "rulefake",
            kind: gateway_sdk::PluginKind::South,
            description: None,
            version: "0.0.0",
            name_zh: None,
            name_en: None,
            description_zh: None,
            description_en: None,
        }
    }

    async fn open(&self, _node_id: NodeId, _config: PluginConfig) -> PluginResult<()> {
        Ok(())
    }

    async fn close(&self, _node_id: NodeId) -> PluginResult<()> {
        Ok(())
    }

    async fn poll_group(
        &self,
        _node_id: NodeId,
        _group_id: GroupId,
        tags: &[Tag],
    ) -> PluginResult<Vec<(TagId, DataValue)>> {
        self.polls.fetch_add(1, Ordering::Relaxed);
        let v = *self.value.read().await;
        Ok(tags.iter().map(|t| (t.id, DataValue::Float64(v))).collect())
    }

    async fn write_tags(&self, _node_id: NodeId, values: &[(Tag, DataValue)]) -> PluginResult<()> {
        let mut w = self.writes.lock().await;
        for (tag, v) in values {
            match v.as_f64() {
                Some(f) => w.push((tag.name.clone(), f)),
                None => return Err(PluginError::msg("rule action wrote a non-numeric value")),
            }
        }
        Ok(())
    }

    async fn list_groups(&self, _node_id: NodeId) -> PluginResult<Vec<Group>> {
        Ok(Vec::new())
    }

    async fn list_tags(&self, _node_id: NodeId, _group_id: GroupId) -> PluginResult<Vec<Tag>> {
        Ok(Vec::new())
    }
}

struct Env {
    mgr: Manager,
    plugin: Arc<RuleFake>,
    node: NodeId,
    group: GroupId,
}

/// 南向节点 + 一个组 + 两个点位（temperature / heater）
async fn setup(node_name: &str, initial: f64) -> Env {
    let plugin = RuleFake::new(initial);
    let mut mgr = Manager::with_limits(256, DEFAULT_MAX_CONCURRENT_POLLS);
    mgr.register_south("rulefake", plugin.clone());
    let node = mgr
        .node_create(
            node_name.to_string(),
            NodeKind::South,
            "rulefake".to_string(),
            PluginConfig::new(),
        )
        .await
        .expect("create node");
    let node_id = node.id();
    let g = Group::new("g", 30);
    let group = g.id;
    mgr.group_add(node_id, g).expect("add group");
    for name in ["temperature", "heater"] {
        mgr.tag_add(node_id, Tag::new(name, "dummy", group));
    }
    Env {
        mgr,
        plugin,
        node: node_id,
        group,
    }
}

fn rule(id: &str, env: &Env, op: CompareOp, threshold: f64, value: f64) -> Rule {
    Rule {
        id: id.to_string(),
        name: format!("rule-{}", id),
        enabled: true,
        source: RuleSource {
            south_node_id: env.node,
            group_id: env.group,
            tag_name: "temperature".to_string(),
        },
        condition: RuleCondition { op, threshold },
        for_ms: 0,
        clear_ms: 0,
        action: RuleAction::WriteTag {
            tag_name: "heater".to_string(),
            value,
        },
    }
}

async fn wait_for<F: Fn() -> bool>(cond: F, timeout: Duration) -> bool {
    let deadline = Instant::now() + timeout;
    while Instant::now() < deadline {
        if cond() {
            return true;
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    cond()
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rule_fires_and_writes_when_threshold_is_crossed() {
    let env = setup("r-hit", 90.0).await;
    env.mgr
        .rule_insert(rule("t-hit", &env, CompareOp::Gt, 80.0, 1.0));
    env.mgr.node_start(env.node).await.expect("start");

    let plugin = env.plugin.clone();
    let fired = wait_for(|| plugin.write_count() > 0, Duration::from_secs(5)).await;
    env.mgr.node_stop(env.node).await.ok();

    assert!(fired, "温度越过阈值后规则应触发");
    let writes = env.plugin.writes().await;
    assert!(
        writes
            .iter()
            .any(|(n, v)| n == "heater" && (*v - 1.0).abs() < 1e-9),
        "动作应把 heater 写成 1.0，实际 {:?}",
        writes
    );
    assert!(env.plugin.polls() > 0, "触发应来自真实采集链路");
    assert!(
        env.mgr.data_flow_snapshot().rules_fired >= 1,
        "指标应记录触发次数"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rule_does_not_fire_below_threshold() {
    let env = setup("r-miss", 20.0).await;
    env.mgr
        .rule_insert(rule("t-miss", &env, CompareOp::Gt, 80.0, 1.0));
    env.mgr.node_start(env.node).await.expect("start");

    let plugin = env.plugin.clone();
    wait_for(|| plugin.polls() >= 5, Duration::from_secs(5)).await;
    env.mgr.node_stop(env.node).await.ok();

    assert!(env.plugin.writes().await.is_empty(), "未越限不应有动作");
    let snap = env.mgr.data_flow_snapshot();
    assert_eq!(snap.rules_fired, 0);
    assert_eq!(snap.rules_action_err, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rule_fires_once_per_rising_edge() {
    let env = setup("r-edge", 90.0).await;
    env.mgr
        .rule_insert(rule("t-edge", &env, CompareOp::Gt, 80.0, 1.0));
    env.mgr.node_start(env.node).await.expect("start");

    let plugin = env.plugin.clone();
    wait_for(|| plugin.write_count() > 0, Duration::from_secs(5)).await;
    let after_first = plugin.write_count();

    // 持续满足：不得重复写值（对 PLC 重复写是灾难）
    tokio::time::sleep(Duration::from_millis(300)).await;
    let while_holding = plugin.write_count();

    // 恢复到阈值以下，再越限 → 应再次触发
    plugin.set_value(10.0).await;
    tokio::time::sleep(Duration::from_millis(150)).await;
    plugin.set_value(95.0).await;
    wait_for(
        || plugin.write_count() > while_holding,
        Duration::from_secs(5),
    )
    .await;
    let after_second = plugin.write_count();
    env.mgr.node_stop(env.node).await.ok();

    assert_eq!(after_first, 1, "第一次越限只触发一次");
    assert_eq!(
        while_holding,
        after_first,
        "条件持续满足期间不得重复触发（实际 {:?}）",
        plugin.writes().await
    );
    assert!(
        after_second > while_holding,
        "恢复后再次越限应再次触发（{} → {}）",
        while_holding,
        after_second
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn disabled_rule_never_acts() {
    let env = setup("r-off", 90.0).await;
    let mut r = rule("t-off", &env, CompareOp::Gt, 80.0, 1.0);
    r.enabled = false;
    env.mgr.rule_insert(r);
    env.mgr.node_start(env.node).await.expect("start");

    let plugin = env.plugin.clone();
    wait_for(|| plugin.polls() >= 5, Duration::from_secs(5)).await;
    env.mgr.node_stop(env.node).await.ok();

    assert!(env.plugin.writes().await.is_empty());
    assert_eq!(env.mgr.data_flow_snapshot().rules_fired, 0);
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn action_failure_is_counted_and_polling_continues() {
    let env = setup("r-err", 90.0).await;
    // 目标点位不存在 → 动作失败，但采集必须继续
    let mut r = rule("t-err", &env, CompareOp::Gt, 80.0, 1.0);
    r.action = RuleAction::WriteTag {
        tag_name: "no-such-tag".to_string(),
        value: 1.0,
    };
    env.mgr.rule_insert(r);
    env.mgr.node_start(env.node).await.expect("start");

    wait_for(
        || env.mgr.data_flow_snapshot().rules_action_err >= 1,
        Duration::from_secs(5),
    )
    .await;
    let polls_then = env.plugin.polls();
    tokio::time::sleep(Duration::from_millis(150)).await;
    let polls_later = env.plugin.polls();
    env.mgr.node_stop(env.node).await.ok();

    assert!(polls_later > polls_then, "动作失败后采集应继续推进");
    assert!(env.plugin.writes().await.is_empty());
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn rules_survive_snapshot_roundtrip() {
    let env = setup("r-persist", 90.0).await;
    let mut r = rule("t-persist", &env, CompareOp::Le, 42.0, 7.0);
    r.for_ms = 100;
    r.clear_ms = 200;
    env.mgr.rule_insert(r.clone());

    let snap = env.mgr.build_snapshot().await;
    assert_eq!(
        snap.rules.len(),
        1,
        "规则必须进入快照（备份/导出才会带上它）"
    );
    assert_eq!(snap.rules[0].condition.op, CompareOp::Le);
    assert_eq!(snap.rules[0].for_ms, 100);
    assert_eq!(snap.rules[0].action, r.action);

    // 磁盘往返
    let dir = std::env::temp_dir().join(format!("gw-rule-{}", uuid::Uuid::new_v4()));
    std::fs::create_dir_all(&dir).unwrap();
    let db = dir.join("data.db");
    gateway_core::persist_save(&db, &snap).await.expect("save");
    let loaded = gateway_core::persist_load(&db)
        .await
        .expect("load")
        .expect("snapshot");
    assert_eq!(loaded.rules.len(), 1);
    assert_eq!(loaded.rules[0].name, r.name);
    assert_eq!(loaded.rules[0].action, r.action);
    assert_eq!(loaded.rules[0].source.tag_name, "temperature");

    let _ = std::fs::remove_dir_all(&dir);
}
