//! .so plugin C ABI exports

use super::InfluxDBPlugin;
use gateway_sdk::ffi::{alloc_c_string, meta_to_ffi, FfiResult};
use gateway_sdk::{GroupData, GroupSubscription, NodeId, NorthPlugin, PluginConfig};
use std::ffi::CStr;
use std::os::raw::{c_char, c_void};
use std::sync::Arc;

fn block_on<F: std::future::Future>(f: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("build runtime")
        .block_on(f)
}

fn result_json(r: gateway_sdk::PluginResult<()>) -> *mut c_char {
    match r {
        Ok(()) => alloc_c_string(&serde_json::to_string(&FfiResult::success()).unwrap_or_default()),
        Err(e) => alloc_c_string(&serde_json::to_string(&FfiResult::failure(e.to_string())).unwrap_or_default()),
    }
}

fn ptr_from_cstr(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(ptr).to_str().ok().map(|s| s.to_string()) }
}

#[no_mangle]
pub extern "C" fn gateway_north_plugin_create() -> *mut c_void {
    Box::into_raw(Box::new(InfluxDBPlugin::new())) as *mut c_void
}

#[no_mangle]
pub unsafe extern "C" fn gateway_north_plugin_destroy(handle: *mut c_void) {
    if handle.is_null() {
        return;
    }
    let _ = Box::from_raw(handle as *mut InfluxDBPlugin);
}

#[no_mangle]
pub unsafe extern "C" fn gateway_north_plugin_meta(handle: *mut c_void) -> *mut c_char {
    if handle.is_null() {
        return std::ptr::null_mut();
    }
    let p = &*(handle as *const InfluxDBPlugin);
    let m = meta_to_ffi(&p.meta());
    alloc_c_string(&serde_json::to_string(&m).unwrap_or_default())
}

unsafe fn plugin(handle: *mut c_void) -> &'static mut InfluxDBPlugin {
    &mut *(handle as *mut InfluxDBPlugin)
}

#[no_mangle]
pub unsafe extern "C" fn gateway_north_plugin_open(
    handle: *mut c_void,
    node_id_json: *const c_char,
    config_json: *const c_char,
) -> *mut c_char {
    let n = ptr_from_cstr(node_id_json).unwrap_or_default();
    let c = ptr_from_cstr(config_json).unwrap_or_default();
    let node_id: NodeId = serde_json::from_str(&n).unwrap_or_default();
    let config: PluginConfig = serde_json::from_str(&c).unwrap_or_default();
    let p = plugin(handle);
    result_json(block_on(p.open(node_id, config)))
}

#[no_mangle]
pub unsafe extern "C" fn gateway_north_plugin_close(
    handle: *mut c_void,
    node_id_json: *const c_char,
) -> *mut c_char {
    let n = ptr_from_cstr(node_id_json).unwrap_or_default();
    let node_id: NodeId = serde_json::from_str(&n).unwrap_or_default();
    let p = plugin(handle);
    result_json(block_on(p.close(node_id)))
}

#[no_mangle]
pub unsafe extern "C" fn gateway_north_plugin_init(
    handle: *mut c_void,
    node_id_json: *const c_char,
) -> *mut c_char {
    let n = ptr_from_cstr(node_id_json).unwrap_or_default();
    let node_id: NodeId = serde_json::from_str(&n).unwrap_or_default();
    let p = plugin(handle);
    result_json(block_on(p.init(node_id)))
}

#[no_mangle]
pub unsafe extern "C" fn gateway_north_plugin_uninit(
    handle: *mut c_void,
    node_id_json: *const c_char,
) -> *mut c_char {
    let n = ptr_from_cstr(node_id_json).unwrap_or_default();
    let node_id: NodeId = serde_json::from_str(&n).unwrap_or_default();
    let p = plugin(handle);
    result_json(block_on(p.uninit(node_id)))
}

#[no_mangle]
pub unsafe extern "C" fn gateway_north_plugin_start(
    handle: *mut c_void,
    node_id_json: *const c_char,
) -> *mut c_char {
    let n = ptr_from_cstr(node_id_json).unwrap_or_default();
    let node_id: NodeId = serde_json::from_str(&n).unwrap_or_default();
    let p = plugin(handle);
    result_json(block_on(p.start(node_id)))
}

#[no_mangle]
pub unsafe extern "C" fn gateway_north_plugin_stop(
    handle: *mut c_void,
    node_id_json: *const c_char,
) -> *mut c_char {
    let n = ptr_from_cstr(node_id_json).unwrap_or_default();
    let node_id: NodeId = serde_json::from_str(&n).unwrap_or_default();
    let p = plugin(handle);
    result_json(block_on(p.stop(node_id)))
}

#[no_mangle]
pub unsafe extern "C" fn gateway_north_plugin_set_subscriptions(
    handle: *mut c_void,
    node_id_json: *const c_char,
    subscriptions_json: *const c_char,
) -> *mut c_char {
    let n = ptr_from_cstr(node_id_json).unwrap_or_default();
    let s = ptr_from_cstr(subscriptions_json).unwrap_or_default();
    let node_id: NodeId = serde_json::from_str(&n).unwrap_or_default();
    let subs: Vec<GroupSubscription> = serde_json::from_str(&s).unwrap_or_default();
    let p = plugin(handle);
    result_json(block_on(p.set_subscriptions(node_id, &subs)))
}

#[no_mangle]
pub unsafe extern "C" fn gateway_north_plugin_on_group_data(
    handle: *mut c_void,
    node_id_json: *const c_char,
    group_data_json: *const c_char,
) -> *mut c_char {
    let n = ptr_from_cstr(node_id_json).unwrap_or_default();
    let d = ptr_from_cstr(group_data_json).unwrap_or_default();
    let node_id: NodeId = serde_json::from_str(&n).unwrap_or_default();
    let data: GroupData = match serde_json::from_str(&d) {
        Ok(x) => x,
        Err(_) => return alloc_c_string(&serde_json::to_string(&FfiResult::failure("invalid group_data json")).unwrap_or_default()),
    };
    let p = plugin(handle);
    result_json(block_on(p.on_group_data(node_id, Arc::new(data))))
}

#[no_mangle]
pub unsafe extern "C" fn gateway_north_plugin_connection_status(
    handle: *mut c_void,
    node_id_json: *const c_char,
) -> *mut c_char {
    if handle.is_null() {
        return std::ptr::null_mut();
    }
    let n = ptr_from_cstr(node_id_json).unwrap_or_default();
    let node_id: NodeId = serde_json::from_str(&n).unwrap_or_default();
    let p = plugin(handle);
    match block_on(p.connection_status(node_id)) {
        Some(v) => alloc_c_string(&serde_json::to_string(&v).unwrap_or_default()),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub unsafe extern "C" fn gateway_north_plugin_config_schema(handle: *mut c_void) -> *mut c_char {
    if handle.is_null() {
        return std::ptr::null_mut();
    }
    let p = &*(handle as *const InfluxDBPlugin);
    match p.config_schema() {
        Some(s) => alloc_c_string(&serde_json::to_string(&s).unwrap_or_default()),
        None => std::ptr::null_mut(),
    }
}
