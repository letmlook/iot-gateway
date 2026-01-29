//! 网关配置：端口、数据目录、日志。优先级：环境变量 > 配置文件 > 默认（对标 Neuron）。

use std::path::{Path, PathBuf};

/// 全局 static 目录，main 启动时设置，fallback 读取（fallback 无 State）
pub static STATIC_DIR: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();

/// 网关配置
#[derive(Clone, Debug)]
pub struct Config {
    /// HTTP 监听端口
    pub port: u16,
    /// 数据目录（持久化 SQLite、备份等）
    pub data_dir: PathBuf,
    /// 前端静态资源根目录（如 web/dist）
    pub static_dir: PathBuf,
    /// 插件 .so 目录（对标 Neuron plugins 目录）
    pub plugins_dir: PathBuf,
    /// 日志 filter（RUST_LOG 或默认）
    pub log_filter: String,
    /// 关闭 API 认证（对标 NEURON_DISABLE_AUTH）
    pub disable_auth: bool,
    /// API Bearer Token；若设置则除 /api/health、/api/metrics 外需带 Authorization: Bearer <token>
    pub token: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "snake_case")]
struct ConfigFile {
    #[serde(default = "default_port")]
    port: u16,
    #[serde(default = "default_data_dir_str")]
    data_dir: String,
    #[serde(default = "default_static_dir_str")]
    static_dir: String,
    #[serde(default = "default_plugins_dir_str")]
    plugins_dir: String,
    #[serde(default = "default_log_filter")]
    log_filter: String,
    #[serde(default)]
    disable_auth: bool,
    #[serde(default)]
    token: Option<String>,
}

fn default_data_dir_str() -> String {
    "data".into()
}
fn default_static_dir_str() -> String {
    "web/dist".into()
}
fn default_plugins_dir_str() -> String {
    "plugins".into()
}

fn default_port() -> u16 {
    3000
}
fn default_data_dir() -> PathBuf {
    PathBuf::from("data")
}
fn default_static_dir() -> PathBuf {
    PathBuf::from("web/dist")
}
fn default_plugins_dir() -> PathBuf {
    PathBuf::from("plugins")
}
fn default_log_filter() -> String {
    "info,tower_http=debug".into()
}

impl Default for Config {
    fn default() -> Self {
        Self {
            port: default_port(),
            data_dir: default_data_dir(),
            static_dir: default_static_dir(),
            plugins_dir: default_plugins_dir(),
            log_filter: default_log_filter(),
            disable_auth: false,
            token: None,
        }
    }
}

impl Config {
    /// 从配置文件加载（JSON）。文件不存在或解析失败返回 None。
    pub fn from_file(path: &Path) -> Option<Self> {
        let data = std::fs::read_to_string(path).ok()?;
        let cf: ConfigFile = serde_json::from_str(&data).ok()?;
        Some(Config {
            port: cf.port,
            data_dir: PathBuf::from(cf.data_dir),
            static_dir: PathBuf::from(cf.static_dir),
            plugins_dir: PathBuf::from(cf.plugins_dir),
            log_filter: cf.log_filter,
            disable_auth: cf.disable_auth,
            token: cf.token,
        })
    }

    /// 加载配置：默认 -> 配置文件（若存在）-> 环境变量覆盖。优先级 环境变量 > 配置文件 > 默认。
    pub fn from_env() -> Self {
        let config_path = std::env::var("GATEWAY_CONFIG")
            .ok()
            .map(PathBuf::from)
            .or_else(|| {
                let p = PathBuf::from("config").join("gateway.json");
                if p.exists() {
                    Some(p)
                } else {
                    None
                }
            });
        let mut c = config_path
            .and_then(|p| Self::from_file(&p))
            .unwrap_or_else(Config::default);
        if let Ok(s) = std::env::var("GATEWAY_PORT") {
            if let Ok(p) = s.parse::<u16>() {
                c.port = p;
            }
        }
        if let Ok(s) = std::env::var("GATEWAY_DATA_DIR") {
            c.data_dir = PathBuf::from(s);
        }
        if let Ok(s) = std::env::var("GATEWAY_STATIC_DIR") {
            c.static_dir = PathBuf::from(s);
        }
        if let Ok(s) = std::env::var("GATEWAY_PLUGINS_DIR") {
            c.plugins_dir = PathBuf::from(s);
        }
        if let Ok(s) = std::env::var("RUST_LOG") {
            c.log_filter = s;
        }
        if let Ok(s) = std::env::var("GATEWAY_DISABLE_AUTH") {
            c.disable_auth = s == "1" || s.eq_ignore_ascii_case("true");
        }
        if let Ok(s) = std::env::var("GATEWAY_TOKEN") {
            if !s.is_empty() {
                c.token = Some(s);
            }
        }
        c
    }

    /// SQLite 数据库文件路径（对标 Neuron 配置落盘）
    pub fn data_db(&self) -> PathBuf {
        self.data_dir.join("data.db")
    }

    /// 兼容：原 JSON 路径，可用于迁移或导出
    pub fn data_file(&self) -> PathBuf {
        self.data_dir.join("data.json")
    }
}
