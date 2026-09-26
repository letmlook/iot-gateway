//! .so 插件 C ABI：本系统动态库插件，所有跨边界数据以 JSON 字符串传递。
//!
//! 插件编译为 cdylib，导出约定符号；网关通过 libloading 加载 .so 并调用。

use std::ffi::CStr;
use std::os::raw::c_char;

/// 当前 FFI ABI 版本。
///
/// 宿主加载 .so 时会校验插件导出的 `gateway_plugin_abi_version()`：
/// 版本不一致说明插件与宿主的数据结构约定已经不同，直接拒绝加载比运行期崩溃更安全。
/// 该符号由本 SDK 统一导出，因此所有使用本 SDK 构建的插件都会自带。
pub const FFI_ABI_VERSION: u32 = 1;

/// 插件 ABI 版本（由 SDK 导出，宿主通过 libloading 读取）
#[no_mangle]
pub extern "C-unwind" fn gateway_plugin_abi_version() -> u32 {
    FFI_ABI_VERSION
}

/// 宿主提供给 .so 插件的节点日志回调：level (0=Error,1=Warn,2=Info,3=Debug,4=Trace)，node_id 与 message 均为 UTF-8 C 字符串。
pub type PluginLogCallback =
    unsafe extern "C-unwind" fn(level: u8, node_id: *const c_char, message: *const c_char);

/// 结果 JSON：`{"ok":true}` 或 `{"ok":false,"err":"..."}`。插件分配，宿主复制后调用 `gateway_plugin_free_string` 释放。
#[derive(serde::Serialize, serde::Deserialize)]
pub struct FfiResult {
    pub ok: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub err: Option<String>,
}

impl FfiResult {
    pub fn success() -> Self {
        Self {
            ok: true,
            err: None,
        }
    }
    pub fn failure(msg: impl Into<String>) -> Self {
        Self {
            ok: false,
            err: Some(msg.into()),
        }
    }
}

/// 元信息 JSON，与 PluginMeta 对应。
/// 默认使用 `name: "?"`, `kind: "south"`, `version: "0.0.0"`。
#[derive(serde::Serialize, serde::Deserialize)]
pub struct FfiPluginMeta {
    pub name: String,
    pub kind: String, // "south" | "north"
    pub description: Option<String>,
    pub version: String,
    /// 中文名称
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_zh: Option<String>,
    /// 英文名称
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub name_en: Option<String>,
    /// 中文描述
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description_zh: Option<String>,
    /// 英文描述
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description_en: Option<String>,
    /// 插件编译时使用的 FFI ABI 版本（0 表示旧插件未声明）
    #[serde(default)]
    pub abi_version: u32,
}

impl Default for FfiPluginMeta {
    fn default() -> Self {
        Self {
            name: "?".to_string(),
            kind: "south".to_string(),
            description: None,
            version: "0.0.0".to_string(),
            name_zh: None,
            name_en: None,
            description_zh: None,
            description_en: None,
            abi_version: 0,
        }
    }
}

// ---------- 符号名 ----------

pub const SYM_FREE_STRING: &[u8] = b"gateway_plugin_free_string";

pub const SYM_SOUTH_CREATE: &[u8] = b"gateway_south_plugin_create";
pub const SYM_SOUTH_DESTROY: &[u8] = b"gateway_south_plugin_destroy";
pub const SYM_SOUTH_META: &[u8] = b"gateway_south_plugin_meta";
pub const SYM_SOUTH_OPEN: &[u8] = b"gateway_south_plugin_open";
pub const SYM_SOUTH_CLOSE: &[u8] = b"gateway_south_plugin_close";
pub const SYM_SOUTH_INIT: &[u8] = b"gateway_south_plugin_init";
pub const SYM_SOUTH_UNINIT: &[u8] = b"gateway_south_plugin_uninit";
pub const SYM_SOUTH_START: &[u8] = b"gateway_south_plugin_start";
pub const SYM_SOUTH_STOP: &[u8] = b"gateway_south_plugin_stop";
pub const SYM_SOUTH_SETTING: &[u8] = b"gateway_south_plugin_setting";
pub const SYM_SOUTH_VALIDATE_TAG: &[u8] = b"gateway_south_plugin_validate_tag";
pub const SYM_SOUTH_POLL_GROUP: &[u8] = b"gateway_south_plugin_poll_group";
pub const SYM_SOUTH_WRITE_TAGS: &[u8] = b"gateway_south_plugin_write_tags";
pub const SYM_SOUTH_LIST_GROUPS: &[u8] = b"gateway_south_plugin_list_groups";
pub const SYM_SOUTH_LIST_TAGS: &[u8] = b"gateway_south_plugin_list_tags";
pub const SYM_SOUTH_CONFIG_SCHEMA: &[u8] = b"gateway_south_plugin_config_schema";
pub const SYM_SOUTH_TAG_SCHEMA: &[u8] = b"gateway_south_plugin_tag_schema";
/// 可选：南向插件实现此符号后，宿主在每次 open 成功后调用，传入 (handle, node_id_json, log_callback)，插件可据此按节点打日志。
pub const SYM_SOUTH_SET_LOG: &[u8] = b"gateway_south_plugin_set_log";

pub const SYM_NORTH_CREATE: &[u8] = b"gateway_north_plugin_create";
pub const SYM_NORTH_DESTROY: &[u8] = b"gateway_north_plugin_destroy";
pub const SYM_NORTH_META: &[u8] = b"gateway_north_plugin_meta";
pub const SYM_NORTH_OPEN: &[u8] = b"gateway_north_plugin_open";
pub const SYM_NORTH_CLOSE: &[u8] = b"gateway_north_plugin_close";
pub const SYM_NORTH_INIT: &[u8] = b"gateway_north_plugin_init";
pub const SYM_NORTH_UNINIT: &[u8] = b"gateway_north_plugin_uninit";
pub const SYM_NORTH_START: &[u8] = b"gateway_north_plugin_start";
pub const SYM_NORTH_STOP: &[u8] = b"gateway_north_plugin_stop";
pub const SYM_NORTH_SETTING: &[u8] = b"gateway_north_plugin_setting";
pub const SYM_NORTH_SET_SUBSCRIPTIONS: &[u8] = b"gateway_north_plugin_set_subscriptions";
pub const SYM_NORTH_ON_GROUP_DATA: &[u8] = b"gateway_north_plugin_on_group_data";
pub const SYM_NORTH_CONFIG_SCHEMA: &[u8] = b"gateway_north_plugin_config_schema";
/// 可选：北向插件实现此符号后，宿主可查询连接状态；传入 (handle, node_id_json)，返回 JSON 如 {"connected":true,"last_error":null} 或 null 表示无状态。
pub const SYM_NORTH_CONNECTION_STATUS: &[u8] = b"gateway_north_plugin_connection_status";
/// 可选：北向插件实现此符号后，宿主在每次 open 成功后调用，传入 (handle, node_id_json, log_callback)，插件可据此按节点打日志。
pub const SYM_NORTH_SET_LOG: &[u8] = b"gateway_north_plugin_set_log";

// ---------- 插件侧：Meta 转换与分配 ----------

/// 将 PluginMeta 转为 FfiPluginMeta（用于 meta JSON）
pub fn meta_to_ffi(m: &crate::plugin::PluginMeta) -> FfiPluginMeta {
    FfiPluginMeta {
        name: m.name.to_string(),
        kind: match m.kind {
            crate::PluginKind::South => "south".to_string(),
            crate::PluginKind::North => "north".to_string(),
        },
        description: m.description.map(|s| s.to_string()),
        version: m.version.to_string(),
        name_zh: m.name_zh.map(|s| s.to_string()),
        name_en: m.name_en.map(|s| s.to_string()),
        description_zh: m.description_zh.map(|s| s.to_string()),
        description_en: m.description_en.map(|s| s.to_string()),
        abi_version: FFI_ABI_VERSION,
    }
}

/// 插件分配并返回 C 字符串；宿主须调用 `gateway_plugin_free_string` 释放。
pub fn alloc_c_string(s: &str) -> *mut c_char {
    use std::ffi::CString;
    match CString::new(s) {
        Ok(c) => c.into_raw(),
        Err(_) => std::ptr::null_mut(),
    }
}

/// 释放 `alloc_c_string` 分配的指针；.so 插件须重新导出此符号供宿主调用。
#[no_mangle]
pub unsafe extern "C-unwind" fn gateway_plugin_free_string(ptr: *mut c_char) {
    if ptr.is_null() {
        return;
    }
    let _ = std::ffi::CString::from_raw(ptr);
}

// ---------- 宿主侧：解析 FFI 返回 ----------

/// 从插件返回的 JSON 解析 FfiResult；msg 为 null 或空视为 ok。
pub fn parse_result(json: Option<&str>) -> Result<(), String> {
    let s = match json {
        Some(x) if !x.is_empty() => x,
        _ => return Ok(()),
    };
    let v: FfiResult = serde_json::from_str(s).map_err(|e| e.to_string())?;
    if v.ok {
        Ok(())
    } else {
        Err(v.err.unwrap_or_else(|| "unknown error".to_string()))
    }
}

/// 从 *const c_char 读取 C 字符串并转为 String；调用方负责 free。
pub unsafe fn ptr_to_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    CStr::from_ptr(ptr).to_str().ok().map(|s| s.to_string())
}
