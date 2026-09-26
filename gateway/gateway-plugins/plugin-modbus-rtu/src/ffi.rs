//! .so/.dll 插件 C ABI 导出，供网关 libloading 动态加载。

use gateway_sdk::ffi::{alloc_c_string, meta_to_ffi, FfiResult};
use gateway_sdk::types::DataValue;
use gateway_sdk::{GroupId, NodeId, PluginConfig, SouthPlugin, Tag, TagId};
use std::ffi::CStr;
use std::os::raw::{c_char, c_void};

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
        Err(e) => alloc_c_string(
            &serde_json::to_string(&FfiResult::failure(e.to_string())).unwrap_or_default(),
        ),
    }
}

fn ptr_from_cstr(ptr: *const c_char) -> Option<String> {
    if ptr.is_null() {
        return None;
    }
    unsafe { CStr::from_ptr(ptr).to_str().ok().map(|s| s.to_string()) }
}

#[no_mangle]
pub extern "C-unwind" fn gateway_south_plugin_create() -> *mut c_void {
    Box::into_raw(Box::new(super::ModbusRtuPlugin::new())) as *mut c_void
}

#[no_mangle]
pub unsafe extern "C-unwind" fn gateway_south_plugin_destroy(handle: *mut c_void) {
    if handle.is_null() {
        return;
    }
    let _ = Box::from_raw(handle as *mut super::ModbusRtuPlugin);
}

#[no_mangle]
pub unsafe extern "C-unwind" fn gateway_south_plugin_meta(handle: *mut c_void) -> *mut c_char {
    if handle.is_null() {
        return std::ptr::null_mut();
    }
    let p = &*(handle as *const super::ModbusRtuPlugin);
    let m = meta_to_ffi(&p.meta());
    alloc_c_string(&serde_json::to_string(&m).unwrap_or_default())
}

unsafe fn south(handle: *mut c_void) -> &'static mut super::ModbusRtuPlugin {
    &mut *(handle as *mut super::ModbusRtuPlugin)
}

#[no_mangle]
pub unsafe extern "C-unwind" fn gateway_south_plugin_open(
    handle: *mut c_void,
    node_id_json: *const c_char,
    config_json: *const c_char,
) -> *mut c_char {
    let n = ptr_from_cstr(node_id_json).unwrap_or_default();
    let c = ptr_from_cstr(config_json).unwrap_or_default();
    let node_id: NodeId = serde_json::from_str(&n).unwrap_or_default();
    let config: PluginConfig = serde_json::from_str(&c).unwrap_or_default();
    let p = south(handle);
    result_json(block_on(p.open(node_id, config)))
}

#[no_mangle]
pub unsafe extern "C-unwind" fn gateway_south_plugin_close(
    handle: *mut c_void,
    node_id_json: *const c_char,
) -> *mut c_char {
    let n = ptr_from_cstr(node_id_json).unwrap_or_default();
    let node_id: NodeId = serde_json::from_str(&n).unwrap_or_default();
    let p = south(handle);
    result_json(block_on(p.close(node_id)))
}

#[no_mangle]
pub unsafe extern "C-unwind" fn gateway_south_plugin_init(
    handle: *mut c_void,
    node_id_json: *const c_char,
) -> *mut c_char {
    let n = ptr_from_cstr(node_id_json).unwrap_or_default();
    let node_id: NodeId = serde_json::from_str(&n).unwrap_or_default();
    let p = south(handle);
    result_json(block_on(p.init(node_id)))
}

#[no_mangle]
pub unsafe extern "C-unwind" fn gateway_south_plugin_uninit(
    handle: *mut c_void,
    node_id_json: *const c_char,
) -> *mut c_char {
    let n = ptr_from_cstr(node_id_json).unwrap_or_default();
    let node_id: NodeId = serde_json::from_str(&n).unwrap_or_default();
    let p = south(handle);
    result_json(block_on(p.uninit(node_id)))
}

#[no_mangle]
pub unsafe extern "C-unwind" fn gateway_south_plugin_start(
    handle: *mut c_void,
    node_id_json: *const c_char,
) -> *mut c_char {
    let n = ptr_from_cstr(node_id_json).unwrap_or_default();
    let node_id: NodeId = serde_json::from_str(&n).unwrap_or_default();
    let p = south(handle);
    result_json(block_on(p.start(node_id)))
}

#[no_mangle]
pub unsafe extern "C-unwind" fn gateway_south_plugin_stop(
    handle: *mut c_void,
    node_id_json: *const c_char,
) -> *mut c_char {
    let n = ptr_from_cstr(node_id_json).unwrap_or_default();
    let node_id: NodeId = serde_json::from_str(&n).unwrap_or_default();
    let p = south(handle);
    result_json(block_on(p.stop(node_id)))
}

#[no_mangle]
pub unsafe extern "C-unwind" fn gateway_south_plugin_setting(
    handle: *mut c_void,
    node_id_json: *const c_char,
    config_json: *const c_char,
) -> *mut c_char {
    let n = ptr_from_cstr(node_id_json).unwrap_or_default();
    let c = ptr_from_cstr(config_json).unwrap_or_default();
    let node_id: NodeId = serde_json::from_str(&n).unwrap_or_default();
    let config: PluginConfig = serde_json::from_str(&c).unwrap_or_default();
    let p = south(handle);
    result_json(block_on(p.setting(node_id, config)))
}

#[no_mangle]
pub unsafe extern "C-unwind" fn gateway_south_plugin_validate_tag(
    handle: *mut c_void,
    node_id_json: *const c_char,
    tag_json: *const c_char,
) -> *mut c_char {
    let n = ptr_from_cstr(node_id_json).unwrap_or_default();
    let t = ptr_from_cstr(tag_json).unwrap_or_default();
    let node_id: NodeId = serde_json::from_str(&n).unwrap_or_default();
    let tag: Tag = match serde_json::from_str(&t) {
        Ok(x) => x,
        Err(_) => {
            return alloc_c_string(
                &serde_json::to_string(&FfiResult::failure("invalid tag json")).unwrap_or_default(),
            )
        }
    };
    let p = south(handle);
    result_json(block_on(p.validate_tag(node_id, &tag)))
}

#[no_mangle]
pub unsafe extern "C-unwind" fn gateway_south_plugin_poll_group(
    handle: *mut c_void,
    node_id_json: *const c_char,
    group_id_json: *const c_char,
    tags_json: *const c_char,
) -> *mut c_char {
    let n = ptr_from_cstr(node_id_json).unwrap_or_default();
    let g = ptr_from_cstr(group_id_json).unwrap_or_default();
    let t = ptr_from_cstr(tags_json).unwrap_or_default();
    let node_id: NodeId = serde_json::from_str(&n).unwrap_or_default();
    let group_id: GroupId = serde_json::from_str(&g).unwrap_or_default();
    let tags: Vec<Tag> = serde_json::from_str(&t).unwrap_or_default();
    let p = south(handle);
    match block_on(p.poll_group(node_id, group_id, &tags)) {
        Ok(v) => alloc_c_string(&serde_json::to_string(&v).unwrap_or_default()),
        Err(e) => alloc_c_string(
            &serde_json::to_string(&FfiResult::failure(e.to_string())).unwrap_or_default(),
        ),
    }
}

#[no_mangle]
pub unsafe extern "C-unwind" fn gateway_south_plugin_write_tags(
    handle: *mut c_void,
    node_id_json: *const c_char,
    values_json: *const c_char,
) -> *mut c_char {
    let n = ptr_from_cstr(node_id_json).unwrap_or_default();
    let v = ptr_from_cstr(values_json).unwrap_or_default();
    let node_id: NodeId = serde_json::from_str(&n).unwrap_or_default();
    let values: Vec<(Tag, DataValue)> = serde_json::from_str(&v).unwrap_or_default();
    let p = south(handle);
    result_json(block_on(p.write_tags(node_id, &values)))
}

#[no_mangle]
pub unsafe extern "C-unwind" fn gateway_south_plugin_list_groups(
    handle: *mut c_void,
    node_id_json: *const c_char,
) -> *mut c_char {
    let n = ptr_from_cstr(node_id_json).unwrap_or_default();
    let node_id: NodeId = serde_json::from_str(&n).unwrap_or_default();
    let p = south(handle);
    match block_on(p.list_groups(node_id)) {
        Ok(v) => alloc_c_string(&serde_json::to_string(&v).unwrap_or_default()),
        Err(e) => alloc_c_string(
            &serde_json::to_string(&FfiResult::failure(e.to_string())).unwrap_or_default(),
        ),
    }
}

#[no_mangle]
pub unsafe extern "C-unwind" fn gateway_south_plugin_list_tags(
    handle: *mut c_void,
    node_id_json: *const c_char,
    group_id_json: *const c_char,
) -> *mut c_char {
    let n = ptr_from_cstr(node_id_json).unwrap_or_default();
    let g = ptr_from_cstr(group_id_json).unwrap_or_default();
    let node_id: NodeId = serde_json::from_str(&n).unwrap_or_default();
    let group_id: GroupId = serde_json::from_str(&g).unwrap_or_default();
    let p = south(handle);
    match block_on(p.list_tags(node_id, group_id)) {
        Ok(v) => alloc_c_string(&serde_json::to_string(&v).unwrap_or_default()),
        Err(e) => alloc_c_string(
            &serde_json::to_string(&FfiResult::failure(e.to_string())).unwrap_or_default(),
        ),
    }
}

#[no_mangle]
pub unsafe extern "C-unwind" fn gateway_south_plugin_config_schema(
    handle: *mut c_void,
) -> *mut c_char {
    if handle.is_null() {
        return std::ptr::null_mut();
    }
    let p = &*(handle as *const super::ModbusRtuPlugin);
    match p.config_schema() {
        Some(s) => alloc_c_string(&serde_json::to_string(&s).unwrap_or_default()),
        None => std::ptr::null_mut(),
    }
}

#[no_mangle]
pub unsafe extern "C-unwind" fn gateway_south_plugin_tag_schema(
    handle: *mut c_void,
) -> *mut c_char {
    if handle.is_null() {
        return std::ptr::null_mut();
    }
    let p = &*(handle as *const super::ModbusRtuPlugin);
    match p.tag_schema() {
        Some(s) => alloc_c_string(&serde_json::to_string(&s).unwrap_or_default()),
        None => std::ptr::null_mut(),
    }
}
