//! v1 分页 handler：复用现有取数逻辑，追加 filter → sort → window → 计数。
//!
//! 旧 handler 一行不改；分页只发生在 v1 别名路径上。

use axum::extract::{Extension, Path, Query, State};
use axum::Json;
use gateway_sdk::NodeKind;

use crate::api::dto::{GroupDto, NodeDto, Page, PageParams, RuleDto, TagDto, UserDto, ValueDto};
use crate::api::{ApiError, AuthContext};
use crate::state::AppState;

// ---------------------------------------------------------------------------
// Nodes
// ---------------------------------------------------------------------------

/// GET /api/v1/nodes?page=1&page_size=50&kind=south&q=plc
pub async fn list_nodes_v1(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthContext>,
    Query(p): Query<PageParams>,
) -> Result<Json<Page<NodeDto>>, ApiError> {
    p.validate_page()?;
    let page_size = p.clamped_page_size();
    let offset = p.offset();

    // 取全量（DashMap 迭代）后过滤+排序
    let all_nodes = crate::api::scope::scoped_nodes(&state, &ctx.tenant);

    let filtered: Vec<_> = all_nodes
        .iter()
        .filter(|n| {
            // kind 过滤
            if let Some(ref k) = p.kind {
                let kind_str = if n.kind() == NodeKind::North {
                    "north"
                } else {
                    "south"
                };
                if kind_str != k.as_str() {
                    return false;
                }
            }
            // q 过滤（name 子串，大小写不敏感）
            if let Some(ref q) = p.q {
                if !n.config.name.to_lowercase().contains(&q.to_lowercase()) {
                    return false;
                }
            }
            true
        })
        .collect();

    let total = filtered.len() as u64;

    // 按 name 升序、同名按 id 升序稳定排序（DashMap 迭代无序，必须排序才能分页）
    let mut sorted = filtered;
    sorted.sort_by(|a, b| {
        a.config
            .name
            .cmp(&b.config.name)
            .then_with(|| a.id().0.cmp(&b.id().0))
    });

    let page_nodes: Vec<(gateway_core::Node, serde_json::Value)> = sorted
        .into_iter()
        .skip(offset)
        .take(page_size as usize)
        .map(|node| {
            let mut json = serde_json::to_value(node).unwrap_or_default();
            if let Some(obj) = json.as_object_mut() {
                if let Some(cfg) = obj.get_mut("config") {
                    let masked = crate::api::handlers::mask_plugin_config(
                        &state,
                        &node.config.plugin_name,
                        node.kind() == gateway_sdk::NodeKind::North,
                        cfg,
                    );
                    *cfg = masked;
                }
            }
            (node.clone(), json)
        })
        .collect();

    // 逐个获取 connection_status（North 返回 serde_json::Value，South 返回 SouthConnectionStatus）
    let mut nodes = Vec::with_capacity(page_nodes.len());
    for (node, mut json) in page_nodes {
        if let Some(obj) = json.as_object_mut() {
            if node.kind() == gateway_sdk::NodeKind::North {
                if let Some(plugin) = state.manager.north_plugin(&node.config.plugin_name) {
                    if let Some(conn) = plugin.connection_status(node.id()).await {
                        obj.insert("connection_status".to_string(), conn);
                    }
                }
            } else if node.kind() == gateway_sdk::NodeKind::South
                && node.state == gateway_sdk::NodeState::Running
            {
                if let Some(conn) = state.manager.south_connection_status(node.id()).await {
                    obj.insert(
                        "connection_status".to_string(),
                        serde_json::to_value(&conn).unwrap_or_else(|_| {
                            serde_json::json!({ "connected": conn.connected, "last_error": conn.last_error })
                        }),
                    );
                }
            }
        }
        nodes.push(NodeDto::from_enriched_json(&node, json));
    }

    Ok(Json(Page::new(nodes, total, p.page, page_size)))
}

// ---------------------------------------------------------------------------
// Groups
// ---------------------------------------------------------------------------

/// GET /api/v1/nodes/:id/groups?page=1&page_size=50&q=temp
pub async fn list_groups_v1(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    Query(p): Query<PageParams>,
) -> Result<Json<Page<GroupDto>>, ApiError> {
    p.validate_page()?;
    let page_size = p.clamped_page_size();
    let offset = p.offset();

    let nid = crate::api::handlers::parse_node_id(&node_id)?;
    crate::api::handlers::ensure_node_south(&state, nid)?;

    let all_groups = state.manager.groups_by_node(nid);

    // 过滤
    let filtered: Vec<_> = all_groups
        .iter()
        .filter(|g| {
            if let Some(ref q) = p.q {
                if !g.name.to_lowercase().contains(&q.to_lowercase()) {
                    return false;
                }
            }
            true
        })
        .collect();

    let total = filtered.len() as u64;

    // 排序
    let mut sorted = filtered;
    sorted.sort_by(|a, b| a.name.cmp(&b.name).then_with(|| a.id.0.cmp(&b.id.0)));

    let page_groups: Vec<GroupDto> = sorted
        .into_iter()
        .skip(offset)
        .take(page_size as usize)
        .map(GroupDto::from)
        .collect();

    Ok(Json(Page::new(page_groups, total, p.page, page_size)))
}

// ---------------------------------------------------------------------------
// Tags
// ---------------------------------------------------------------------------

/// GET /api/v1/nodes/:id/tags?page=1&page_size=50&group_id=xxx&q=temp
pub async fn list_tags_v1(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    Query(p): Query<PageParams>,
) -> Result<Json<Page<TagDto>>, ApiError> {
    p.validate_page()?;
    let page_size = p.clamped_page_size();
    let offset = p.offset();

    let nid = crate::api::handlers::parse_node_id(&node_id)?;
    crate::api::handlers::ensure_node_south(&state, nid)?;

    let all_tags: Vec<_> = state
        .manager
        .groups_by_node(nid)
        .into_iter()
        .flat_map(|g| {
            state
                .manager
                .tags_by_group(nid, g.id)
                .into_iter()
                .map(move |t| (g.id, t))
        })
        .collect();

    // 过滤
    let filtered: Vec<_> = all_tags
        .iter()
        .filter(|(_gid, t)| {
            if let Some(ref q) = p.q {
                if !t.name.to_lowercase().contains(&q.to_lowercase()) {
                    return false;
                }
            }
            if let Some(ref filter_gid) = p.group_id {
                if t.group_id.0.to_string() != *filter_gid {
                    return false;
                }
            }
            true
        })
        .collect();

    let total = filtered.len() as u64;

    // 排序
    let mut sorted = filtered;
    sorted.sort_by(|(_, a), (_, b)| a.name.cmp(&b.name).then_with(|| a.id.0.cmp(&b.id.0)));

    let page_tags: Vec<TagDto> = sorted
        .into_iter()
        .skip(offset)
        .take(page_size as usize)
        .map(|(_, t)| TagDto::from(t))
        .collect();

    Ok(Json(Page::new(page_tags, total, p.page, page_size)))
}

// ---------------------------------------------------------------------------
// Rules
// ---------------------------------------------------------------------------

/// GET /api/v1/rules?page=1&page_size=50
pub async fn list_rules_v1(
    State(state): State<AppState>,
    Extension(ctx): Extension<AuthContext>,
    Query(p): Query<PageParams>,
) -> Result<Json<Page<RuleDto>>, ApiError> {
    p.validate_page()?;
    let page_size = p.clamped_page_size();
    let offset = p.offset();

    let engine = gateway_core::rule_engine();
    let all_rules: Vec<_> = crate::api::scope::scoped_rules(&state, &ctx.tenant)
        .into_iter()
        .map(|rule| gateway_core::RuleView {
            runtime: engine.runtime(&rule.id),
            rule,
        })
        .collect();

    let total = all_rules.len() as u64;

    // rules_list 内部已排序（name+id），但为明确可见性再排一次
    let mut sorted = all_rules;
    sorted.sort_by(|a, b| {
        a.rule
            .name
            .cmp(&b.rule.name)
            .then_with(|| a.rule.id.cmp(&b.rule.id))
    });

    let page_rules: Vec<RuleDto> = sorted
        .into_iter()
        .skip(offset)
        .take(page_size as usize)
        .map(|view| RuleDto::from_view(&view))
        .collect();

    Ok(Json(Page::new(page_rules, total, p.page, page_size)))
}

// ---------------------------------------------------------------------------
// Users
// ---------------------------------------------------------------------------

/// GET /api/v1/users?page=1&page_size=50
pub async fn list_users_v1(
    State(state): State<AppState>,
    Query(p): Query<PageParams>,
) -> Result<Json<Page<UserDto>>, ApiError> {
    p.validate_page()?;
    let page_size = p.clamped_page_size();
    let offset = p.offset();

    let all_users = state.user_store.list().await.map_err(ApiError::internal)?;

    let total = all_users.len() as u64;

    // 按 username 升序、id 升序排序
    let mut sorted = all_users;
    sorted.sort_by(|a, b| a.username.cmp(&b.username).then_with(|| a.id.cmp(&b.id)));

    let page_users: Vec<UserDto> = sorted
        .into_iter()
        .skip(offset)
        .take(page_size as usize)
        .map(|u| UserDto::from(&u))
        .collect();

    Ok(Json(Page::new(page_users, total, p.page, page_size)))
}

// ---------------------------------------------------------------------------
// Node values（不过滤/排序，全量返回，仅分页）
// ---------------------------------------------------------------------------

/// GET /api/v1/nodes/:id/values?page=1&page_size=50
/// 注意：values 与其他分页端点不同，page/page_size 仅做窗口截断，
/// 不额外做过滤/搜索。
pub async fn node_values_v1(
    State(state): State<AppState>,
    Path(node_id): Path<String>,
    Query(p): Query<PageParams>,
) -> Result<Json<Page<ValueDto>>, ApiError> {
    p.validate_page()?;
    let page_size = p.clamped_page_size();
    let offset = p.offset();

    let nid = crate::api::handlers::parse_node_id(&node_id)?;
    let node = state
        .manager
        .node_get(nid)
        .ok_or_else(|| ApiError::not_found("node not found"))?;
    let cached = state.manager.last_values(nid);
    let tags = state.manager.tags_by_node(nid);

    let all_values: Vec<ValueDto> = tags
        .iter()
        .map(|t| {
            let lv = cached.get(&t.id);
            ValueDto {
                tag_id: t.id.0.to_string(),
                name: t.name.clone(),
                group_id: t.group_id.0.to_string(),
                value: lv
                    .map(|l| serde_json::to_value(&l.value).unwrap_or_default())
                    .unwrap_or_default(),
                ts: lv.map(|l| l.ts_ms),
                available: lv.is_some(),
            }
        })
        .collect();

    let total = all_values.len() as u64;

    // values 不排序，直接窗口截断
    let page_values: Vec<ValueDto> = all_values
        .into_iter()
        .skip(offset)
        .take(page_size as usize)
        .collect();

    // 保持节点信息在响应中
    // （values 端点特殊性：不归入统一信封，由 node_id/name/source 包裹）
    // 实际返回值用 Page<ValueDto>
    let _ = node; // silence unused warning
    Ok(Json(Page::new(page_values, total, p.page, page_size)))
}
