//! plugin-grpc — gRPC north plugin using tonic + prost (no protoc required)
//!
//! Implements the `NorthPlugin` FFI interface via `gateway_north_plugin_*` symbols in `ffi.rs`.
//! `on_group_data` performs a real Unary gRPC call using tonic transport + prost encoding.

use std::ffi::{CStr, CString};
use std::os::raw::{c_char, c_void};
use std::sync::Mutex;

use bytes::{Buf, BufMut, Bytes, BytesMut};
use prost::Message;
use tonic::transport::Channel;
use tonic::body::BoxBody;
use tonic::client::GrpcService;
use http::{Request, Response};

use crate::ffi::*;

// =============================================================================
// Protobuf message definitions (manual Encode/Decode — no protoc needed)
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
    fn encode_raw<B>(&self, buf: &mut B)
    where
        B: BufMut,
        Self: Sized,
    {
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
    fn merge_field<B>(
        &mut self,
        tag: u32,
        wire_type: u8,
        buf: &mut B,
        ctx: prost::bytes::Ctx,
    ) -> Result<(), prost::Error>
    where
        B: Buf,
        Self: Sized,
    {
        match (tag, wire_type) {
            (1, 2) => {
                let s = decode_string(buf, ctx)?;
                self.name = s;
            }
            (2, 2) => {
                let s = decode_string(buf, ctx)?;
                self.value = s;
            }
            (3, 0) => {
                self.timestamp = decode_varint(buf, ctx)? as i64;
            }
            (4, 2) => {
                let s = decode_string(buf, ctx)?;
                self.quality = s;
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
    fn encode_raw<B>(&self, buf: &mut B)
    where
        B: BufMut,
        Self: Sized,
    {
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
    fn merge_field<B>(
        &mut self,
        tag: u32,
        wire_type: u8,
        buf: &mut B,
        ctx: prost::bytes::Ctx,
    ) -> Result<(), prost::Error>
    where
        B: Buf,
        Self: Sized,
    {
        match (tag, wire_type) {
            (1, 2) => {
                self.group_id = decode_string(buf, ctx)?;
            }
            (2, 2) => {
                self.node_id = decode_string(buf, ctx)?;
            }
            (3, 2) => {
                let mut t = TagValue::default();
                t.merge(buf, ctx)?;
                self.tags.push(t);
            }
            (4, 0) => {
                self.timestamp = decode_varint(buf, ctx)? as i64;
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
    fn encode_raw<B>(&self, buf: &mut B)
    where
        B: BufMut,
        Self: Sized,
    {
        if let Some(ref d) = self.data {
            encode_field(1, encode_message(d), buf);
        }
    }
    fn merge_field<B>(
        &mut self,
        tag: u32,
        wire_type: u8,
        buf: &mut B,
        ctx: prost::bytes::Ctx,
    ) -> Result<(), prost::Error>
    where
        B: Buf,
        Self: Sized,
    {
        match (tag, wire_type) {
            (1, 2) => {
                let mut d = GroupData::default();
                d.merge(buf, ctx)?;
                self.data = Some(d);
            }
            _ => {}
        }
        Ok(())
    }
    fn encoded_len(&self) -> usize {
        let mut len = 0;
        if let Some(ref d) = self.data {
            len += encoded_len_field(1, encode_message(d));
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
    fn encode_raw<B>(&self, buf: &mut B)
    where
        B: BufMut,
        Self: Sized,
    {
        if self.success {
            encode_field(1, vec![1u8], buf);
        }
        if !self.message.is_empty() {
            encode_field(2, encode_string(&self.message), buf);
        }
        if self.tags_received != 0 {
            encode_field(3, encode_varint(self.tags_received as u64), buf);
        }
    }
    fn merge_field<B>(
        &mut self,
        tag: u32,
        wire_type: u8,
        buf: &mut B,
        ctx: prost::bytes::Ctx,
    ) -> Result<(), prost::Error>
    where
        B: Buf,
        Self: Sized,
    {
        match (tag, wire_type) {
            (1, 0) => {
                self.success = decode_varint(buf, ctx)? != 0;
            }
            (2, 2) => {
                self.message = decode_string(buf, ctx)?;
            }
            (3, 0) => {
                self.tags_received = decode_varint(buf, ctx)? as i32;
            }
            _ => {}
        }
        Ok(())
    }
    fn encoded_len(&self) -> usize {
        let mut len = 0;
        if self.success {
            len += encoded_len_field(1, vec![1u8]);
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
// Protobuf encoding/decoding helpers
// =============================================================================

fn encode_varint(mut v: u64) -> Vec<u8> {
    let mut buf = Vec::new();
    loop {
        if v < 0x80 {
            buf.push(v as u8);
            break;
        } else {
            buf.push(((v & 0x7F) | 0x80) as u8);
            v >>= 7;
        }
    }
    buf
}

fn decode_varint<B>(buf: &mut B, _ctx: prost::bytes::Ctx) -> Result<u64, prost::Error>
where
    B: Buf,
{
    let mut v: u64 = 0;
    let mut shift = 0;
    loop {
        if !buf.has_remaining() {
            return Err(prost::Error::InvalidLength {});
        }
        let b = buf.get_u8();
        v |= ((b & 0x7F) as u64) << shift;
        if b & 0x80 == 0 {
            break;
        }
        shift += 7;
    }
    Ok(v)
}

fn encode_string(s: &str) -> Vec<u8> {
    let mut buf = encode_varint(s.len() as u64);
    buf.extend_from_slice(s.as_bytes());
    buf
}

fn decode_string<B>(buf: &mut B, ctx: prost::bytes::Ctx) -> Result<String, prost::Error>
where
    B: Buf,
{
    let len = decode_varint(buf, ctx)? as usize;
    if buf.remaining() < len {
        return Err(prost::Error::InvalidLength {});
    }
    let b = buf.copy_to_bytes(len);
    String::from_utf8(b.to_vec())
        .map_err(|_| prost::Error::InvalidLength {})
}

fn encode_message<M: Message>(msg: &M) -> Vec<u8> {
    let mut buf = BytesMut::with_capacity(msg.encoded_len());
    msg.encode_raw(&mut buf);
    let mut v = encode_varint(buf.len() as u64);
    v.extend_from_slice(&buf);
    v
}

fn encoded_len_field(field: u32, data: Vec<u8>) -> usize {
    let tag_len = encoded_len_varint((field << 3) | 2); // wire_type = 2 (length-delimited)
    tag_len + encoded_len_varint(data.len() as u64) + data.len()
}

fn encoded_len_varint(v: u64) -> usize {
    encode_varint(v).len()
}

fn encode_field(field: u32, mut data: Vec<u8>, buf: &mut impl BufMut) {
    let tag = (field << 3) | 2; // wire_type = 2 (length-delimited)
    buf.put_slice(&encode_varint(tag));
    buf.put_slice(&encode_varint(data.len() as u64));
    buf.put_slice(&data);
}

// =============================================================================
// gRPC client (manual framing — no protoc/tonic-build needed)
// =============================================================================

/// Encodes a gRPC request for a unary method using the `grpc-web` framing.
/// For a true gRPC unary, we use the standard 5-byte header + length-prefixed message.
fn encode_grpc_request(path: &str, msg: &PushDataRequest) -> Bytes {
    let mut body = BytesMut::new();
    // gRPC response framing is: 1 byte flags + 4 bytes content-length + content
    // For request we use the same length-prefixed framing
    let mut msg_bytes = BytesMut::with_capacity(msg.encoded_len());
    msg.encode_raw(&mut msg_bytes);

    // Compress flag = 0, message length = 4 bytes BE
    body.put_u8(0);
    body.put_u32(msg_bytes.len() as u32);
    body.put_slice(&msg_bytes);

    let body_bytes = body.freeze();

    // Build HTTP/2 request
    let req = Request::builder()
        .method("POST")
        .uri(path)
        .header("content-type", "application/grpc")
        .header("te", "trailers")
        .header("user-agent", "grpc-rust/0.1.0")
        .body(BoxBody::new(body_bytes))
        .unwrap();

    // Serialize request to bytes for tonic transport
    // Actually, we use tonic's GrpcService to make the call directly
    let full = format!(
        "POST {} HTTP/2\r\nhost: localhost\r\ncontent-type: application/grpc\r\nte: trailers\r\nuser-agent: grpc-rust/0.1.0\r\n\r\n",
        path
    );
    Bytes::from(full)
}

/// Decode a gRPC response (length-prefixed + trailers)
fn decode_grpc_response<B>(buf: &mut B) -> Result<PushDataResponse, String>
where
    B: Buf,
{
    // Skip HTTP headers — find the grpc-status header
    // This is a simplified decoder for the happy path
    // In production you'd parse headers properly
    let data = buf.remaining_bytes();
    if data < 5 {
        return Err("response too short".to_string());
    }

    // Try to find trailer delimiter and decode
    // gRPC response format: 0x00 (flags) + 4-byte length + protobuf + 0x80 0x00 (trailer magic)
    buf.get_u8(); // skip flags
    let len = buf.get_u32() as usize;
    if buf.remaining() < len {
        return Err(format!("expected {} bytes, got {}", len, buf.remaining()));
    }

    let mut msg_buf = vec![0u8; len];
    buf.copy_to_slice(&mut msg_buf);

    let mut buf2 = Bytes::from(msg_buf);
    PushDataResponse::decode(&mut buf2).map_err(|e| format!("decode error: {:?}", e))
}

// =============================================================================
// Plugin state
// =============================================================================

struct GrpcState {
    url: String,
    channel: Option<Channel>,
}

static STATE: once_cell::sync::Lazy<Mutex<Option<GrpcState>>> =
    once_cell::sync::Lazy::new(|| Mutex::new(None));

fn result_to_json(v: serde_json::Value) -> *mut c_char {
    CString::new(v.to_string()).unwrap().into_raw()
}

fn block_on<F, T>(fut: F) -> T
where
    F: std::future::Future<Output = T>,
{
    let rt = tokio::runtime::Runtime::new().unwrap();
    rt.block_on(fut)
}

// =============================================================================
// FFI exports (north plugin interface)
// =============================================================================

#[no_mangle]
pub unsafe extern "C" fn north_create() -> *mut c_void {
    Box::into_raw(Box::new(())) as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn north_destroy(handle: *mut c_void) {
    drop(unsafe { Box::from_raw(handle as *mut ()) });
}

#[no_mangle]
pub unsafe extern "C" fn north_meta() -> *mut c_char {
    let meta = serde_json::json!({
        "name": "grpc",
        "kind": "north",
        "description": "gRPC — push data to gRPC server via Protocol Buffers",
        "version": "0.1.0",
        "name_zh": "gRPC",
        "name_en": "gRPC",
        "description_zh": "通过gRPC协议推送数据（Protocol Buffers）",
        "description_en": "Push data to gRPC server using Protocol Buffers over HTTP/2"
    });
    CString::new(meta.to_string()).unwrap().into_raw()
}

#[no_mangle]
pub unsafe extern "C" fn north_free_string(s: *mut c_char) {
    if !s.is_null() {
        drop(unsafe { CString::from_raw(s) });
    }
}

#[no_mangle]
pub unsafe extern "C" fn north_open(
    _handle: *mut c_void,
    config_json: *const c_char,
) -> *mut c_char {
    let config = if config_json.is_null() {
        return result_to_json(serde_json::json!({"error": "null"}));
    };
    let config_str = unsafe { CStr::from_ptr(config_json) }.to_string_lossy();
    let config: serde_json::Value = serde_json::from_str(&config_str).unwrap_or_default();

    let url = config
        .get("url")
        .and_then(|v| v.as_str())
        .unwrap_or("http://localhost:50051")
        .to_string();

    // Establish gRPC channel synchronously via tokio runtime
    let channel = block_on(async { Channel::connect(&url).await });

    match channel {
        Ok(ch) => {
            let mut guard = STATE.lock().unwrap();
            *guard = Some(GrpcState {
                url: url.clone(),
                channel: Some(ch),
            });
            result_to_json(serde_json::json!({
                "status": "connected",
                "url": url
            }))
        }
        Err(e) => result_to_json(serde_json::json!({
            "error": format!("failed to connect: {}", e)
        })),
    }
}

#[no_mangle]
pub unsafe extern "C" fn north_close(_handle: *mut c_void, _node_id: *const c_char) -> *mut c_char {
    let mut guard = STATE.lock().unwrap();
    *guard = None;
    result_to_json(serde_json::json!({ "status": "disconnected" }))
}

#[no_mangle]
pub unsafe extern "C" fn north_init(
    _handle: *mut c_void,
    _config_json: *const c_char,
) -> *mut c_char {
    result_to_json(serde_json::json!({}))
}

#[no_mangle]
pub unsafe extern "C" fn north_uninit(
    _handle: *mut c_void,
    _node_id: *const c_char,
) -> *mut c_char {
    result_to_json(serde_json::json!({}))
}

#[no_mangle]
pub unsafe extern "C" fn north_start(
    _handle: *mut c_void,
    _config_json: *const c_char,
) -> *mut c_char {
    result_to_json(serde_json::json!({}))
}

#[no_mangle]
pub unsafe extern "C" fn north_stop(_handle: *mut c_void, _node_id: *const c_char) -> *mut c_char {
    result_to_json(serde_json::json!({}))
}

#[no_mangle]
pub unsafe extern "C" fn north_on_group_data(
    _handle: *mut c_void,
    _node_id: *const c_char,
    group_json: *const c_char,
) -> *mut c_char {
    let guard = STATE.lock().unwrap();
    let state = match guard.as_ref() {
        Some(s) => s,
        None => return result_to_json(serde_json::json!({"error": "not connected"})),
    };

    if group_json.is_null() {
        return result_to_json(serde_json::json!({"error": "null"}));
    }

    let data_str = unsafe { CStr::from_ptr(group_json) }.to_string_lossy();
    let data: serde_json::Value = match serde_json::from_str(&data_str) {
        Ok(v) => v,
        Err(e) => return result_to_json(serde_json::json!({"error": format!("parse error: {}", e)})),
    };

    // Convert JSON -> protobuf
    let proto_data = json_to_proto_group_data(&data);
    let request = PushDataRequest { data: Some(proto_data) };

    // Make real gRPC Unary call via tonic Channel
    let path = "/iot.DataSink/PushData";
    let service_url = state.url.strip_prefix("http://").unwrap_or(&state.url);

    let response = block_on(async {
        let channel = state.channel.as_ref().ok_or("no channel")?;

        // Build gRPC request manually
        let mut body = BytesMut::with_capacity(request.encoded_len() + 5);
        body.put_u8(0); // flags
        let msg_len = request.encoded_len() as u32;
        body.put_u32(msg_len);
        request.encode_raw(&mut body);

        let req = Request::builder()
            .method("POST")
            .uri(path)
            .header("content-type", "application/grpc")
            .header("te", "trailers")
            .header("user-agent", "grpc-rust/0.1.0")
            .body(BoxBody::new(body.freeze()))
            .unwrap();

        let resp = channel.clone().ready().await
            .map_err(|e| format!("channel not ready: {}", e))?
            .call(req)
            .await
            .map_err(|e| format!("call failed: {}", e))?;

        let parts = resp.into_parts();
        let body_bytes = hyper::body::to_bytes(parts.body)
            .await
            .map_err(|e| format!("body read failed: {}", e))?;

        // Decode gRPC response
        let mut buf = body_bytes;
        if buf.len() < 5 {
            return Err("response too short".to_string());
        }
        buf.get_u8(); // flags
        let _len = buf.get_u32() as usize;

        let mut proto_resp = PushDataResponse::default();
        proto_resp.merge(&mut buf, prost::bytes::Ctx::default())
            .map_err(|e| format!("merge error: {:?}", e))?;

        Ok::<_, String>(proto_resp)
    });

    match response {
        Ok(resp) => result_to_json(serde_json::json!({
            "success": resp.success,
            "message": resp.message,
            "tags_received": resp.tags_received,
        })),
        Err(err) => result_to_json(serde_json::json!({
            "error": err
        })),
    }
}

fn json_to_proto_group_data(data: &serde_json::Value) -> ProtoGroupData {
    #[derive(Message)]
    struct ProtoGroupData {
        #[prost(string, tag = "1")]
        pub group_id: String,
        #[prost(string, tag = "2")]
        pub node_id: String,
        #[prost(message, repeated, tag = "3")]
        pub tags: Vec<ProtoTagValue>,
        #[prost(int64, tag = "4")]
        pub timestamp: i64,
    }

    #[derive(Message)]
    struct ProtoTagValue {
        #[prost(string, tag = "1")]
        pub name: String,
        #[prost(string, tag = "2")]
        pub value: String,
        #[prost(int64, tag = "3")]
        pub timestamp: i64,
        #[prost(string, tag = "4")]
        pub quality: String,
    }

    use prost::Message;

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

    let tags: Vec<ProtoTagValue> = obj
        .get("tags")
        .and_then(|v| v.as_array())
        .map(|arr| {
            arr.iter().filter_map(|tag| {
                let tag_obj = tag.as_object()?;
                Some(ProtoTagValue {
                    name: tag_obj.get("name")?.as_str()?.to_string(),
                    value: tag_obj.get("value")?.as_str()?.to_string(),
                    timestamp: tag_obj.get("timestamp")?.as_i64().unwrap_or(0),
                    quality: tag_obj.get("quality")?.as_str()?.to_string(),
                })
            }).collect()
        })
        .unwrap_or_default();

    ProtoGroupData {
        group_id,
        node_id,
        tags,
        timestamp,
    }
}

fn system_timestamp() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[no_mangle]
pub unsafe extern "C" fn north_set_subscriptions(
    _handle: *mut c_void,
    _node_id: *const c_char,
    _sub_json: *const c_char,
) -> *mut c_char {
    result_to_json(serde_json::json!({}))
}

#[no_mangle]
pub unsafe extern "C" fn north_config_schema(_handle: *mut c_void) -> *mut c_char {
    let schema = serde_json::json!({
        "type": "object",
        "properties": {
            "url": { "type": "string", "default": "http://localhost:50051" },
            "service": { "type": "string", "default": "iot.DataSink" },
            "method": { "type": "string", "default": "PushData" },
            "tls": { "type": "boolean", "default": false }
        }
    });
    CString::new(schema.to_string()).unwrap().into_raw()
}

#[no_mangle]
pub unsafe extern "C" fn north_connection_status(
    _handle: *mut c_void,
    _node_id: *const c_char,
) -> *mut c_char {
    let guard = STATE.lock().unwrap();
    result_to_json(serde_json::json!({ "status": if guard.is_some() { "connected" } else { "disconnected" } }))
}
