//! 获取当前设备的机器码。
//! 运行：cargo run --example get_machine_id -p gateway-server

use machineid_rs::{Encryption, HWIDComponent, IdBuilder};
use sha2::{Digest, Sha256};

fn main() {
    match machine_id() {
        Ok(id) => println!("{}", id),
        Err(e) => eprintln!("获取机器码失败: {}", e),
    }
}

fn machine_id() -> Result<String, String> {
    let mut builder = IdBuilder::new(Encryption::SHA256);
    builder
        .add_component(HWIDComponent::SystemID)
        .add_component(HWIDComponent::CPUCores)
        .add_component(HWIDComponent::MachineName);
    let system_part = builder.build("").map_err(|e| e.to_string())?;

    let mac_str = first_mac_string();

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
