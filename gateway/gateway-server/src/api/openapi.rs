//! OpenAPI 文档定义（utoipa 生成）。
//!
//! paths 在本文件以「文档桩」形式声明：每个 `#[utoipa::path]` 桩对应
//! [`crate::api::mod::router`] 注册的一条路由（含 v1 分页别名路径），方法、路径
//! 参数、查询参数、请求/响应体与鉴权要求以 handlers.rs / handlers_page.rs 的真实
//! 实现为准。桩只服务于文档生成，运行时不调用；`openapi_paths_match_route_table`
//! 测试保证桩集合与路由表一一对应，新增/删除路由时需同步维护本文件。
//!
//! 鉴权模型（见 mod.rs `required_role` / `auth_middleware` 白名单）：
//! - 白名单公开端点（health/metrics/version/license 状态与机器码/auth/login/ws）不标 security；
//! - 其余端点统一 `security(("bearerAuth" = []))`（Bearer token）；
//! - Admin-only（users/tenants/audit/backup/restore/license 上传与重置/system 配置等）
//!   在 description 中注明。
//!
//! 通过 `export-openapi` bin 直接输出 JSON，不依赖运行时；web/ 下 `npm run gen:api`
//! 消费该 JSON 生成前端类型（src/types/api.d.ts）。

#![allow(dead_code)] // 文档桩与请求体 schema 仅参与 OpenAPI 生成，运行时不调用

use utoipa::{OpenApi, ToSchema};

use crate::api::dto::{
    AuditDto, Error, GroupDto, NodeDto, Page, PageParams, RuleDto, RuleRuntime, TagDto, UserDto,
    ValueDto, WsClientFrame, WsServerFrame,
};

// ---------------------------------------------------------------------------
// 请求体 schema（文档桩）：字段与 handlers.rs 中同名请求结构一一对应，仅用于
// OpenAPI 生成。修改 handlers.rs 请求结构时需同步更新此处；外部类型（PluginConfig、
// RuleSource 等）如实以自由对象（Object）表示。
// ---------------------------------------------------------------------------

/// POST /api/auth/login 请求体（handlers::LoginRequest）
#[derive(ToSchema)]
pub struct LoginRequest {
    pub username: Option<String>,
    pub password: Option<String>,
}

/// POST /api/users 请求体（handlers::CreateUserRequest）
#[derive(ToSchema)]
pub struct CreateUserRequest {
    pub username: Option<String>,
    pub password: Option<String>,
    /// admin | operator | viewer（默认 operator）
    pub role: Option<String>,
    /// 所属租户 ID，默认 "default"
    pub tenant: String,
}

/// PUT /api/users/{id} 请求体（handlers::UpdateUserRequest）
#[derive(ToSchema)]
pub struct UpdateUserRequest {
    pub username: Option<String>,
    /// admin | operator | viewer
    pub role: Option<String>,
    /// 变更租户 ID（仅 Admin 可指定）
    pub tenant: Option<String>,
}

/// PUT /api/users/{id}/password 请求体（handlers::ChangePasswordRequest）
#[derive(ToSchema)]
pub struct ChangePasswordRequest {
    pub password: Option<String>,
}

/// POST /api/tenants 请求体（handlers::CreateTenantReq）
#[derive(ToSchema)]
pub struct CreateTenantReq {
    /// 租户 ID（不得为空、不得为内置 "default"）
    pub id: String,
    pub name: String,
}

/// POST /api/backup 请求体（handlers::BackupReq）
#[derive(ToSchema)]
pub struct BackupReq {
    /// 可选备份口令；缺省使用服务端配置密钥
    pub password: Option<String>,
}

/// POST /api/nodes 请求体（handlers::CreateNodeReq）
#[derive(ToSchema)]
pub struct CreateNodeReq {
    pub name: String,
    /// south | north
    pub kind: String,
    pub plugin_name: String,
    /// 插件配置（键值对，随插件 schema 而定）
    #[schema(value_type = Object)]
    pub config: serde_json::Value,
}

/// PUT /api/nodes/{id} 请求体（handlers::UpdateNodeReq）
#[derive(ToSchema)]
pub struct UpdateNodeReq {
    pub name: Option<String>,
}

/// POST /api/nodes/{id}/groups 请求体（handlers::AddGroupReq）
#[derive(ToSchema)]
pub struct AddGroupReq {
    pub name: String,
    /// 采集周期毫秒，下限见 MIN_POLL_INTERVAL_MS
    pub interval_ms: u64,
    pub description: Option<String>,
}

/// PUT /api/nodes/{id}/groups/{gid} 请求体（handlers::UpdateGroupReq）
#[derive(ToSchema)]
pub struct UpdateGroupReq {
    pub name: Option<String>,
    pub interval_ms: Option<u64>,
    /// 传 null 表示清空描述
    pub description: Option<String>,
}

/// POST /api/nodes/{id}/tags 请求体（handlers::AddTagReq）
#[derive(ToSchema)]
pub struct AddTagReq {
    pub name: String,
    /// 协议侧地址，如 Modbus 寄存器地址、OPC UA 节点等
    pub address: String,
    /// 所属组 ID（UUID）
    pub group_id: String,
    /// read | write | readwrite（默认 read）
    pub attr: Option<String>,
    pub data_type: Option<String>,
    pub description: Option<String>,
}

/// POST /api/nodes/{id}/tags/batch 请求体（handlers::BatchAddTagsReq）
#[derive(ToSchema)]
pub struct BatchAddTagsReq {
    pub tags: Vec<AddTagReq>,
}

/// PUT /api/nodes/{id}/tags/{tid} 请求体（handlers::UpdateTagReq）
#[derive(ToSchema)]
pub struct UpdateTagReq {
    pub name: Option<String>,
    pub address: Option<String>,
    /// read | write | readwrite
    pub attr: Option<String>,
    /// 传 null 表示清空 data_type
    pub data_type: Option<String>,
    /// 传 null 表示清空描述
    pub description: Option<String>,
}

/// PUT /api/nodes/{id}/subscriptions 请求体（handlers::SetSubscriptionsReq）
#[derive(ToSchema)]
pub struct SetSubscriptionsReq {
    /// GroupSubscription 数组，元素形如 { "south_node_id": "<uuid>", "group_id": "<uuid>" }
    #[schema(value_type = [Object])]
    pub subscriptions: serde_json::Value,
}

/// PUT /api/nodes/{id}/setting 请求体（handlers::NodeSettingReq）
#[derive(ToSchema)]
pub struct NodeSettingReq {
    /// 插件配置；敏感字段回传 "***" 占位符时保留原值
    #[schema(value_type = Object)]
    pub config: serde_json::Value,
}

/// POST /api/nodes/{id}/read_tags 请求体（handlers::ReadTagsReq）
#[derive(ToSchema)]
pub struct ReadTagsReq {
    pub tag_ids: Vec<String>,
}

/// POST /api/nodes/{id}/write_tags 请求体（handlers::WriteTagsReq）
#[derive(ToSchema)]
pub struct WriteTagsReq {
    /// 元素为 [tagId, value] 二元组（tuple 序列化），如 [["<uuid>", 1.5]]
    #[schema(value_type = [Object])]
    pub values: serde_json::Value,
}

/// PUT /api/logs/config 请求体（handlers::LogConfigReq）
#[derive(ToSchema)]
pub struct LogConfigReq {
    /// trace | debug | info | warn | error（写入配置文件，重启后生效）
    pub level: Option<String>,
    pub upload_enabled: Option<bool>,
}

/// PUT /api/system/config 请求体（handlers::SystemConfigReq）
#[derive(ToSchema)]
pub struct SystemConfigReq {
    /// trace | debug | info | warn | error
    pub log_level: Option<String>,
    pub log_filter: Option<String>,
}

/// POST /api/rules、PUT /api/rules/{id} 请求体（handlers::CreateRuleReq）
#[derive(ToSchema)]
pub struct CreateRuleReq {
    /// 为空时由服务端生成 UUID
    pub id: String,
    pub name: String,
    /// 默认 true；PUT 时以 body 为准
    pub enabled: bool,
    /// RuleSource：{ "south_node_id": "<uuid>", "group_id": "<uuid>", "tag_name": "..." }
    #[schema(value_type = Object)]
    pub source: serde_json::Value,
    /// RuleCondition
    #[schema(value_type = Object)]
    pub condition: serde_json::Value,
    pub for_ms: u64,
    pub clear_ms: u64,
    /// RuleAction
    #[schema(value_type = Object)]
    pub action: serde_json::Value,
}

/// POST /api/rules/{id}/enable 请求体（handlers::EnableRuleReq）
#[derive(ToSchema)]
pub struct EnableRuleReq {
    pub enabled: bool,
}

// ---------------------------------------------------------------------------
// Bearer 鉴权方案
// ---------------------------------------------------------------------------

/// 注册 bearerAuth 安全方案，供各受保护端点 `security(("bearerAuth" = []))` 引用。
struct SecurityAddon;

impl utoipa::Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        use utoipa::openapi::security::{Http, HttpAuthScheme, SecurityScheme};
        openapi
            .components
            .get_or_insert_with(Default::default)
            .add_security_scheme(
                "bearerAuth",
                SecurityScheme::Http(Http::new(HttpAuthScheme::Bearer)),
            );
    }
}

// ---------------------------------------------------------------------------
// 文档桩：与 api/mod.rs router() 注册的路由一一对应（含 v1 分页别名路径）。
// ---------------------------------------------------------------------------

// ---------- 认证 ----------

/// 登录：用户表有账号时校验用户名密码；仅有静态 token 时走遗留 admin 登录；
/// disable_auth 模式返回 token: null。
#[utoipa::path(
    post,
    path = "/api/auth/login",
    tag = "auth",
    request_body(content = LoginRequest, description = "用户名与密码"),
    responses(
        (status = 200, description = "token、expires_at 与 user 信息", body = Object),
        (status = 401, description = "用户名或密码错误", body = Error),
    )
)]
fn login() {}

/// 登出：使当前 Bearer token 立即失效。
#[utoipa::path(
    post,
    path = "/api/auth/logout",
    tag = "auth",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "当前 token 已失效", body = Object),
        (status = 401, description = "未认证", body = Error),
    )
)]
fn logout() {}

// ---------- 健康探针 / 系统 ----------

/// 健康检查：status、节点数、运行中节点数、南/北向插件数。
#[utoipa::path(
    get,
    path = "/api/health",
    tag = "system",
    responses((status = 200, description = "服务状态", body = Object))
)]
fn health() {}

/// Prometheus 格式指标（节点数、数据流链路计数、授权到期天数等）。
#[utoipa::path(
    get,
    path = "/api/metrics",
    tag = "system",
    responses(
        (status = 200, description = "Prometheus 文本", body = String, content_type = "text/plain")
    )
)]
fn metrics() {}

/// 数据流链路监控：各环节计数与点位级统计（按会话租户域过滤）。
#[utoipa::path(
    get,
    path = "/api/data-flow",
    tag = "system",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "metrics / per_tag / troubleshoot", body = Object),
        (status = 401, description = "未认证", body = Error),
    )
)]
fn data_flow() {}

/// 硬件与运行环境信息（CPU/内存/磁盘/主机名等）。
#[utoipa::path(
    get,
    path = "/api/hardware",
    tag = "system",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "硬件信息", body = Object),
        (status = 401, description = "未认证", body = Error),
    )
)]
fn hardware() {}

/// 版本信息：version、build_date、revision。
#[utoipa::path(
    get,
    path = "/api/version",
    tag = "system",
    responses((status = 200, description = "版本信息", body = Object))
)]
fn version() {}

/// 当前日志配置。
#[utoipa::path(
    get,
    path = "/api/logs/config",
    tag = "logs",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "日志配置", body = Object),
        (status = 401, description = "未认证", body = Error),
    )
)]
fn get_log_config() {}

/// 修改日志级别；写入配置文件以便重启后继续生效。
#[utoipa::path(
    put,
    path = "/api/logs/config",
    tag = "logs",
    security(("bearerAuth" = [])),
    request_body(content = LogConfigReq),
    responses(
        (status = 200, description = "生效中的配置（restart_required=true 表示需重启）", body = Object),
        (status = 400, description = "非法日志级别", body = Error),
        (status = 401, description = "未认证", body = Error),
    )
)]
fn put_log_config() {}

/// 日志下载：type=system|driver|node|all（node 需配合 node_id）；One 域禁拉 system/all。
#[utoipa::path(
    get,
    path = "/api/logs/download",
    tag = "logs",
    security(("bearerAuth" = [])),
    params(
        ("type" = String, Query, description = "system | driver | node | all，默认 all"),
        ("node_id" = Option<String>, Query, description = "type=node 时指定节点 ID"),
    ),
    responses(
        (status = 200, description = "日志文本（attachment 下载）", body = String, content_type = "text/plain"),
        (status = 400, description = "非法 type", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 404, description = "日志文件不存在或域越权", body = Error),
    )
)]
fn download_log() {}

/// 当前生效的系统配置（脱敏：token、备份密钥等敏感项不返回明文）。Admin-only。
#[utoipa::path(
    get,
    path = "/api/system/config",
    tag = "system",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "系统配置（Admin-only）", body = Object),
        (status = 401, description = "未认证", body = Error),
        (status = 403, description = "需要 Admin 角色", body = Error),
    )
)]
fn get_system_config() {}

/// 热改安全子集（日志级别/filter）；其余配置需改配置文件后重启。Admin-only。
#[utoipa::path(
    put,
    path = "/api/system/config",
    tag = "system",
    security(("bearerAuth" = [])),
    request_body(content = SystemConfigReq),
    responses(
        (status = 200, description = "更新后的配置（Admin-only）", body = Object),
        (status = 400, description = "非法日志级别", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 403, description = "需要 Admin 角色", body = Error),
    )
)]
fn put_system_config() {}

/// 上传节点配置用文件（如证书）：保存到 data/uploads，返回绝对路径。
#[utoipa::path(
    post,
    path = "/api/upload",
    tag = "system",
    security(("bearerAuth" = [])),
    request_body(
        content = Object,
        content_type = "multipart/form-data",
        description = "multipart 字段 file：待上传文件"
    ),
    responses(
        (status = 200, description = "保存路径 { path }", body = Object),
        (status = 400, description = "缺少文件", body = Error),
        (status = 401, description = "未认证", body = Error),
    )
)]
fn upload_config_file() {}

// ---------- 授权（License） ----------

/// 返回当前设备机器码，供客户发送给管理员生成授权文件。
#[utoipa::path(
    get,
    path = "/api/license/machine-id",
    tag = "license",
    responses((status = 200, description = "机器码 { machineId }", body = Object))
)]
fn license_machine_id() {}

/// 授权状态：是否已加载有效授权、已授权功能/插件列表、点位数限制。
#[utoipa::path(
    get,
    path = "/api/license/status",
    tag = "license",
    responses((status = 200, description = "授权状态", body = Object))
)]
fn license_status() {}

/// 上传授权文件：接收 license.dat（multipart 字段 file），验证后立即生效。Admin-only。
#[utoipa::path(
    post,
    path = "/api/license/upload",
    tag = "license",
    security(("bearerAuth" = [])),
    request_body(
        content = Object,
        content_type = "multipart/form-data",
        description = "multipart 字段 file：license.dat 文件"
    ),
    responses(
        (status = 200, description = "授权激活", body = Object),
        (status = 400, description = "授权校验失败", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 403, description = "需要 Admin 角色", body = Error),
    )
)]
fn upload_license() {}

/// 重置授权：删除授权文件并清除内存授权状态，不可逆。Admin-only。
#[utoipa::path(
    post,
    path = "/api/license/reset",
    tag = "license",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "已重置", body = Object),
        (status = 401, description = "未认证", body = Error),
        (status = 403, description = "需要 Admin 角色", body = Error),
    )
)]
fn reset_license() {}

/// 演示收费功能 pro_tool：仅当授权包含 pro_tool 时可访问。
#[utoipa::path(
    get,
    path = "/api/license/pro-tool",
    tag = "license",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "已授权", body = Object),
        (status = 401, description = "未认证", body = Error),
        (status = 403, description = "需要 pro_tool 授权", body = Error),
    )
)]
fn license_pro_tool() {}

// ---------- 插件 ----------

/// 南向插件列表（含多语言名称/描述、授权状态）。
#[utoipa::path(
    get,
    path = "/api/plugins/south",
    tag = "plugins",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "插件数组", body = [Object]),
        (status = 401, description = "未认证", body = Error),
    )
)]
fn list_south_plugins() {}

/// 南向插件配置 schema。
#[utoipa::path(
    get,
    path = "/api/plugins/south/{name}/config_schema",
    tag = "plugins",
    security(("bearerAuth" = [])),
    params(("name" = String, Path, description = "插件名")),
    responses(
        (status = 200, description = "ConfigSchema", body = Object),
        (status = 401, description = "未认证", body = Error),
        (status = 404, description = "插件不存在或无 schema", body = Error),
    )
)]
fn south_plugin_config_schema() {}

/// 南向插件点位 schema。
#[utoipa::path(
    get,
    path = "/api/plugins/south/{name}/tag_schema",
    tag = "plugins",
    security(("bearerAuth" = [])),
    params(("name" = String, Path, description = "插件名")),
    responses(
        (status = 200, description = "TagSchema", body = Object),
        (status = 401, description = "未认证", body = Error),
        (status = 404, description = "插件不存在或无 schema", body = Error),
    )
)]
fn south_plugin_tag_schema() {}

/// 北向插件列表。
#[utoipa::path(
    get,
    path = "/api/plugins/north",
    tag = "plugins",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "插件数组", body = [Object]),
        (status = 401, description = "未认证", body = Error),
    )
)]
fn list_north_plugins() {}

/// 北向插件配置 schema。
#[utoipa::path(
    get,
    path = "/api/plugins/north/{name}/config_schema",
    tag = "plugins",
    security(("bearerAuth" = [])),
    params(("name" = String, Path, description = "插件名")),
    responses(
        (status = 200, description = "ConfigSchema", body = Object),
        (status = 401, description = "未认证", body = Error),
        (status = 404, description = "插件不存在或无 schema", body = Error),
    )
)]
fn north_plugin_config_schema() {}

// ---------- 备份 / 恢复（Admin） ----------

/// 备份：返回压缩加密的二进制（Content-Disposition attachment）；body 可选 { password }。Admin-only。
#[utoipa::path(
    post,
    path = "/api/backup",
    tag = "backup",
    security(("bearerAuth" = [])),
    request_body(content = BackupReq, description = "可选备份口令（缺省用服务端密钥）"),
    responses(
        (status = 200, description = "加密备份二进制", body = String, content_type = "application/octet-stream"),
        (status = 401, description = "未认证", body = Error),
        (status = 403, description = "需要 Admin 角色", body = Error),
    )
)]
fn backup() {}

/// 恢复：multipart 字段 file（加密备份文件）+ 可选 password。Admin-only。
#[utoipa::path(
    post,
    path = "/api/restore",
    tag = "backup",
    security(("bearerAuth" = [])),
    request_body(
        content = Object,
        content_type = "multipart/form-data",
        description = "multipart 字段：file（备份文件，必填）、password（可选）"
    ),
    responses(
        (status = 200, description = "恢复成功", body = Object),
        (status = 400, description = "备份损坏或版本不支持", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 403, description = "需要 Admin 角色 / License 门禁拒绝", body = Error),
    )
)]
fn restore() {}

// ---------- 用户（Admin） ----------

/// 用户列表（旧路径）：{ users: [...] }，含 tenant_id。Admin-only。
#[utoipa::path(
    get,
    path = "/api/users",
    tag = "users",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "用户数组（Admin-only）", body = Object),
        (status = 401, description = "未认证", body = Error),
        (status = 403, description = "需要 Admin 角色", body = Error),
    )
)]
fn list_users() {}

/// 创建用户。Admin-only。
#[utoipa::path(
    post,
    path = "/api/users",
    tag = "users",
    security(("bearerAuth" = [])),
    request_body(content = CreateUserRequest),
    responses(
        (status = 200, description = "创建后的用户", body = Object),
        (status = 400, description = "用户名/密码非法", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 403, description = "需要 Admin 角色", body = Error),
    )
)]
fn create_user() {}

/// v1 别名路径创建用户（与 POST /api/users 同 handler）。Admin-only。
#[utoipa::path(
    post,
    path = "/api/v1/users",
    tag = "users",
    security(("bearerAuth" = [])),
    request_body(content = CreateUserRequest),
    responses(
        (status = 200, description = "创建后的用户", body = Object),
        (status = 400, description = "用户名/密码非法", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 403, description = "需要 Admin 角色", body = Error),
    )
)]
fn create_user_v1() {}

/// 用户分页列表（v1 别名）。Admin-only。
#[utoipa::path(
    get,
    path = "/api/v1/users",
    tag = "users",
    security(("bearerAuth" = [])),
    params(
        ("page" = u32, Query, description = "页码，从 1 开始"),
        ("page_size" = u32, Query, description = "每页条数，默认 50，clamp [1,1000]"),
    ),
    responses(
        (status = 200, description = "用户分页信封（Admin-only）", body = Page<UserDto>),
        (status = 400, description = "分页参数非法", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 403, description = "需要 Admin 角色", body = Error),
    )
)]
fn list_users_v1() {}

/// 用户详情。
#[utoipa::path(
    get,
    path = "/api/users/{id}",
    tag = "users",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "用户 ID")),
    responses(
        (status = 200, description = "用户信息", body = Object),
        (status = 401, description = "未认证", body = Error),
        (status = 403, description = "需要 Admin 角色", body = Error),
        (status = 404, description = "用户不存在", body = Error),
    )
)]
fn get_user() {}

/// 更新用户（用户名/角色/租户）。
#[utoipa::path(
    put,
    path = "/api/users/{id}",
    tag = "users",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "用户 ID")),
    request_body(content = UpdateUserRequest),
    responses(
        (status = 200, description = "更新后的用户", body = Object),
        (status = 400, description = "参数非法", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 403, description = "需要 Admin 角色", body = Error),
        (status = 404, description = "用户不存在", body = Error),
    )
)]
fn update_user() {}

/// 删除用户。
#[utoipa::path(
    delete,
    path = "/api/users/{id}",
    tag = "users",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "用户 ID")),
    responses(
        (status = 204, description = "已删除"),
        (status = 401, description = "未认证", body = Error),
        (status = 403, description = "需要 Admin 角色", body = Error),
    )
)]
fn delete_user() {}

/// 修改用户密码。
#[utoipa::path(
    put,
    path = "/api/users/{id}/password",
    tag = "users",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "用户 ID")),
    request_body(content = ChangePasswordRequest),
    responses(
        (status = 204, description = "密码已更新"),
        (status = 400, description = "密码非法", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 403, description = "需要 Admin 角色", body = Error),
    )
)]
fn change_password() {}

// ---------- 租户（Admin） ----------

/// 租户列表：Admin 返回全部；非 Admin 会话仅返回自身租户（中间件按 Admin 放行）。
#[utoipa::path(
    get,
    path = "/api/tenants",
    tag = "tenants",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "{ tenants: [{id, name}] }", body = Object),
        (status = 401, description = "未认证", body = Error),
    )
)]
fn list_tenants() {}

/// 创建租户。Admin-only。
#[utoipa::path(
    post,
    path = "/api/tenants",
    tag = "tenants",
    security(("bearerAuth" = [])),
    request_body(content = CreateTenantReq),
    responses(
        (status = 200, description = "{ id, name }", body = Object),
        (status = 400, description = "租户 ID 非法或已存在", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 403, description = "需要 Admin 角色", body = Error),
    )
)]
fn create_tenant() {}

/// 删除租户（租户下有用户时拒绝）。
#[utoipa::path(
    delete,
    path = "/api/tenants/{id}",
    tag = "tenants",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "租户 ID")),
    responses(
        (status = 204, description = "已删除"),
        (status = 400, description = "内置租户不可删 / 租户下仍有用户", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 403, description = "需要 Admin 角色", body = Error),
    )
)]
fn delete_tenant() {}

// ---------- 审计日志（T7，Admin） ----------

/// 审计日志分页查询（T7）：仅 Admin 可用，按 id 倒序（最新在前）。
/// 窗口在 SQL 端完成；page < 1 返回 400，page_size clamp 到 [1, 1000]（默认 50）。
#[utoipa::path(
    get,
    path = "/api/audit",
    tag = "audit",
    security(("bearerAuth" = [])),
    params(
        ("page" = u32, Query, description = "页码，从 1 开始，page < 1 返回 400"),
        ("page_size" = u32, Query, description = "每页条数，默认 50，clamp [1,1000]"),
    ),
    responses(
        (status = 200, description = "审计日志分页信封（items/total/page/pageSize）", body = Page<AuditDto>),
        (status = 400, description = "分页参数非法（如 page=0）", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 403, description = "非 Admin 角色", body = Error),
    )
)]
fn list_audit() {}

// ---------- 节点 ----------

/// 节点列表（旧路径，全量返回）：核心 Node 序列化（snake_case，含 tenant_id），
/// config 已按插件 schema 脱敏；按会话租户域过滤。
#[utoipa::path(
    get,
    path = "/api/nodes",
    tag = "nodes",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "节点数组（snake_case）", body = [Object]),
        (status = 401, description = "未认证", body = Error),
    )
)]
fn list_nodes() {}

/// 创建节点（插件需已授权；租户域由创建者会话盖章）。
#[utoipa::path(
    post,
    path = "/api/nodes",
    tag = "nodes",
    security(("bearerAuth" = [])),
    request_body(content = CreateNodeReq),
    responses(
        (status = 200, description = "创建后的节点", body = Object),
        (status = 400, description = "参数非法", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 403, description = "插件未授权", body = Error),
    )
)]
fn create_node() {}

/// v1 别名路径创建节点（与 POST /api/nodes 同 handler）。
#[utoipa::path(
    post,
    path = "/api/v1/nodes",
    tag = "nodes",
    security(("bearerAuth" = [])),
    request_body(content = CreateNodeReq),
    responses(
        (status = 200, description = "创建后的节点", body = Object),
        (status = 400, description = "参数非法", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 403, description = "插件未授权", body = Error),
    )
)]
fn create_node_v1() {}

/// 节点分页列表（v1 别名，camelCase NodeDto）。
#[utoipa::path(
    get,
    path = "/api/v1/nodes",
    tag = "nodes",
    security(("bearerAuth" = [])),
    params(
        ("page" = u32, Query, description = "页码，从 1 开始"),
        ("page_size" = u32, Query, description = "每页条数，默认 50，clamp [1,1000]"),
        ("q" = Option<String>, Query, description = "按名称子串过滤（大小写不敏感）"),
        ("kind" = Option<String>, Query, description = "south | north"),
    ),
    responses(
        (status = 200, description = "节点分页信封", body = Page<NodeDto>),
        (status = 400, description = "分页参数非法", body = Error),
        (status = 401, description = "未认证", body = Error),
    )
)]
fn list_nodes_v1() {}

/// 节点详情（脱敏 config + connection_status）。
#[utoipa::path(
    get,
    path = "/api/nodes/{id}",
    tag = "nodes",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "节点 ID（UUID）")),
    responses(
        (status = 200, description = "节点信息", body = Object),
        (status = 401, description = "未认证", body = Error),
        (status = 404, description = "节点不存在或跨域（防探测一律 404）", body = Error),
    )
)]
fn get_node() {}

/// 更新节点名称。
#[utoipa::path(
    put,
    path = "/api/nodes/{id}",
    tag = "nodes",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "节点 ID（UUID）")),
    request_body(content = UpdateNodeReq),
    responses(
        (status = 204, description = "已更新"),
        (status = 400, description = "节点 ID 非法", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 404, description = "节点不存在或跨域", body = Error),
    )
)]
fn update_node() {}

/// 删除节点。
#[utoipa::path(
    delete,
    path = "/api/nodes/{id}",
    tag = "nodes",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "节点 ID（UUID）")),
    responses(
        (status = 204, description = "已删除"),
        (status = 400, description = "节点 ID 非法", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 404, description = "节点不存在或跨域", body = Error),
    )
)]
fn delete_node() {}

/// 启动节点。
#[utoipa::path(
    post,
    path = "/api/nodes/{id}/start",
    tag = "nodes",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "节点 ID（UUID）")),
    responses(
        (status = 204, description = "已启动"),
        (status = 400, description = "启动失败", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 404, description = "节点不存在或跨域", body = Error),
    )
)]
fn start_node() {}

/// 停止节点。
#[utoipa::path(
    post,
    path = "/api/nodes/{id}/stop",
    tag = "nodes",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "节点 ID（UUID）")),
    responses(
        (status = 204, description = "已停止"),
        (status = 400, description = "停止失败", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 404, description = "节点不存在或跨域", body = Error),
    )
)]
fn stop_node() {}

/// 节点连接状态：北向由插件上报（如 MQTT）；南向由最近一次采集结果推断。
#[utoipa::path(
    get,
    path = "/api/nodes/{id}/connection-status",
    tag = "nodes",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "节点 ID（UUID）")),
    responses(
        (status = 200, description = "{ connected, last_error }", body = Object),
        (status = 401, description = "未认证", body = Error),
        (status = 404, description = "节点/插件不存在或跨域", body = Error),
    )
)]
fn get_node_connection_status() {}

// ---------- 组 ----------

/// 组列表（旧路径，全量返回；仅南向节点）。
#[utoipa::path(
    get,
    path = "/api/nodes/{id}/groups",
    tag = "groups",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "节点 ID（UUID）")),
    responses(
        (status = 200, description = "组数组（snake_case）", body = [Object]),
        (status = 401, description = "未认证", body = Error),
        (status = 404, description = "节点不存在 / 北向节点无组", body = Error),
    )
)]
fn list_groups() {}

/// 添加组（interval_ms 下限见 MIN_POLL_INTERVAL_MS）。
#[utoipa::path(
    post,
    path = "/api/nodes/{id}/groups",
    tag = "groups",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "节点 ID（UUID）")),
    request_body(content = AddGroupReq),
    responses(
        (status = 200, description = "创建后的组", body = Object),
        (status = 400, description = "参数非法（interval_ms 过小）", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 404, description = "节点不存在 / 北向节点无组", body = Error),
    )
)]
fn add_group() {}

/// 组分页列表（v1 别名，camelCase GroupDto）。
#[utoipa::path(
    get,
    path = "/api/v1/nodes/{id}/groups",
    tag = "groups",
    security(("bearerAuth" = [])),
    params(
        ("id" = String, Path, description = "节点 ID（UUID）"),
        ("page" = u32, Query, description = "页码，从 1 开始"),
        ("page_size" = u32, Query, description = "每页条数，默认 50，clamp [1,1000]"),
        ("q" = Option<String>, Query, description = "按名称子串过滤（大小写不敏感）"),
    ),
    responses(
        (status = 200, description = "组分页信封", body = Page<GroupDto>),
        (status = 400, description = "分页参数非法", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 404, description = "节点不存在 / 北向节点无组", body = Error),
    )
)]
fn list_groups_v1() {}

/// v1 别名路径添加组（与 POST /api/nodes/{id}/groups 同 handler）。
#[utoipa::path(
    post,
    path = "/api/v1/nodes/{id}/groups",
    tag = "groups",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "节点 ID（UUID）")),
    request_body(content = AddGroupReq),
    responses(
        (status = 200, description = "创建后的组", body = Object),
        (status = 400, description = "参数非法", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 404, description = "节点不存在 / 北向节点无组", body = Error),
    )
)]
fn add_group_v1() {}

/// 组详情。
#[utoipa::path(
    get,
    path = "/api/nodes/{id}/groups/{gid}",
    tag = "groups",
    security(("bearerAuth" = [])),
    params(
        ("id" = String, Path, description = "节点 ID（UUID）"),
        ("gid" = String, Path, description = "组 ID（UUID）"),
    ),
    responses(
        (status = 200, description = "组信息", body = Object),
        (status = 401, description = "未认证", body = Error),
        (status = 404, description = "节点或组不存在", body = Error),
    )
)]
fn get_group() {}

/// 更新组（名称/周期/描述）。
#[utoipa::path(
    put,
    path = "/api/nodes/{id}/groups/{gid}",
    tag = "groups",
    security(("bearerAuth" = [])),
    params(
        ("id" = String, Path, description = "节点 ID（UUID）"),
        ("gid" = String, Path, description = "组 ID（UUID）"),
    ),
    request_body(content = UpdateGroupReq),
    responses(
        (status = 204, description = "已更新"),
        (status = 400, description = "参数非法", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 404, description = "节点或组不存在", body = Error),
    )
)]
fn update_group() {}

/// 删除组（级联删除组内标签与策略）。
#[utoipa::path(
    delete,
    path = "/api/nodes/{id}/groups/{gid}",
    tag = "groups",
    security(("bearerAuth" = [])),
    params(
        ("id" = String, Path, description = "节点 ID（UUID）"),
        ("gid" = String, Path, description = "组 ID（UUID）"),
    ),
    responses(
        (status = 204, description = "已删除"),
        (status = 401, description = "未认证", body = Error),
        (status = 404, description = "节点或组不存在", body = Error),
    )
)]
fn remove_group() {}

// ---------- 标签 ----------

/// 标签列表（旧路径，全量返回；仅南向节点）。
#[utoipa::path(
    get,
    path = "/api/nodes/{id}/tags",
    tag = "tags",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "节点 ID（UUID）")),
    responses(
        (status = 200, description = "标签数组（snake_case）", body = [Object]),
        (status = 401, description = "未认证", body = Error),
        (status = 404, description = "节点不存在 / 北向节点无标签", body = Error),
    )
)]
fn list_tags() {}

/// 添加标签（受 License 点位上限约束）。
#[utoipa::path(
    post,
    path = "/api/nodes/{id}/tags",
    tag = "tags",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "节点 ID（UUID）")),
    request_body(content = AddTagReq),
    responses(
        (status = 200, description = "创建后的标签", body = Object),
        (status = 400, description = "参数非法（地址重复等）", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 403, description = "超出点位数上限", body = Error),
        (status = 404, description = "节点不存在 / 北向节点无标签", body = Error),
    )
)]
fn add_tag() {}

/// 标签分页列表（v1 别名，camelCase TagDto）。
#[utoipa::path(
    get,
    path = "/api/v1/nodes/{id}/tags",
    tag = "tags",
    security(("bearerAuth" = [])),
    params(
        ("id" = String, Path, description = "节点 ID（UUID）"),
        ("page" = u32, Query, description = "页码，从 1 开始"),
        ("page_size" = u32, Query, description = "每页条数，默认 50，clamp [1,1000]"),
        ("q" = Option<String>, Query, description = "按名称子串过滤（大小写不敏感）"),
        ("group_id" = Option<String>, Query, description = "按组 ID 过滤"),
    ),
    responses(
        (status = 200, description = "标签分页信封", body = Page<TagDto>),
        (status = 400, description = "分页参数非法", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 404, description = "节点不存在 / 北向节点无标签", body = Error),
    )
)]
fn list_tags_v1() {}

/// v1 别名路径添加标签（与 POST /api/nodes/{id}/tags 同 handler）。
#[utoipa::path(
    post,
    path = "/api/v1/nodes/{id}/tags",
    tag = "tags",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "节点 ID（UUID）")),
    request_body(content = AddTagReq),
    responses(
        (status = 200, description = "创建后的标签", body = Object),
        (status = 400, description = "参数非法", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 403, description = "超出点位数上限", body = Error),
        (status = 404, description = "节点不存在 / 北向节点无标签", body = Error),
    )
)]
fn add_tag_v1() {}

/// 批量添加标签（受 License 点位上限约束）。
#[utoipa::path(
    post,
    path = "/api/nodes/{id}/tags/batch",
    tag = "tags",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "节点 ID（UUID）")),
    request_body(content = BatchAddTagsReq),
    responses(
        (status = 200, description = "创建后的标签数组", body = [Object]),
        (status = 400, description = "参数非法", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 403, description = "超出点位数上限", body = Error),
        (status = 404, description = "节点不存在 / 北向节点无标签", body = Error),
    )
)]
fn batch_add_tags() {}

/// 标签详情。
#[utoipa::path(
    get,
    path = "/api/nodes/{id}/tags/{tid}",
    tag = "tags",
    security(("bearerAuth" = [])),
    params(
        ("id" = String, Path, description = "节点 ID（UUID）"),
        ("tid" = String, Path, description = "标签 ID（UUID）"),
    ),
    responses(
        (status = 200, description = "标签信息", body = Object),
        (status = 401, description = "未认证", body = Error),
        (status = 404, description = "节点或标签不存在", body = Error),
    )
)]
fn get_tag() {}

/// 更新标签。
#[utoipa::path(
    put,
    path = "/api/nodes/{id}/tags/{tid}",
    tag = "tags",
    security(("bearerAuth" = [])),
    params(
        ("id" = String, Path, description = "节点 ID（UUID）"),
        ("tid" = String, Path, description = "标签 ID（UUID）"),
    ),
    request_body(content = UpdateTagReq),
    responses(
        (status = 204, description = "已更新"),
        (status = 400, description = "参数非法", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 404, description = "节点或标签不存在", body = Error),
    )
)]
fn update_tag() {}

/// 删除标签。
#[utoipa::path(
    delete,
    path = "/api/nodes/{id}/tags/{tid}",
    tag = "tags",
    security(("bearerAuth" = [])),
    params(
        ("id" = String, Path, description = "节点 ID（UUID）"),
        ("tid" = String, Path, description = "标签 ID（UUID）"),
    ),
    responses(
        (status = 204, description = "已删除"),
        (status = 401, description = "未认证", body = Error),
        (status = 404, description = "节点或标签不存在", body = Error),
    )
)]
fn remove_tag() {}

// ---------- 订阅（北向） ----------

/// 北向节点订阅列表（仅北向节点）。
#[utoipa::path(
    get,
    path = "/api/nodes/{id}/subscriptions",
    tag = "nodes",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "节点 ID（UUID）")),
    responses(
        (status = 200, description = "GroupSubscription 数组", body = [Object]),
        (status = 401, description = "未认证", body = Error),
        (status = 404, description = "节点不存在 / 南向节点无订阅", body = Error),
    )
)]
fn get_subscriptions() {}

/// 覆盖设置北向订阅。
#[utoipa::path(
    put,
    path = "/api/nodes/{id}/subscriptions",
    tag = "nodes",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "节点 ID（UUID）")),
    request_body(content = SetSubscriptionsReq),
    responses(
        (status = 204, description = "已设置"),
        (status = 401, description = "未认证", body = Error),
        (status = 404, description = "节点不存在 / 南向节点无订阅", body = Error),
    )
)]
fn set_subscriptions() {}

// ---------- 节点设置 / 读写值 ----------

/// 获取节点插件配置（脱敏；仅 config）。
#[utoipa::path(
    get,
    path = "/api/nodes/{id}/setting",
    tag = "nodes",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "节点 ID（UUID）")),
    responses(
        (status = 200, description = "{ config }", body = Object),
        (status = 401, description = "未认证", body = Error),
        (status = 404, description = "节点不存在", body = Error),
    )
)]
fn get_node_setting() {}

/// 更新节点插件配置（敏感字段回传 \"***\" 占位符时保留原值）。
#[utoipa::path(
    put,
    path = "/api/nodes/{id}/setting",
    tag = "nodes",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "节点 ID（UUID）")),
    request_body(content = NodeSettingReq),
    responses(
        (status = 204, description = "已更新"),
        (status = 400, description = "配置非法", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 404, description = "节点不存在", body = Error),
    )
)]
fn put_node_setting() {}

/// 立即读取设备点位值（真实下发，仅南向）。
#[utoipa::path(
    post,
    path = "/api/nodes/{id}/read_tags",
    tag = "tags",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "节点 ID（UUID）")),
    request_body(content = ReadTagsReq),
    responses(
        (status = 200, description = "[[tagId, value], ...] 二元组数组", body = [Object]),
        (status = 400, description = "读取失败", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 404, description = "节点不存在 / 北向节点不可读", body = Error),
    )
)]
fn read_tags() {}

/// 点位实时值（来自采集缓存，不访问设备；旧路径全量返回）。
#[utoipa::path(
    get,
    path = "/api/nodes/{id}/values",
    tag = "nodes",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "节点 ID（UUID）")),
    responses(
        (status = 200, description = "{ node_id, name, source, values }", body = Object),
        (status = 400, description = "节点 ID 非法", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 404, description = "节点不存在", body = Error),
    )
)]
fn node_values() {}

/// 点位实时值分页（v1 别名，camelCase ValueDto；page/page_size 仅做窗口截断）。
#[utoipa::path(
    get,
    path = "/api/v1/nodes/{id}/values",
    tag = "nodes",
    security(("bearerAuth" = [])),
    params(
        ("id" = String, Path, description = "节点 ID（UUID）"),
        ("page" = u32, Query, description = "页码，从 1 开始"),
        ("page_size" = u32, Query, description = "每页条数，默认 50，clamp [1,1000]"),
    ),
    responses(
        (status = 200, description = "实时值分页信封", body = Page<ValueDto>),
        (status = 400, description = "分页参数非法", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 404, description = "节点不存在", body = Error),
    )
)]
fn node_values_v1() {}

/// 写值（对现场设备反控，仅南向；Operator 及以上）。
#[utoipa::path(
    post,
    path = "/api/nodes/{id}/write_tags",
    tag = "tags",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "节点 ID（UUID）")),
    request_body(content = WriteTagsReq),
    responses(
        (status = 204, description = "已下发"),
        (status = 400, description = "写入失败", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 404, description = "节点不存在或跨域", body = Error),
    )
)]
fn write_tags() {}

// ---------- 组数据策略 ----------

/// 获取组数据策略（死区/变化上报/滑动窗口聚合）；未配置时为 null。
#[utoipa::path(
    get,
    path = "/api/nodes/{id}/groups/{gid}/policy",
    tag = "groups",
    security(("bearerAuth" = [])),
    params(
        ("id" = String, Path, description = "节点 ID（UUID）"),
        ("gid" = String, Path, description = "组 ID（UUID）"),
    ),
    responses(
        (status = 200, description = "GroupPolicy（未配置为 null）", body = Object),
        (status = 401, description = "未认证", body = Error),
        (status = 404, description = "节点不存在 / 北向节点无策略", body = Error),
    )
)]
fn get_policy() {}

/// 设置组数据策略（校验 tag 在组内；emit_ms < interval_ms 时仅告警）。
#[utoipa::path(
    put,
    path = "/api/nodes/{id}/groups/{gid}/policy",
    tag = "groups",
    security(("bearerAuth" = [])),
    params(
        ("id" = String, Path, description = "节点 ID（UUID）"),
        ("gid" = String, Path, description = "组 ID（UUID）"),
    ),
    request_body(content = Object, description = "GroupPolicy（south_node_id/group_id 以路径参数为准）"),
    responses(
        (status = 200, description = "生效的策略", body = Object),
        (status = 400, description = "策略非法 / tag 不在组内", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 404, description = "节点不存在 / 北向节点无策略", body = Error),
    )
)]
fn put_policy() {}

/// 删除组数据策略。
#[utoipa::path(
    delete,
    path = "/api/nodes/{id}/groups/{gid}/policy",
    tag = "groups",
    security(("bearerAuth" = [])),
    params(
        ("id" = String, Path, description = "节点 ID（UUID）"),
        ("gid" = String, Path, description = "组 ID（UUID）"),
    ),
    responses(
        (status = 204, description = "已删除"),
        (status = 401, description = "未认证", body = Error),
        (status = 404, description = "节点或策略不存在", body = Error),
    )
)]
fn delete_policy() {}

// ---------- 历史数据 ----------

/// 历史序列查询：SQL 侧分桶降采样，点数不超过 max_points（默认 500，上限 5000）。
#[utoipa::path(
    get,
    path = "/api/history/series",
    tag = "history",
    security(("bearerAuth" = [])),
    params(
        ("node_id" = String, Query, description = "节点 ID（UUID）"),
        ("group_id" = String, Query, description = "组 ID（UUID）"),
        ("tag" = String, Query, description = "标签名"),
        ("from" = Option<i64>, Query, description = "起始时间毫秒（默认 1 小时前）"),
        ("to" = Option<i64>, Query, description = "结束时间毫秒（默认当前）"),
        ("max_points" = Option<u32>, Query, description = "期望最大点数，默认 500，clamp [1,5000]"),
    ),
    responses(
        (status = 200, description = "序列与降采样点集", body = Object),
        (status = 400, description = "参数非法 / 历史存储未启用", body = Error),
        (status = 401, description = "未认证", body = Error),
    )
)]
fn history_series() {}

/// 库中有哪些序列（供前端选择；limit 默认 200，clamp [1,2000]）。
#[utoipa::path(
    get,
    path = "/api/history/series/list",
    tag = "history",
    security(("bearerAuth" = [])),
    params(("limit" = Option<u32>, Query, description = "返回条数上限，默认 200")),
    responses(
        (status = 200, description = "{ series: [...] }", body = Object),
        (status = 400, description = "查询失败", body = Error),
        (status = 401, description = "未认证", body = Error),
    )
)]
fn history_series_list() {}

/// 历史存储统计：行数、磁盘占用、时间范围与配置。
#[utoipa::path(
    get,
    path = "/api/history/stats",
    tag = "history",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "统计信息", body = Object),
        (status = 401, description = "未认证", body = Error),
    )
)]
fn history_stats() {}

// ---------- 规则（读 Viewer+，写 Admin） ----------

/// 规则列表（旧路径）：RuleView（配置 + 运行期状态），snake_case。
#[utoipa::path(
    get,
    path = "/api/rules",
    tag = "rules",
    security(("bearerAuth" = [])),
    responses(
        (status = 200, description = "规则视图数组", body = [Object]),
        (status = 401, description = "未认证", body = Error),
    )
)]
fn list_rules() {}

/// 新建规则（规则域以 source 节点域盖章；写操作 Admin）。
#[utoipa::path(
    post,
    path = "/api/rules",
    tag = "rules",
    security(("bearerAuth" = [])),
    request_body(content = CreateRuleReq),
    responses(
        (status = 201, description = "创建后的规则", body = Object),
        (status = 400, description = "校验失败 / id 已存在", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 403, description = "需要 Admin 角色", body = Error),
    )
)]
fn create_rule() {}

/// 更新规则（id 取自路径；tenant_id 不可更改；写操作 Admin）。
#[utoipa::path(
    put,
    path = "/api/rules/{id}",
    tag = "rules",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "规则 ID")),
    request_body(content = CreateRuleReq),
    responses(
        (status = 200, description = "更新后的规则", body = Object),
        (status = 400, description = "校验失败", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 403, description = "需要 Admin 角色", body = Error),
        (status = 404, description = "规则不存在", body = Error),
    )
)]
fn update_rule() {}

/// 删除规则（写操作 Admin）。
#[utoipa::path(
    delete,
    path = "/api/rules/{id}",
    tag = "rules",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "规则 ID")),
    responses(
        (status = 204, description = "已删除"),
        (status = 401, description = "未认证", body = Error),
        (status = 403, description = "需要 Admin 角色", body = Error),
        (status = 404, description = "规则不存在", body = Error),
    )
)]
fn delete_rule() {}

/// 启用 / 停用规则（写操作 Admin）。
#[utoipa::path(
    post,
    path = "/api/rules/{id}/enable",
    tag = "rules",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "规则 ID")),
    request_body(content = EnableRuleReq),
    responses(
        (status = 200, description = "更新后的规则", body = Object),
        (status = 400, description = "body 非法", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 403, description = "需要 Admin 角色", body = Error),
        (status = 404, description = "规则不存在", body = Error),
    )
)]
fn enable_rule() {}

/// 规则分页列表（v1 别名，camelCase RuleDto）。
#[utoipa::path(
    get,
    path = "/api/v1/rules",
    tag = "rules",
    security(("bearerAuth" = [])),
    params(
        ("page" = u32, Query, description = "页码，从 1 开始"),
        ("page_size" = u32, Query, description = "每页条数，默认 50，clamp [1,1000]"),
    ),
    responses(
        (status = 200, description = "规则分页信封", body = Page<RuleDto>),
        (status = 400, description = "分页参数非法", body = Error),
        (status = 401, description = "未认证", body = Error),
    )
)]
fn list_rules_v1() {}

/// v1 别名路径新建规则（与 POST /api/rules 同 handler，201）。
#[utoipa::path(
    post,
    path = "/api/v1/rules",
    tag = "rules",
    security(("bearerAuth" = [])),
    request_body(content = CreateRuleReq),
    responses(
        (status = 201, description = "创建后的规则", body = Object),
        (status = 400, description = "校验失败 / id 已存在", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 403, description = "需要 Admin 角色", body = Error),
    )
)]
fn create_rule_v1() {}

/// v1 别名路径更新规则。
#[utoipa::path(
    put,
    path = "/api/v1/rules/{id}",
    tag = "rules",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "规则 ID")),
    request_body(content = CreateRuleReq),
    responses(
        (status = 200, description = "更新后的规则", body = Object),
        (status = 400, description = "校验失败", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 403, description = "需要 Admin 角色", body = Error),
        (status = 404, description = "规则不存在", body = Error),
    )
)]
fn update_rule_v1() {}

/// v1 别名路径删除规则。
#[utoipa::path(
    delete,
    path = "/api/v1/rules/{id}",
    tag = "rules",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "规则 ID")),
    responses(
        (status = 204, description = "已删除"),
        (status = 401, description = "未认证", body = Error),
        (status = 403, description = "需要 Admin 角色", body = Error),
        (status = 404, description = "规则不存在", body = Error),
    )
)]
fn delete_rule_v1() {}

/// v1 别名路径启用 / 停用规则。
#[utoipa::path(
    post,
    path = "/api/v1/rules/{id}/enable",
    tag = "rules",
    security(("bearerAuth" = [])),
    params(("id" = String, Path, description = "规则 ID")),
    request_body(content = EnableRuleReq),
    responses(
        (status = 200, description = "更新后的规则", body = Object),
        (status = 400, description = "body 非法", body = Error),
        (status = 401, description = "未认证", body = Error),
        (status = 403, description = "需要 Admin 角色", body = Error),
        (status = 404, description = "规则不存在", body = Error),
    )
)]
fn enable_rule_v1() {}

// ---------- WebSocket ----------

/// WebSocket 实时订阅（GET 升级）：HTTP 层在白名单中，升级成功后首帧需发送
/// auth 帧（token），帧格式见 WsClientFrame（客户端 → 服务器）与
/// WsServerFrame（服务器 → 客户端）组件。
#[utoipa::path(
    get,
    path = "/api/v1/ws",
    tag = "ws",
    responses(
        (status = 101, description = "协议升级为 WebSocket，后续按 WsClientFrame/WsServerFrame 通信")
    )
)]
fn ws_entry() {}

// ---------------------------------------------------------------------------
// OpenAPI 汇总
// ---------------------------------------------------------------------------

#[derive(OpenApi)]
#[openapi(
    paths(
        login,
        logout,
        health,
        metrics,
        data_flow,
        hardware,
        version,
        get_log_config,
        put_log_config,
        download_log,
        get_system_config,
        put_system_config,
        upload_config_file,
        license_machine_id,
        license_status,
        upload_license,
        reset_license,
        license_pro_tool,
        list_south_plugins,
        south_plugin_config_schema,
        south_plugin_tag_schema,
        list_north_plugins,
        north_plugin_config_schema,
        backup,
        restore,
        list_users,
        create_user,
        create_user_v1,
        list_users_v1,
        get_user,
        update_user,
        delete_user,
        change_password,
        list_tenants,
        create_tenant,
        delete_tenant,
        list_audit,
        list_nodes,
        create_node,
        create_node_v1,
        list_nodes_v1,
        get_node,
        update_node,
        delete_node,
        start_node,
        stop_node,
        get_node_connection_status,
        list_groups,
        add_group,
        list_groups_v1,
        add_group_v1,
        get_group,
        update_group,
        remove_group,
        list_tags,
        add_tag,
        list_tags_v1,
        add_tag_v1,
        batch_add_tags,
        get_tag,
        update_tag,
        remove_tag,
        get_subscriptions,
        set_subscriptions,
        get_node_setting,
        put_node_setting,
        read_tags,
        node_values,
        node_values_v1,
        write_tags,
        get_policy,
        put_policy,
        delete_policy,
        history_series,
        history_series_list,
        history_stats,
        list_rules,
        create_rule,
        update_rule,
        delete_rule,
        enable_rule,
        list_rules_v1,
        create_rule_v1,
        update_rule_v1,
        delete_rule_v1,
        enable_rule_v1,
        ws_entry,
    ),
    components(
        schemas(
            crate::api::error::ApiErrorBody,
            Error,
            NodeDto,
            GroupDto,
            TagDto,
            RuleDto,
            RuleRuntime,
            UserDto,
            ValueDto,
            AuditDto,
            PageParams,
            Page<NodeDto>,
            Page<GroupDto>,
            Page<TagDto>,
            Page<RuleDto>,
            Page<UserDto>,
            Page<ValueDto>,
            Page<AuditDto>,
            WsClientFrame,
            WsServerFrame,
            // 请求体 schema（文档桩，见文件顶部说明）
            LoginRequest,
            CreateUserRequest,
            UpdateUserRequest,
            ChangePasswordRequest,
            CreateTenantReq,
            BackupReq,
            CreateNodeReq,
            UpdateNodeReq,
            AddGroupReq,
            UpdateGroupReq,
            AddTagReq,
            BatchAddTagsReq,
            UpdateTagReq,
            SetSubscriptionsReq,
            NodeSettingReq,
            ReadTagsReq,
            WriteTagsReq,
            LogConfigReq,
            SystemConfigReq,
            CreateRuleReq,
            EnableRuleReq,
        )
    ),
    modifiers(&SecurityAddon),
    tags(
        (name = "auth", description = "登录/登出"),
        (name = "system", description = "系统信息与配置、上传"),
        (name = "license", description = "授权文件管理"),
        (name = "logs", description = "日志管理"),
        (name = "plugins", description = "插件"),
        (name = "users", description = "用户管理（Admin）"),
        (name = "tenants", description = "租户管理（Admin）"),
        (name = "audit", description = "审计日志查询（T7，Admin）"),
        (name = "nodes", description = "节点管理、订阅与实时值"),
        (name = "groups", description = "组管理与组数据策略"),
        (name = "tags", description = "标签管理与读写值"),
        (name = "rules", description = "规则管理"),
        (name = "history", description = "历史数据查询"),
        (name = "backup", description = "备份与恢复（Admin）"),
        (name = "ws", description = "WebSocket 实时订阅"),
    )
)]
pub struct ApiDoc;

// ---------------------------------------------------------------------------
// 测试：文档桩集合与路由表一一对应 + /api/audit 文档完整性
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    /// 路由表快照：与 api/mod.rs `router()` 中注册的路由一一对应，格式
    /// "<METHOD> <公开路径>"（服务经 main.rs nest("/api") 挂载后的最终形状）。
    /// 新增/删除路由时：同步维护 openapi.rs 文档桩与本列表（两处都改，测试才会通过）。
    const ROUTE_TABLE: &[&str] = &[
        "POST /api/auth/login",
        "POST /api/auth/logout",
        "GET /api/health",
        "GET /api/metrics",
        "GET /api/data-flow",
        "GET /api/hardware",
        "GET /api/version",
        "GET /api/logs/config",
        "PUT /api/logs/config",
        "GET /api/logs/download",
        "GET /api/system/config",
        "PUT /api/system/config",
        "GET /api/license/machine-id",
        "GET /api/license/status",
        "POST /api/license/upload",
        "POST /api/license/reset",
        "GET /api/license/pro-tool",
        "GET /api/plugins/south",
        "GET /api/plugins/south/{name}/config_schema",
        "GET /api/plugins/south/{name}/tag_schema",
        "GET /api/plugins/north",
        "GET /api/plugins/north/{name}/config_schema",
        "POST /api/backup",
        "POST /api/restore",
        "GET /api/users",
        "POST /api/users",
        "GET /api/v1/users",
        "POST /api/v1/users",
        "GET /api/users/{id}",
        "PUT /api/users/{id}",
        "DELETE /api/users/{id}",
        "PUT /api/users/{id}/password",
        "GET /api/tenants",
        "POST /api/tenants",
        "DELETE /api/tenants/{id}",
        "GET /api/audit",
        "GET /api/nodes",
        "POST /api/nodes",
        "GET /api/v1/nodes",
        "POST /api/v1/nodes",
        "GET /api/nodes/{id}",
        "PUT /api/nodes/{id}",
        "DELETE /api/nodes/{id}",
        "POST /api/nodes/{id}/start",
        "POST /api/nodes/{id}/stop",
        "GET /api/nodes/{id}/connection-status",
        "GET /api/nodes/{id}/groups",
        "POST /api/nodes/{id}/groups",
        "GET /api/v1/nodes/{id}/groups",
        "POST /api/v1/nodes/{id}/groups",
        "GET /api/nodes/{id}/groups/{gid}",
        "PUT /api/nodes/{id}/groups/{gid}",
        "DELETE /api/nodes/{id}/groups/{gid}",
        "GET /api/nodes/{id}/tags",
        "POST /api/nodes/{id}/tags",
        "GET /api/v1/nodes/{id}/tags",
        "POST /api/v1/nodes/{id}/tags",
        "POST /api/nodes/{id}/tags/batch",
        "GET /api/nodes/{id}/tags/{tid}",
        "PUT /api/nodes/{id}/tags/{tid}",
        "DELETE /api/nodes/{id}/tags/{tid}",
        "GET /api/nodes/{id}/subscriptions",
        "PUT /api/nodes/{id}/subscriptions",
        "GET /api/nodes/{id}/setting",
        "PUT /api/nodes/{id}/setting",
        "POST /api/nodes/{id}/read_tags",
        "GET /api/nodes/{id}/values",
        "GET /api/v1/nodes/{id}/values",
        "POST /api/nodes/{id}/write_tags",
        "GET /api/nodes/{id}/groups/{gid}/policy",
        "PUT /api/nodes/{id}/groups/{gid}/policy",
        "DELETE /api/nodes/{id}/groups/{gid}/policy",
        "GET /api/history/series",
        "GET /api/history/series/list",
        "GET /api/history/stats",
        "POST /api/upload",
        "GET /api/rules",
        "POST /api/rules",
        "PUT /api/rules/{id}",
        "DELETE /api/rules/{id}",
        "POST /api/rules/{id}/enable",
        "GET /api/v1/rules",
        "POST /api/v1/rules",
        "PUT /api/v1/rules/{id}",
        "DELETE /api/v1/rules/{id}",
        "POST /api/v1/rules/{id}/enable",
        "GET /api/v1/ws",
    ];

    /// 从导出的 OpenAPI JSON 提取 "METHOD path" 操作集合
    fn exported_operations(json: &serde_json::Value) -> Vec<String> {
        let paths = json["paths"].as_object().expect("paths object");
        let mut ops = Vec::new();
        for (path, item) in paths {
            for method in ["get", "put", "post", "delete", "patch", "head", "options"] {
                if item.get(method).is_some() {
                    ops.push(format!("{} {}", method.to_uppercase(), path));
                }
            }
        }
        ops.sort();
        ops
    }

    /// 导出的 OpenAPI paths 必须与路由表完全一致：不多、不少、方法不差。
    #[test]
    fn openapi_paths_match_route_table() {
        let doc = ApiDoc::openapi();
        let json = serde_json::to_value(&doc).expect("serialize OpenAPI");

        let exported = exported_operations(&json);
        assert!(
            !exported.is_empty(),
            "导出的 OpenAPI 必须包含 paths（路由级文档）"
        );

        let mut expected: Vec<String> = ROUTE_TABLE.iter().map(|s| s.to_string()).collect();
        expected.sort();

        assert_eq!(
            exported, expected,
            "文档桩与 router() 路由表出现分歧：新增/删除路由时需同步维护 openapi.rs"
        );
    }

    /// /api/audit（T7）：必须存在、Admin 鉴权（bearerAuth）、分页参数与
    /// Page<AuditDto> 响应结构齐全，且组件真实注册。
    #[test]
    fn audit_endpoint_is_admin_paged_documentation() {
        let doc = ApiDoc::openapi();
        let json = serde_json::to_value(&doc).expect("serialize OpenAPI");

        // GET /api/audit 存在且仅有 GET
        let get = &json["paths"]["/api/audit"]["get"];
        assert!(get.is_object(), "/api/audit 必须在 paths 中且有 GET 操作");
        assert!(
            json["paths"]["/api/audit"].as_object().unwrap().len() == 1,
            "/api/audit 只注册了 GET"
        );

        // 鉴权要求：security 数组中含 bearerAuth
        let security = get["security"].as_array().expect("security requirement");
        assert!(
            security.iter().any(|s| s
                .as_object()
                .map(|o| o.contains_key("bearerAuth"))
                .unwrap_or(false)),
            "审计查询必须标注 bearerAuth 鉴权: {security:?}"
        );

        // 分页参数：page / page_size
        let params = get["parameters"].as_array().expect("query parameters");
        let names: Vec<&str> = params.iter().filter_map(|p| p["name"].as_str()).collect();
        assert!(
            names.contains(&"page") && names.contains(&"page_size"),
            "审计查询必须文档化 page/page_size 分页参数: {names:?}"
        );
        assert!(
            params.iter().all(|p| p["in"] == "query"),
            "审计查询参数必须都是 query 参数: {names:?}"
        );

        // 响应结构：200 -> Page<AuditDto> 组件引用；错误码 -> Error 组件
        let ok_ref = get["responses"]["200"]["content"]["application/json"]["schema"]["$ref"]
            .as_str()
            .expect("200 响应必须是 Page<AuditDto> 组件引用");
        assert!(
            ok_ref.ends_with("/Page_AuditDto"),
            "审计 200 响应应引用 Page_AuditDto，实际 {ok_ref}"
        );
        for status in ["400", "401", "403"] {
            let err_ref = get["responses"][status]["content"]["application/json"]["schema"]["$ref"]
                .as_str()
                .unwrap_or_default();
            assert!(
                err_ref.ends_with("/Error"),
                "{status} 响应应引用 Error 组件，实际 {err_ref:?}"
            );
        }

        // 组件真实注册：Page_AuditDto / AuditDto
        let schemas = json["components"]["schemas"]
            .as_object()
            .expect("components.schemas");
        assert!(
            schemas.contains_key("Page_AuditDto") && schemas.contains_key("AuditDto"),
            "组件中必须注册 Page_AuditDto 与 AuditDto: {:?}",
            schemas.keys().collect::<Vec<_>>()
        );
        // bearerAuth 安全方案已注册
        assert!(
            json["components"]["securitySchemes"]["bearerAuth"].is_object(),
            "components.securitySchemes 必须注册 bearerAuth"
        );
    }
}
