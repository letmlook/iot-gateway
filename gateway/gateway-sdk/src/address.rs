//! Unified South Plugin Address Model
//!
//! Supports addresses from: Modbus TCP/RTU, BACnet/IP, Siemens S7, Ethernet/IP,
//! Mitsubishi MC, Omron FINS, DL/T645, IEC61850, KNX, M-Bus, etc.

use serde::{Deserialize, Serialize};

/// Unified address for all south plugins.
/// Each variant corresponds to a specific protocol's addressing scheme.
///
/// This enum is serialized as a JSON object with a `type` field:
/// ```json
/// { "type": "modbus", "host": "192.168.1.100", "port": 502, "unit_id": 1, "address": "400001" }
/// ```
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum SouthAddress {
    // ---- Modbus ----
    /// Modbus TCP/RTU addressing
    /// - host/port: TCP connection (omit for RTU)
    /// - unit_id: Modbus unit/slave ID (0-255, use 255 for broadcast)
    /// - address: Modbus address string (e.g. "400001" = holding register 1)
    /// - data_type: optional hint for parsing (u16/u32/float/bool)
    Modbus {
        host: Option<String>,
        port: Option<u16>,
        unit_id: u8,
        address: String,
        #[serde(default)]
        data_type: Option<String>,
    },
    /// Modbus RTU (serial)
    #[serde(rename = "modbus_rtu")]
    ModbusRtu {
        device: String, // e.g. "/dev/ttyUSB0"
        baud: u32,
        data_bits: u8,
        stop_bits: u8,
        parity: String, // "none", "odd", "even"
        unit_id: u8,
        address: String,
        #[serde(default)]
        data_type: Option<String>,
    },

    // ---- BACnet ----
    /// BACnet/IP addressing
    /// - device_id: BACnet device instance ID
    /// - object_type: e.g. "analogInput", "analogValue", "binaryInput"
    /// - object_instance: instance number of the object
    /// - property_id: e.g. "presentValue", "statusFlags"
    BACnet {
        host: String,
        port: u16,
        device_id: u32,
        object_type: String,
        object_instance: u32,
        property_id: String,
        #[serde(default)]
        array_index: Option<u32>,
    },

    // ---- Siemens S7 ----
    /// Siemens S7 (ISO-TCP) addressing
    /// - host: PLC IP address
    /// - rack: CPU rack number (usually 0)
    /// - slot: CPU slot number (1 for S7-300, 1 or 2 for S7-400, 1 for S7-1200/1500)
    /// - area: memory area (DB/M/E/A/PA/TC/CT)
    /// - db_number: data block number (only for DB area)
    /// - start: byte offset within the area
    /// - bit_offset: optional bit offset (0-7)
    /// - data_size: expected data size in bytes for parsing
    S7 {
        host: String,
        rack: u8,
        slot: u8,
        area: S7Area,
        db_number: Option<u16>,
        start: u32,
        #[serde(default)]
        bit_offset: Option<u8>,
        data_size: u8,
    },

    // ---- Ethernet/IP (CIP) ----
    /// Ethernet/IP (CIP) addressing
    /// - host: PLC IP
    /// - path: CIP symbolic path (e.g. "class/instance/attribute")
    ///   Common: "0x67,1,0" = program file, instance 1 (AB PLC legacy)
    /// - tag_name: for tag-based access (preferred over path)
    EthernetIp {
        host: String,
        port: Option<u16>,
        tag_name: Option<String>,
        #[serde(default)]
        path: Option<String>,
        #[serde(default)]
        element_count: Option<u32>,
    },

    // ---- Mitsubishi MC ----
    /// Mitsubishi MELSEC MC protocol addressing
    /// - host: PLC IP
    /// - network: network number (usually 0)
    /// - station: station number
    /// - cpu_type: "Q" (Q series), "iQ-R", "iQ-F"
    /// - area: X/Y/M/L/SM/B/D/W/TC/CC/RC/CS/SS/STS/STC/FC/SC
    /// - start: start address
    /// - bit_offset: optional bit address (3 digits hex)
    MitsubishiMc {
        host: String,
        network: u8,
        station: u8,
        cpu_type: String,
        area: String,
        start: u32,
        #[serde(default)]
        bit_offset: Option<u8>,
        data_size: u8,
    },

    // ---- Omron FINS ----
    /// Omron FINS protocol addressing
    /// - host: PLC IP
    /// - network: FINS network number (0-127)
    /// - node: FINS node number (1-254)
    /// - cpu_unit: CPU unit number (0-3, usually 0)
    /// - area: CIO/WR/HR/AR/DM/EM/IR/DR/PC
    /// - start: word address
    /// - bit_offset: optional bit (0-15)
    OmronFins {
        host: String,
        network: u8,
        node: u8,
        cpu_unit: u8,
        area: String,
        start: u32,
        #[serde(default)]
        bit_offset: Option<u8>,
    },

    // ---- DL/T645 (electricity meter) ----
    /// DL/T645 (electricity meter) addressing
    /// - interface: "tcp" or "rtu"
    /// - host/port: for TCP mode
    /// - device: serial device for RTU mode
    /// - address: 12-digit meter address string
    /// - register: 4-digit register code
    Dlt645 {
        interface: String, // "tcp" or "rtu"
        #[serde(default)]
        host: Option<String>,
        #[serde(default)]
        port: Option<u16>,
        #[serde(default)]
        device: Option<String>,
        #[serde(default)]
        baud: Option<u32>,
        address: String,
        register: String,
    },

    // ---- IEC 61850 ----
    /// IEC 61850 MMS addressing
    /// - host: MMS server IP
    /// - ied_name: Logical Device / Logical Node name (e.g. "IED1/LLN0")
    /// - data_ref: Data Attribute reference (e.g. "MX$svID$daName$q$stVal")
    IEC61850 {
        host: String,
        port: Option<u16>,
        ied_name: String,
        data_ref: String,
    },

    // ---- Generic / raw ----
    /// Fallback: raw protocol-specific address string.
    /// Plugins that don't have a structured address can use this.
    /// The `plugin` field tells the runtime which plugin should parse this.
    Generic { plugin: String, address: String },
}

/// S7 memory area codes
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum S7Area {
    /// PE (Process Inputs) - read-only
    PE,
    /// PA (Process Outputs)
    PA,
    /// MK (Merker flags)
    MK,
    /// DB (Data Blocks)
    DB,
    /// CT (Counters)
    CT,
    /// TM (Timers)
    TM,
    /// BA (Analog inputs) - some PLCs
    BA,
    /// AK (Outputs) - some PLCs
    AK,
    /// E / I (Inputs) - same as PE/PA for some systems
    E,
    /// A / Q (Outputs) - same as PA for some systems
    A,
}
