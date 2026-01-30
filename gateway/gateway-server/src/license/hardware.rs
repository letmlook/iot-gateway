//! 硬件特征采集：CPU/系统标识 + MAC，经 SHA-256 生成唯一机器码。
//! 完全离线，不进行任何网络请求。

use machineid_rs::{Encryption, IdBuilder, HWIDComponent};
use sha2::{Digest, Sha256};

/// 生成当前设备的唯一机器码（SHA-256 十六进制字符串）。
/// 组合：系统 UUID/CPU 相关组件 + 首块网卡 MAC，再整体做 SHA-256。
pub fn machine_id() -> Result<String, String> {
    // 1) 系统/CPU 相关组件（跨平台，无需管理员权限）
    let mut builder = IdBuilder::new(Encryption::SHA256);
    builder
        .add_component(HWIDComponent::SystemID)
        .add_component(HWIDComponent::CPUCores)
        .add_component(HWIDComponent::MachineName);
    let system_part = builder.build("").map_err(|e| e.to_string())?;

    // 2) 首块非回环网卡的 MAC（无则用占位，保证同一策略）
    let mac_str = first_mac_string();

    // 3) 组合后再做一次 SHA-256，作为最终机器码
    let combined = format!("{}:{}", system_part, mac_str);
    let hash = Sha256::digest(combined.as_bytes());
    Ok(hex::encode(hash))
}

fn first_mac_string() -> String {
    match mac_address::get_mac_address() {
        Ok(Some(addr)) => addr.to_string(),
        Ok(None) | Err(_) => "no-mac".to_string(),
    }
}

/// 仅在启用 license 时链接 hex；避免未使用 license 时多依赖。
/// 此处我们直接依赖 hex，保持实现简单；若希望零额外依赖，可改为手写 hex 编码。
pub(crate) fn sha256_hex(input: &[u8]) -> String {
    let hash = Sha256::digest(input);
    hex::encode(hash)
}
