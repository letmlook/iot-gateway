//! 完全离线的软件授权：机器码、RSA 验签、功能分级。
//! 禁止任何网络请求，仅依赖本地 license.dat 与内置公钥。

mod error;
mod feature;
mod hardware;
mod license;

pub use feature::FeatureManager;
pub use hardware::machine_id;
pub use license::{load_and_verify_license, LicensePayload};
pub use license::LICENSE_FILENAME;
