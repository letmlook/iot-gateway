//! 多租户数据域隔离（形态 B）：订阅同域过滤的核心防线。
//!
//! 设计依据：docs/design/多租户.md §3.5.3 / §4.2-8。
//! 关键点：订阅表有两条写入路径（API set_north_subscriptions 与快照 apply_snapshot），
//! 共用同一 filter_subscriptions 过滤器。伪造/异地导入的备份走 apply_snapshot 直灌，
//! **API 路由矩阵测试抓不到这一绕过**，本文件是唯一防线验证：
//! - 跨域订阅项（域 A 北向 → 域 B 南向组）必须被滤除；
//! - 引用已删除南向节点的孤儿项必须被滤除；
//! - 同域合法项必须保留。
//!
//! 数据面（总线/插件/FFI）零改动：只要订阅表同域，北向收到的数据天然同域。

use gateway_core::{Manager, Snapshot};
use gateway_sdk::{Group, GroupSubscription, NodeKind, PluginConfig};

fn node_with_tenant(name: &str, kind: NodeKind, tenant: &str) -> gateway_core::Node {
    let mut n = gateway_core::Node::new(name, kind, "fake", PluginConfig::new());
    n.config.tenant_id = tenant.to_string();
    n
}

fn snapshot_with_cross_domain_subs() -> Snapshot {
    let mut snap = Snapshot::default();
    snap.version = gateway_core::SNAPSHOT_VERSION;

    // 域 A：北向节点 + 南向节点 sa（组 ga）
    let mut north = node_with_tenant("north-a", NodeKind::North, "tenant-a");
    let sa = node_with_tenant("south-a", NodeKind::South, "tenant-a");
    let sb = node_with_tenant("south-b", NodeKind::South, "tenant-b");
    let ga = Group::new("ga", 1000);
    let gb = Group::new("gb", 1000);
    snap.nodes = vec![north.clone(), sa.clone(), sb.clone()];
    snap.groups = vec![(sa.config.id, ga.clone()), (sb.config.id, gb.clone())];

    // 订阅表（模拟伪造/异地导入的备份）：
    // 1. north-a → sa/ga  同域合法，必须保留
    // 2. north-a → sb/gb  跨域（A 北向 → B 南向），必须滤除
    // 3. north-a → 孤儿南向节点（不在快照里），必须滤除
    let orphan = gateway_sdk::NodeId::new();
    snap.subscriptions = vec![(
        north.config.id,
        vec![
            GroupSubscription {
                south_node_id: sa.config.id,
                group_id: ga.id,
            },
            GroupSubscription {
                south_node_id: sb.config.id,
                group_id: gb.id,
            },
            GroupSubscription {
                south_node_id: orphan,
                group_id: ga.id,
            },
        ],
    )];
    let _ = &mut north;
    snap
}

/// §4.2-8：apply_snapshot 直灌路径的同域过滤（API 路由矩阵抓不到的 core 层绕过）
#[tokio::test]
async fn apply_snapshot_filters_cross_domain_and_orphan_subscriptions() {
    let mgr = Manager::new();
    let snap = snapshot_with_cross_domain_subs();
    mgr.apply_snapshot(&snap).await;

    let north_id = snap.nodes[0].config.id;
    let subs = mgr.get_north_subscriptions(north_id).await;
    assert_eq!(subs.len(), 1, "只应保留同域合法项，实际: {subs:?}");
    assert_eq!(subs[0].south_node_id, snap.nodes[1].config.id);
    assert_eq!(subs[0].group_id, snap.groups[0].1.id);
}

/// API 路径（set_north_subscriptions）同样被过滤：跨域项进不去
#[tokio::test]
async fn set_subscriptions_filters_cross_domain_items() {
    let mgr = Manager::new();
    let snap = snapshot_with_cross_domain_subs();
    mgr.apply_snapshot(&snap).await;

    let north_id = snap.nodes[0].config.id;
    let sb_id = snap.nodes[2].config.id;
    let gb_id = snap.groups[1].1.id;

    // 经 API 路径塞入跨域项 + 同域项
    mgr.set_north_subscriptions(
        north_id,
        vec![
            GroupSubscription {
                south_node_id: snap.nodes[1].config.id,
                group_id: snap.groups[0].1.id,
            },
            GroupSubscription {
                south_node_id: sb_id,
                group_id: gb_id,
            },
        ],
    )
    .await;

    let subs = mgr.get_north_subscriptions(north_id).await;
    assert_eq!(subs.len(), 1, "跨域项必须被滤除");
    assert_eq!(subs[0].south_node_id, snap.nodes[1].config.id);
}

/// 北向节点本身不存在时：该节点的全部订阅项被丢弃
#[tokio::test]
async fn unknown_north_node_yields_no_subscriptions() {
    let mgr = Manager::new();
    let snap = snapshot_with_cross_domain_subs();
    mgr.apply_snapshot(&snap).await;

    let ghost = gateway_sdk::NodeId::new();
    mgr.set_north_subscriptions(
        ghost,
        vec![GroupSubscription {
            south_node_id: snap.nodes[1].config.id,
            group_id: snap.groups[0].1.id,
        }],
    )
    .await;
    assert!(mgr.get_north_subscriptions(ghost).await.is_empty());
}
