//! OPC UA 插件运行时状态。

use gateway_sdk::{Group, Tag};

#[allow(dead_code)]
pub struct OpcuaState {
    pub endpoint_url: String,
    pub username: Option<String>,
    pub password: Option<String>,
    pub certificate: Option<String>,
    pub key: Option<String>,
    pub groups: Vec<Group>,
    pub tags: Vec<Tag>,
}
