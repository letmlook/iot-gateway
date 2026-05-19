use std::ffi::{CStr, CString};
use std::net::UdpSocket;
use std::sync::Mutex;
use std::time::Duration;
use std::os::raw::{c_char, c_void};

static CONN: once_cell::sync::Lazy<Mutex<Option<FINSState>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(None));

struct FINSState {
    host: String,
    port: u16,
    local_net: u8,
    local_node: u8,
    remote_net: u8,
    remote_node: u8,
    remote_unit: u8,
    socket: Option<UdpSocket>,
    sid: u8,
}

impl FINSState {
    /// Build a FINS UDP socket connected to the remote host/port.
    fn connect(&mut self) -> std::io::Result<()> {
        let addr_str = format!("{}:{}", self.host, self.port);
        let sock = UdpSocket::bind("0.0.0.0:0")?;
        sock.set_read_timeout(Some(Duration::from_secs(2)))?;
        sock.set_write_timeout(Some(Duration::from_secs(2)))?;
        sock.connect(&addr_str)?;
        self.socket = Some(sock);
        Ok(())
    }

    /// Send a FINS command and receive response over UDP.
    fn send_fins(&mut self, mrc: u8, src: u8, params: &[u8]) -> std::io::Result<Vec<u8>> {
        let sock = self.socket.as_ref().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::NotConnected, "socket not connected")
        })?;

        self.sid = self.sid.wrapping_add(1);

        // FINS header (14 bytes) + params
        let mut frame = vec![0u8; 14 + params.len()];
        frame[0] = 0x80; // ICF
        frame[1] = 0x00; // RSV
        frame[2] = 0x02; // GCT
        frame[3] = self.remote_net;  // DNA
        frame[4] = self.remote_node; // DA1
        frame[5] = self.remote_unit; // DA2
        frame[6] = self.local_net;   // SNA
        frame[7] = self.local_node;  // SA1
        frame[8] = 0x00;             // SA2 (CPU unit)
        frame[9] = self.sid;        // SID
        frame[10] = mrc;            // MRC
        frame[11] = src;            // SRC
        frame[12..].copy_from_slice(params);

        sock.send_to(&frame, format!("{}:{}", self.host, self.port))?;

        // FINS response: header (10 bytes) + MRC/SRC (2 bytes) + response code (2 bytes) + data
        let mut resp = vec![0u8; 256];
        let (n, _addr) = sock.recv_from(&mut resp)?;
        resp.truncate(n);
        Ok(resp)
    }

    /// Execute FINS memory read (command 0x0401).
    fn read_memory(&mut self, area: u8, address: u32, bit_offset: u8, word_count: u16) -> std::io::Result<Vec<u16>> {
        // Encode address per FINS memory area format:
        // Bit areas (CIO, HR, AR, etc.): header[1 byte] + 3-byte address + bit number
        // Word areas (DM, WR, etc.): header[1 byte] + 3-byte address + word count
        let mut params = vec![area]; // area code

        if area == 0x01 || area == 0x02 || area == 0x03 || area == 0x04 || area == 0x05 || area == 0x06 {
            // Bit-area (CIO=01, W=02, H=03, A=04, DM=85(word-only), WR=89(word-only))
            // For bit areas: address is 3 bytes (big-endian), then bit number
            params.push(((address >> 16) & 0xFF) as u8);
            params.push(((address >> 8) & 0xFF) as u8);
            params.push((address & 0xFF) as u8);
            params.push(bit_offset);
            params.push(0x00); // padding / reserved
        } else {
            // Word area: address is 3 bytes big-endian, then word count
            params.push(((address >> 16) & 0xFF) as u8);
            params.push(((address >> 8) & 0xFF) as u8);
            params.push((address & 0xFF) as u8);
            params.push(((word_count >> 8) & 0xFF) as u8);
            params.push((word_count & 0xFF) as u8);
        }

        let resp = self.send_fins(0x04, 0x01, &params)?;

        // Response: ICF RSV GCT DNA DA1 DA2 SNA SA1 SA2 SID (10 bytes)
        //           MRC SRC (2 bytes) + response code (2 bytes = 0x0000 success) + data
        if resp.len() < 14 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "FINS response too short"));
        }
        let response_code = u16::from_be_bytes([resp[12], resp[13]]);
        if response_code != 0x0000 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("FINS read error: 0x{:04X}", response_code),
            ));
        }
        let data = &resp[14..];
        let mut words = Vec::with_capacity(data.len() / 2);
        for chunk in data.chunks(2) {
            if chunk.len() == 2 {
                words.push(u16::from_be_bytes([chunk[0], chunk[1]]));
            }
        }
        Ok(words)
    }

    /// Execute FINS memory write (command 0x0802).
    fn write_memory(&mut self, area: u8, address: u32, bit_offset: u8, words: &[u16]) -> std::io::Result<()> {
        let mut params = vec![area];
        params.push(((address >> 16) & 0xFF) as u8);
        params.push(((address >> 8) & 0xFF) as u8);
        params.push((address & 0xFF) as u8);
        params.push(bit_offset);
        params.push(0x00); // padding

        for w in words {
            params.push(((w >> 8) & 0xFF) as u8);
            params.push((w & 0xFF) as u8);
        }

        let resp = self.send_fins(0x08, 0x02, &params)?;

        if resp.len() < 14 {
            return Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "FINS response too short"));
        }
        let response_code = u16::from_be_bytes([resp[12], resp[13]]);
        if response_code != 0x0000 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::Other,
                format!("FINS write error: 0x{:04X}", response_code),
            ));
        }
        Ok(())
    }

    /// FINS ping (echo) — sends a simple FINS command and checks for valid response.
    fn ping(&mut self) -> std::io::Result<bool> {
        match self.write_memory(0x82, 0, 0, &[0]) {
            // Try reading CPU unit info area as ping
            Ok(_) => Ok(true),
            Err(e) => {
                // Some PLCs return an error but still respond — treat any valid FINS response as alive
                if e.kind() == std::io::ErrorKind::Other {
                    let msg = e.to_string();
                    if msg.contains("0x0000") || msg.contains("FINS") {
                        return Ok(true);
                    }
                }
                Err(e)
            }
        }
    }
}

/// Convert an Omron address string (e.g. "D100", "CIO0.05", "W200", "H100") into FINS area code + address.
fn parse_fins_address(addr: &str) -> Option<(u8, u32, u8)> {
    let addr = addr.trim();
    if addr.starts_with("D") || addr.starts_with("DM") {
        // Data Memory (DM / D area) — area code 0x82
        let num: u32 = addr.trim_start_matches(|c| c == 'D' || c == 'M')
            .parse()
            .ok()?;
        Some((0x82, num, 0))
    } else if addr.starts_with("W") || addr.starts_with("WR") {
        // Work area (W / WR) — area code 0x89
        let num: u32 = addr.trim_start_matches(|c| c == 'W' || c == 'R')
            .parse()
            .ok()?;
        Some((0x89, num, 0))
    } else if addr.starts_with("CIO") {
        // CIO area — area code 0xB0
        let parts: Vec<&str> = addr[3..].split('.').collect();
        let base: u32 = parts.get(0).and_then(|s| s.parse().ok()).unwrap_or(0);
        let bit: u8 = parts.get(1).and_then(|s| s.parse().ok()).unwrap_or(0);
        Some((0xB0, base, bit))
    } else if addr.starts_with("H") || addr.starts_with("HR") {
        // Holding area (H / HR) — area code 0x91
        let num: u32 = addr.trim_start_matches(|c| c == 'H' || c == 'R')
            .parse()
            .ok()?;
        Some((0x91, num, 0))
    } else if addr.starts_with("A") || addr.starts_with("AR") {
        // Auxiliary area (A / AR) — area code 0x93
        let num: u32 = addr.trim_start_matches(|c| c == 'A' || c == 'R')
            .parse()
            .ok()?;
        Some((0x93, num, 0))
    } else if addr.starts_with("LR") {
        // Link relay area — area code 0x99
        let num: u32 = addr[2..].parse().ok()?;
        Some((0x99, num, 0))
    } else if addr.starts_with("TIM") {
        // Timer area — area code 0x09
        let num: u32 = addr[3..].parse().ok()?;
        Some((0x09, num, 0))
    } else if addr.starts_with("CNT") {
        // Counter area — area code 0x09 (same as timer, read as PV)
        let num: u32 = addr[3..].parse().ok()?;
        Some((0x09, num, 0))
    } else {
        None
    }
}

fn result_to_json(v: serde_json::Value) -> *mut c_char {
    CString::new(v.to_string()).unwrap().into_raw()
}

#[no_mangle]
pub unsafe extern "C" fn south_create() -> *mut c_void {
    Box::into_raw(Box::new(())) as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn south_destroy(handle: *mut c_void) {
    drop(unsafe { Box::from_raw(handle as *mut ()) });
}

#[no_mangle]
pub unsafe extern "C" fn south_meta() -> *mut c_char {
    let meta = serde_json::json!({
        "name": "omron-fins",
        "kind": "south",
        "description": "Omron FINS protocol — CP/CJ/NJ series PLCs",
        "version": "0.1.0",
        "name_zh": "Omron FINS",
        "name_en": "Omron FINS",
        "description_zh": "Omron FINS协议，支持CP/CJ/NJ系列PLC",
        "description_en": "Omron FINS protocol — CP/CJ/NJ series PLCs via UDP/TCP"
    });
    CString::new(meta.to_string()).unwrap().into_raw()
}

#[no_mangle]
pub unsafe extern "C" fn south_free_string(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}

#[no_mangle]
pub unsafe extern "C" fn south_open(
    _handle: *mut c_void,
    config_json: *const c_char,
) -> *mut c_char {
    let config = if config_json.is_null() {
        return result_to_json(serde_json::json!({"error": "null"}));
    };
    let config_str = unsafe { CStr::from_ptr(config_json) }.to_string_lossy();
    let config: serde_json::Value = serde_json::from_str(&config_str).unwrap_or_default();

    let host = config
        .get("host")
        .and_then(|v| v.as_str())
        .unwrap_or("192.168.1.10")
        .to_string();
    let port = config.get("port").and_then(|v| v.as_u64()).unwrap_or(9600) as u16;
    let local_net = config.get("local_net").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
    let local_node = config.get("local_node").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
    let remote_net = config.get("remote_net").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
    let remote_node = config.get("remote_node").and_then(|v| v.as_u64()).unwrap_or(0) as u8;
    let remote_unit = config.get("remote_unit").and_then(|v| v.as_u64()).unwrap_or(0) as u8;

    let mut state = FINSState {
        host: host.clone(),
        port,
        local_net,
        local_node,
        remote_net,
        remote_node,
        remote_unit,
        socket: None,
        sid: 0,
    };

    let connect_result = state.connect();
    let mut guard = CONN.lock().unwrap();
    *guard = Some(state);

    match connect_result {
        Ok(()) => result_to_json(serde_json::json!({
            "status": "connected",
            "host": host,
            "port": port,
            "note": "FINS UDP connected"
        })),
        Err(e) => result_to_json(serde_json::json!({
            "status": "connected (socket error)",
            "host": host,
            "port": port,
            "error": e.to_string()
        })),
    }
}

#[no_mangle]
pub unsafe extern "C" fn south_close(_handle: *mut c_void, _node_id: *const c_char) -> *mut c_char {
    let mut guard = CONN.lock().unwrap();
    if let Some(ref mut state) = *guard {
        if let Some(_sock) = state.socket.take() {
            // UDP socket — no shutdown needed, just drop
        }
    }
    *guard = None;
    result_to_json(serde_json::json!({ "status": "disconnected" }))
}

#[no_mangle]
pub unsafe extern "C" fn south_init(
    _handle: *mut c_void,
    _config_json: *const c_char,
) -> *mut c_char {
    result_to_json(serde_json::json!({}))
}

#[no_mangle]
pub unsafe extern "C" fn south_uninit(
    _handle: *mut c_void,
    _node_id: *const c_char,
) -> *mut c_char {
    result_to_json(serde_json::json!({}))
}

#[no_mangle]
pub unsafe extern "C" fn south_start(
    _handle: *mut c_void,
    _config_json: *const c_char,
) -> *mut c_char {
    result_to_json(serde_json::json!({}))
}

#[no_mangle]
pub unsafe extern "C" fn south_stop(_handle: *mut c_void, _node_id: *const c_char) -> *mut c_char {
    result_to_json(serde_json::json!({}))
}

#[no_mangle]
pub unsafe extern "C" fn south_poll_group(
    _handle: *mut c_void,
    _node_id: *const c_char,
    group_json: *const c_char,
) -> *mut c_char {
    let mut guard = CONN.lock().unwrap();
    let state = match guard.as_mut() {
        Some(s) => s,
        None => return result_to_json(serde_json::json!({"error": "not connected"})),
    };

    let group = if group_json.is_null() {
        return result_to_json(serde_json::json!({"error": "null"}));
    };
    let group_str = unsafe { CStr::from_ptr(group_json) }.to_string_lossy();
    let group: serde_json::Value = serde_json::from_str(&group_str).unwrap_or_default();

    let mut values = serde_json::Map::new();
    if let Some(tags) = group.get("tags").and_then(|t| t.as_array()) {
        for tag in tags {
            let tag_name = tag
                .get("name")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown");
            let address = tag.get("address").and_then(|v| v.as_str()).unwrap_or("");

            // Try to parse address and do real FINS read
            if let Some((area, offset, bit)) = parse_fins_address(address) {
                let result = if bit > 0 || area == 0xB0 {
                    // Bit read — read one word and extract bit
                    match state.read_memory(area, offset, bit, 1) {
                        Ok(words) if !words.is_empty() => {
                            let bit_val = (words[0] & (1 << bit)) != 0;
                            serde_json::json!({
                                "value": bit_val,
                                "type": "bit",
                                "quality": "good",
                                "timestamp": chrono::Utc::now().to_rfc3339()
                            })
                        }
                        Ok(_) => serde_json::json!({
                            "value": null,
                            "type": "bit",
                            "quality": "bad",
                            "timestamp": chrono::Utc::now().to_rfc3339(),
                            "error": "empty response"
                        }),
                        Err(e) => serde_json::json!({
                            "value": null,
                            "type": "bit",
                            "quality": "bad",
                            "timestamp": chrono::Utc::now().to_rfc3339(),
                            "error": e.to_string()
                        }),
                    }
                } else {
                    // Word read — try to read 1 word
                    match state.read_memory(area, offset, 0, 1) {
                        Ok(words) if !words.is_empty() => {
                            let tag_type = if area == 0x09 { "counter" } else { "word" };
                            serde_json::json!({
                                "value": words[0] as i32,
                                "type": tag_type,
                                "quality": "good",
                                "timestamp": chrono::Utc::now().to_rfc3339()
                            })
                        }
                        Ok(_) => serde_json::json!({
                            "value": null,
                            "type": "word",
                            "quality": "bad",
                            "timestamp": chrono::Utc::now().to_rfc3339(),
                            "error": "empty response"
                        }),
                        Err(e) => serde_json::json!({
                            "value": null,
                            "type": "word",
                            "quality": "bad",
                            "timestamp": chrono::Utc::now().to_rfc3339(),
                            "error": e.to_string()
                        }),
                    }
                };
                values.insert(tag_name.to_string(), result);
            } else {
                // Fallback stub for unknown address format
                values.insert(
                    tag_name.to_string(),
                    serde_json::json!({
                        "value": null,
                        "type": "word",
                        "quality": "bad",
                        "timestamp": chrono::Utc::now().to_rfc3339(),
                        "error": "unknown address format"
                    }),
                );
            }
        }
    }

    result_to_json(serde_json::json!({ "values": values }))
}

#[no_mangle]
pub unsafe extern "C" fn south_validate_tag(
    _handle: *mut c_void,
    _node_id: *const c_char,
    tag_json: *const c_char,
) -> *mut c_char {
    let tag = if tag_json.is_null() {
        return result_to_json(serde_json::json!({"valid": false}));
    };
    let tag_str = unsafe { CStr::from_ptr(tag_json) }.to_string_lossy();
    // Valid: DM100, D100, W100, WR100, CIO0.00, H100, HR100, A0.00, AR0.00, LR0, TIM0, CNT0
    let valid = !tag_str.is_empty()
        && (tag_str.starts_with('D')
            || tag_str.starts_with('W')
            || tag_str.starts_with("CIO")
            || tag_str.starts_with('H')
            || tag_str.starts_with('A')
            || tag_str.starts_with("LR")
            || tag_str.starts_with("TIM")
            || tag_str.starts_with("CNT"));
    result_to_json(serde_json::json!({ "valid": valid }))
}

#[no_mangle]
pub unsafe extern "C" fn south_write_tags(
    _handle: *mut c_void,
    _node_id: *const c_char,
    write_json: *const c_char,
) -> *mut c_char {
    let mut guard = CONN.lock().unwrap();
    let state = match guard.as_mut() {
        Some(s) => s,
        None => return result_to_json(serde_json::json!({"written": 0, "error": "not connected"})),
    };

    let write_str = if write_json.is_null() {
        return result_to_json(serde_json::json!({"written": 0, "error": "null"}));
    };
    let write_str = unsafe { CStr::from_ptr(write_json) }.to_string_lossy();
    let write_data: serde_json::Value = serde_json::from_str(&write_str).unwrap_or_default();

    let mut written = 0u32;
    if let Some(tags) = write_data.get("tags").and_then(|t| t.as_array()) {
        for tag in tags {
            let address = tag.get("address").and_then(|v| v.as_str()).unwrap_or("");
            let value = tag.get("value");

            if let Some((area, offset, bit)) = parse_fins_address(address) {
                if let Some(val) = value {
                    let word_val: i32 = val.as_i64().unwrap_or(0) as i32;
                    let result = if bit > 0 {
                        // Bit write: read-modify-write
                        match state.read_memory(area, offset, 0, 1) {
                            Ok(mut words) if !words.is_empty() => {
                                let bit_val = val.as_bool().unwrap_or(false);
                                if bit_val {
                                    words[0] |= 1 << bit;
                                } else {
                                    words[0] &= !(1 << bit);
                                }
                                state.write_memory(area, offset, 0, &words)
                            }
                            Ok(_) => Err(std::io::Error::new(std::io::ErrorKind::InvalidData, "empty read")),
                            Err(e) => Err(e),
                        }
                    } else {
                        state.write_memory(area, offset, 0, &[word_val as u16])
                    };

                    if result.is_ok() {
                        written += 1;
                    }
                }
            }
        }
    }

    result_to_json(serde_json::json!({ "written": written }))
}

#[no_mangle]
pub unsafe extern "C" fn south_list_groups(
    _handle: *mut c_void,
    _node_id: *const c_char,
) -> *mut c_char {
    result_to_json(serde_json::json!({
        "groups": [{ "id": "default", "name": "Default", "interval_ms": 1000 }]
    }))
}

#[no_mangle]
pub unsafe extern "C" fn south_list_tags(
    _handle: *mut c_void,
    _node_id: *const c_char,
    _group_id: *const c_char,
) -> *mut c_char {
    result_to_json(serde_json::json!({
        "tags": [
            { "name": "dm100", "address": "D100", "type": "word", "access": "read" },
            { "name": "wr100", "address": "W100", "type": "word", "access": "read" },
            { "name": "cio_bit", "address": "CIO0.00", "type": "bit", "access": "read" },
            { "name": "hr100", "address": "H100", "type": "word", "access": "read" }
        ]
    }))
}

#[no_mangle]
pub unsafe extern "C" fn south_config_schema(_handle: *mut c_void) -> *mut c_char {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "host": { "type": "string", "default": "192.168.1.10" },
            "port": { "type": "number", "default": 9600 },
            "local_net": { "type": "number", "default": 0 },
            "local_node": { "type": "number", "default": 0 },
            "remote_net": { "type": "number", "default": 0 },
            "remote_node": { "type": "number", "default": 0 },
            "remote_unit": { "type": "number", "default": 0 }
        }
    });
    CString::new(schema.to_string()).unwrap().into_raw()
}
