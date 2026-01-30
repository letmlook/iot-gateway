//! 一次性生成 RSA 密钥对，用于离线授权。
//! 运行：cargo run --example gen_license_keys
//! 会生成 scripts/license_private.pem 并打印公钥，将公钥粘贴到 src/license/license.rs 的 BUILTIN_PUBLIC_KEY_PEM。
//! 之后用 scripts/gen_license.py 读取私钥生成 license.dat。

use pkcs8::EncodePrivateKey;
use rsa::RsaPrivateKey;
use spki::EncodePublicKey;
use std::path::PathBuf;

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let mut rng = rand::rngs::OsRng;
    let private_key = RsaPrivateKey::new(&mut rng, 2048)?;
    let public_key = private_key.to_public_key();

    let private_pem = private_key.to_pkcs8_pem(Default::default())?;
    let public_pem = public_key.to_public_key_pem(Default::default())?;

    // 写入 scripts/license_private.pem（相对于 workspace 根目录）
    let manifest_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    let scripts_dir = manifest_dir.join("..").join("..").join("scripts");
    let private_path = scripts_dir.join("license_private.pem");
    std::fs::create_dir_all(&scripts_dir)?;
    std::fs::write(&private_path, private_pem.as_bytes())?;
    eprintln!("已写入私钥: {}", private_path.display());

    eprintln!("\n请将以下公钥粘贴到 gateway-server/src/license/license.rs 的 BUILTIN_PUBLIC_KEY_PEM：\n");
    println!("{}", public_pem.trim());
    Ok(())
}
