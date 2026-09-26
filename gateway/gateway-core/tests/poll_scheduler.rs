//! 采集调度器（`south_scheduler_loop`）行为测试。
//!
//! 这些测试覆盖的是**调度语义**而不是数据正确性：
//! - 每组按自己的周期采集（周期不再被采集耗时累加放大）；
//! - 运行中新增 / 删除组立即生效（旧实现要重启节点才会为新增组建任务）；
//! - 全局并发上限生效，并在批次超期时计数；
//! - 节点停止后不再采集。
//!
//! 用假插件而非真实插件：只有假插件能让「采集耗时」可控，从而稳定地观察并发与超期。

use async_trait::async_trait;
use gateway_core::{Manager, DEFAULT_MAX_CONCURRENT_POLLS};
use gateway_sdk::{
    DataValue, Group, GroupData, GroupId, GroupSubscription, NodeId, NodeKind, PluginConfig,
    PluginError, PluginMeta, PluginResult, SouthPlugin, Tag, TagId,
};
use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;

/// 可观测的假南向插件：记录每个组的采集次数，并跟踪同时在飞的采集数
#[derive(Default)]
struct FakePlugin {
    polls: RwLock<HashMap<GroupId, u64>>,
    inflight: AtomicUsize,
    max_inflight: AtomicUsize,
    /// 每次采集的耗时（毫秒）
    delay_ms: AtomicU64,
}

impl FakePlugin {
    fn new(delay_ms: u64) -> Arc<Self> {
        Arc::new(Self {
            delay_ms: AtomicU64::new(delay_ms),
            ..Default::default()
        })
    }

    async fn count(&self, gid: GroupId) -> u64 {
        self.polls.read().await.get(&gid).copied().unwrap_or(0)
    }

    fn max_inflight(&self) -> usize {
        self.max_inflight.load(Ordering::Relaxed)
    }
}

#[async_trait]
impl SouthPlugin for FakePlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "fake",
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
        node_id: NodeId,
        group_id: GroupId,
        tags: &[Tag],
    ) -> PluginResult<Vec<(TagId, DataValue)>> {
        // 统计同时在飞的采集数，用于验证并发上限
        let now = self.inflight.fetch_add(1, Ordering::SeqCst) + 1;
        self.max_inflight.fetch_max(now, Ordering::SeqCst);

        let delay = self.delay_ms.load(Ordering::Relaxed);
        if delay > 0 {
            tokio::time::sleep(Duration::from_millis(delay)).await;
        }

        {
            let mut m = self.polls.write().await;
            *m.entry(group_id).or_insert(0) += 1;
        }
        self.inflight.fetch_sub(1, Ordering::SeqCst);

        let _ = node_id;
        Ok(tags.iter().map(|t| (t.id, DataValue::Int32(1))).collect())
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

/// 建一个「已注册假插件」的 Manager，返回 (manager, plugin, node_id)
async fn setup(max_polls: usize, delay_ms: u64) -> (Manager, Arc<FakePlugin>, NodeId) {
    let plugin = FakePlugin::new(delay_ms);
    let mut mgr = Manager::with_limits(256, max_polls);
    mgr.register_south("fake", plugin.clone());
    let node = mgr
        .node_create(
            "n1".to_string(),
            NodeKind::South,
            "fake".to_string(),
            PluginConfig::new(),
        )
        .await
        .expect("create node");
    (mgr, plugin, node.id())
}

/// 注册一个空实现的北向插件，用于让总线有订阅者（避免走 bus_no_subscribers 分支）
struct NullNorth;

#[async_trait]
impl gateway_sdk::NorthPlugin for NullNorth {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "null-north",
            kind: gateway_sdk::PluginKind::North,
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

    async fn set_subscriptions(
        &self,
        _node_id: NodeId,
        _subscriptions: &[GroupSubscription],
    ) -> PluginResult<()> {
        Ok(())
    }

    async fn on_group_data(&self, _node_id: NodeId, _data: Arc<GroupData>) -> PluginResult<()> {
        Ok(())
    }
}

/// 让指定节点在 `*_ms` 内不被判定为「完全没跑」——这里只是可读性辅助
async fn tick(ms: u64) {
    tokio::time::sleep(Duration::from_millis(ms)).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn each_group_runs_at_its_own_interval() {
    let (mgr, plugin, nid) = setup(DEFAULT_MAX_CONCURRENT_POLLS, 0).await;
    let fast = Group::new("fast", 50);
    let slow = Group::new("slow", 200);
    let (fast_id, slow_id) = (fast.id, slow.id);
    mgr.group_add(nid, fast).expect("add fast group");
    mgr.group_add(nid, slow).expect("add slow group");

    mgr.node_start(nid).await.expect("start");
    tick(700).await;
    mgr.node_stop(nid).await.expect("stop");

    let fast_count = plugin.count(fast_id).await;
    let slow_count = plugin.count(slow_id).await;
    assert!(
        fast_count >= 6,
        "50ms 周期在 700ms 内应采集 >= 6 次，实际 {}",
        fast_count
    );
    assert!(
        slow_count >= 2,
        "200ms 周期在 700ms 内应采集 >= 2 次，实际 {}",
        slow_count
    );
    assert!(
        fast_count >= slow_count * 2,
        "快组({}) 的次数应明显多于慢组({})",
        fast_count,
        slow_count
    );
    // 固定节拍：若周期被采集耗时累加放大，快组次数会显著偏少
    assert!(
        fast_count <= 20,
        "快组次数 {} 明显超出 700ms/50ms 的合理范围",
        fast_count
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn group_added_while_running_is_picked_up() {
    let (mgr, plugin, nid) = setup(DEFAULT_MAX_CONCURRENT_POLLS, 0).await;
    mgr.node_start(nid).await.expect("start");
    tick(120).await; // 先跑起来，此时没有任何组

    // 运行中新增组：旧实现在 node_start 时就固定了任务集合，新增组不会被采集
    let g = Group::new("added-later", 50);
    let gid = g.id;
    mgr.group_add(nid, g).expect("add group while running");
    tick(400).await;
    mgr.node_stop(nid).await.expect("stop");

    let count = plugin.count(gid).await;
    assert!(
        count >= 3,
        "运行中新增的组应立即被调度，实际采集 {} 次",
        count
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn group_removed_while_running_stops_polling() {
    let (mgr, plugin, nid) = setup(DEFAULT_MAX_CONCURRENT_POLLS, 0).await;
    let g = Group::new("temp", 30);
    let gid = g.id;
    mgr.group_add(nid, g).expect("add group");
    mgr.node_start(nid).await.expect("start");
    tick(300).await;

    mgr.group_remove(nid, gid).await.expect("group exists");
    let after_remove = plugin.count(gid).await;
    tick(300).await;
    let later = plugin.count(gid).await;
    mgr.node_stop(nid).await.expect("stop");

    assert!(
        later <= after_remove + 1,
        "组删除后不应继续采集（删除时 {} → 之后 {}）",
        after_remove,
        later
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn concurrency_is_bounded_and_overruns_are_counted() {
    // 8 个组、周期 10ms、每次采集 60ms、全局并发上限 2：
    // 并发必须被限制在 2，且批次必然超期（记入 south_poll_overrun）
    let (mgr, plugin, nid) = setup(2, 60).await;
    for i in 0..8 {
        mgr.group_add(nid, Group::new(format!("g{}", i), 10))
            .expect("add group");
    }
    mgr.node_start(nid).await.expect("start");
    tick(600).await;
    mgr.node_stop(nid).await.expect("stop");

    assert!(
        plugin.max_inflight() <= 2,
        "同时进行的采集数不得超过全局上限 2，实际峰值 {}",
        plugin.max_inflight()
    );
    let overrun = mgr.data_flow_snapshot().south_poll_overrun;
    assert!(
        overrun > 0,
        "批次耗时 60ms 远大于 10ms 周期，应记录到 south_poll_overrun"
    );
    assert_eq!(
        mgr.data_flow_snapshot().south_poll_err,
        0,
        "假插件不报错，采集错误计数应为 0"
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn node_stop_halts_polling() {
    let (mgr, plugin, nid) = setup(DEFAULT_MAX_CONCURRENT_POLLS, 0).await;
    let g = Group::new("g", 30);
    let gid = g.id;
    mgr.group_add(nid, g).expect("add group");
    mgr.node_start(nid).await.expect("start");
    tick(250).await;

    mgr.node_stop(nid).await.expect("stop");
    let at_stop = plugin.count(gid).await;
    tick(300).await;
    let later = plugin.count(gid).await;

    assert!(
        later <= at_stop + 1,
        "停止后不应再采集（停止时 {} → 之后 {}）",
        at_stop,
        later
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn published_data_reaches_north_subscribers() {
    // 调度器改造后仍应把数据送到总线订阅者：用真实订阅关系端到端验证一次
    let plugin = FakePlugin::new(0);
    let mut mgr = Manager::with_limits(256, DEFAULT_MAX_CONCURRENT_POLLS);
    mgr.register_south("fake", plugin.clone());
    mgr.register_north("null-north", Arc::new(NullNorth));

    let south = mgr
        .node_create(
            "s".to_string(),
            NodeKind::South,
            "fake".to_string(),
            PluginConfig::new(),
        )
        .await
        .expect("south node");
    let g = Group::new("g", 30);
    let gid = g.id;
    mgr.group_add(south.id(), g).expect("group");
    let tag = Tag::new("t", "dummy", gid);
    let tag_id = tag.id;
    mgr.tag_add(south.id(), tag);
    mgr.node_start(south.id()).await.expect("start south");

    let north = mgr
        .node_create(
            "n".to_string(),
            NodeKind::North,
            "null-north".to_string(),
            PluginConfig::new(),
        )
        .await
        .expect("north node");
    mgr.set_north_subscriptions(
        north.id(),
        vec![GroupSubscription {
            south_node_id: south.id(),
            group_id: gid,
        }],
    )
    .await;
    mgr.node_start(north.id()).await.expect("start north");

    // 等到出现「已投递」为止
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    let mut delivered = 0;
    while std::time::Instant::now() < deadline {
        delivered = mgr.data_flow_snapshot().south_published;
        if delivered > 0 {
            break;
        }
        tick(50).await;
    }
    mgr.node_stop(north.id()).await.ok();
    mgr.node_stop(south.id()).await.ok();

    assert!(
        delivered > 0,
        "订阅方存在时总线投递应成功（south_published 应 > 0）"
    );
    assert!(plugin.count(gid).await > 0);
    let _ = tag_id;
}
