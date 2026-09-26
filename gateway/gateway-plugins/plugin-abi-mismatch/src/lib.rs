//! ABI 不匹配夹具（**测试夹具**）。
//!
//! 只导出 `gateway_plugin_abi_version`，且故意返回一个与宿主不同的值。
//! 宿主应在加载阶段就拒绝它——如果这个夹具被成功注册，说明 ABI 门禁失效了。

/// 与宿主 `gateway_sdk::ffi::FFI_ABI_VERSION` 故意不一致的版本号
#[no_mangle]
pub extern "C-unwind" fn gateway_plugin_abi_version() -> u32 {
    0xDEAD_BEEF
}
