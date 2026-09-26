//! MQTT 主题模板与上报 payload 格式化（values_format / tags_format / ecp_format / group_data / raw_data）。

use gateway_sdk::types::DataValue;
use gateway_sdk::{GroupData, GroupId, NodeId, TagId};

use crate::config::{
    UPLOAD_FORMAT_ECP_FORMAT, UPLOAD_FORMAT_RAW_DATA, UPLOAD_FORMAT_TAGS_FORMAT,
    UPLOAD_FORMAT_VALUES_FORMAT,
};

/// 从主题模板生成主题。支持变量：${node_id} ${group_id} ${node_name} ${group_name} ${timestamp}；有名称时用名称，否则用 id
pub fn topic_from_template(
    template: &str,
    node_id: NodeId,
    group_id: GroupId,
    node_name: Option<&str>,
    group_name: Option<&str>,
    ts: chrono::DateTime<chrono::Utc>,
) -> String {
    let node_id_s = node_id.0.to_string();
    let group_id_s = group_id.0.to_string();
    let node_s = node_name.unwrap_or(&node_id_s);
    let group_s = group_name.unwrap_or(&group_id_s);
    let timestamp_s = ts.to_rfc3339();
    template
        .replace("${node_id}", node_s)
        .replace("${group_id}", group_s)
        .replace("${node_name}", node_s)
        .replace("${group_name}", group_s)
        .replace("${timestamp}", &timestamp_s)
}

/// ECP 类型：1=布尔 2=整型 3=浮点 4=字符串
pub fn data_value_ecp_type(v: &DataValue) -> u8 {
    match v {
        DataValue::Bool(_) => 1,
        DataValue::Int8(_) | DataValue::Int16(_) | DataValue::Int32(_) | DataValue::Int64(_)
        | DataValue::UInt8(_) | DataValue::UInt16(_) | DataValue::UInt32(_) | DataValue::UInt64(_) => 2,
        DataValue::Float32(_) | DataValue::Float64(_) => 3,
        DataValue::String(_) | DataValue::Bytes(_) => 4,
    }
}

/// DataValue 转为 JSON 标量（values/tags 格式用）
pub fn data_value_to_json_scalar(v: &DataValue) -> serde_json::Value {
    if let Some(b) = v.as_bool() {
        return serde_json::json!(b);
    }
    if let Some(n) = v.as_i64() {
        return serde_json::json!(n);
    }
    if let Some(f) = v.as_f64() {
        return serde_json::json!(f);
    }
    if let Some(s) = v.as_string() {
        return serde_json::json!(s);
    }
    if let DataValue::Bytes(b) = v {
        return serde_json::Value::Array(b.iter().map(|&x| serde_json::json!(x)).collect());
    }
    serde_json::json!(null)
}

/// 将 DataValue 转为 raw_data 用的数字数组（单值转 [f64]）
pub fn data_value_to_number_array(v: &DataValue) -> Vec<f64> {
    if let Some(f) = v.as_f64() {
        return vec![f];
    }
    if let Some(b) = v.as_bool() {
        return vec![if b { 1.0 } else { 0.0 }];
    }
    vec![0.0]
}

/// 点位 key：优先用点位名，无则用 tag_id 字符串
pub fn tag_key(data: &GroupData, tag_id: &TagId) -> String {
    data.tag_names
        .as_ref()
        .and_then(|m| m.get(tag_id).cloned())
        .unwrap_or_else(|| tag_id.0.to_string())
}

/// 按上传格式生成 payload。取值：values_format / tags_format / ecp_format / group_data / raw_data
/// node/group/values 的 key 均用名称，不把 id 放到 topic 和字段中。
pub fn payload_for_format(data: &GroupData, upload_format: &str) -> Vec<u8> {
    let node_s = data
        .node_name
        .clone()
        .unwrap_or_else(|| data.node_id.0.to_string());
    let group_s = data
        .group_name
        .clone()
        .unwrap_or_else(|| data.group_id.0.to_string());
    let timestamp_ms = data.ts.timestamp_millis();

    if upload_format == UPLOAD_FORMAT_VALUES_FORMAT {
        #[derive(serde::Serialize)]
        struct ValuesFormatPayload {
            timestamp: i64,
            node: String,
            group: String,
            values: std::collections::HashMap<String, serde_json::Value>,
            errors: std::collections::HashMap<String, i32>,
            metas: std::collections::HashMap<String, serde_json::Value>,
        }
        let values: std::collections::HashMap<String, serde_json::Value> = data
            .values
            .iter()
            .map(|(id, v)| (tag_key(data, id), data_value_to_json_scalar(v)))
            .collect();
        let payload = ValuesFormatPayload {
            timestamp: timestamp_ms,
            node: node_s,
            group: group_s,
            values,
            errors: std::collections::HashMap::new(),
            metas: std::collections::HashMap::new(),
        };
        serde_json::to_vec(&payload).unwrap_or_default()
    } else if upload_format == UPLOAD_FORMAT_TAGS_FORMAT {
        #[derive(serde::Serialize)]
        struct TagItem {
            name: String,
            value: serde_json::Value,
        }
        #[derive(serde::Serialize)]
        struct TagsFormatPayload {
            timestamp: i64,
            node: String,
            group: String,
            tags: Vec<TagItem>,
        }
        let tags: Vec<TagItem> = data
            .values
            .iter()
            .map(|(id, v)| TagItem {
                name: tag_key(data, id),
                value: data_value_to_json_scalar(v),
            })
            .collect();
        let payload = TagsFormatPayload {
            timestamp: timestamp_ms,
            node: node_s,
            group: group_s,
            tags,
        };
        serde_json::to_vec(&payload).unwrap_or_default()
    } else if upload_format == UPLOAD_FORMAT_ECP_FORMAT {
        #[derive(serde::Serialize)]
        struct EcpTagItem {
            name: String,
            value: serde_json::Value,
            #[serde(rename = "type")]
            ty: u8,
        }
        #[derive(serde::Serialize)]
        struct EcpFormatPayload {
            timestamp: i64,
            node: String,
            group: String,
            tags: Vec<EcpTagItem>,
        }
        let tags: Vec<EcpTagItem> = data
            .values
            .iter()
            .map(|(id, v)| EcpTagItem {
                name: tag_key(data, id),
                value: data_value_to_json_scalar(v),
                ty: data_value_ecp_type(v),
            })
            .collect();
        let payload = EcpFormatPayload {
            timestamp: timestamp_ms,
            node: node_s,
            group: group_s,
            tags,
        };
        serde_json::to_vec(&payload).unwrap_or_default()
    } else if upload_format == UPLOAD_FORMAT_RAW_DATA {
        #[derive(serde::Serialize)]
        struct RawDataPayload {
            node: String,
            group: String,
            timestamp: i64,
            values: std::collections::HashMap<String, Vec<f64>>,
            errors: std::collections::HashMap<String, String>,
            metas: std::collections::HashMap<String, serde_json::Value>,
        }
        let values: std::collections::HashMap<String, Vec<f64>> = data
            .values
            .iter()
            .map(|(id, v)| (tag_key(data, id), data_value_to_number_array(v)))
            .collect();
        let payload = RawDataPayload {
            node: node_s,
            group: group_s,
            timestamp: data.ts.timestamp_millis(),
            values,
            errors: std::collections::HashMap::new(),
            metas: std::collections::HashMap::new(),
        };
        serde_json::to_vec(&payload).unwrap_or_default()
    } else {
        serde_json::to_vec(data).unwrap_or_default()
    }
}
