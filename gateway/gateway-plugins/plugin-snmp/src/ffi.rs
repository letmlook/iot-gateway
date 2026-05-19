use super::lib::*;
use std::os::raw::{c_char, c_void};
#[no_mangle]
pub unsafe extern "C" fn gateway_south_plugin_create() -> *mut c_void {
    south_create()
}
#[no_mangle]
pub unsafe extern "C" fn gateway_south_plugin_destroy(handle: *mut c_void) {
    south_destroy(handle)
}
#[no_mangle]
pub unsafe extern "C" fn gateway_south_plugin_meta() -> *mut c_char {
    south_meta()
}
#[no_mangle]
pub unsafe extern "C" fn gateway_south_plugin_free_string(s: *mut c_char) {
    south_free_string(s)
}
#[no_mangle]
pub unsafe extern "C" fn gateway_south_plugin_open(
    handle: *mut c_void,
    cfg: *const c_char,
) -> *mut c_char {
    south_open(handle, cfg)
}
#[no_mangle]
pub unsafe extern "C" fn gateway_south_plugin_close(
    handle: *mut c_void,
    nid: *const c_char,
) -> *mut c_char {
    south_close(handle, nid)
}
#[no_mangle]
pub unsafe extern "C" fn gateway_south_plugin_init(
    handle: *mut c_void,
    cfg: *const c_char,
) -> *mut c_char {
    south_init(handle, cfg)
}
#[no_mangle]
pub unsafe extern "C" fn gateway_south_plugin_uninit(
    handle: *mut c_void,
    nid: *const c_char,
) -> *mut c_char {
    south_uninit(handle, nid)
}
#[no_mangle]
pub unsafe extern "C" fn gateway_south_plugin_start(
    handle: *mut c_void,
    cfg: *const c_char,
) -> *mut c_char {
    south_start(handle, cfg)
}
#[no_mangle]
pub unsafe extern "C" fn gateway_south_plugin_stop(
    handle: *mut c_void,
    nid: *const c_char,
) -> *mut c_char {
    south_stop(handle, nid)
}
#[no_mangle]
pub unsafe extern "C" fn gateway_south_plugin_poll_group(
    handle: *mut c_void,
    nid: *const c_char,
    grp: *const c_char,
) -> *mut c_char {
    south_poll_group(handle, nid, grp)
}
#[no_mangle]
pub unsafe extern "C" fn gateway_south_plugin_validate_tag(
    handle: *mut c_void,
    nid: *const c_char,
    tag: *const c_char,
) -> *mut c_char {
    south_validate_tag(handle, nid, tag)
}
#[no_mangle]
pub unsafe extern "C" fn gateway_south_plugin_write_tags(
    handle: *mut c_void,
    nid: *const c_char,
    w: *const c_char,
) -> *mut c_char {
    south_write_tags(handle, nid, w)
}
#[no_mangle]
pub unsafe extern "C" fn gateway_south_plugin_list_groups(
    handle: *mut c_void,
    nid: *const c_char,
) -> *mut c_char {
    south_list_groups(handle, nid)
}
#[no_mangle]
pub unsafe extern "C" fn gateway_south_plugin_list_tags(
    handle: *mut c_void,
    nid: *const c_char,
    gid: *const c_char,
) -> *mut c_char {
    south_list_tags(handle, nid, gid)
}
#[no_mangle]
pub unsafe extern "C" fn gateway_south_plugin_config_schema(handle: *mut c_void) -> *mut c_char {
    south_config_schema(handle)
}
