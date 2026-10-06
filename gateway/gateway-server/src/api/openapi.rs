//! OpenAPI 文档定义（utoipa 生成）。
//!
//! 通过 `export-openapi` bin 直接输出 JSON，不依赖运行时。

use utoipa::OpenApi;

use crate::api::dto::{
    Error, GroupDto, NodeDto, Page, PageParams, RuleDto, RuleRuntime, TagDto, UserDto, ValueDto,
    WsClientFrame, WsServerFrame,
};

#[derive(OpenApi)]
#[openapi(
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
            PageParams,
            Page<NodeDto>,
            Page<GroupDto>,
            Page<TagDto>,
            Page<RuleDto>,
            Page<UserDto>,
            Page<ValueDto>,
            WsClientFrame,
            WsServerFrame,
        )
    ),
    tags(
        (name = "auth", description = "登录/登出"),
        (name = "nodes", description = "节点管理"),
        (name = "groups", description = "组管理"),
        (name = "tags", description = "标签管理"),
        (name = "rules", description = "规则管理"),
        (name = "users", description = "用户管理"),
        (name = "plugins", description = "插件"),
        (name = "system", description = "系统信息"),
    )
)]
pub struct ApiDoc;
