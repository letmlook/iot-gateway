use crate::*;
use std::os::raw::{c_char, c_void};
#[no_mangle]
pub unsafe extern "C" fn gateway_north_plugin_create() -> *mut c_void {
    north_create()
}
#[no_mangle]
pub unsafe extern "C" fn gateway_north_plugin_destroy(handle: *mut c_void) {
    north_destroy(handle)
}
#[no_mangle]
pub unsafe extern "C" fn gateway_north_plugin_meta() -> *mut c_char {
    north_meta()
}
#[no_mangle]
pub unsafe extern "C" fn gateway_north_plugin_free_string(s: *mut c_char) {
    north_free_string(s)
}
#[no_mangle]
pub unsafe extern "C" fn gateway_north_plugin_open(
    handle: *mut c_void,
    cfg: *const c_char,
) -> *mut c_char {
    north_open(handle, cfg)
}
#[no_mangle]
pub unsafe extern "C" fn gateway_north_plugin_close(
    handle: *mut c_void,
    nid: *const c_char,
) -> *mut c_char {
    north_close(handle, nid)
}
#[no_mangle]
pub unsafe extern "C" fn gateway_north_plugin_init(
    handle: *mut c_void,
    cfg: *const c_char,
) -> *mut c_char {
    north_init(handle, cfg)
}
#[no_mangle]
pub unsafe extern "C" fn gateway_north_plugin_uninit(
    handle: *mut c_void,
    nid: *const c_char,
) -> *mut c_char {
    north_uninit(handle, nid)
}
#[no_mangle]
pub unsafe extern "C" fn gateway_north_plugin_start(
    handle: *mut c_void,
    cfg: *const c_char,
) -> *mut c_char {
    north_start(handle, cfg)
}
#[no_mangle]
pub unsafe extern "C" fn gateway_north_plugin_stop(
    handle: *mut c_void,
    nid: *const c_char,
) -> *mut c_char {
    north_stop(handle, nid)
}
#[no_mangle]
pub unsafe extern "C" fn gateway_north_plugin_on_group_data(
    handle: *mut c_void,
    nid: *const c_char,
    data: *const c_char,
) -> *mut c_char {
    north_on_group_data(handle, nid, data)
}
#[no_mangle]
pub unsafe extern "C" fn gateway_north_plugin_set_subscriptions(
    handle: *mut c_void,
    nid: *const c_char,
    sub: *const c_char,
) -> *mut c_char {
    north_set_subscriptions(handle, nid, sub)
}
#[no_mangle]
pub unsafe extern "C" fn gateway_north_plugin_config_schema(handle: *mut c_void) -> *mut c_char {
    north_config_schema(handle)
}
#[no_mangle]
pub unsafe extern "C" fn gateway_north_plugin_connection_status(
    handle: *mut c_void,
    nid: *const c_char,
) -> *mut c_char {
    north_connection_status(handle, nid)
}
