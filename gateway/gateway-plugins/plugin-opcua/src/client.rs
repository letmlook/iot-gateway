//! OPC UA 客户端读写（需启用 opcua-client feature）。

use gateway_sdk::types::DataValue;
use gateway_sdk::{PluginError, PluginResult};
use gateway_sdk::{Tag, TagId};

use crate::address::{parse_address, OpcNodeId};

#[cfg(feature = "opcua-client")]
pub fn opcua_read(
    endpoint_url: &str,
    username: Option<&str>,
    password: Option<&str>,
    tags: &[(TagId, Tag)],
) -> PluginResult<Vec<(TagId, DataValue)>> {
    use opcua::client::prelude::*;
    use std::collections::HashMap;

    let mut client = ClientBuilder::new()
        .application_name("iot-gateway-opcua")
        .application_uri("urn:iot-gateway:opcua")
        .create_sample_keypair(true)
        .trust_server_certs(true)
        .session_retry_policy(SessionRetryPolicy::default())
        .client()
        .map_err(|e| PluginError::msg(format!("client build: {}", e)))?;

    let endpoint = (
        endpoint_url,
        "None",
        MessageSecurityMode::None,
        UserTokenPolicy::anonymous(),
    );
    let (mut session, _event_loop) = client
        .connect_to_endpoint(endpoint, IdentityToken::Anonymous)
        .map_err(|e| PluginError::msg(format!("connect: {}", e)))?;

    if let (Some(u), Some(p)) = (username, password) {
        if !u.is_empty() && !p.is_empty() {
            session
                .activate_session(IdentityToken::UserName(u.to_string(), p.to_string()))
                .map_err(|e| PluginError::msg(format!("activate_session: {}", e)))?;
        }
    }

    let mut tag_order: Vec<(TagId, Tag)> = Vec::new();
    let mut nodes_to_read: Vec<ReadValueId> = Vec::new();
    for (_id, tag) in tags.iter() {
        let Some(parsed) = parse_address(&tag.address) else {
            continue;
        };
        let node_id = match &parsed.node_id {
            OpcNodeId::Numeric(n) => opcua::types::NodeId::new(parsed.namespace, *n),
            OpcNodeId::String(s) => opcua::types::NodeId::new(parsed.namespace, s.clone()),
        };
        tag_order.push((*_id, tag.clone()));
        nodes_to_read.push(ReadValueId {
            node_id,
            attribute_id: AttributeId::Value as u32,
            index_range: UAString::null(),
            data_encoding: QualifiedName::null(),
        });
    }

    if nodes_to_read.is_empty() {
        return Ok(tags
            .iter()
            .map(|(id, _)| (*id, DataValue::UInt32(0)))
            .collect());
    }

    let timestamps = TimestampsToReturn::Neither;
    let max_age = 0.0;
    let results = session
        .read(&nodes_to_read, timestamps, max_age)
        .map_err(|e| PluginError::msg(format!("read: {}", e)))?;

    let value_map: HashMap<TagId, DataValue> = tag_order
        .into_iter()
        .zip(results.into_iter())
        .map(|((tag_id, tag), dv)| {
            let v = dv
                .value
                .as_ref()
                .map(|v| opcua_value_to_data_value(v, tag.data_type.as_deref()))
                .unwrap_or(DataValue::UInt32(0));
            (tag_id, v)
        })
        .collect();

    let out: Vec<(TagId, DataValue)> = tags
        .iter()
        .map(|(id, _)| {
            (
                *id,
                value_map.get(id).copied().unwrap_or(DataValue::UInt32(0)),
            )
        })
        .collect();
    Ok(out)
}

#[cfg(feature = "opcua-client")]
pub fn opcua_value_to_data_value(v: &opcua::types::Variant, _data_type: Option<&str>) -> DataValue {
    match v {
        opcua::types::Variant::Boolean(b) => DataValue::Bool(*b),
        opcua::types::Variant::SByte(i) => DataValue::Int8(*i),
        opcua::types::Variant::Byte(u) => DataValue::UInt8(*u),
        opcua::types::Variant::Int16(i) => DataValue::Int16(*i),
        opcua::types::Variant::UInt16(u) => DataValue::UInt16(*u),
        opcua::types::Variant::Int32(i) => DataValue::Int32(*i),
        opcua::types::Variant::UInt32(u) => DataValue::UInt32(*u),
        opcua::types::Variant::Int64(i) => DataValue::Int64(*i),
        opcua::types::Variant::UInt64(u) => DataValue::UInt64(*u),
        opcua::types::Variant::Float(f) => DataValue::Float32(*f),
        opcua::types::Variant::Double(d) => DataValue::Float64(*d),
        opcua::types::Variant::String(s) => DataValue::String(s.value.clone().unwrap_or_default()),
        opcua::types::Variant::ByteString(bs) => {
            let bytes = bs.value.as_ref().map(|v| v.clone()).unwrap_or_default();
            DataValue::Bytes(bytes)
        }
        opcua::types::Variant::DateTime(dt) => {
            let t = dt.ticks_since_epoch();
            DataValue::UInt32(t as u32)
        }
        _ => DataValue::UInt32(0),
    }
}

#[cfg(feature = "opcua-client")]
pub fn opcua_write(
    endpoint_url: &str,
    username: Option<&str>,
    password: Option<&str>,
    values: &[(Tag, DataValue)],
) -> PluginResult<()> {
    use opcua::client::prelude::*;

    let mut client = ClientBuilder::new()
        .application_name("iot-gateway-opcua")
        .application_uri("urn:iot-gateway:opcua")
        .create_sample_keypair(true)
        .trust_server_certs(true)
        .session_retry_policy(SessionRetryPolicy::default())
        .client()
        .map_err(|e| PluginError::msg(format!("client build: {}", e)))?;

    let endpoint = (
        endpoint_url,
        "None",
        MessageSecurityMode::None,
        UserTokenPolicy::anonymous(),
    );
    let (mut session, _event_loop) = client
        .connect_to_endpoint(endpoint, IdentityToken::Anonymous)
        .map_err(|e| PluginError::msg(format!("connect: {}", e)))?;

    if let (Some(u), Some(p)) = (username, password) {
        if !u.is_empty() && !p.is_empty() {
            session
                .activate_session(IdentityToken::UserName(u.to_string(), p.to_string()))
                .map_err(|e| PluginError::msg(format!("activate_session: {}", e)))?;
        }
    }

    for (tag, value) in values {
        let Some(parsed) = parse_address(&tag.address) else {
            continue;
        };
        let node_id = match &parsed.node_id {
            OpcNodeId::Numeric(n) => opcua::types::NodeId::new(parsed.namespace, *n),
            OpcNodeId::String(s) => opcua::types::NodeId::new(parsed.namespace, s.clone()),
        };
        let variant = data_value_to_opcua_variant(value);
        let write_value = WriteValue {
            node_id,
            attribute_id: AttributeId::Value as u32,
            index_range: UAString::null(),
            value: opcua::types::DataValue::new(variant),
        };
        let statuses = session
            .write(&[write_value])
            .map_err(|e| PluginError::msg(format!("write: {}", e)))?;
        if let Some(&code) = statuses.first() {
            if !code.is_good() {
                return Err(PluginError::msg(format!(
                    "write node {} failed: {}",
                    tag.address, code
                )));
            }
        }
    }
    Ok(())
}

#[cfg(feature = "opcua-client")]
pub fn data_value_to_opcua_variant(v: &DataValue) -> opcua::types::Variant {
    use opcua::types::{ByteString, Variant};
    match v {
        DataValue::Bool(b) => Variant::Boolean(*b),
        DataValue::Int8(i) => Variant::SByte(*i),
        DataValue::UInt8(u) => Variant::Byte(*u),
        DataValue::Int16(i) => Variant::Int16(*i),
        DataValue::UInt16(u) => Variant::UInt16(*u),
        DataValue::Int32(i) => Variant::Int32(*i),
        DataValue::UInt32(u) => Variant::UInt32(*u),
        DataValue::Int64(i) => Variant::Int64(*i),
        DataValue::UInt64(u) => Variant::UInt64(*u),
        DataValue::Float32(f) => Variant::Float(*f),
        DataValue::Float64(d) => Variant::Double(*d),
        DataValue::String(s) => Variant::String(UAString::from(s.as_str())),
        DataValue::Bytes(b) => Variant::ByteString(ByteString::from(b.as_slice())),
    }
}
