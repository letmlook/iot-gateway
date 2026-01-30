//! 编译时可选自动生成 license：与后端一致的机器码逻辑，生成 data/license.dat。
//! 仅当设置环境变量 GEN_LICENSE=1 时执行（避免 cargo 内再调 cargo 的锁问题）；否则可手动运行：python scripts/gen_license_auto.py

use std::path::PathBuf;
use std::process::Command;

fn main() {
    if std::env::var("GEN_LICENSE").as_deref() != Ok("1") {
        return;
    }

    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let workspace_root = manifest_dir.join("..").join("..").canonicalize();

    let Ok(root) = workspace_root else {
        return;
    };

    let script = root.join("scripts").join("gen_license_auto.py");
    if !script.is_file() {
        return;
    }

    let python_cmd = ["python3", "python"]
        .iter()
        .find(|cmd| Command::new(cmd).arg("--version").output().is_ok());

    let Some(python) = python_cmd else {
        eprintln!("cargo:warning=GEN_LICENSE=1: Python not found, skip (run: python scripts/gen_license_auto.py)");
        return;
    };

    let status = Command::new(python)
        .arg(script.as_os_str())
        .current_dir(&root)
        .status();

    if let Ok(s) = status {
        if !s.success() {
            eprintln!(
                "cargo:warning=gen_license_auto failed (pip install cryptography? run: python scripts/gen_license_auto.py)"
            );
        }
    }
}
