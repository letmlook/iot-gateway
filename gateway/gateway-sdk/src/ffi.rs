//! .so 插件 C ABI：本系统动态库插件，所有跨边界数据以 JSON 字符串传递。
//!
//! 插件编译为 cdylib，导出约定符号；网关通过 libloading 加载 .so 并调用。

use std::ffi::CStr;
use std::os::raw::{c_char, c_void};

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
#[allow(clippy::missing_safety_doc)]
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
#[allow(clippy::missing_safety_doc)]
pub unsafe fn ptr_to_string(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    CStr::from_ptr(ptr).to_str().ok().map(|s| s.to_string())
}

/// 解析「成功返回数据、失败返回 `FfiResult`」的插件返回值。
///
/// 插件约定：成功时返回数据本身的 JSON（数组/对象），失败时返回 `{"ok":false,"err":"..."}`。
/// 若直接按目标类型反序列化，失败时会得到「invalid type: map, expected a sequence」这类
/// 与真实原因无关的报错；因此这里先探测失败对象，把插件给出的原因原样透出。
pub fn parse_value_result<T: serde::de::DeserializeOwned>(json: Option<&str>) -> Result<T, String> {
    let s = match json {
        Some(x) if !x.is_empty() => x,
        _ => return Err("plugin returned empty result".to_string()),
    };
    if let Ok(r) = serde_json::from_str::<FfiResult>(s) {
        if !r.ok {
            return Err(r.err.unwrap_or_else(|| "unknown plugin error".to_string()));
        }
    }
    serde_json::from_str(s).map_err(|e| e.to_string())
}

// ---------- 插件侧：panic 隔离 ----------

/// 把 panic 负载转成可读消息
fn panic_message(payload: &(dyn std::any::Any + Send)) -> String {
    if let Some(s) = payload.downcast_ref::<&'static str>() {
        format!("plugin panicked: {}", s)
    } else if let Some(s) = payload.downcast_ref::<String>() {
        format!("plugin panicked: {}", s)
    } else {
        "plugin panicked".to_string()
    }
}

/// 在**插件侧**捕获 panic，把一次异常转成一次失败的 FFI 调用。
///
/// # 为什么必须在插件侧捕获
///
/// 插件（cdylib）与宿主各自静态链接了一份 Rust 运行时。跨动态库传播的 panic 在宿主侧
/// 会被判定为 foreign exception 并直接 abort 整个进程（实测报错：
/// `fatal runtime error: Rust cannot catch foreign exceptions, aborting`）。
/// 所以**绝不允许异常越过 C ABI 边界**——每个导出函数都必须在自己的 crate 内
/// `catch_unwind`。本函数是该边界的统一实现，由 `export_south_plugin!` /
/// `export_north_plugin!` 宏自动套用，手写导出时请自行包裹。
///
/// 捕获到的 panic 仍会经 panic hook 打印到 stderr，便于现场定位。
pub fn guard_ptr<F>(f: F) -> *mut c_char
where
    F: FnOnce() -> *mut c_char,
{
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(f)) {
        Ok(p) => p,
        Err(e) => json_ptr(&FfiResult::failure(panic_message(e.as_ref()))),
    }
}

// ---------- 插件侧：导出用的小工具 ----------

/// 空指针（表示「无结果」，宿主按可选项处理）
pub fn null_ptr() -> *mut c_char {
    std::ptr::null_mut()
}

/// 可序列化值 → JSON C 字符串
pub fn json_ptr<T: serde::Serialize>(v: &T) -> *mut c_char {
    match serde_json::to_string(v) {
        Ok(s) => alloc_c_string(&s),
        Err(_) => std::ptr::null_mut(),
    }
}

/// `PluginResult<()>` → JSON C 字符串
pub fn result_ptr(r: crate::PluginResult<()>) -> *mut c_char {
    match r {
        Ok(()) => json_ptr(&FfiResult::success()),
        Err(e) => json_ptr(&FfiResult::failure(e.to_string())),
    }
}

/// `PluginResult<T>` → JSON C 字符串
pub fn value_ptr<T: serde::Serialize>(r: crate::PluginResult<T>) -> *mut c_char {
    match r {
        Ok(v) => json_ptr(&v),
        Err(e) => json_ptr(&FfiResult::failure(e.to_string())),
    }
}

/// 把插件句柄还原为 `&mut T`；空指针返回 None（不 panic，避免导出函数在边界上崩）
///
/// # Safety
/// `handle` 必须来自本 SDK 生成的 `*_plugin_create` 导出。
pub unsafe fn handle_mut<'a, T>(handle: *mut c_void) -> Option<&'a mut T> {
    if handle.is_null() {
        None
    } else {
        Some(&mut *(handle as *mut T))
    }
}

/// 把插件句柄还原为 `&T`
///
/// # Safety
/// 同 [`handle_mut`]。
pub unsafe fn handle_ref<'a, T>(handle: *mut c_void) -> Option<&'a T> {
    if handle.is_null() {
        None
    } else {
        Some(&*(handle as *const T))
    }
}

/// C 字符串 → String（null/非法 UTF-8 时返回空串）
///
/// # Safety
/// `p` 必须为 null 或指向 NUL 结尾的合法 C 字符串。
pub unsafe fn cstr_or_default(p: *const c_char) -> String {
    ptr_to_string(p).unwrap_or_default()
}

/// JSON 解析，失败时返回 `Default`（跨边界入参容错）
pub fn parse_or_default<T: serde::de::DeserializeOwned + Default>(s: &str) -> T {
    serde_json::from_str(s).unwrap_or_default()
}

// ---------- 插件侧：导出宏 ----------

/// 生成南向插件的全部 C ABI 导出（含 panic 隔离与运行时管理）。
///
/// ```ignore
/// // 插件 crate 的 src/ffi.rs
/// gateway_sdk::export_south_plugin!(crate::MyPlugin);
/// ```
///
/// 约定：
/// - `MyPlugin` 必须实现 `SouthPlugin` 且提供 `new()`；
/// - 调用方需依赖 `tokio`（本宏为每次调用创建 current_thread runtime；宿主保证调用发生在独立线程上）；
/// - 宏已包含 `gateway_south_plugin_*` 全量符号，勿再手写同名的 `#[no_mangle]` 函数。
#[macro_export]
macro_rules! export_south_plugin {
    ($plugin_ty:ty) => {
        // 把 trait 引入作用域，使宏展开不依赖调用方是否 import 过 SouthPlugin
        use $crate::SouthPlugin as _;

        /// 单线程 runtime：宿主保证每次 FFI 调用都在独立线程执行，故此处 `block_on` 是安全的。
        /// （若在 tokio worker 线程上直接 `block_on`，会 panic。见 gateway-core 的 `run_sync`。）
        #[doc(hidden)]
        pub fn __gateway_plugin_block_on<F: ::std::future::Future>(f: F) -> F::Output {
            ::tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build plugin runtime")
                .block_on(f)
        }

        #[no_mangle]
        pub extern "C-unwind" fn gateway_south_plugin_create() -> *mut ::std::os::raw::c_void {
            ::std::boxed::Box::into_raw(::std::boxed::Box::new(<$plugin_ty>::new()))
                as *mut ::std::os::raw::c_void
        }

        #[no_mangle]
        pub unsafe extern "C-unwind" fn gateway_south_plugin_destroy(
            handle: *mut ::std::os::raw::c_void,
        ) {
            // 析构里的 panic 无法转成返回值：吞掉并泄漏，也好过 abort 整个网关
            let _ = ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| {
                if !handle.is_null() {
                    let _ = ::std::boxed::Box::from_raw(handle as *mut $plugin_ty);
                }
            }));
        }

        #[no_mangle]
        pub unsafe extern "C-unwind" fn gateway_south_plugin_meta(
            handle: *mut ::std::os::raw::c_void,
        ) -> *mut ::std::os::raw::c_char {
            $crate::ffi::guard_ptr(|| match $crate::ffi::handle_ref::<$plugin_ty>(handle) {
                Some(p) => $crate::ffi::json_ptr(&$crate::ffi::meta_to_ffi(&p.meta())),
                None => $crate::ffi::null_ptr(),
            })
        }

        #[no_mangle]
        pub unsafe extern "C-unwind" fn gateway_south_plugin_open(
            handle: *mut ::std::os::raw::c_void,
            node_id_json: *const ::std::os::raw::c_char,
            config_json: *const ::std::os::raw::c_char,
        ) -> *mut ::std::os::raw::c_char {
            $crate::ffi::guard_ptr(|| match $crate::ffi::handle_mut::<$plugin_ty>(handle) {
                Some(p) => {
                    let node_id: $crate::NodeId =
                        $crate::ffi::parse_or_default(&$crate::ffi::cstr_or_default(node_id_json));
                    let config: $crate::PluginConfig =
                        $crate::ffi::parse_or_default(&$crate::ffi::cstr_or_default(config_json));
                    $crate::ffi::result_ptr(__gateway_plugin_block_on(p.open(node_id, config)))
                }
                None => $crate::ffi::null_ptr(),
            })
        }

        #[no_mangle]
        pub unsafe extern "C-unwind" fn gateway_south_plugin_close(
            handle: *mut ::std::os::raw::c_void,
            node_id_json: *const ::std::os::raw::c_char,
        ) -> *mut ::std::os::raw::c_char {
            $crate::ffi::guard_ptr(|| match $crate::ffi::handle_mut::<$plugin_ty>(handle) {
                Some(p) => {
                    let node_id: $crate::NodeId =
                        $crate::ffi::parse_or_default(&$crate::ffi::cstr_or_default(node_id_json));
                    $crate::ffi::result_ptr(__gateway_plugin_block_on(p.close(node_id)))
                }
                None => $crate::ffi::null_ptr(),
            })
        }

        #[no_mangle]
        pub unsafe extern "C-unwind" fn gateway_south_plugin_init(
            handle: *mut ::std::os::raw::c_void,
            node_id_json: *const ::std::os::raw::c_char,
        ) -> *mut ::std::os::raw::c_char {
            $crate::ffi::guard_ptr(|| match $crate::ffi::handle_mut::<$plugin_ty>(handle) {
                Some(p) => {
                    let node_id: $crate::NodeId =
                        $crate::ffi::parse_or_default(&$crate::ffi::cstr_or_default(node_id_json));
                    $crate::ffi::result_ptr(__gateway_plugin_block_on(p.init(node_id)))
                }
                None => $crate::ffi::null_ptr(),
            })
        }

        #[no_mangle]
        pub unsafe extern "C-unwind" fn gateway_south_plugin_uninit(
            handle: *mut ::std::os::raw::c_void,
            node_id_json: *const ::std::os::raw::c_char,
        ) -> *mut ::std::os::raw::c_char {
            $crate::ffi::guard_ptr(|| match $crate::ffi::handle_mut::<$plugin_ty>(handle) {
                Some(p) => {
                    let node_id: $crate::NodeId =
                        $crate::ffi::parse_or_default(&$crate::ffi::cstr_or_default(node_id_json));
                    $crate::ffi::result_ptr(__gateway_plugin_block_on(p.uninit(node_id)))
                }
                None => $crate::ffi::null_ptr(),
            })
        }

        #[no_mangle]
        pub unsafe extern "C-unwind" fn gateway_south_plugin_start(
            handle: *mut ::std::os::raw::c_void,
            node_id_json: *const ::std::os::raw::c_char,
        ) -> *mut ::std::os::raw::c_char {
            $crate::ffi::guard_ptr(|| match $crate::ffi::handle_mut::<$plugin_ty>(handle) {
                Some(p) => {
                    let node_id: $crate::NodeId =
                        $crate::ffi::parse_or_default(&$crate::ffi::cstr_or_default(node_id_json));
                    $crate::ffi::result_ptr(__gateway_plugin_block_on(p.start(node_id)))
                }
                None => $crate::ffi::null_ptr(),
            })
        }

        #[no_mangle]
        pub unsafe extern "C-unwind" fn gateway_south_plugin_stop(
            handle: *mut ::std::os::raw::c_void,
            node_id_json: *const ::std::os::raw::c_char,
        ) -> *mut ::std::os::raw::c_char {
            $crate::ffi::guard_ptr(|| match $crate::ffi::handle_mut::<$plugin_ty>(handle) {
                Some(p) => {
                    let node_id: $crate::NodeId =
                        $crate::ffi::parse_or_default(&$crate::ffi::cstr_or_default(node_id_json));
                    $crate::ffi::result_ptr(__gateway_plugin_block_on(p.stop(node_id)))
                }
                None => $crate::ffi::null_ptr(),
            })
        }

        #[no_mangle]
        pub unsafe extern "C-unwind" fn gateway_south_plugin_setting(
            handle: *mut ::std::os::raw::c_void,
            node_id_json: *const ::std::os::raw::c_char,
            config_json: *const ::std::os::raw::c_char,
        ) -> *mut ::std::os::raw::c_char {
            $crate::ffi::guard_ptr(|| match $crate::ffi::handle_mut::<$plugin_ty>(handle) {
                Some(p) => {
                    let node_id: $crate::NodeId =
                        $crate::ffi::parse_or_default(&$crate::ffi::cstr_or_default(node_id_json));
                    let config: $crate::PluginConfig =
                        $crate::ffi::parse_or_default(&$crate::ffi::cstr_or_default(config_json));
                    $crate::ffi::result_ptr(__gateway_plugin_block_on(p.setting(node_id, config)))
                }
                None => $crate::ffi::null_ptr(),
            })
        }

        #[no_mangle]
        pub unsafe extern "C-unwind" fn gateway_south_plugin_validate_tag(
            handle: *mut ::std::os::raw::c_void,
            node_id_json: *const ::std::os::raw::c_char,
            tag_json: *const ::std::os::raw::c_char,
        ) -> *mut ::std::os::raw::c_char {
            $crate::ffi::guard_ptr(|| match $crate::ffi::handle_mut::<$plugin_ty>(handle) {
                Some(p) => {
                    let node_id: $crate::NodeId =
                        $crate::ffi::parse_or_default(&$crate::ffi::cstr_or_default(node_id_json));
                    match serde_json::from_str::<$crate::Tag>(&$crate::ffi::cstr_or_default(
                        tag_json,
                    )) {
                        Ok(tag) => $crate::ffi::result_ptr(__gateway_plugin_block_on(
                            p.validate_tag(node_id, &tag),
                        )),
                        Err(_) => $crate::ffi::json_ptr(&$crate::ffi::FfiResult::failure(
                            "invalid tag json",
                        )),
                    }
                }
                None => $crate::ffi::null_ptr(),
            })
        }

        #[no_mangle]
        pub unsafe extern "C-unwind" fn gateway_south_plugin_poll_group(
            handle: *mut ::std::os::raw::c_void,
            node_id_json: *const ::std::os::raw::c_char,
            group_id_json: *const ::std::os::raw::c_char,
            tags_json: *const ::std::os::raw::c_char,
        ) -> *mut ::std::os::raw::c_char {
            $crate::ffi::guard_ptr(|| match $crate::ffi::handle_mut::<$plugin_ty>(handle) {
                Some(p) => {
                    let node_id: $crate::NodeId =
                        $crate::ffi::parse_or_default(&$crate::ffi::cstr_or_default(node_id_json));
                    let group_id: $crate::GroupId =
                        $crate::ffi::parse_or_default(&$crate::ffi::cstr_or_default(group_id_json));
                    let tags: ::std::vec::Vec<$crate::Tag> =
                        $crate::ffi::parse_or_default(&$crate::ffi::cstr_or_default(tags_json));
                    $crate::ffi::value_ptr(__gateway_plugin_block_on(
                        p.poll_group(node_id, group_id, &tags),
                    ))
                }
                None => $crate::ffi::null_ptr(),
            })
        }

        #[no_mangle]
        pub unsafe extern "C-unwind" fn gateway_south_plugin_write_tags(
            handle: *mut ::std::os::raw::c_void,
            node_id_json: *const ::std::os::raw::c_char,
            values_json: *const ::std::os::raw::c_char,
        ) -> *mut ::std::os::raw::c_char {
            $crate::ffi::guard_ptr(|| match $crate::ffi::handle_mut::<$plugin_ty>(handle) {
                Some(p) => {
                    let node_id: $crate::NodeId =
                        $crate::ffi::parse_or_default(&$crate::ffi::cstr_or_default(node_id_json));
                    let values: ::std::vec::Vec<($crate::Tag, $crate::DataValue)> =
                        $crate::ffi::parse_or_default(&$crate::ffi::cstr_or_default(values_json));
                    $crate::ffi::result_ptr(__gateway_plugin_block_on(
                        p.write_tags(node_id, &values),
                    ))
                }
                None => $crate::ffi::null_ptr(),
            })
        }

        #[no_mangle]
        pub unsafe extern "C-unwind" fn gateway_south_plugin_list_groups(
            handle: *mut ::std::os::raw::c_void,
            node_id_json: *const ::std::os::raw::c_char,
        ) -> *mut ::std::os::raw::c_char {
            $crate::ffi::guard_ptr(|| match $crate::ffi::handle_mut::<$plugin_ty>(handle) {
                Some(p) => {
                    let node_id: $crate::NodeId =
                        $crate::ffi::parse_or_default(&$crate::ffi::cstr_or_default(node_id_json));
                    $crate::ffi::value_ptr(__gateway_plugin_block_on(p.list_groups(node_id)))
                }
                None => $crate::ffi::null_ptr(),
            })
        }

        #[no_mangle]
        pub unsafe extern "C-unwind" fn gateway_south_plugin_list_tags(
            handle: *mut ::std::os::raw::c_void,
            node_id_json: *const ::std::os::raw::c_char,
            group_id_json: *const ::std::os::raw::c_char,
        ) -> *mut ::std::os::raw::c_char {
            $crate::ffi::guard_ptr(|| match $crate::ffi::handle_mut::<$plugin_ty>(handle) {
                Some(p) => {
                    let node_id: $crate::NodeId =
                        $crate::ffi::parse_or_default(&$crate::ffi::cstr_or_default(node_id_json));
                    let group_id: $crate::GroupId =
                        $crate::ffi::parse_or_default(&$crate::ffi::cstr_or_default(group_id_json));
                    $crate::ffi::value_ptr(__gateway_plugin_block_on(
                        p.list_tags(node_id, group_id),
                    ))
                }
                None => $crate::ffi::null_ptr(),
            })
        }

        #[no_mangle]
        pub unsafe extern "C-unwind" fn gateway_south_plugin_config_schema(
            handle: *mut ::std::os::raw::c_void,
        ) -> *mut ::std::os::raw::c_char {
            $crate::ffi::guard_ptr(|| match $crate::ffi::handle_ref::<$plugin_ty>(handle) {
                Some(p) => match p.config_schema() {
                    Some(s) => $crate::ffi::json_ptr(&s),
                    None => $crate::ffi::null_ptr(),
                },
                None => $crate::ffi::null_ptr(),
            })
        }

        #[no_mangle]
        pub unsafe extern "C-unwind" fn gateway_south_plugin_tag_schema(
            handle: *mut ::std::os::raw::c_void,
        ) -> *mut ::std::os::raw::c_char {
            $crate::ffi::guard_ptr(|| match $crate::ffi::handle_ref::<$plugin_ty>(handle) {
                Some(p) => match p.tag_schema() {
                    Some(s) => $crate::ffi::json_ptr(&s),
                    None => $crate::ffi::null_ptr(),
                },
                None => $crate::ffi::null_ptr(),
            })
        }
    };
}

/// 生成北向插件的全部 C ABI 导出（含 panic 隔离与运行时管理）。
///
/// ```ignore
/// // 插件 crate 的 src/ffi.rs
/// gateway_sdk::export_north_plugin!(crate::MyPlugin);
/// ```
///
/// 约定同 [`export_south_plugin!`]。
#[macro_export]
macro_rules! export_north_plugin {
    ($plugin_ty:ty) => {
        use $crate::NorthPlugin as _;

        /// 单线程 runtime：宿主保证每次 FFI 调用都在独立线程执行，故此处 `block_on` 是安全的。
        #[doc(hidden)]
        pub fn __gateway_plugin_block_on<F: ::std::future::Future>(f: F) -> F::Output {
            ::tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .expect("build plugin runtime")
                .block_on(f)
        }

        #[no_mangle]
        pub extern "C-unwind" fn gateway_north_plugin_create() -> *mut ::std::os::raw::c_void {
            ::std::boxed::Box::into_raw(::std::boxed::Box::new(<$plugin_ty>::new()))
                as *mut ::std::os::raw::c_void
        }

        #[no_mangle]
        pub unsafe extern "C-unwind" fn gateway_north_plugin_destroy(
            handle: *mut ::std::os::raw::c_void,
        ) {
            let _ = ::std::panic::catch_unwind(::std::panic::AssertUnwindSafe(|| {
                if !handle.is_null() {
                    let _ = ::std::boxed::Box::from_raw(handle as *mut $plugin_ty);
                }
            }));
        }

        #[no_mangle]
        pub unsafe extern "C-unwind" fn gateway_north_plugin_meta(
            handle: *mut ::std::os::raw::c_void,
        ) -> *mut ::std::os::raw::c_char {
            $crate::ffi::guard_ptr(|| match $crate::ffi::handle_ref::<$plugin_ty>(handle) {
                Some(p) => $crate::ffi::json_ptr(&$crate::ffi::meta_to_ffi(&p.meta())),
                None => $crate::ffi::null_ptr(),
            })
        }

        #[no_mangle]
        pub unsafe extern "C-unwind" fn gateway_north_plugin_open(
            handle: *mut ::std::os::raw::c_void,
            node_id_json: *const ::std::os::raw::c_char,
            config_json: *const ::std::os::raw::c_char,
        ) -> *mut ::std::os::raw::c_char {
            $crate::ffi::guard_ptr(|| match $crate::ffi::handle_mut::<$plugin_ty>(handle) {
                Some(p) => {
                    let node_id: $crate::NodeId =
                        $crate::ffi::parse_or_default(&$crate::ffi::cstr_or_default(node_id_json));
                    let config: $crate::PluginConfig =
                        $crate::ffi::parse_or_default(&$crate::ffi::cstr_or_default(config_json));
                    $crate::ffi::result_ptr(__gateway_plugin_block_on(p.open(node_id, config)))
                }
                None => $crate::ffi::null_ptr(),
            })
        }

        #[no_mangle]
        pub unsafe extern "C-unwind" fn gateway_north_plugin_close(
            handle: *mut ::std::os::raw::c_void,
            node_id_json: *const ::std::os::raw::c_char,
        ) -> *mut ::std::os::raw::c_char {
            $crate::ffi::guard_ptr(|| match $crate::ffi::handle_mut::<$plugin_ty>(handle) {
                Some(p) => {
                    let node_id: $crate::NodeId =
                        $crate::ffi::parse_or_default(&$crate::ffi::cstr_or_default(node_id_json));
                    $crate::ffi::result_ptr(__gateway_plugin_block_on(p.close(node_id)))
                }
                None => $crate::ffi::null_ptr(),
            })
        }

        #[no_mangle]
        pub unsafe extern "C-unwind" fn gateway_north_plugin_init(
            handle: *mut ::std::os::raw::c_void,
            node_id_json: *const ::std::os::raw::c_char,
        ) -> *mut ::std::os::raw::c_char {
            $crate::ffi::guard_ptr(|| match $crate::ffi::handle_mut::<$plugin_ty>(handle) {
                Some(p) => {
                    let node_id: $crate::NodeId =
                        $crate::ffi::parse_or_default(&$crate::ffi::cstr_or_default(node_id_json));
                    $crate::ffi::result_ptr(__gateway_plugin_block_on(p.init(node_id)))
                }
                None => $crate::ffi::null_ptr(),
            })
        }

        #[no_mangle]
        pub unsafe extern "C-unwind" fn gateway_north_plugin_uninit(
            handle: *mut ::std::os::raw::c_void,
            node_id_json: *const ::std::os::raw::c_char,
        ) -> *mut ::std::os::raw::c_char {
            $crate::ffi::guard_ptr(|| match $crate::ffi::handle_mut::<$plugin_ty>(handle) {
                Some(p) => {
                    let node_id: $crate::NodeId =
                        $crate::ffi::parse_or_default(&$crate::ffi::cstr_or_default(node_id_json));
                    $crate::ffi::result_ptr(__gateway_plugin_block_on(p.uninit(node_id)))
                }
                None => $crate::ffi::null_ptr(),
            })
        }

        #[no_mangle]
        pub unsafe extern "C-unwind" fn gateway_north_plugin_start(
            handle: *mut ::std::os::raw::c_void,
            node_id_json: *const ::std::os::raw::c_char,
        ) -> *mut ::std::os::raw::c_char {
            $crate::ffi::guard_ptr(|| match $crate::ffi::handle_mut::<$plugin_ty>(handle) {
                Some(p) => {
                    let node_id: $crate::NodeId =
                        $crate::ffi::parse_or_default(&$crate::ffi::cstr_or_default(node_id_json));
                    $crate::ffi::result_ptr(__gateway_plugin_block_on(p.start(node_id)))
                }
                None => $crate::ffi::null_ptr(),
            })
        }

        #[no_mangle]
        pub unsafe extern "C-unwind" fn gateway_north_plugin_stop(
            handle: *mut ::std::os::raw::c_void,
            node_id_json: *const ::std::os::raw::c_char,
        ) -> *mut ::std::os::raw::c_char {
            $crate::ffi::guard_ptr(|| match $crate::ffi::handle_mut::<$plugin_ty>(handle) {
                Some(p) => {
                    let node_id: $crate::NodeId =
                        $crate::ffi::parse_or_default(&$crate::ffi::cstr_or_default(node_id_json));
                    $crate::ffi::result_ptr(__gateway_plugin_block_on(p.stop(node_id)))
                }
                None => $crate::ffi::null_ptr(),
            })
        }

        #[no_mangle]
        pub unsafe extern "C-unwind" fn gateway_north_plugin_setting(
            handle: *mut ::std::os::raw::c_void,
            node_id_json: *const ::std::os::raw::c_char,
            config_json: *const ::std::os::raw::c_char,
        ) -> *mut ::std::os::raw::c_char {
            $crate::ffi::guard_ptr(|| match $crate::ffi::handle_mut::<$plugin_ty>(handle) {
                Some(p) => {
                    let node_id: $crate::NodeId =
                        $crate::ffi::parse_or_default(&$crate::ffi::cstr_or_default(node_id_json));
                    let config: $crate::PluginConfig =
                        $crate::ffi::parse_or_default(&$crate::ffi::cstr_or_default(config_json));
                    $crate::ffi::result_ptr(__gateway_plugin_block_on(p.setting(node_id, config)))
                }
                None => $crate::ffi::null_ptr(),
            })
        }

        #[no_mangle]
        pub unsafe extern "C-unwind" fn gateway_north_plugin_set_subscriptions(
            handle: *mut ::std::os::raw::c_void,
            node_id_json: *const ::std::os::raw::c_char,
            subscriptions_json: *const ::std::os::raw::c_char,
        ) -> *mut ::std::os::raw::c_char {
            $crate::ffi::guard_ptr(|| match $crate::ffi::handle_mut::<$plugin_ty>(handle) {
                Some(p) => {
                    let node_id: $crate::NodeId =
                        $crate::ffi::parse_or_default(&$crate::ffi::cstr_or_default(node_id_json));
                    let subs: ::std::vec::Vec<$crate::GroupSubscription> =
                        $crate::ffi::parse_or_default(&$crate::ffi::cstr_or_default(
                            subscriptions_json,
                        ));
                    $crate::ffi::result_ptr(__gateway_plugin_block_on(
                        p.set_subscriptions(node_id, &subs),
                    ))
                }
                None => $crate::ffi::null_ptr(),
            })
        }

        #[no_mangle]
        pub unsafe extern "C-unwind" fn gateway_north_plugin_on_group_data(
            handle: *mut ::std::os::raw::c_void,
            node_id_json: *const ::std::os::raw::c_char,
            group_data_json: *const ::std::os::raw::c_char,
        ) -> *mut ::std::os::raw::c_char {
            $crate::ffi::guard_ptr(|| match $crate::ffi::handle_mut::<$plugin_ty>(handle) {
                Some(p) => {
                    let node_id: $crate::NodeId =
                        $crate::ffi::parse_or_default(&$crate::ffi::cstr_or_default(node_id_json));
                    match serde_json::from_str::<$crate::GroupData>(&$crate::ffi::cstr_or_default(
                        group_data_json,
                    )) {
                        Ok(data) => $crate::ffi::result_ptr(__gateway_plugin_block_on(
                            p.on_group_data(node_id, ::std::sync::Arc::new(data)),
                        )),
                        Err(_) => $crate::ffi::json_ptr(&$crate::ffi::FfiResult::failure(
                            "invalid group_data json",
                        )),
                    }
                }
                None => $crate::ffi::null_ptr(),
            })
        }

        #[no_mangle]
        pub unsafe extern "C-unwind" fn gateway_north_plugin_connection_status(
            handle: *mut ::std::os::raw::c_void,
            node_id_json: *const ::std::os::raw::c_char,
        ) -> *mut ::std::os::raw::c_char {
            $crate::ffi::guard_ptr(|| match $crate::ffi::handle_mut::<$plugin_ty>(handle) {
                Some(p) => {
                    let node_id: $crate::NodeId =
                        $crate::ffi::parse_or_default(&$crate::ffi::cstr_or_default(node_id_json));
                    match __gateway_plugin_block_on(p.connection_status(node_id)) {
                        Some(v) => $crate::ffi::json_ptr(&v),
                        None => $crate::ffi::null_ptr(),
                    }
                }
                None => $crate::ffi::null_ptr(),
            })
        }

        #[no_mangle]
        pub unsafe extern "C-unwind" fn gateway_north_plugin_config_schema(
            handle: *mut ::std::os::raw::c_void,
        ) -> *mut ::std::os::raw::c_char {
            $crate::ffi::guard_ptr(|| match $crate::ffi::handle_ref::<$plugin_ty>(handle) {
                Some(p) => match p.config_schema() {
                    Some(s) => $crate::ffi::json_ptr(&s),
                    None => $crate::ffi::null_ptr(),
                },
                None => $crate::ffi::null_ptr(),
            })
        }
    };
}
