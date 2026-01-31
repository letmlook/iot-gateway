//! 模拟插件状态与默认组/标签。

use gateway_sdk::{Group, GroupId, Tag};
use gateway_sdk::{PluginConfig, TagId};

/// 每个节点占用的“设备”状态
pub struct SimState {
    pub _config: PluginConfig,
    /// 默认组与标签，用于 list_groups / list_tags / poll_group
    pub groups: Vec<Group>,
    pub tags: Vec<Tag>,
}

pub fn default_groups() -> Vec<Group> {
    vec![Group {
        id: GroupId::new(),
        name: "default".to_string(),
        interval_ms: 1000,
        description: Some("默认采集组".to_string()),
    }]
}

pub fn default_tags(group_id: GroupId) -> Vec<Tag> {
    use gateway_sdk::TagAttr;
    let gid = group_id;
    vec![
        Tag {
            id: TagId::new(),
            name: "temperature".to_string(),
            address: "0".to_string(),
            attr: TagAttr::Read,
            data_type: Some("float64".to_string()),
            description: Some("模拟温度".to_string()),
            group_id: gid,
        },
        Tag {
            id: TagId::new(),
            name: "humidity".to_string(),
            address: "1".to_string(),
            attr: TagAttr::Read,
            data_type: Some("float64".to_string()),
            description: Some("模拟湿度".to_string()),
            group_id: gid,
        },
    ]
}
