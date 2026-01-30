//! 生成授权文件（调试用）。
//! 运行：cargo run --example gen_license -p gateway-server
//! 会在 data 目录下生成 license.dat

use base64::Engine;
use machineid_rs::{Encryption, HWIDComponent, IdBuilder};
use pkcs8::DecodePrivateKey;
use rsa::sha2::{Digest, Sha256};
use rsa::signature::Signer;
use serde_json::json;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // 1. 获取机器码
    let machine_id = get_machine_id()?;
    println!("机器码: {}", machine_id);

    // 2. 读取私钥
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let scripts_dir = manifest_dir.join("..").join("..").join("scripts");
    let private_path = scripts_dir.join("license_private.pem");
    let private_pem = std::fs::read_to_string(&private_path)?;
    let private_key = rsa::RsaPrivateKey::from_pkcs8_pem(&private_pem)?;

    // 3. 构建 payload
    let payload = json!({
        "machineId": machine_id,
        "expiryDate": "2030-12-31",
        "features": ["all_plugins"]
    });
    let payload_bytes = serde_json::to_vec(&payload)?;

    // 4. 签名
    let signing_key = rsa::pkcs1v15::SigningKey::<rsa::sha2::Sha256>::new(private_key);
    let signature = signing_key.sign(&payload_bytes);
    let signature_bytes: Box<[u8]> = signature.into();

    // 5. 生成授权文件
    let license = json!({
        "payload_b64": base64::engine::general_purpose::STANDARD.encode(&payload_bytes),
        "signature_b64": base64::engine::general_purpose::STANDARD.encode(&*signature_bytes)
    });
    let license_json = serde_json::to_string_pretty(&license)?;

    // 6. 写入 data/license.dat
    let data_dir = manifest_dir.join("..").join("..").join("data");
    std::fs::create_dir_all(&data_dir)?;
    let license_path = data_dir.join("license.dat");
    std::fs::write(&license_path, &license_json)?;

    println!("\n授权信息:");
    println!("  到期时间: 2030-12-31");
    println!("  功能: all_plugins (允许全部插件)");
    println!("\n已写入: {}", license_path.display());
    println!("\n授权文件内容:");
    println!("{}", license_json);

    Ok(())
}

fn get_machine_id() -> Result<String, String> {
    let mut builder = IdBuilder::new(Encryption::SHA256);
    builder
        .add_component(HWIDComponent::SystemID)
        .add_component(HWIDComponent::CPUCores)
        .add_component(HWIDComponent::MachineName);
    let system_part = builder.build("").map_err(|e| e.to_string())?;

    let mac_str = match mac_address::get_mac_address() {
        Ok(Some(addr)) => addr.to_string(),
        Ok(None) | Err(_) => "no-mac".to_string(),
    };

    let combined = format!("{}:{}", system_part, mac_str);
    let hash = Sha256::digest(combined.as_bytes());
    Ok(hex::encode(hash))
}
