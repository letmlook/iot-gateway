//! 流程运行时（设计 docs/design/流程编排-v2.md §3.1 方案 A）：
//! 每个 published 流程一个总线消费任务——订阅 source 声明的 (south_node, group) 分区通道，
//! 经 `gateway_flow::engine::evaluate` 单拍求值，sink 投递：
//! - `sink_north` → 目标北向节点插件 `on_group_data`（节点须 Running）；
//! - `sink_alarm` → `AlarmStore::raise`（同键 active 去重前移）。
//!
//! 取消语义：任务句柄存于 `tasks`，stop 用 `abort()`（任务体只持总线接收器与预解析数据，
//! 无跨 await 的锁）；改图重新部署 = stop + start。进程重启后由 main 启动序列
//! `start_published` 恢复全部 published 流程。

use std::collections::HashMap;
use std::sync::Arc;

use gateway_flow::{FlowGraph, NodeKind};
use serde::Deserialize;
use tokio::sync::Mutex;
use tokio::task::JoinHandle;
use tracing::{error, info, warn};
use uuid::Uuid;

use crate::flows::FlowRow;
use crate::state::AppState;

#[derive(Debug, Deserialize)]
struct SourceCfg {
    south_node_id: Uuid,
    group_id: Uuid,
}

#[derive(Debug, Deserialize)]
struct SinkNorthCfg {
    node_id: Uuid,
}

/// sink 节点的预解析投递目标。
enum SinkTarget {
    North { north_node_id: Uuid },
    Alarm,
}

#[derive(Default)]
pub struct FlowRuntime {
    tasks: Mutex<HashMap<Uuid, JoinHandle<()>>>,
}

impl FlowRuntime {
    pub fn new() -> Self {
        Self::default()
    }

    /// 启动（或以新图重启）一个流程的消费任务。
    pub async fn start(&self, state: &AppState, row: FlowRow) {
        self.stop(row.id).await;
        let Ok(graph) = serde_json::from_value::<FlowGraph>(row.graph_json.clone()) else {
            warn!(flow_id = ?row.id, "stored graph is malformed, flow not started");
            return;
        };
        let Some(src) = graph.nodes.iter().find(|n| n.kind == NodeKind::Source) else {
            warn!(flow_id = ?row.id, "no source node, flow not started");
            return;
        };
        let Ok(sc) = serde_json::from_value::<SourceCfg>(src.config.clone()) else {
            warn!(flow_id = ?row.id, "source config invalid, flow not started");
            return;
        };
        // 预解析 sink 目标（名称 + 投递方式）
        let mut sinks: HashMap<Uuid, (String, SinkTarget)> = HashMap::new();
        for n in &graph.nodes {
            if n.kind != NodeKind::Sink {
                continue;
            }
            match n.operator.as_deref() {
                Some("sink_north") => {
                    match serde_json::from_value::<SinkNorthCfg>(n.config.clone()) {
                        Ok(c) => {
                            sinks.insert(
                                n.id,
                                (
                                    n.name.clone(),
                                    SinkTarget::North {
                                        north_node_id: c.node_id,
                                    },
                                ),
                            );
                        }
                        Err(e) => {
                            warn!(flow_id = ?row.id, node = %n.name, "sink_north config invalid: {e}")
                        }
                    }
                }
                Some("sink_alarm") => {
                    sinks.insert(n.id, (n.name.clone(), SinkTarget::Alarm));
                }
                _ => {}
            }
        }
        // 断流检测配置与组名（告警 tag 用；查不到回退 group_id 字符串）
        let stale = parse_stale_detection(&src.config);
        let group_name = state
            .manager
            .store
            .group_get(
                gateway_sdk::NodeId(sc.south_node_id),
                gateway_sdk::GroupId(sc.group_id),
            )
            .map(|g| g.name)
            .unwrap_or_else(|| sc.group_id.to_string());
        // flow→flow 禁环（设计 §3.4）：输入只来自南向组分区通道
        let mut rxs = state.manager.bus().subscribe_groups(&[(
            gateway_sdk::NodeId(sc.south_node_id),
            gateway_sdk::GroupId(sc.group_id),
        )]);
        let Some(mut rx) = rxs.pop() else {
            return;
        };
        let engine_flow = gateway_flow::Flow {
            id: row.id,
            tenant_id: row.tenant_id.clone(),
            name: row.name.clone(),
            description: row.description.clone(),
            status: gateway_flow::FlowStatus::Published,
            version: row.version,
            graph,
            created_at: None,
            updated_at: None,
        };
        let manager = state.manager.clone();
        let alarm_store = state.alarm_store.clone();
        let alarm_feed = state.alarm_feed.clone();
        let tenant = row.tenant_id.clone();
        let flow_id = row.id;
        let flow_name = row.name.clone();
        let handle = tokio::spawn(async move {
            let mut flow_state = gateway_flow::FlowState::default();
            // 本批断流告警的全部 raise id（多 sink_alarm 出口各一条；恢复时逐个 resolve）
            let mut stale_alarm_ids: Vec<Uuid> = Vec::new();
            let mut deadline = match &stale {
                Some(s) => {
                    tokio::time::Instant::now() + tokio::time::Duration::from_millis(s.timeout_ms)
                }
                None => {
                    tokio::time::Instant::now() + tokio::time::Duration::from_secs(365 * 24 * 3600)
                }
            };
            let Ok(plan) = gateway_flow::ExecutionPlan::build(&engine_flow) else {
                error!(flow_id = ?flow_id, "execution plan failed, flow task exits");
                return;
            };
            info!(flow_id = ?flow_id, name = %flow_name, "flow task started");
            loop {
                tokio::select! {
                    msg = rx.recv() => {
                        match msg {
                            Ok(data) => {
                                // 数据恢复：自动解除 active 断流告警（spec §4.2）。
                                // 多出口逐条 resolve，单条失败 warn 继续，不中断其余。
                                if !stale_alarm_ids.is_empty() {
                                    let scope =
                                        crate::api::scope::TenantScope::One(tenant.clone());
                                    for id in stale_alarm_ids.drain(..) {
                                        if let Err(e) = alarm_store.resolve(id, &scope) {
                                            warn!(flow_id = ?flow_id, "stale alarm resolve error: {e}");
                                        }
                                    }
                                }
                                let deliveries = gateway_flow::evaluate(
                                    &plan, &engine_flow, &data, &mut flow_state,
                                );
                                match deliveries {
                                    Ok(d) => {
                                        deliver_deliveries(
                                            &sinks, &manager, &alarm_store, &alarm_feed,
                                            &tenant, flow_id, d,
                                        ).await;
                                        if let Some(s) = stale.as_ref() {
                                            deadline = tokio::time::Instant::now()
                                                + tokio::time::Duration::from_millis(s.timeout_ms);
                                        }
                                    }
                                    Err(e) => {
                                        error!(flow_id = ?flow_id, "evaluate error: {e}");
                                    }
                                }
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                                warn!(flow_id = ?flow_id, lagged = n, "flow bus recv lagged, skipped messages");
                            }
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                        }
                    }
                    _ = tokio::time::sleep_until(deadline) => {
                        let Some(s) = stale.clone() else {
                            // M-2（backlog）：stale=None 时 deadline 只是 select 的哨兵；
                            // 触发后必须 re-arm 再 continue，否则 sleep_until 永远立即
                            // 返回，任务退化为忙循环烧 CPU（365 天后爆发，首轮 soak 就
                            // 是跑不满 72h 的活证据）
                            deadline = tokio::time::Instant::now()
                                + tokio::time::Duration::from_secs(365 * 24 * 3600);
                            continue;
                        };
                        let ev = stale_alarm_event(&group_name, s.timeout_ms, &s.level);
                        let stub = gateway_sdk::GroupData {
                            node_id: gateway_sdk::NodeId(sc.south_node_id),
                            group_id: gateway_sdk::GroupId(sc.group_id),
                            ts: chrono::Utc::now(),
                            values: vec![],
                            node_name: None,
                            group_name: None,
                            tag_names: None,
                        };
                        let deliveries = gateway_flow::evaluate_with_inject(
                            &plan, &engine_flow, &stub, &mut flow_state, Some(&ev),
                        );
                        match deliveries {
                            Ok(d) => {
                                let ids = deliver_deliveries(
                                    &sinks, &manager, &alarm_store, &alarm_feed,
                                    &tenant, flow_id, d,
                                ).await;
                                // 记录本批 raise 的全部告警 id（多 sink_alarm 各一条）
                                stale_alarm_ids = ids;
                            }
                            Err(e) => {
                                error!(flow_id = ?flow_id, "stale inject evaluate error: {e}");
                            }
                        }
                        deadline = tokio::time::Instant::now()
                            + tokio::time::Duration::from_millis(s.timeout_ms);
                    }
                }
            }
            info!(flow_id = ?flow_id, name = %flow_name, "flow task stopped");
        });
        self.tasks.lock().await.insert(row.id, handle);
    }

    /// 停止流程任务（不存在则静默）。
    pub async fn stop(&self, flow_id: Uuid) {
        if let Some(handle) = self.tasks.lock().await.remove(&flow_id) {
            handle.abort();
        }
    }

    /// 优雅停机：停止全部流程任务（main.rs 停机序列在停节点之前调用）。
    pub async fn stop_all(&self) {
        let mut tasks = self.tasks.lock().await;
        for (_, handle) in tasks.drain() {
            handle.abort();
        }
    }

    /// 开机恢复：启动全部 published 流程。
    pub async fn start_published(&self, state: &AppState) {
        let Ok(rows) = state.flow_store.list(&crate::api::scope::TenantScope::All) else {
            warn!("flows list failed at startup, no flow started");
            return;
        };
        let n = rows.iter().filter(|r| r.status == "published").count();
        for row in rows.into_iter().filter(|r| r.status == "published") {
            self.start(state, row).await;
        }
        if n > 0 {
            info!("recovered {n} published flow(s)");
        }
    }
}

/// deliveries 分发（数据帧与断流注入共用）。返回本批落库的告警 id（断流路径据此自动 resolve）。
async fn deliver_deliveries(
    sinks: &HashMap<Uuid, (String, SinkTarget)>,
    manager: &Arc<gateway_core::Manager>,
    alarm_store: &Arc<crate::alarms::AlarmStore>,
    alarm_feed: &crate::ws::AlarmFeed,
    tenant: &str,
    flow_id: Uuid,
    deliveries: Vec<gateway_flow::SinkDelivery>,
) -> Vec<Uuid> {
    let mut raised: Vec<Uuid> = Vec::new();
    for d in deliveries {
        let Some((node_name, target)) = sinks.get(&d.node_id) else {
            continue;
        };
        match (target, &d.payload) {
            (SinkTarget::Alarm, gateway_flow::FlowPayload::Alarm(ev)) => {
                match alarm_store.raise(tenant, &flow_id, node_name, ev) {
                    Ok(id) => {
                        raised.push(id);
                        // WS alarm 帧（机会式；REST 为可靠来源）
                        alarm_feed.notify(crate::ws::WsAlarmData {
                            id: id.to_string(),
                            flow_id: flow_id.to_string(),
                            node_name: node_name.clone(),
                            level: ev.level.as_str().into(),
                            tag: ev.tag.clone(),
                            value: ev.value,
                            threshold: ev.threshold,
                            message: ev.message.clone(),
                            ts: ev.ts.timestamp_millis(),
                        });
                    }
                    Err(e) => {
                        error!(flow_id = ?flow_id, "alarm raise error: {e}");
                    }
                }
            }
            (SinkTarget::North { north_node_id }, gateway_flow::FlowPayload::Data(gd)) => {
                deliver_north(manager, *north_node_id, Arc::new(gd.clone())).await;
            }
            _ => {}
        }
    }
    raised
}

/// sink_north 投递：目标北向节点须存在且 Running；失败仅记录（不影响采集主链路）。
async fn deliver_north(
    manager: &Arc<gateway_core::Manager>,
    north_node_id: Uuid,
    gd: Arc<gateway_sdk::GroupData>,
) {
    use gateway_sdk::NodeState;
    let Some(node) = manager.node_get(gateway_sdk::NodeId(north_node_id)) else {
        warn!(north_node_id = ?north_node_id, "sink_north target node not found");
        return;
    };
    if node.state != NodeState::Running {
        return; // 目标未运行：静默跳过（与 north 消费任务「未 Running 不转发」同语义）
    }
    let Some(plugin) = manager.north_plugin(&node.config.plugin_name) else {
        return;
    };
    if let Err(e) = plugin
        .on_group_data(gateway_sdk::NodeId(north_node_id), gd)
        .await
    {
        warn!(north_node_id = ?north_node_id, "sink_north on_group_data error: {e}");
    }
}

/// 断流检测配置（source config 解析结果；spec §4.2）。
#[derive(Debug, Clone)]
pub(crate) struct StaleDetection {
    pub timeout_ms: u64,
    pub level: String,
}

/// stale_timeout_ms 钳制上限（365 天）：`Instant + Duration` 在超亚毫秒级 u64 时
/// 会溢出 panic（M-3，backlog）。365 天与 stale=None 的哨兵 deadline 同数量级，
/// 超过该上限的配置视为配置错误，钳制 + warn（拒绝创建会让已发布 flow 升级失败）。
pub(crate) const MAX_STALE_TIMEOUT_MS: u64 = 365 * 24 * 3600 * 1000;

/// 解析 source config 的断流检测配置；0 或缺省 = None。
pub(crate) fn parse_stale_detection(config: &serde_json::Value) -> Option<StaleDetection> {
    let c: serde_json::Value = config.clone();
    let timeout = c
        .get("stale_timeout_ms")
        .and_then(|v| v.as_u64())
        .unwrap_or(0);
    if timeout == 0 {
        return None;
    }
    if timeout > MAX_STALE_TIMEOUT_MS {
        tracing::warn!(
            timeout,
            cap = MAX_STALE_TIMEOUT_MS,
            "stale_timeout_ms exceeds cap, clamped"
        );
    }
    Some(StaleDetection {
        timeout_ms: timeout.min(MAX_STALE_TIMEOUT_MS),
        level: c
            .get("stale_level")
            .and_then(|v| v.as_str())
            .unwrap_or("high")
            .to_string(),
    })
}

/// 断流告警事件构造（纯函数，便于单测）。level 非法回退 high（校验层已挡，防御性兜底）。
pub(crate) fn stale_alarm_event(
    group_name: &str,
    timeout_ms: u64,
    level: &str,
) -> gateway_flow::AlarmEvent {
    gateway_flow::AlarmEvent {
        level: gateway_flow::AlarmLevel::parse(level).unwrap_or(gateway_flow::AlarmLevel::High),
        tag: group_name.to_string(),
        value: None,
        threshold: Some(timeout_ms as f64),
        message: format!("组 {group_name} 超过 {} 秒未上报数据", timeout_ms / 1000),
        ts: chrono::Utc::now(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stale_alarm_event_shape() {
        let ev = stale_alarm_event("grp1", 5000, "medium");
        assert_eq!(ev.tag, "grp1");
        assert_eq!(ev.level, gateway_flow::AlarmLevel::Medium);
        assert_eq!(ev.value, None);
        assert_eq!(ev.threshold, Some(5000.0));
        assert!(ev.message.contains("组 grp1"), "{}", ev.message);
        assert!(ev.message.contains("5 秒"), "{}", ev.message);
    }

    #[test]
    fn stale_alarm_event_falls_back_level() {
        // 非法 level 回退 high（校验层已挡，此处纯防御）
        let ev = stale_alarm_event("g", 1000, "nope");
        assert_eq!(ev.level, gateway_flow::AlarmLevel::High);
    }

    #[test]
    fn parse_stale_detection_gates_on_zero() {
        let on = parse_stale_detection(&serde_json::json!({
            "south_node_id": uuid::Uuid::new_v4(),
            "group_id": uuid::Uuid::new_v4(),
            "stale_timeout_ms": 5000,
            "stale_level": "high"
        }));
        assert_eq!(on.as_ref().map(|s| s.timeout_ms), Some(5000));
        let off = parse_stale_detection(&serde_json::json!({
            "south_node_id": uuid::Uuid::new_v4(),
            "group_id": uuid::Uuid::new_v4()
        }));
        assert!(off.is_none(), "缺省 0 = 不检测");
    }

    #[test]
    fn stale_timeout_clamped_to_cap() {
        // M-3：超大 stale_timeout_ms 钳制——u64::MAX 会让 Instant+Duration 溢出 panic
        let cfg = serde_json::json!({
            "south_node_id": uuid::Uuid::new_v4(),
            "group_id": uuid::Uuid::new_v4(),
            "stale_timeout_ms": u64::MAX
        });
        let s = parse_stale_detection(&cfg).expect("non-zero timeout must parse");
        assert_eq!(s.timeout_ms, MAX_STALE_TIMEOUT_MS);
        // 正常值不受影响
        let cfg = serde_json::json!({
            "south_node_id": uuid::Uuid::new_v4(),
            "group_id": uuid::Uuid::new_v4(),
            "stale_timeout_ms": 5000
        });
        let s = parse_stale_detection(&cfg).expect("normal timeout must parse");
        assert_eq!(s.timeout_ms, 5000);
    }

    #[tokio::test]
    async fn stale_inject_batch_collects_all_sink_alarm_ids() {
        // 多 sink_alarm 出口（publish 门禁允许 ≥1）：同一注入批次的 raise id 必须
        // 全数返回——AlarmStore 去重键含 node_name，各出口是独立条目，恢复路径
        // 才能逐条 resolve（fix I-1：此前只收 last，其余永久残留 active）。
        let mut sinks: HashMap<Uuid, (String, SinkTarget)> = HashMap::new();
        let sink_a = Uuid::new_v4();
        let sink_b = Uuid::new_v4();
        sinks.insert(sink_a, ("alarm-a".into(), SinkTarget::Alarm));
        sinks.insert(sink_b, ("alarm-b".into(), SinkTarget::Alarm));

        let manager = Arc::new(gateway_core::Manager::new());
        let alarm_store = crate::alarms::AlarmStore::open_best_effort(None);
        let alarm_feed = crate::ws::AlarmFeed::new();
        let flow_id = Uuid::new_v4();
        let ev = gateway_flow::AlarmEvent {
            level: gateway_flow::AlarmLevel::High,
            tag: "grp1".into(),
            value: None,
            threshold: Some(5000.0),
            message: "组 grp1 超过 5 秒未上报数据".into(),
            ts: chrono::Utc::now(),
        };
        let deliveries = vec![
            gateway_flow::SinkDelivery {
                node_id: sink_a,
                payload: gateway_flow::FlowPayload::Alarm(ev.clone()),
            },
            gateway_flow::SinkDelivery {
                node_id: sink_b,
                payload: gateway_flow::FlowPayload::Alarm(ev),
            },
        ];

        let ids = deliver_deliveries(
            &sinks,
            &manager,
            &alarm_store,
            &alarm_feed,
            "t1",
            flow_id,
            deliveries,
        )
        .await;

        assert_eq!(ids.len(), 2, "两个 sink_alarm 各应产出一条 raise id");
        assert_ne!(
            ids[0], ids[1],
            "不同出口 node_name → 不同条目，不得去重合并"
        );
    }
}
