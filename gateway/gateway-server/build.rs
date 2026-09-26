//! 1) 注入 BUILD_DATE / GIT_REV（供 /api/version 返回）
//! 2) 编译时可选自动生成 license
//!
//! 第 2 项与后端一致的机器码逻辑，生成 data/license.dat；
//! 仅当设置环境变量 GEN_LICENSE=1 时执行（避免 cargo 内再调 cargo 的锁问题），
//! 否则可手动运行：python scripts/gen_license_auto.py

use std::path::PathBuf;
use std::process::Command;
use std::time::{SystemTime, UNIX_EPOCH};

/// 注入 BUILD_DATE 与 GIT_REV 环境变量，使 `/api/version` 不再返回空字符串
fn inject_build_metadata() {
    let build_date = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| format_timestamp(d.as_secs()))
        .unwrap_or_else(|_| "unknown".to_string());
    println!("cargo:rustc-env=BUILD_DATE={}", build_date);

    let git_rev = Command::new("git")
        .args(["rev-parse", "HEAD"])
        .output()
        .ok()
        .filter(|o| o.status.success())
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown".to_string());
    println!("cargo:rustc-env=GIT_REV={}", git_rev);

    // 每次构建刷新构建时间
    println!("cargo:rerun-if-changed=build.rs");
}

/// 秒级时间戳格式化为 YYYY-MM-DD HH:MM:SS（UTC），用整数运算避免为此引入新依赖
fn format_timestamp(secs: u64) -> String {
    let days = secs / 86_400;
    let rem = secs % 86_400;
    let hour = rem / 3600;
    let min = (rem % 3600) / 60;
    let sec = rem % 60;
    let (y, m, d) = civil_from_days(days as i64);
    format!(
        "{:04}-{:02}-{:02} {:02}:{:02}:{:02}",
        y, m, d, hour, min, sec
    )
}

/// 将「1970-01-01 起的天数」转换为年月日（civil_from_days 算法）
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 } / 146_097;
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let y = if m <= 2 { y + 1 } else { y };
    (y, m, d)
}

fn main() {
    // 无论是否生成 license，都注入构建元信息（供 /api/version 返回）
    inject_build_metadata();

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
