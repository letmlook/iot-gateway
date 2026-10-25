//! 回归测试：锁住本轮 code review 修复的缺陷。
//!
//! 每个用例对应一条「曾经存在、修复后不应复现」的问题，注释里写清触发场景。

use gateway_core::{Manager, Node, Store};
use gateway_sdk::{
    DataType, DataValue, Group, GroupId, GroupSubscription, NodeId, NodeKind, PluginConfig, Tag,
    TagId,
};

// ---------- 1. 删除组/节点必须级联删除点位 ----------

#[test]
fn group_remove_cascades_to_tags() {
    let store = Store::new();
    let node_id = NodeId::new();
    let g = Group::new("g1", 1000);
    let group_id = g.id;
    store.group_insert(node_id, g);
    let tag = Tag::new("t1", "4x!1", group_id);
    store.tag_insert(node_id, tag.clone());

    assert_eq!(store.tags_total_count(), 1);
    store.group_remove(node_id, group_id);

    // 点位必须随之消失：否则授权点位数被永久占用，且该点位仍可被写入设备
    assert_eq!(store.tags_total_count(), 0, "删除组后点位必须级联删除");
    assert!(store.tag_get(tag.id).is_none(), "组删除后点位不应残留");
}

#[test]
fn node_remove_cascades_to_groups_and_tags() {
    let store = Store::new();
    let node_id = NodeId::new();
    let g = Group::new("g1", 1000);
    let group_id = g.id;
    store.node_insert(Node::new(
        "n1",
        NodeKind::South,
        "sim",
        PluginConfig::default(),
    ));
    store.group_insert(node_id, g);
    store.tag_insert(node_id, Tag::new("t1", "4x!1", group_id));

    store.node_remove(node_id);

    assert_eq!(store.tags_total_count(), 0, "删除节点后点位必须级联删除");
    assert!(
        store.group_get(node_id, group_id).is_none(),
        "删除节点后组必须级联删除"
    );
}

// ---------- 2. 跨节点删除点位必须被拒绝 ----------

#[tokio::test]
async fn tag_remove_rejects_tag_of_another_node() {
    let mgr = Manager::new();
    let node_a = NodeId::new();
    let node_b = NodeId::new();
    let gb = Group::new("gb", 1000);
    let group_id = gb.id;

    mgr.store.node_insert(Node::new(
        "A",
        NodeKind::South,
        "sim",
        PluginConfig::default(),
    ));
    mgr.store.node_insert(Node::new(
        "B",
        NodeKind::South,
        "sim",
        PluginConfig::default(),
    ));
    mgr.store.group_insert(node_b, gb);
    let tag_b = Tag::new("tb", "4x!1", group_id);
    mgr.store.tag_insert(node_b, tag_b.clone());

    // 用 A 的路径去删 B 的点位：必须删不掉（B 的点位不能被 A 的接口删掉）
    assert!(
        mgr.tag_remove(node_a, tag_b.id).is_none(),
        "跨节点删除必须被拒绝"
    );
    assert!(mgr.store.tag_get(tag_b.id).is_some(), "B 的点位必须还在");

    // 正确归属则可删
    assert!(mgr.tag_remove(node_b, tag_b.id).is_some());
    assert!(mgr.store.tag_get(tag_b.id).is_none());
}

// ---------- 3. 规则写值必须按点位数据类型装箱 ----------

#[test]
fn rule_write_value_follows_tag_data_type() {
    use gateway_core::rule_value_for_tag;

    let mut coil = Tag::new("alarm", "0x!0", GroupId::new());
    coil.data_type = Some("bool".to_string());
    assert_eq!(
        rule_value_for_tag(&coil, 1.0),
        DataValue::Bool(true),
        "线圈必须写成 Bool，否则写值插件取不到 as_bool"
    );

    let mut i16 = Tag::new("sp", "4x!100", GroupId::new());
    i16.data_type = Some("int16".to_string());
    assert_eq!(
        rule_value_for_tag(&i16, 42.0),
        DataValue::Int16(42),
        "16 位点位必须写成 Int16，否则会按 32 位浮点写 4 个寄存器"
    );

    let mut u16 = Tag::new("reg", "4x!1", GroupId::new());
    u16.data_type = Some("uint16".to_string());
    assert_eq!(rule_value_for_tag(&u16, 1234.0), DataValue::UInt16(1234));

    let mut f64 = Tag::new("temp", "4x!2", GroupId::new());
    f64.data_type = Some("float64".to_string());
    assert_eq!(rule_value_for_tag(&f64, 36.5), DataValue::Float64(36.5));
}

// ---------- 4. 点位必须归属本节点的组 ----------

#[tokio::test]
async fn tag_add_rejects_group_of_another_node() {
    let mgr = Manager::new();
    let node_a = NodeId::new();
    let node_b = NodeId::new();
    let gb = Group::new("gb", 1000);
    let group_b = gb.id;
    mgr.store.node_insert(Node::new(
        "A",
        NodeKind::South,
        "sim",
        PluginConfig::default(),
    ));
    mgr.store.node_insert(Node::new(
        "B",
        NodeKind::South,
        "sim",
        PluginConfig::default(),
    ));
    mgr.store.group_insert(node_b, gb);

    // 把 B 的组挂到 A 的点位上：必须拒绝，否则会产生「看不见也采不到」的幽灵点位
    let mut t = Tag::new("t", "4x!1", group_b);
    t.data_type = Some("uint16".to_string());
    let err = mgr.tag_add_validated(node_a, t).await;
    assert!(err.is_err(), "点位不能挂到别的节点的组上");
}

// ---------- 5. 订阅去重：同一 (南向节点, 组) 不能订阅两次 ----------

#[tokio::test]
async fn duplicate_subscription_is_deduplicated() {
    let mgr = Manager::new();
    let g = Group::new("g", 1000);
    let group = g.id;
    mgr.store.node_insert(Node::new(
        "N",
        NodeKind::North,
        "mqtt",
        PluginConfig::default(),
    ));
    mgr.store.node_insert(Node::new(
        "S",
        NodeKind::South,
        "sim",
        PluginConfig::default(),
    ));
    // Node::new 会自己生成 id：这里取回真实 id 再用
    let north = mgr
        .store
        .nodes_list()
        .into_iter()
        .find(|n| n.config.name == "N")
        .map(|n| n.id())
        .unwrap();
    let south = mgr
        .store
        .nodes_list()
        .into_iter()
        .find(|n| n.config.name == "S")
        .map(|n| n.id())
        .unwrap();
    mgr.store.group_insert(south, g);

    let dup = GroupSubscription {
        south_node_id: south,
        group_id: group,
    };
    mgr.set_north_subscriptions(north, vec![dup.clone(), dup.clone()])
        .await;

    let subs = mgr.get_north_subscriptions(north).await;
    assert_eq!(
        subs.len(),
        1,
        "重复订阅必须去重，否则同一条数据会被转发两次到北向"
    );
}

// ---------- 6. 类型名稳定性（前端写值依赖） ----------

#[test]
fn data_type_names_are_stable() {
    assert_eq!(DataType::UInt16.as_str(), "uint16");
    assert_eq!(DataType::Float64.as_str(), "float64");
    let _ = TagId::new();
}
