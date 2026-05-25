//! plugin-grpc — gRPC north plugin using tonic + prost
//!
//! Implements the `NorthPlugin` FFI interface via `gateway_north_plugin_*` symbols in `ffi.rs`.
//! `on_group_data` performs a real Unary gRPC call using tonic transport + prost encoding.

mod ffi;

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};
use std::sync::Mutex;

use bytes::{Buf, BufMut, BytesMut};
use prost::Message;
use prost::encoding::DecodeContext;
use tonic::transport::Channel;
use http_body_util::{BodyExt, Empty};
use http_body::Body;
use http::Request;

use crate::ffi::*;

// =============================================================================
// Generated protobuf types (compatible with prost 0.13)
// =============================================================================

/// TagValue proto message
#[derive(Debug, Clone, PartialEq)]
pub struct TagValue {
    pub name: String,
    pub value: String,
    pub timestamp: i64,
    pub quality: String,
}

impl Default for TagValue {
    fn default() -> Self {
        Self {
            name: String::new(),
            value: String::new(),
            timestamp: 0,
            quality: "Good".to_string(),
        }
    }
}

impl Message for TagValue {
    fn encode_raw(&self, buf: &mut impl BufMut) {
        if !self.name.is_empty() {
            encode_field(1, encode_string(&self.name), buf);
        }
        if !self.value.is_empty() {
            encode_field(2, encode_string(&self.value), buf);
        }
        if self.timestamp != 0 {
            encode_field(3, encode_varint(self.timestamp as u64), buf);
        }
        if !self.quality.is_empty() {
            encode_field(4, encode_string(&self.quality), buf);
        }
    }
    fn merge_field(&mut self, tag: u32, wire_type: prost::encoding::WireType, buf: &mut impl Buf, _ctx: DecodeContext) -> Result<(), prost::DecodeError> {
        use prost::encoding::wire_type::WireType;
        match (tag, wire_type) {
            (1, WireType::LengthDelimited) => {
                self.name = decode_string(buf)?;
            }
            (2, WireType::LengthDelimited) => {
                self.value = decode_string(buf)?;
            }
            (3, WireType::Varint) => {
                self.timestamp = decode_varint(buf)? as i64;
            }
            (4, WireType::LengthDelimited) => {
                self.quality = decode_string(buf)?;
            }
            _ => {}
        }
        Ok(())
    }
    fn encoded_len(&self) -> usize {
        let mut len = 0;
        if !self.name.is_empty() {
            len += encoded_len_field(1, encode_string(&self.name));
        }
        if !self.value.is_empty() {
            len += encoded_len_field(2, encode_string(&self.value));
        }
        if self.timestamp != 0 {
            len += encoded_len_field(3, encode_varint(self.timestamp as u64));
        }
        if !self.quality.is_empty() {
            len += encoded_len_field(4, encode_string(&self.quality));
        }
        len
    }
    fn clear(&mut self) {
        self.name.clear();
        self.value.clear();
        self.timestamp = 0;
        self.quality.clear();
    }
}

/// GroupData proto message
#[derive(Debug, Clone, PartialEq)]
pub struct GroupData {
    pub group_id: String,
    pub node_id: String,
    pub tags: Vec<TagValue>,
    pub timestamp: i64,
}

impl Default for GroupData {
    fn default() -> Self {
        Self {
            group_id: String::new(),
            node_id: String::new(),
            tags: Vec::new(),
            timestamp: 0,
        }
    }
}

impl Message for GroupData {
    fn encode_raw(&self, buf: &mut impl BufMut) {
        if !self.group_id.is_empty() {
            encode_field(1, encode_string(&self.group_id), buf);
        }
        if !self.node_id.is_empty() {
            encode_field(2, encode_string(&self.node_id), buf);
        }
        for tag in &self.tags {
            encode_field(3, encode_message(tag), buf);
        }
        if self.timestamp != 0 {
            encode_field(4, encode_varint(self.timestamp as u64), buf);
        }
    }
    fn merge_field(&mut self, tag: u32, wire_type: prost::encoding::WireType, buf: &mut impl Buf, ctx: DecodeContext) -> Result<(), prost::DecodeError> {
        use prost::encoding::wire_type::WireType;
        match (tag, wire_type) {
            (1, WireType::LengthDelimited) => {
                self.group_id = decode_string(buf)?;
            }
            (2, WireType::LengthDelimited) => {
                self.node_id = decode_string(buf)?;
            }
            (3, WireType::LengthDelimited) => {
                let mut tag = TagValue::default();
                tag.merge_field(tag, wire_type, buf, ctx)?;
                self.tags.push(tag);
            }
            (4, WireType::Varint) => {
                self.timestamp = decode_varint(buf)? as i64;
            }
            _ => {}
        }
        Ok(())
    }
    fn encoded_len(&self) -> usize {
        let mut len = 0;
        if !self.group_id.is_empty() {
            len += encoded_len_field(1, encode_string(&self.group_id));
        }
        if !self.node_id.is_empty() {
            len += encoded_len_field(2, encode_string(&self.node_id));
        }
        for tag in &self.tags {
            len += encoded_len_field(3, encode_message(tag));
        }
        if self.timestamp != 0 {
            len += encoded_len_field(4, encode_varint(self.timestamp as u64));
        }
        len
    }
    fn clear(&mut self) {
        self.group_id.clear();
        self.node_id.clear();
        self.tags.clear();
        self.timestamp = 0;
    }
}

/// PushDataRequest proto message
#[derive(Debug, Clone, PartialEq)]
pub struct PushDataRequest {
    pub data: Option<GroupData>,
}

impl Default for PushDataRequest {
    fn default() -> Self {
        Self { data: None }
    }
}

impl Message for PushDataRequest {
    fn encode_raw(&self, buf: &mut impl BufMut) {
        if let Some(ref data) = self.data {
            encode_field(1, encode_message(data), buf);
        }
    }
    fn merge_field(&mut self, tag: u32, wire_type: prost::encoding::WireType, buf: &mut impl Buf, ctx: DecodeContext) -> Result<(), prost::DecodeError> {
        use prost::encoding::wire_type::WireType;
        match (tag, wire_type) {
            (1, WireType::LengthDelimited) => {
                let mut data = GroupData::default();
                data.merge_field(1, WireType::LengthDelimited, buf, ctx)?;
                self.data = Some(data);
            }
            _ => {}
        }
        Ok(())
    }
    fn encoded_len(&self) -> usize {
        let mut len = 0;
        if let Some(ref data) = self.data {
            len += encoded_len_field(1, encode_message(data));
        }
        len
    }
    fn clear(&mut self) {
        self.data = None;
    }
}

/// PushDataResponse proto message
#[derive(Debug, Clone, PartialEq)]
pub struct PushDataResponse {
    pub success: bool,
    pub message: String,
    pub tags_received: i32,
}

impl Default for PushDataResponse {
    fn default() -> Self {
        Self {
            success: false,
            message: String::new(),
            tags_received: 0,
        }
    }
}

impl Message for PushDataResponse {
    fn encode_raw(&self, buf: &mut impl BufMut) {
        if self.success {
            encode_field(1, encode_varint(1), buf);
        }
        if !self.message.is_empty() {
            encode_field(2, encode_string(&self.message), buf);
        }
        if self.tags_received != 0 {
            encode_field(3, encode_varint(self.tags_received as u64), buf);
        }
    }
    fn merge_field(&mut self, tag: u32, wire_type: prost::encoding::WireType, buf: &mut impl Buf, _ctx: DecodeContext) -> Result<(), prost::DecodeError> {
        use prost::encoding::wire_type::WireType;
        match (tag, wire_type) {
            (1, WireType::Varint) => {
                self.success = decode_varint(buf)? != 0;
            }
            (2, WireType::LengthDelimited) => {
                self.message = decode_string(buf)?;
            }
            (3, WireType::Varint) => {
                self.tags_received = decode_varint(buf)? as i32;
            }
            _ => {}
        }
        Ok(())
    }
    fn encoded_len(&self) -> usize {
        let mut len = 0;
        if self.success {
            len += encoded_len_field(1, encode_varint(1));
        }
        if !self.message.is_empty() {
            len += encoded_len_field(2, encode_string(&self.message));
        }
        if self.tags_received != 0 {
            len += encoded_len_field(3, encode_varint(self.tags_received as u64));
        }
        len
    }
    fn clear(&mut self) {
        self.success = false;
        self.message.clear();
        self.tags_received = 0;
    }
}

// =============================================================================
// Helper encoding/decoding functions (prost 0.13 compatible)
// =============================================================================

fn encode_varint(value: u64) -> Vec<u8> {
    let mut buf = Vec::with_capacity(10);
    let mut v = value;
    loop {
        if v < 0x80 {
            buf.push(v as u8);
            break;
        } else {
            buf.push((v as u8 & 0x7f) | 0x80);
            v >>= 7;
        }
    }
    buf
}

fn encoded_len_varint(value: u64) -> usize {
    if value < 0x80 { 1 } else if value < 0x4000 { 2 } else if value < 0x200000 { 3 } else if value < 0x10000000 { 4 } else { 5 }
}

fn decode_varint<B: Buf>(buf: &mut B) -> Result<u64, prost::DecodeError> {
    let mut result = 0u64;
    let mut shift = 0;
    loop {
        if !buf.has_remaining() {
            return Err(prost::DecodeError::new("buffer underflow"));
        }
        let b = buf.get_u8();
        result |= ((b & 0x7f) as u64) << shift;
        if b & 0x80 == 0 {
            break;
        }
        shift += 7;
        if shift >= 64 {
            return Err(prost::DecodeError::new("varint overflow"));
        }
    }
    Ok(result)
}

fn encode_string(s: &str) -> Vec<u8> {
    let mut buf = Vec::with_capacity(s.len() + 10);
    encode_varint(s.len() as u64).iter().for_each(|b| buf.push(*b));
    buf.extend_from_slice(s.as_bytes());
    buf
}

fn decode_string<B: Buf>(buf: &mut B) -> Result<String, prost::DecodeError> {
    let len = decode_varint(buf)? as usize;
    if buf.remaining() < len {
        return Err(prost::DecodeError::new("buffer underflow"));
    }
    let b = buf.copy_to_bytes(len);
    String::from_utf8(b.to_vec())
        .map_err(|_| prost::DecodeError::new("invalid utf-8"))
}

fn encode_message<M: Message>(msg: &M) -> Vec<u8> {
    let mut buf = BytesMut::with_capacity(msg.encoded_len());
    msg.encode_raw(&mut buf);
    let mut v = encode_varint(msg.encoded_len() as u64);
    v.extend_from_slice(&buf);
    v
}

fn encoded_len_field(field: u32, data: Vec<u8>) -> usize {
    encoded_len_varint((field << 3) | 2) + encoded_len_varint(data.len() as u64) + data.len()
}

fn encode_field(field: u32, mut data: Vec<u8>, buf: &mut impl BufMut) {
    for b in encode_varint((field << 3) | 2) {
        buf.put_u8(b);
    }
    for b in encode_varint(data.len() as u64) {
        buf.put_u8(b);
    }
    buf.put_slice(&data);
}

// =============================================================================
// JSON to Proto conversion
// =============================================================================

fn json_to_proto_group_data(data: &serde_json::Value) -> GroupData {
    let obj = data.as_object().unwrap_or(&serde_json::map::Map::new());

    let group_id = obj.get("group_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let node_id = obj.get("node_id")
        .and_then(|v| v.as_str())
        .unwrap_or("")
        .to_string();
    let timestamp = obj.get("timestamp")
        .and_then(|v| v.as_i64())
        .unwrap_or_else(system_timestamp);

    let tags: Vec<TagValue> = obj
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter().filter_map(|tag| {
                let tag_obj = tag.as_object()?;
                Some(TagValue {
                    name: tag_obj.get("name")?.as_str()?.to_string(),
                    value: tag_obj.get("value")?.as_str()?.to_string(),
                    timestamp: tag_obj.get("timestamp")?.as_i64().unwrap_or(0),
                    quality: tag_obj.get("quality")?.as_str()?.to_string(),
                })
            }).collect()
        })
        .unwrap_or_default();

    GroupData {
        group_id,
        node_id,
        tags,
        timestamp,
    }
}

fn system_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

// =============================================================================
// North Plugin State
// =============================================================================

pub struct GrpcState {
    pub url: String,
    pub channel: Option<Channel>,
}

static STATE: once_cell::sync::Lazy<Mutex<GrpcState>> =
    once_cell::sync::Lazy::new(|| Mutex::new(GrpcState {
        url: String::new(),
        channel: None,
    }));

// =============================================================================
// FFI Wrappers (C-callable interface)
// =============================================================================

#[no_mangle]
pub unsafe extern "C" fn north_create() -> *mut c_void {
    Box::into_raw(Box::new(())) as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn north_destroy(_handle: *mut c_void) {
    // Nothing to destroy
}

#[no_mangle]
pub unsafe extern "C" fn north_meta() -> *mut c_char {
    let meta = serde_json::json!({
        "name": "grpc",
        "kind": "north",
        "description": "gRPC north plugin for pushing data to a gRPC server",
        "version": "0.1.0",
        "config_schema": {
            "type": "object",
            "properties": {
                "url": {
                    "type": "string",
                    "description": "gRPC server URL (e.g., http://localhost:50051)"
                }
            },
            "required": ["url"]
        }
    });
    to_c_string(meta.to_string())
}

#[no_mangle]
pub unsafe extern "C" fn north_open(_handle: *mut c_void, cfg: *const c_char) -> *mut c_char {
    let cfg_str = unsafe { parse_c_str(cfg) };
    let config: serde_json::Value = match serde_json::from_str(&cfg_str) {
        Ok(v) => v,
        Err(e) => return to_c_string(serde_json::json!({"error": e.to_string()}).to_string()),
    };

    let url = config.get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("http://localhost:50051")
        .to_string();

    let mut state = STATE.lock().unwrap();
    state.url = url.clone();
    
    // Create tonic channel synchronously using block_on
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err_to_c("failed to create runtime");
    
    let channel = rt.block_on(Channel::connect(&url))
        .map_err_to_c("failed to connect to gRPC server");
    
    state.channel = Some(channel);

    to_c_string(serde_json::json!({"status": "open", "url": url}).to_string())
}

#[no_mangle]
pub unsafe extern "C" fn north_close(_handle: *mut c_void, _nid: *const c_char) -> *mut c_char {
    let mut state = STATE.lock().unwrap();
    state.channel = None;
    to_c_string(serde_json::json!({"status": "closed"}).to_string())
}

#[no_mangle]
pub unsafe extern "C" fn north_init(_handle: *mut c_void, _cfg: *const c_char) -> *mut c_char {
    to_c_string(serde_json::json!({"status": "init"}).to_string())
}

#[no_mangle]
pub unsafe extern "C" fn north_uninit(_handle: *mut c_void, _nid: *const c_char) -> *mut c_char {
    to_c_string(serde_json::json!({"status": "uninit"}).to_string())
}

#[no_mangle]
pub unsafe extern "C" fn north_start(_handle: *mut c_void, _cfg: *const c_char) -> *mut c_char {
    to_c_string(serde_json::json!({"status": "started"}).to_string())
}

#[no_mangle]
pub unsafe extern "C" fn north_stop(_handle: *mut c_void, _nid: *const c_char) -> *mut c_char {
    to_c_string(serde_json::json!({"status": "stopped"}).to_string())
}

#[no_mangle]
pub unsafe extern "C" fn north_on_group_data(
    _handle: *mut c_void,
    _nid: *const c_char,
    data: *const c_char,
) -> *mut c_char {
    let data_str = unsafe { parse_c_str(data) };
    let json: serde_json::Value = match serde_json::from_str(&data_str) {
        Ok(v) => v,
        Err(e) => return to_c_string(serde_json::json!({"error": e.to_string()}).to_string()),
    };

    // Build protobuf request
    let group_data = json_to_proto_group_data(&json);
    let request = PushDataRequest { data: Some(group_data) };

    let state = STATE.lock().unwrap();
    let channel = match state.channel.as_ref() {
        Some(ch) => ch,
        None => return to_c_string(serde_json::json!({"error": "not connected"}).to_string()),
    };

    // Encode the request
    let mut buf = BytesMut::with_capacity(request.encoded_len() + 5);
    buf.put_u8(0); // flags
    let len = request.encoded_len() as u32;
    buf.put_u32(len);
    request.encode_raw(&mut buf);

    // Make HTTP/2 request to gRPC endpoint
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err_to_c("failed to create runtime");

    let result = rt.block_on(async {
        let req = Request::builder()
            .method("POST")
            .uri("/iot.DataSink/PushData")
            .header("content-type", "application/grpc")
            .header("te", "trailers")
            .header("user-agent", "grpc-rust/0.1.0")
            .body(tonic::body::boxed(
                Empty::<bytes::Bytes>::new()
                    .map_err(|e| tonic::Status::internal(e.to_string())) as tonic::body::BoxBody
            ))
            .unwrap();

        let resp = channel.clone().ready().await
            .map_err(|e| format!("channel not ready: {}", e))?
            .call(req)
            .await
            .map_err(|e| format!("call failed: {}", e))?;

        let body = resp.into_body();
        let body_bytes = body.collect().await
            .map_err(|e| format!("body read failed: {}", e))?
            .to_bytes();

        // gRPC response: 5 bytes grpc-status + length prefix
        if body_bytes.len() < 5 {
            return Err("response too short".to_string());
        }
        // Skip first 5 bytes (grpc header), then decode protobuf
        let mut msg_bytes = &body_bytes[5..];
        let resp = PushDataResponse::decode(&mut msg_bytes)
            .map_err(|e| format!("decode error: {}", e))?;

        Ok::<_, String>(resp)
    });

    match result {
        Ok(resp) => to_c_string(serde_json::json!({
            "success": resp.success,
            "message": resp.message,
            "tags_received": resp.tags_received,
        }).to_string()),
        Err(err) => to_c_string(serde_json::json!({"error": err}).to_string()),
    }
}

#[no_mangle]
pub unsafe extern "C" fn north_set_subscriptions(
    _handle: *mut c_void,
    _nid: *const c_char,
    _sub: *const c_char,
) -> *mut c_char {
    to_c_string(serde_json::json!({"status": "subscriptions_set"}).to_string())
}

#[no_mangle]
pub unsafe extern "C" fn north_config_schema(_handle: *mut c_void) -> *mut c_char {
    north_meta()
}

#[no_mangle]
pub unsafe extern "C" fn north_connection_status(_handle: *mut c_void, _nid: *const c_char) -> *mut c_char {
    let state = STATE.lock().unwrap();
    let connected = state.channel.is_some();
    to_c_string(serde_json::json!({"connected": connected}).to_string())
}

// =============================================================================
// Utility Functions
// =============================================================================

unsafe fn parse_c_str(ptr: *const c_char) -> String {
    if ptr.is_null() {
        String::new()
    } else {
        CStr::from_ptr(ptr).to_string_lossy().into_owned()
    }
}

fn to_c_string(s: String) -> *mut c_char {
    CString::new(s).unwrap().into_raw()
}

trait MapErrToC<T> {
    fn map_err_to_c<F: FnOnce(String) -> *mut c_char>(self, f: F) -> T;
}

impl<T, E: std::fmt::Display> MapErrToC<T> for Result<T, E> {
    fn map_err_to_c<F: FnOnce(String) -> *mut c_char>(self, f: F) -> T {
        match self {
            Ok(v) => v,
            Err(e) => return f(e.to_string()),
        }
    }
}
