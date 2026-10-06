//! 租户域隔离：集中式节点解析入口与域校验。
//!
//! ## 设计原则
//! - 节点级访问**唯一**入口 `resolve_node`，禁止 handler 内直接 `parse_node_id` + `node_get`
//! - 域不匹配与节点不存在**统一返回 404**（防存在性探测）
//! - 列表端点用 `scoped_nodes` / `scoped_rules` 按 ctx 域过滤

use crate::api::ApiError;
use crate::state::AppState;
use crate::users::UserRole;
use gateway_core::Node;

/// 租户 ID 类型
pub type TenantId = String;

/// 内置默认租户
#[allow(dead_code)]
pub const DEFAULT_TENANT: &str = "default";

/// 会话的域作用域
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum TenantScope {
    /// Admin / 静态 token / disable_auth / 未初始化：可访问所有域
    All,
    /// Operator / Viewer：仅可访问其用户所属域
    One(TenantId),
}

impl TenantScope {
    /// Admin 角色恒为 All；Operator/Viewer 为其用户域
    pub fn from_role_and_tenant(role: UserRole, tenant_id: TenantId) -> Self {
        match role {
            UserRole::Admin => TenantScope::All,
            UserRole::Operator | UserRole::Viewer => TenantScope::One(tenant_id),
        }
    }

    /// 判断给定的 tenant_id 是否被 this 域作用域允许访问
    pub fn allows(&self, tenant_id: &str) -> bool {
        match self {
            TenantScope::All => true,
            TenantScope::One(t) => t == tenant_id,
        }
    }
}

/// AuthContext 扩展：携带域信息
#[derive(Clone, Debug)]
pub struct AuthContext {
    pub role: UserRole,
    pub username: Option<String>,
    pub tenant: TenantScope,
}

impl AuthContext {
    /// 从既有的无租户 AuthContext 升级（用于兼容 disable_auth / 静态 token 等注入 All 的场景）
    #[allow(dead_code)]
    pub fn with_tenant(role: UserRole, username: Option<String>, tenant: TenantScope) -> Self {
        Self {
            role,
            username,
            tenant,
        }
    }
}

/// 判断 ctx 是否允许访问 owner 域的资源
pub fn tenant_allows(ctx: &TenantScope, owner: &str) -> bool {
    ctx.allows(owner)
}

/// 集中式节点解析入口：节点不存在或域不匹配均返回 404。
/// 所有 `/nodes/:id/*` 端点必须经由此函数，禁止 handler 内直接 node_get。
pub fn resolve_node(
    state: &AppState,
    ctx: &AuthContext,
    raw: &str,
) -> Result<(gateway_sdk::NodeId, Node), ApiError> {
    let nid = crate::api::handlers::parse_node_id(raw)?;
    let node = state
        .manager
        .node_get(nid)
        .ok_or_else(|| ApiError::not_found("node not found"))?;
    if !tenant_allows(&ctx.tenant, &node.config.tenant_id) {
        return Err(ApiError::not_found("node not found"));
    }
    Ok((nid, node))
}

/// 按 ctx 域过滤节点列表
pub fn scoped_nodes(state: &AppState, ctx: &TenantScope) -> Vec<Node> {
    state
        .manager
        .nodes_list()
        .into_iter()
        .filter(|n| tenant_allows(ctx, &n.config.tenant_id))
        .collect()
}

/// 按 ctx 域过滤规则列表
pub fn scoped_rules(state: &AppState, ctx: &TenantScope) -> Vec<gateway_core::Rule> {
    state
        .manager
        .rules_list()
        .into_iter()
        .filter(|r| tenant_allows(ctx, &r.tenant_id))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tenant_scope_allows() {
        assert!(TenantScope::All.allows("any-tenant"));
        assert!(TenantScope::One("tenant-a".into()).allows("tenant-a"));
        assert!(!TenantScope::One("tenant-a".into()).allows("tenant-b"));
    }

    #[test]
    fn tenant_scope_from_role() {
        assert_eq!(
            TenantScope::from_role_and_tenant(UserRole::Admin, "any".into()),
            TenantScope::All
        );
        assert_eq!(
            TenantScope::from_role_and_tenant(UserRole::Operator, "t1".into()),
            TenantScope::One("t1".into())
        );
        assert_eq!(
            TenantScope::from_role_and_tenant(UserRole::Viewer, "t2".into()),
            TenantScope::One("t2".into())
        );
    }
}
