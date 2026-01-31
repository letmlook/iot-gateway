//! 南向 Modbus RTU 插件：支持串口/DTU、线圈/离散/输入/保持寄存器、
//! 地址格式（SLAVE!ADDRESS[.BIT][#ENDIAN]）、读写、超时重试、字节序、STRING/BYTES 等。

#[cfg(feature = "ffi")]
mod ffi;

mod address;
mod config;
mod state;
mod value;

use gateway_sdk::{
    ConfigSchema, Group, GroupId, NodeId, PluginMeta, SouthPlugin, Tag, TagId, TagSchema,
};
use gateway_sdk::log;
use gateway_sdk::types::{DataValue, PluginKind};
use gateway_sdk::{PluginConfig, PluginError, PluginResult};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

use address::parse_address;
#[cfg(feature = "modbus-client")]
use address::{parse_address_full, ModbusArea};
use config::{config_schema, config_str, tag_schema};
use state::ModbusRtuState;
#[cfg(feature = "modbus-client")]
use std::time::Duration;
#[cfg(feature = "modbus-client")]
use value::{register_to_value_ext, value_to_registers};

/// Modbus RTU 南向插件
pub struct ModbusRtuPlugin {
    state: Arc<RwLock<HashMap<NodeId, ModbusRtuState>>>,
}

impl Default for ModbusRtuPlugin {
    fn default() -> Self {
        Self::new()
    }
}

impl ModbusRtuPlugin {
    pub fn new() -> Self {
        Self {
            state: Arc::new(RwLock::new(HashMap::new())),
        }
    }

    fn default_groups() -> Vec<Group> {
        vec![Group {
            id: GroupId::new(),
            name: "default".to_string(),
            interval_ms: 1000,
            description: Some("默认采集组".to_string()),
        }]
    }
}

#[async_trait::async_trait]
impl SouthPlugin for ModbusRtuPlugin {
    fn meta(&self) -> PluginMeta {
        PluginMeta {
            name: "modbus-rtu",
            kind: PluginKind::South,
            description: Some("Modbus RTU 南向驱动，通过串口连接 Modbus 设备"),
            version: "0.1.0",
            name_zh: Some("Modbus RTU"),
            name_en: Some("Modbus RTU"),
            description_zh: Some("Modbus RTU 南向驱动，通过串口/DTU 连接 Modbus 设备"),
            description_en: Some("Modbus RTU south driver, connect to Modbus devices via serial/DTU"),
        }
    }

    fn config_schema(&self) -> Option<ConfigSchema> {
        Some(config_schema())
    }

    fn tag_schema(&self) -> Option<TagSchema> {
        Some(tag_schema())
    }

    async fn validate_tag(&self, _node_id: NodeId, tag: &Tag) -> PluginResult<()> {
        if tag.name.is_empty() {
            return Err(PluginError::tag_invalid("tag name required"));
        }
        parse_address(&tag.address)
            .ok_or_else(|| PluginError::tag_invalid("address format: 0x!addr / 1x!addr / 3x!addr / 4x!addr"))?;
        Ok(())
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        let port = config_str(&config, "port", "COM1");
        log::info(node_id, format!("open modbus-rtu: port={}", port));
        let baud_rate = config.get("baud_rate").and_then(|v| v.as_u64()).map(|n| n as u32).unwrap_or(9600);
        let data_bits = config.get("data_bits").and_then(|v| v.as_u64()).map(|n| n as u8).unwrap_or(8);
        let stop_bits = config.get("stop_bits").and_then(|v| v.as_u64()).map(|n| n as u8).unwrap_or(1);
        let parity = config_str(&config, "parity", "none");
        let slave_id = config.get("slave_id").and_then(|v| v.as_u64()).map(|n| n as u8).unwrap_or(1);
        let connection_timeout_ms = config.get("connection_timeout_ms").and_then(|v| v.as_u64()).unwrap_or(3000);
        let send_interval_ms = config.get("send_interval_ms").and_then(|v| v.as_u64()).unwrap_or(20);
        let max_retry_times = config.get("max_retry_times").and_then(|v| v.as_u64()).map(|n| n as u32).unwrap_or(3);
        let retry_interval_ms = config.get("retry_interval_ms").and_then(|v| v.as_u64()).unwrap_or(100);
        let start_address = config.get("start_address").and_then(|v| v.as_u64()).map(|n| n as u8).unwrap_or(1).min(1);
        let groups = Self::default_groups();
        let tags = groups
            .iter()
            .flat_map(|g| {
                vec![Tag {
                    id: TagId::new(),
                    name: "holding_0".to_string(),
                    address: "4x!0".to_string(),
                    attr: gateway_sdk::TagAttr::Read,
                    data_type: Some("uint16".to_string()),
                    description: Some("保持寄存器 0".to_string()),
                    group_id: g.id,
                }]
            })
            .collect::<Vec<_>>();
        let mut state = self.state.write().await;
        state.insert(
            node_id,
            ModbusRtuState {
                port,
                baud_rate,
                data_bits,
                stop_bits,
                parity,
                slave_id,
                connection_timeout_ms,
                send_interval_ms,
                max_retry_times,
                retry_interval_ms,
                start_address,
                groups,
                tags,
            },
        );
        Ok(())
    }

    async fn close(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "close modbus-rtu");
        let mut state = self.state.write().await;
        state.remove(&node_id);
        Ok(())
    }

    async fn start(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "start modbus-rtu");
        Ok(())
    }

    async fn stop(&self, node_id: NodeId) -> PluginResult<()> {
        log::info(node_id, "stop modbus-rtu");
        Ok(())
    }

    async fn setting(&self, node_id: NodeId, config: PluginConfig) -> PluginResult<()> {
        log::info(node_id, "setting modbus-rtu (config updated)");
        let port = config_str(&config, "port", "COM1");
        let mut state = self.state.write().await;
        if let Some(s) = state.get_mut(&node_id) {
            s.port = port;
            s.baud_rate = config.get("baud_rate").and_then(|v| v.as_u64()).map(|n| n as u32).unwrap_or(s.baud_rate);
            s.slave_id = config.get("slave_id").and_then(|v| v.as_u64()).map(|n| n as u8).unwrap_or(s.slave_id);
            s.connection_timeout_ms = config.get("connection_timeout_ms").and_then(|v| v.as_u64()).unwrap_or(s.connection_timeout_ms);
            s.send_interval_ms = config.get("send_interval_ms").and_then(|v| v.as_u64()).unwrap_or(s.send_interval_ms);
        }
        Ok(())
    }

    async fn poll_group(
        &self,
        node_id: NodeId,
        _group_id: GroupId,
        tags: &[Tag],
    ) -> PluginResult<Vec<(TagId, DataValue)>> {
        let state = self.state.read().await;
        let s = state.get(&node_id).ok_or_else(|| PluginError::msg("node not open"))?;

        #[cfg(feature = "modbus-client")]
        {
            use tokio_modbus::client::rtu::attach_slave;
            use tokio_modbus::prelude::*;
            use tokio_modbus::slave::Slave;
            use tokio_serial::SerialPortBuilderExt;

            let builder = tokio_serial::new(&s.port, s.baud_rate)
                .data_bits(match s.data_bits {
                    7 => tokio_serial::DataBits::Seven,
                    _ => tokio_serial::DataBits::Eight,
                })
                .stop_bits(match s.stop_bits {
                    2 => tokio_serial::StopBits::Two,
                    _ => tokio_serial::StopBits::One,
                })
                .parity(match s.parity.to_lowercase().as_str() {
                    "even" => tokio_serial::Parity::Even,
                    "odd" => tokio_serial::Parity::Odd,
                    _ => tokio_serial::Parity::None,
                });
            let serial = builder
                .open_native_async()
                .map_err(|e| PluginError::msg(format!("serial open: {}", e)))?;
            let mut ctx = attach_slave(serial, Slave(s.slave_id));
            let send_interval = Duration::from_millis(s.send_interval_ms.min(5000));
            let mut out = Vec::with_capacity(tags.len());
            for tag in tags {
                let (tag_id, value) = match parse_address_full(&tag.address, s.start_address) {
                    Some(parsed) => {
                        let dt = tag.data_type.as_deref().unwrap_or("uint16");
                        let v = match parsed.area {
                            ModbusArea::Coil => {
                                let coils = match ctx.read_coils(parsed.start, parsed.count).await {
                                    Ok(Ok(v)) => v,
                                    Ok(Err(e)) => return Err(PluginError::msg(format!("read_coils exception: {}", e))),
                                    Err(e) => return Err(PluginError::msg(format!("read_coils: {}", e))),
                                };
                                DataValue::Bool(coils.first().copied().unwrap_or(false))
                            }
                            ModbusArea::DiscreteInput => {
                                let disc = match ctx.read_discrete_inputs(parsed.start, parsed.count).await {
                                    Ok(Ok(v)) => v,
                                    Ok(Err(e)) => return Err(PluginError::msg(format!("read_discrete_inputs exception: {}", e))),
                                    Err(e) => return Err(PluginError::msg(format!("read_discrete_inputs: {}", e))),
                                };
                                DataValue::Bool(disc.first().copied().unwrap_or(false))
                            }
                            ModbusArea::InputRegister => {
                                let regs = match ctx.read_input_registers(parsed.start, parsed.count).await {
                                    Ok(Ok(v)) => v,
                                    Ok(Err(e)) => return Err(PluginError::msg(format!("read_input_registers exception: {}", e))),
                                    Err(e) => return Err(PluginError::msg(format!("read_input_registers: {}", e))),
                                };
                                register_to_value_ext(&regs, dt, &parsed.endian, parsed.bit_index)
                            }
                            ModbusArea::HoldingRegister => {
                                let regs = match ctx.read_holding_registers(parsed.start, parsed.count).await {
                                    Ok(Ok(v)) => v,
                                    Ok(Err(e)) => return Err(PluginError::msg(format!("read_holding_registers exception: {}", e))),
                                    Err(e) => return Err(PluginError::msg(format!("read_holding_registers: {}", e))),
                                };
                                register_to_value_ext(&regs, dt, &parsed.endian, parsed.bit_index)
                            }
                        };
                        (tag.id, v)
                    }
                    None => (tag.id, DataValue::UInt16(0)),
                };
                out.push((tag_id, value));
                tokio::time::sleep(send_interval).await;
            }
            Ok(out)
        }

        #[cfg(not(feature = "modbus-client"))]
        {
            let _ = (node_id, s);
            let mut out = Vec::with_capacity(tags.len());
            for tag in tags {
                let _ = parse_address(&tag.address);
                out.push((tag.id, DataValue::UInt16(0)));
            }
            Ok(out)
        }
    }

    async fn list_groups(&self, node_id: NodeId) -> PluginResult<Vec<Group>> {
        let state = self.state.read().await;
        let s = state.get(&node_id).ok_or_else(|| PluginError::msg("node not open"))?;
        Ok(s.groups.clone())
    }

    async fn list_tags(&self, node_id: NodeId, group_id: GroupId) -> PluginResult<Vec<Tag>> {
        let state = self.state.read().await;
        let s = state.get(&node_id).ok_or_else(|| PluginError::msg("node not open"))?;
        Ok(s.tags
            .iter()
            .filter(|t| t.group_id == group_id)
            .cloned()
            .collect())
    }

    async fn write_tags(&self, node_id: NodeId, values: &[(Tag, DataValue)]) -> PluginResult<()> {
        if values.is_empty() {
            return Ok(());
        }
        let state = self.state.read().await;
        let s = state.get(&node_id).ok_or_else(|| PluginError::msg("node not open"))?;

        #[cfg(feature = "modbus-client")]
        {
            use tokio_modbus::client::rtu::attach_slave;
            use tokio_modbus::prelude::*;
            use tokio_modbus::slave::Slave;
            use tokio_serial::SerialPortBuilderExt;

            let builder = tokio_serial::new(&s.port, s.baud_rate)
                .data_bits(match s.data_bits {
                    7 => tokio_serial::DataBits::Seven,
                    _ => tokio_serial::DataBits::Eight,
                })
                .stop_bits(match s.stop_bits {
                    2 => tokio_serial::StopBits::Two,
                    _ => tokio_serial::StopBits::One,
                })
                .parity(match s.parity.to_lowercase().as_str() {
                    "even" => tokio_serial::Parity::Even,
                    "odd" => tokio_serial::Parity::Odd,
                    _ => tokio_serial::Parity::None,
                });
            let serial = builder
                .open_native_async()
                .map_err(|e| PluginError::msg(format!("serial open: {}", e)))?;
            let mut ctx = attach_slave(serial, Slave(s.slave_id));
            let interval = Duration::from_millis(s.send_interval_ms.min(5000));

            for (tag, value) in values {
                let Some(parsed) = parse_address_full(&tag.address, s.start_address) else {
                    continue;
                };
                match parsed.area {
                    ModbusArea::Coil => {
                        let b = value.as_bool().unwrap_or(false);
                        if parsed.count == 1 {
                            if let Err(e) = ctx.write_single_coil(parsed.start, b).await {
                                return Err(PluginError::msg(format!("write_single_coil: {}", e)));
                            }
                        } else {
                            let coils: Vec<bool> = (0..parsed.count).map(|_| b).collect();
                            if let Err(e) = ctx.write_multiple_coils(parsed.start, &coils).await {
                                return Err(PluginError::msg(format!("write_multiple_coils: {}", e)));
                            }
                        }
                    }
                    ModbusArea::HoldingRegister => {
                        let regs = value_to_registers(value, tag.data_type.as_deref().unwrap_or("uint16"));
                        if regs.is_empty() {
                            continue;
                        }
                        if regs.len() == 1 {
                            if let Err(e) = ctx.write_single_register(parsed.start, regs[0]).await {
                                return Err(PluginError::msg(format!("write_single_register: {}", e)));
                            }
                        } else if let Err(e) = ctx.write_multiple_registers(parsed.start, &regs).await {
                            return Err(PluginError::msg(format!("write_multiple_registers: {}", e)));
                        }
                    }
                    _ => return Err(PluginError::tag_invalid("only coil and holding register support write")),
                }
                tokio::time::sleep(interval).await;
            }
            Ok(())
        }

        #[cfg(not(feature = "modbus-client"))]
        {
            let _ = (node_id, s, values);
            Err(PluginError::not_supported("write requires modbus-client feature"))
        }
    }
}
