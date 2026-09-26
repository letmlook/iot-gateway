//! 从 `plugins_dir` 扫描并加载 .so 插件。

use gateway_sdk::ffi::*;
use gateway_sdk::types::{DataValue, PluginKind};
use gateway_sdk::{
    ConfigSchema, Group, GroupData, GroupSubscription, NorthPlugin, PluginConfig, PluginMeta,
    SouthPlugin, Tag, TagSchema,
};
use gateway_sdk::{GroupId, NodeId, TagId};
use libloading::Library;
use std::ffi::CString;
use std::os::raw::{c_char, c_void};
use std::path::Path;
use std::sync::Arc;
use tracing::{info, warn};

type CreateFn = unsafe extern "C-unwind" fn() -> *mut c_void;
type DestroyFn = unsafe extern "C-unwind" fn(*mut c_void);
type MetaFn = unsafe extern "C-unwind" fn(*mut c_void) -> *mut c_char;
type FreeStringFn = unsafe extern "C-unwind" fn(*mut c_char);
type StrStrFn =
    unsafe extern "C-unwind" fn(*mut c_void, *const c_char, *const c_char) -> *mut c_char;
type StrFn = unsafe extern "C-unwind" fn(*mut c_void, *const c_char) -> *mut c_char;
type StrStrStrFn = unsafe extern "C-unwind" fn(
    *mut c_void,
    *const c_char,
    *const c_char,
    *const c_char,
) -> *mut c_char;
type VoidFn = unsafe extern "C-unwind" fn(*mut c_void) -> *mut c_char;
/// set_log(handle, node_id_cstr, log_callback)
type SetLogFn = unsafe extern "C-unwind" fn(*mut c_void, *const c_char, *const c_void);

/// 持有已加载的 .so，保证符号在插件生命周期内有效。
pub struct PluginLoader {
    _libraries: Vec<Library>,
}

fn ffi_result(free_fn: FreeStringFn, ptr: *mut c_char) -> Result<(), String> {
    let json = unsafe { gateway_sdk::ptr_to_string(ptr) };
    if !ptr.is_null() {
        unsafe { free_fn(ptr) };
    }
    gateway_sdk::parse_result(json.as_deref())
}

fn cstr(s: &str) -> CString {
    CString::new(s).unwrap_or_else(|_| CString::new("").unwrap())
}

/// 宿主提供给 .so 插件的节点日志回调：将插件发来的 level/node_id/message 转为 tracing 事件，由 NodeFileLayer 按节点写文件。
#[allow(dead_code)]
pub unsafe extern "C-unwind" fn gateway_host_log(
    level: u8,
    node_id: *const c_char,
    message: *const c_char,
) {
    let node_id_str = match gateway_sdk::ptr_to_string(node_id) {
        Some(s) => s,
        None => return,
    };
    let message_str = match gateway_sdk::ptr_to_string(message) {
        Some(s) => s,
        None => return,
    };
    match level {
        0 => tracing::event!(tracing::Level::ERROR, node_id = %node_id_str, message = %message_str),
        1 => tracing::event!(tracing::Level::WARN, node_id = %node_id_str, message = %message_str),
        2 => tracing::event!(tracing::Level::INFO, node_id = %node_id_str, message = %message_str),
        3 => tracing::event!(tracing::Level::DEBUG, node_id = %node_id_str, message = %message_str),
        4 => tracing::event!(tracing::Level::TRACE, node_id = %node_id_str, message = %message_str),
        _ => tracing::event!(tracing::Level::INFO, node_id = %node_id_str, message = %message_str),
    }
}

/// 南向 .so 适配器：将 FFI 调用转发为 `SouthPlugin`。
/// 每次调用都在独立线程执行（见 `run_sync`），插件内可安全 `block_on`，且 panic 被隔离。
struct SouthSoAdapter {
    handle: *mut c_void,
    free_string: FreeStringFn,
    destroy: DestroyFn,
    meta_fn: MetaFn,
    open: StrStrFn,
    close: StrFn,
    init: StrFn,
    uninit: StrFn,
    start: StrFn,
    stop: StrFn,
    setting: StrStrFn,
    validate_tag: StrStrFn,
    poll_group: StrStrStrFn,
    write_tags: StrStrFn,
    list_groups: StrFn,
    list_tags: StrStrFn,
    config_schema: VoidFn,
    tag_schema: VoidFn,
    /// 可选：插件实现 set_log 时，open 成功后调用以传入宿主日志回调
    set_log: Option<SetLogFn>,
    cached_meta: std::sync::OnceLock<PluginMeta>,
}

impl SouthSoAdapter {
    /// 在**独立线程**上执行 FFI 调用（两个适配器共用同一实现）。
    ///
    /// 为什么不能直接在当前线程调用：插件侧为了在同步 C ABI 内驱动 async 逻辑，会在导出
    /// 函数里新建 runtime 并 `block_on`；若宿主在 tokio worker 线程上直接调用，就会触发
    /// `Cannot start a runtime from within a runtime` 并 panic。
    /// 这里用作用域线程执行：该线程不在 runtime 上下文中，插件可安全 `block_on`；
    /// 同时线程内的 panic 会被 `join()` 捕获并转成错误，不会让网关进程 abort。
    fn run_sync<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(*mut c_void) -> Result<T, String> + Send,
        T: Send,
    {
        let handle = SendHandle(self.handle);
        match std::thread::scope(|scope| scope.spawn(move || f(handle.as_ptr())).join()) {
            Ok(inner) => inner,
            Err(_) => Err("plugin call panicked".to_string()),
        }
    }

    fn meta_json(&self) -> Result<String, String> {
        self.run_sync(|handle| {
            let ptr = unsafe { (self.meta_fn)(handle) };
            let s = unsafe { gateway_sdk::ptr_to_string(ptr) };
            if !ptr.is_null() {
                unsafe { (self.free_string)(ptr) };
            }
            s.ok_or_else(|| "meta null".to_string())
        })
    }
}

#[async_trait::async_trait]
impl SouthPlugin for SouthSoAdapter {
    fn meta(&self) -> PluginMeta {
        self.cached_meta
            .get_or_init(|| {
                let s = self.meta_json().unwrap_or_else(|_| {
                    serde_json::json!({"name":"?","kind":"south","version":"0.0.0"}).to_string()
                });
                let f: FfiPluginMeta = serde_json::from_str(&s).unwrap_or_default();
                PluginMeta {
                    name: Box::leak(f.name.into_boxed_str()),
                    kind: if f.kind == "north" {
                        PluginKind::North
                    } else {
                        PluginKind::South
                    },
                    description: f.description.map(|x| {
                        let b = Box::leak(x.into_boxed_str());
                        b as &str
                    }),
                    version: Box::leak(f.version.into_boxed_str()),
                    name_zh: f.name_zh.map(|x| {
                        let b = Box::leak(x.into_boxed_str());
                        b as &str
                    }),
                    name_en: f.name_en.map(|x| {
                        let b = Box::leak(x.into_boxed_str());
                        b as &str
                    }),
                    description_zh: f.description_zh.map(|x| {
                        let b = Box::leak(x.into_boxed_str());
                        b as &str
                    }),
                    description_en: f.description_en.map(|x| {
                        let b = Box::leak(x.into_boxed_str());
                        b as &str
                    }),
                }
            })
            .clone()
    }

    fn config_schema(&self) -> Option<ConfigSchema> {
        let ptr = unsafe { (self.config_schema)(self.handle) };
        let s = unsafe { gateway_sdk::ptr_to_string(ptr) };
        if !ptr.is_null() {
            unsafe { (self.free_string)(ptr) };
        }
        s.and_then(|s| serde_json::from_str(&s).ok())
    }

    fn tag_schema(&self) -> Option<TagSchema> {
        let ptr = unsafe { (self.tag_schema)(self.handle) };
        let s = unsafe { gateway_sdk::ptr_to_string(ptr) };
        if !ptr.is_null() {
            unsafe { (self.free_string)(ptr) };
        }
        s.and_then(|s| serde_json::from_str(&s).ok())
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> gateway_sdk::PluginResult<()> {
        let n = serde_json::to_string(&node_id)
            .map_err(|e| gateway_sdk::PluginError::msg(e.to_string()))?;
        let c = serde_json::to_string(&config)
            .map_err(|e| gateway_sdk::PluginError::msg(e.to_string()))?;
        let free = self.free_string;
        let open = self.open;
        let set_log = self.set_log;
        self.run_sync(move |handle| {
            let n = cstr(&n);
            let c = cstr(&c);
            let ptr = unsafe { open(handle, n.as_ptr(), c.as_ptr()) };
            ffi_result(free, ptr)?;
            if let Some(set_log_fn) = set_log {
                unsafe {
                    set_log_fn(handle, n.as_ptr(), gateway_host_log as *const c_void);
                }
            }
            Ok(())
        })
        .map_err(gateway_sdk::PluginError::msg)
    }

    async fn close(&self, node_id: NodeId) -> gateway_sdk::PluginResult<()> {
        let n = serde_json::to_string(&node_id)
            .map_err(|e| gateway_sdk::PluginError::msg(e.to_string()))?;
        let free = self.free_string;
        let close = self.close;
        self.run_sync(move |handle| {
            let n = cstr(&n);
            let ptr = unsafe { close(handle, n.as_ptr()) };
            ffi_result(free, ptr)
        })
        .map_err(gateway_sdk::PluginError::msg)
    }

    async fn init(&self, node_id: NodeId) -> gateway_sdk::PluginResult<()> {
        let n = serde_json::to_string(&node_id)
            .map_err(|e| gateway_sdk::PluginError::msg(e.to_string()))?;
        let free = self.free_string;
        let init = self.init;
        self.run_sync(move |handle| {
            let n = cstr(&n);
            let ptr = unsafe { init(handle, n.as_ptr()) };
            ffi_result(free, ptr)
        })
        .map_err(gateway_sdk::PluginError::msg)
    }

    async fn uninit(&self, node_id: NodeId) -> gateway_sdk::PluginResult<()> {
        let n = serde_json::to_string(&node_id)
            .map_err(|e| gateway_sdk::PluginError::msg(e.to_string()))?;
        let free = self.free_string;
        let uninit = self.uninit;
        self.run_sync(move |handle| {
            let n = cstr(&n);
            let ptr = unsafe { uninit(handle, n.as_ptr()) };
            ffi_result(free, ptr)
        })
        .map_err(gateway_sdk::PluginError::msg)
    }

    async fn start(&self, node_id: NodeId) -> gateway_sdk::PluginResult<()> {
        let n = serde_json::to_string(&node_id)
            .map_err(|e| gateway_sdk::PluginError::msg(e.to_string()))?;
        let free = self.free_string;
        let start = self.start;
        self.run_sync(move |handle| {
            let n = cstr(&n);
            let ptr = unsafe { start(handle, n.as_ptr()) };
            ffi_result(free, ptr)
        })
        .map_err(gateway_sdk::PluginError::msg)
    }

    async fn stop(&self, node_id: NodeId) -> gateway_sdk::PluginResult<()> {
        let n = serde_json::to_string(&node_id)
            .map_err(|e| gateway_sdk::PluginError::msg(e.to_string()))?;
        let free = self.free_string;
        let stop = self.stop;
        self.run_sync(move |handle| {
            let n = cstr(&n);
            let ptr = unsafe { stop(handle, n.as_ptr()) };
            ffi_result(free, ptr)
        })
        .map_err(gateway_sdk::PluginError::msg)
    }

    async fn setting(
        &self,
        node_id: NodeId,
        config: PluginConfig,
    ) -> gateway_sdk::PluginResult<()> {
        let n = serde_json::to_string(&node_id)
            .map_err(|e| gateway_sdk::PluginError::msg(e.to_string()))?;
        let c = serde_json::to_string(&config)
            .map_err(|e| gateway_sdk::PluginError::msg(e.to_string()))?;
        let free = self.free_string;
        let setting = self.setting;
        self.run_sync(move |handle| {
            let n = cstr(&n);
            let c = cstr(&c);
            let ptr = unsafe { setting(handle, n.as_ptr(), c.as_ptr()) };
            ffi_result(free, ptr)
        })
        .map_err(gateway_sdk::PluginError::msg)
    }

    async fn validate_tag(&self, node_id: NodeId, tag: &Tag) -> gateway_sdk::PluginResult<()> {
        let n = serde_json::to_string(&node_id)
            .map_err(|e| gateway_sdk::PluginError::msg(e.to_string()))?;
        let t =
            serde_json::to_string(tag).map_err(|e| gateway_sdk::PluginError::msg(e.to_string()))?;
        let free = self.free_string;
        let validate = self.validate_tag;
        self.run_sync(move |handle| {
            let n = cstr(&n);
            let t = cstr(&t);
            let ptr = unsafe { validate(handle, n.as_ptr(), t.as_ptr()) };
            ffi_result(free, ptr)
        })
        .map_err(gateway_sdk::PluginError::msg)
    }

    async fn poll_group(
        &self,
        node_id: NodeId,
        group_id: GroupId,
        tags: &[Tag],
    ) -> gateway_sdk::PluginResult<Vec<(TagId, DataValue)>> {
        let n = serde_json::to_string(&node_id)
            .map_err(|e| gateway_sdk::PluginError::msg(e.to_string()))?;
        let g = serde_json::to_string(&group_id)
            .map_err(|e| gateway_sdk::PluginError::msg(e.to_string()))?;
        let t = serde_json::to_string(tags)
            .map_err(|e| gateway_sdk::PluginError::msg(e.to_string()))?;
        let free = self.free_string;
        let poll = self.poll_group;
        self.run_sync(move |handle| {
            let n = cstr(&n);
            let g = cstr(&g);
            let t = cstr(&t);
            let ptr = unsafe { poll(handle, n.as_ptr(), g.as_ptr(), t.as_ptr()) };
            let s = unsafe { gateway_sdk::ptr_to_string(ptr) };
            if !ptr.is_null() {
                unsafe { free(ptr) };
            }
            let s = s.ok_or_else(|| "poll_group null".to_string())?;
            // 失败时插件返回的是 {"ok":false,"err":...}，需先识别以免报出误导性的类型错误
            gateway_sdk::parse_value_result(Some(s.as_str()))
        })
        .map_err(gateway_sdk::PluginError::msg)
    }

    async fn write_tags(
        &self,
        node_id: NodeId,
        values: &[(Tag, DataValue)],
    ) -> gateway_sdk::PluginResult<()> {
        let n = serde_json::to_string(&node_id)
            .map_err(|e| gateway_sdk::PluginError::msg(e.to_string()))?;
        let v = serde_json::to_string(values)
            .map_err(|e| gateway_sdk::PluginError::msg(e.to_string()))?;
        let free = self.free_string;
        let write = self.write_tags;
        self.run_sync(move |handle| {
            let n = cstr(&n);
            let v = cstr(&v);
            let ptr = unsafe { write(handle, n.as_ptr(), v.as_ptr()) };
            ffi_result(free, ptr)
        })
        .map_err(gateway_sdk::PluginError::msg)
    }

    async fn list_groups(&self, node_id: NodeId) -> gateway_sdk::PluginResult<Vec<Group>> {
        let n = serde_json::to_string(&node_id)
            .map_err(|e| gateway_sdk::PluginError::msg(e.to_string()))?;
        let free = self.free_string;
        let list = self.list_groups;
        self.run_sync(move |handle| {
            let n = cstr(&n);
            let ptr = unsafe { list(handle, n.as_ptr()) };
            let s = unsafe { gateway_sdk::ptr_to_string(ptr) };
            if !ptr.is_null() {
                unsafe { free(ptr) };
            }
            let s = s.ok_or_else(|| "list_groups null".to_string())?;
            // 失败时插件返回的是 {"ok":false,"err":...}，需先识别以免报出误导性的类型错误
            gateway_sdk::parse_value_result(Some(s.as_str()))
        })
        .map_err(gateway_sdk::PluginError::msg)
    }

    async fn list_tags(
        &self,
        node_id: NodeId,
        group_id: GroupId,
    ) -> gateway_sdk::PluginResult<Vec<Tag>> {
        let n = serde_json::to_string(&node_id)
            .map_err(|e| gateway_sdk::PluginError::msg(e.to_string()))?;
        let g = serde_json::to_string(&group_id)
            .map_err(|e| gateway_sdk::PluginError::msg(e.to_string()))?;
        let free = self.free_string;
        let list = self.list_tags;
        self.run_sync(move |handle| {
            let n = cstr(&n);
            let g = cstr(&g);
            let ptr = unsafe { list(handle, n.as_ptr(), g.as_ptr()) };
            let s = unsafe { gateway_sdk::ptr_to_string(ptr) };
            if !ptr.is_null() {
                unsafe { free(ptr) };
            }
            let s = s.ok_or_else(|| "list_tags null".to_string())?;
            // 失败时插件返回的是 {"ok":false,"err":...}，需先识别以免报出误导性的类型错误
            gateway_sdk::parse_value_result(Some(s.as_str()))
        })
        .map_err(gateway_sdk::PluginError::msg)
    }
}

impl Drop for SouthSoAdapter {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { (self.destroy)(self.handle) };
        }
    }
}

/// 北向 .so 适配器
struct NorthSoAdapter {
    handle: *mut c_void,
    free_string: FreeStringFn,
    destroy: DestroyFn,
    meta_fn: MetaFn,
    open: StrStrFn,
    close: StrFn,
    init: StrFn,
    uninit: StrFn,
    start: StrFn,
    stop: StrFn,
    setting: StrStrFn,
    set_subscriptions: StrStrFn,
    on_group_data: StrStrFn,
    config_schema: VoidFn,
    /// 可选：插件实现 connection_status 时返回连接状态 JSON
    connection_status: Option<StrFn>,
    set_log: Option<SetLogFn>,
    cached_meta: std::sync::OnceLock<PluginMeta>,
}

impl NorthSoAdapter {
    /// 在**独立线程**上执行 FFI 调用（两个适配器共用同一实现）。
    ///
    /// 为什么不能直接在当前线程调用：插件侧为了在同步 C ABI 内驱动 async 逻辑，会在导出
    /// 函数里新建 runtime 并 `block_on`；若宿主在 tokio worker 线程上直接调用，就会触发
    /// `Cannot start a runtime from within a runtime` 并 panic。
    /// 这里用作用域线程执行：该线程不在 runtime 上下文中，插件可安全 `block_on`；
    /// 同时线程内的 panic 会被 `join()` 捕获并转成错误，不会让网关进程 abort。
    fn run_sync<F, T>(&self, f: F) -> Result<T, String>
    where
        F: FnOnce(*mut c_void) -> Result<T, String> + Send,
        T: Send,
    {
        let handle = SendHandle(self.handle);
        match std::thread::scope(|scope| scope.spawn(move || f(handle.as_ptr())).join()) {
            Ok(inner) => inner,
            Err(_) => Err("plugin call panicked".to_string()),
        }
    }

    fn meta_json(&self) -> Result<String, String> {
        self.run_sync(|handle| {
            let ptr = unsafe { (self.meta_fn)(handle) };
            let s = unsafe { gateway_sdk::ptr_to_string(ptr) };
            if !ptr.is_null() {
                unsafe { (self.free_string)(ptr) };
            }
            s.ok_or_else(|| "meta null".to_string())
        })
    }
}

#[async_trait::async_trait]
impl NorthPlugin for NorthSoAdapter {
    fn meta(&self) -> PluginMeta {
        self.cached_meta
            .get_or_init(|| {
                let s = self.meta_json().unwrap_or_else(|_| {
                    serde_json::json!({"name":"?","kind":"north","version":"0.0.0"}).to_string()
                });
                let f: FfiPluginMeta = serde_json::from_str(&s).unwrap_or_default();
                PluginMeta {
                    name: Box::leak(f.name.into_boxed_str()),
                    kind: PluginKind::North,
                    description: f.description.map(|x| {
                        let b = Box::leak(x.into_boxed_str());
                        b as &str
                    }),
                    version: Box::leak(f.version.into_boxed_str()),
                    name_zh: f.name_zh.map(|x| {
                        let b = Box::leak(x.into_boxed_str());
                        b as &str
                    }),
                    name_en: f.name_en.map(|x| {
                        let b = Box::leak(x.into_boxed_str());
                        b as &str
                    }),
                    description_zh: f.description_zh.map(|x| {
                        let b = Box::leak(x.into_boxed_str());
                        b as &str
                    }),
                    description_en: f.description_en.map(|x| {
                        let b = Box::leak(x.into_boxed_str());
                        b as &str
                    }),
                }
            })
            .clone()
    }

    fn config_schema(&self) -> Option<ConfigSchema> {
        let ptr = unsafe { (self.config_schema)(self.handle) };
        let s = unsafe { gateway_sdk::ptr_to_string(ptr) };
        if !ptr.is_null() {
            unsafe { (self.free_string)(ptr) };
        }
        s.and_then(|s| serde_json::from_str(&s).ok())
    }

    async fn open(&self, node_id: NodeId, config: PluginConfig) -> gateway_sdk::PluginResult<()> {
        let n = serde_json::to_string(&node_id)
            .map_err(|e| gateway_sdk::PluginError::msg(e.to_string()))?;
        let c = serde_json::to_string(&config)
            .map_err(|e| gateway_sdk::PluginError::msg(e.to_string()))?;
        let free = self.free_string;
        let open = self.open;
        let set_log = self.set_log;
        self.run_sync(move |handle| {
            let n = cstr(&n);
            let c = cstr(&c);
            let ptr = unsafe { open(handle, n.as_ptr(), c.as_ptr()) };
            ffi_result(free, ptr)?;
            if let Some(set_log_fn) = set_log {
                unsafe {
                    set_log_fn(handle, n.as_ptr(), gateway_host_log as *const c_void);
                }
            }
            Ok(())
        })
        .map_err(gateway_sdk::PluginError::msg)
    }

    async fn close(&self, node_id: NodeId) -> gateway_sdk::PluginResult<()> {
        let n = serde_json::to_string(&node_id)
            .map_err(|e| gateway_sdk::PluginError::msg(e.to_string()))?;
        let free = self.free_string;
        let close = self.close;
        self.run_sync(move |handle| {
            let n = cstr(&n);
            let ptr = unsafe { close(handle, n.as_ptr()) };
            ffi_result(free, ptr)
        })
        .map_err(gateway_sdk::PluginError::msg)
    }

    async fn init(&self, node_id: NodeId) -> gateway_sdk::PluginResult<()> {
        let n = serde_json::to_string(&node_id)
            .map_err(|e| gateway_sdk::PluginError::msg(e.to_string()))?;
        let free = self.free_string;
        let init = self.init;
        self.run_sync(move |handle| {
            let n = cstr(&n);
            let ptr = unsafe { init(handle, n.as_ptr()) };
            ffi_result(free, ptr)
        })
        .map_err(gateway_sdk::PluginError::msg)
    }

    async fn uninit(&self, node_id: NodeId) -> gateway_sdk::PluginResult<()> {
        let n = serde_json::to_string(&node_id)
            .map_err(|e| gateway_sdk::PluginError::msg(e.to_string()))?;
        let free = self.free_string;
        let uninit = self.uninit;
        self.run_sync(move |handle| {
            let n = cstr(&n);
            let ptr = unsafe { uninit(handle, n.as_ptr()) };
            ffi_result(free, ptr)
        })
        .map_err(gateway_sdk::PluginError::msg)
    }

    async fn start(&self, node_id: NodeId) -> gateway_sdk::PluginResult<()> {
        let n = serde_json::to_string(&node_id)
            .map_err(|e| gateway_sdk::PluginError::msg(e.to_string()))?;
        let free = self.free_string;
        let start = self.start;
        self.run_sync(move |handle| {
            let n = cstr(&n);
            let ptr = unsafe { start(handle, n.as_ptr()) };
            ffi_result(free, ptr)
        })
        .map_err(gateway_sdk::PluginError::msg)
    }

    async fn stop(&self, node_id: NodeId) -> gateway_sdk::PluginResult<()> {
        let n = serde_json::to_string(&node_id)
            .map_err(|e| gateway_sdk::PluginError::msg(e.to_string()))?;
        let free = self.free_string;
        let stop = self.stop;
        self.run_sync(move |handle| {
            let n = cstr(&n);
            let ptr = unsafe { stop(handle, n.as_ptr()) };
            ffi_result(free, ptr)
        })
        .map_err(gateway_sdk::PluginError::msg)
    }

    async fn setting(
        &self,
        node_id: NodeId,
        config: PluginConfig,
    ) -> gateway_sdk::PluginResult<()> {
        let n = serde_json::to_string(&node_id)
            .map_err(|e| gateway_sdk::PluginError::msg(e.to_string()))?;
        let c = serde_json::to_string(&config)
            .map_err(|e| gateway_sdk::PluginError::msg(e.to_string()))?;
        let free = self.free_string;
        let setting = self.setting;
        self.run_sync(move |handle| {
            let n = cstr(&n);
            let c = cstr(&c);
            let ptr = unsafe { setting(handle, n.as_ptr(), c.as_ptr()) };
            ffi_result(free, ptr)
        })
        .map_err(gateway_sdk::PluginError::msg)
    }

    async fn set_subscriptions(
        &self,
        node_id: NodeId,
        subscriptions: &[GroupSubscription],
    ) -> gateway_sdk::PluginResult<()> {
        let n = serde_json::to_string(&node_id)
            .map_err(|e| gateway_sdk::PluginError::msg(e.to_string()))?;
        let s = serde_json::to_string(subscriptions)
            .map_err(|e| gateway_sdk::PluginError::msg(e.to_string()))?;
        let free = self.free_string;
        let set = self.set_subscriptions;
        self.run_sync(move |handle| {
            let n = cstr(&n);
            let s = cstr(&s);
            let ptr = unsafe { set(handle, n.as_ptr(), s.as_ptr()) };
            ffi_result(free, ptr)
        })
        .map_err(gateway_sdk::PluginError::msg)
    }

    async fn on_group_data(
        &self,
        node_id: NodeId,
        data: Arc<GroupData>,
    ) -> gateway_sdk::PluginResult<()> {
        let n = serde_json::to_string(&node_id)
            .map_err(|e| gateway_sdk::PluginError::msg(e.to_string()))?;
        let d = serde_json::to_string(data.as_ref())
            .map_err(|e| gateway_sdk::PluginError::msg(e.to_string()))?;
        let free = self.free_string;
        let on = self.on_group_data;
        self.run_sync(move |handle| {
            let n = cstr(&n);
            let d = cstr(&d);
            let ptr = unsafe { on(handle, n.as_ptr(), d.as_ptr()) };
            ffi_result(free, ptr)
        })
        .map_err(gateway_sdk::PluginError::msg)
    }

    async fn connection_status(&self, node_id: NodeId) -> Option<serde_json::Value> {
        let conn_fn = self.connection_status?;
        let n = serde_json::to_string(&node_id).ok()?;
        let free_fn = self.free_string;
        let json_opt = self
            .run_sync(move |handle| {
                let n_c = cstr(&n);
                let ptr = unsafe { conn_fn(handle, n_c.as_ptr()) };
                let s = unsafe { gateway_sdk::ptr_to_string(ptr) };
                if !ptr.is_null() {
                    unsafe { free_fn(ptr) };
                }
                Ok(s)
            })
            .ok()??;
        serde_json::from_str(&json_opt).ok()
    }
}

impl Drop for NorthSoAdapter {
    fn drop(&mut self) {
        if !self.handle.is_null() {
            unsafe { (self.destroy)(self.handle) };
        }
    }
}

/// 元数据访问抽象：让南/北向适配器共用同一套 meta 校验逻辑
trait MetaProvider {
    fn meta_json_of(&self) -> Result<String, String>;
}

impl MetaProvider for SouthSoAdapter {
    fn meta_json_of(&self) -> Result<String, String> {
        Self::meta_json(self)
    }
}

impl MetaProvider for NorthSoAdapter {
    fn meta_json_of(&self) -> Result<String, String> {
        Self::meta_json(self)
    }
}

/// FFI handle 的 Send 包装。
///
/// 安全性依据：handle 由适配器持有并在其析构前始终有效；每次调用都在独立线程上执行，
/// 因而插件内部状态不会被并发访问（这与适配器 `unsafe impl Send/Sync` 的前提一致）。
#[derive(Clone, Copy)]
struct SendHandle(*mut c_void);

impl SendHandle {
    /// 取回裸指针。注意：经过方法调用后闭包捕获的是 SendHandle 本身（Send），
    /// 而不是裸指针字段，否则精确捕获会让闭包不满足 Send。
    fn as_ptr(self) -> *mut c_void {
        self.0
    }
}

unsafe impl Send for SendHandle {}

unsafe impl Send for NorthSoAdapter {}
unsafe impl Sync for NorthSoAdapter {}

impl PluginLoader {
    /// 扫描 `plugins_dir` 下动态库（Windows: *.dll，Unix: *.so），加载南/北向插件并注册到 `mgr`；返回 Loader 以保持库常驻。
    pub fn load(plugins_dir: &Path, mgr: &mut crate::manager::Manager) -> Result<Self, String> {
        let ext = std::env::consts::DLL_EXTENSION; // "dll" on Windows, "so" on Unix
        let mut libraries = Vec::new();
        let entries = std::fs::read_dir(plugins_dir)
            .map_err(|e| format!("read_dir {}: {}", plugins_dir.display(), e))?;
        for e in entries {
            let e = e.map_err(|e| e.to_string())?;
            let p = e.path();
            if p.extension()
                .map(|x| x.to_string_lossy() == ext)
                .unwrap_or(false)
            {
                if let Err(e) = Self::load_one(&p, mgr, &mut libraries) {
                    warn!(path = %p.display(), "load plugin: {}", e);
                }
            }
        }
        Ok(PluginLoader {
            _libraries: libraries,
        })
    }

    fn load_one(
        path: &Path,
        mgr: &mut crate::manager::Manager,
        libraries: &mut Vec<Library>,
    ) -> Result<(), String> {
        let lib = unsafe { Library::new(path) }.map_err(|e| format!("Library::new: {}", e))?;

        // ABI 版本校验：符号存在且版本不符时拒绝加载。
        // 版本不一致意味着跨边界的数据结构约定已改变，早失败远好于运行期错乱。
        if let Ok(f) = unsafe {
            lib.get::<unsafe extern "C-unwind" fn() -> u32>(b"gateway_plugin_abi_version\0")
        } {
            let plugin_abi = unsafe { f() };
            let host_abi = gateway_sdk::ffi::FFI_ABI_VERSION;
            if plugin_abi != host_abi {
                return Err(format!(
                    "ABI version mismatch: plugin={} host={} (rebuild the plugin against this SDK)",
                    plugin_abi, host_abi
                ));
            }
        } else {
            warn!(
                path = %path.display(),
                "plugin does not export gateway_plugin_abi_version (built with an older SDK); loading anyway"
            );
        }

        let free_fn: FreeStringFn = unsafe {
            *lib.get(SYM_FREE_STRING)
                .map_err(|e| format!("get free_string: {}", e))?
        };

        if let Ok(adapter) = Self::try_south(&lib, free_fn) {
            let meta = Self::plugin_meta(&adapter, "south")?;
            info!(path = %path.display(), name = %meta.name, "loaded south .so");
            mgr.register_south(&meta.name, Arc::new(adapter));
            libraries.push(lib);
            return Ok(());
        }

        if let Ok(adapter) = Self::try_north(&lib, free_fn) {
            let meta = Self::plugin_meta(&adapter, "north")?;
            info!(path = %path.display(), name = %meta.name, "loaded north .so");
            mgr.register_north(&meta.name, Arc::new(adapter));
            libraries.push(lib);
            return Ok(());
        }

        Err("no south/north symbols found".to_string())
    }

    /// 校验插件 meta 是否可用，并返回解析结果。
    ///
    /// 注意不能只看 `meta_json()` 的 `Ok/Err`：插件的 meta 若 panic，导出函数内部的
    /// panic 捕获会把它转成 `{"ok":false,"err":...}`——那依然是一次**成功的字符串返回**。
    /// 若不做内容校验，`meta()` 会退化为占位名 `"?"`，从而把一个永远不可用的插件注册进去。
    fn plugin_meta<T: MetaProvider>(adapter: &T, kind: &str) -> Result<FfiPluginMeta, String> {
        let json = adapter
            .meta_json_of()
            .map_err(|e| format!("{} plugin meta unavailable: {}", kind, e))?;
        let meta: FfiPluginMeta = gateway_sdk::parse_value_result(Some(json.as_str()))
            .map_err(|e| format!("{} plugin meta unusable: {}", kind, e))?;
        if meta.name.trim().is_empty() || meta.name == "?" {
            return Err(format!("{} plugin meta has no usable name", kind));
        }
        Ok(meta)
    }

    fn try_south(lib: &Library, free_fn: FreeStringFn) -> Result<SouthSoAdapter, String> {
        let create: libloading::Symbol<CreateFn> =
            unsafe { lib.get(SYM_SOUTH_CREATE).map_err(|e| e.to_string())? };
        let handle = unsafe { create() };
        if handle.is_null() {
            return Err("south create returned null".to_string());
        }
        let destroy: DestroyFn = *unsafe { lib.get(SYM_SOUTH_DESTROY).map_err(|e| e.to_string())? };
        let meta_fn: MetaFn = *unsafe { lib.get(SYM_SOUTH_META).map_err(|e| e.to_string())? };
        let open: StrStrFn = *unsafe { lib.get(SYM_SOUTH_OPEN).map_err(|e| e.to_string())? };
        let close: StrFn = *unsafe { lib.get(SYM_SOUTH_CLOSE).map_err(|e| e.to_string())? };
        let init: StrFn = *unsafe { lib.get(SYM_SOUTH_INIT).map_err(|e| e.to_string())? };
        let uninit: StrFn = *unsafe { lib.get(SYM_SOUTH_UNINIT).map_err(|e| e.to_string())? };
        let start: StrFn = *unsafe { lib.get(SYM_SOUTH_START).map_err(|e| e.to_string())? };
        let stop: StrFn = *unsafe { lib.get(SYM_SOUTH_STOP).map_err(|e| e.to_string())? };
        let setting: StrStrFn = *unsafe { lib.get(SYM_SOUTH_SETTING).map_err(|e| e.to_string())? };
        let validate_tag: StrStrFn =
            *unsafe { lib.get(SYM_SOUTH_VALIDATE_TAG).map_err(|e| e.to_string())? };
        let poll_group: StrStrStrFn =
            *unsafe { lib.get(SYM_SOUTH_POLL_GROUP).map_err(|e| e.to_string())? };
        let write_tags: StrStrFn =
            *unsafe { lib.get(SYM_SOUTH_WRITE_TAGS).map_err(|e| e.to_string())? };
        let list_groups: StrFn =
            *unsafe { lib.get(SYM_SOUTH_LIST_GROUPS).map_err(|e| e.to_string())? };
        let list_tags: StrStrFn =
            *unsafe { lib.get(SYM_SOUTH_LIST_TAGS).map_err(|e| e.to_string())? };
        let config_schema: VoidFn = *unsafe {
            lib.get(SYM_SOUTH_CONFIG_SCHEMA)
                .map_err(|e| e.to_string())?
        };
        let tag_schema: VoidFn =
            *unsafe { lib.get(SYM_SOUTH_TAG_SCHEMA).map_err(|e| e.to_string())? };
        let set_log: Option<SetLogFn> = unsafe { lib.get(SYM_SOUTH_SET_LOG).ok().map(|s| *s) };

        Ok(SouthSoAdapter {
            handle,
            free_string: free_fn,
            destroy,
            meta_fn,
            open,
            close,
            init,
            uninit,
            start,
            stop,
            setting,
            validate_tag,
            poll_group,
            write_tags,
            list_groups,
            list_tags,
            config_schema,
            tag_schema,
            set_log,
            cached_meta: std::sync::OnceLock::new(),
        })
    }
}

unsafe impl Send for SouthSoAdapter {}
unsafe impl Sync for SouthSoAdapter {}

impl PluginLoader {
    fn try_north(lib: &Library, free_fn: FreeStringFn) -> Result<NorthSoAdapter, String> {
        let create: libloading::Symbol<CreateFn> =
            unsafe { lib.get(SYM_NORTH_CREATE).map_err(|e| e.to_string())? };
        let handle = unsafe { create() };
        if handle.is_null() {
            return Err("north create returned null".to_string());
        }
        let destroy: DestroyFn = *unsafe { lib.get(SYM_NORTH_DESTROY).map_err(|e| e.to_string())? };
        let meta_fn: MetaFn = *unsafe { lib.get(SYM_NORTH_META).map_err(|e| e.to_string())? };
        let open: StrStrFn = *unsafe { lib.get(SYM_NORTH_OPEN).map_err(|e| e.to_string())? };
        let close: StrFn = *unsafe { lib.get(SYM_NORTH_CLOSE).map_err(|e| e.to_string())? };
        let init: StrFn = *unsafe { lib.get(SYM_NORTH_INIT).map_err(|e| e.to_string())? };
        let uninit: StrFn = *unsafe { lib.get(SYM_NORTH_UNINIT).map_err(|e| e.to_string())? };
        let start: StrFn = *unsafe { lib.get(SYM_NORTH_START).map_err(|e| e.to_string())? };
        let stop: StrFn = *unsafe { lib.get(SYM_NORTH_STOP).map_err(|e| e.to_string())? };
        let setting: StrStrFn = *unsafe { lib.get(SYM_NORTH_SETTING).map_err(|e| e.to_string())? };
        let set_subscriptions: StrStrFn = *unsafe {
            lib.get(SYM_NORTH_SET_SUBSCRIPTIONS)
                .map_err(|e| e.to_string())?
        };
        let on_group_data: StrStrFn = *unsafe {
            lib.get(SYM_NORTH_ON_GROUP_DATA)
                .map_err(|e| e.to_string())?
        };
        let config_schema: VoidFn = *unsafe {
            lib.get(SYM_NORTH_CONFIG_SCHEMA)
                .map_err(|e| e.to_string())?
        };
        let connection_status: Option<StrFn> =
            unsafe { lib.get(SYM_NORTH_CONNECTION_STATUS).ok().map(|s| *s) };
        let set_log: Option<SetLogFn> = unsafe { lib.get(SYM_NORTH_SET_LOG).ok().map(|s| *s) };

        Ok(NorthSoAdapter {
            handle,
            free_string: free_fn,
            destroy,
            meta_fn,
            open,
            close,
            init,
            uninit,
            start,
            stop,
            setting,
            set_subscriptions,
            on_group_data,
            config_schema,
            connection_status,
            set_log,
            cached_meta: std::sync::OnceLock::new(),
        })
    }
}
