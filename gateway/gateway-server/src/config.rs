//! 网关配置：端口、数据目录、日志。优先级：环境变量 > 配置文件 > 默认。

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
    /// 插件 .so 目录
    pub plugins_dir: PathBuf,
    /// 日志 filter（RUST_LOG 或默认），用于细粒度控制（如 info,tower_http=debug）
    pub log_filter: String,
    /// 主程序日志打印级别：trace | debug | info | warn | error；若设置则作为默认级别，可与 log_filter 组合
    pub log_level: String,
    /// 主程序日志文件路径；若设置则同时写入该文件（按日滚动）
    pub log_file: Option<PathBuf>,
    /// 各节点独立日志目录；若设置则带 node_id 的日志会额外写入 logs/nodes/<node_id>.log
    pub log_dir_nodes: Option<PathBuf>,
    /// 关闭 API 认证（GATEWAY_DISABLE_AUTH=1 或 true）
    pub disable_auth: bool,
    /// API Bearer Token；若设置则除 /api/health、/api/metrics 外需带 Authorization: Bearer <token>
    pub token: Option<String>,
    /// 备份加密密钥（GATEWAY_BACKUP_SECRET）；生产环境务必设置强密钥
    pub backup_secret: String,
}

#[derive(serde::Serialize, serde::Deserialize)]
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
    #[serde(default = "default_log_level")]
    log_level: String,
    #[serde(default)]
    log_file: Option<String>,
    #[serde(default)]
    log_dir_nodes: Option<String>,
    #[serde(default)]
    disable_auth: bool,
    #[serde(default)]
    token: Option<String>,
    #[serde(default = "default_backup_secret_str")]
    backup_secret: String,
}

fn default_backup_secret_str() -> String {
    std::env::var("GATEWAY_BACKUP_SECRET").unwrap_or_else(|_| {
        "gateway-backup-default-secret-change-in-production".to_string()
    })
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
fn default_log_level() -> String {
    "info".into()
}
/// 默认主日志文件路径（相对当前工作目录）
fn default_log_file() -> PathBuf {
    PathBuf::from("logs/gateway.log")
}
/// 默认节点日志目录（相对当前工作目录），各节点日志写入 logs/nodes/<node_id>.log
fn default_log_dir_nodes() -> PathBuf {
    PathBuf::from("logs/nodes")
}

impl Default for Config {
    fn default() -> Self {
        Self {
            port: default_port(),
            data_dir: default_data_dir(),
            static_dir: default_static_dir(),
            plugins_dir: default_plugins_dir(),
            log_filter: default_log_filter(),
            log_level: default_log_level(),
            log_file: Some(default_log_file()),
            log_dir_nodes: Some(default_log_dir_nodes()),
            disable_auth: false,
            token: None,
            backup_secret: default_backup_secret_str(),
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
            log_level: cf.log_level,
            log_file: cf.log_file.map(PathBuf::from),
            log_dir_nodes: cf.log_dir_nodes.map(PathBuf::from),
            disable_auth: cf.disable_auth,
            token: cf.token,
            backup_secret: if cf.backup_secret.is_empty() {
                default_backup_secret_str()
            } else {
                cf.backup_secret
            },
        })
    }

    /// 若配置文件不存在，则创建 config 目录并写入默认 gateway.json，便于用户查看和编辑。
    pub fn ensure_default_config_file() {
        let path = PathBuf::from("config").join("gateway.json");
        if path.exists() {
            return;
        }
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let default_cfg = ConfigFile {
            port: default_port(),
            data_dir: default_data_dir_str(),
            static_dir: default_static_dir_str(),
            plugins_dir: default_plugins_dir_str(),
            log_filter: default_log_filter(),
            log_level: default_log_level(),
            log_file: Some(default_log_file().to_string_lossy().into_owned()),
            log_dir_nodes: Some(default_log_dir_nodes().to_string_lossy().into_owned()),
            disable_auth: false,
            token: None,
            backup_secret: default_backup_secret_str(),
        };
        if let Ok(json) = serde_json::to_string_pretty(&default_cfg) {
            let _ = std::fs::write(&path, json);
        }
    }

    /// 加载配置：默认 -> 配置文件（若存在）-> 环境变量覆盖。优先级 环境变量 > 配置文件 > 默认。
    /// 若未设置 GATEWAY_CONFIG 且 config/gateway.json 不存在，会先创建默认配置文件再加载。
    pub fn from_env() -> Self {
        let config_path = std::env::var("GATEWAY_CONFIG")
            .ok()
            .map(PathBuf::from)
            .or_else(|| {
                let p = PathBuf::from("config").join("gateway.json");
                if p.exists() {
                    Some(p)
                } else {
                    Self::ensure_default_config_file();
                    Some(p)
                }
            });
        let mut c = config_path
            .and_then(|p| Self::from_file(&p))
            .unwrap_or_else(Config::default);
        if c.backup_secret.is_empty() {
            c.backup_secret = default_backup_secret_str();
        }
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
        if let Ok(s) = std::env::var("GATEWAY_LOG_LEVEL") {
            if !s.is_empty() {
                c.log_level = s;
            }
        }
        if let Ok(s) = std::env::var("GATEWAY_LOG_FILE") {
            if !s.is_empty() {
                c.log_file = Some(PathBuf::from(s));
            }
        }
        if let Ok(s) = std::env::var("GATEWAY_LOG_DIR_NODES") {
            if !s.is_empty() {
                c.log_dir_nodes = Some(PathBuf::from(s));
            }
        }
        if let Ok(s) = std::env::var("GATEWAY_DISABLE_AUTH") {
            c.disable_auth = s == "1" || s.eq_ignore_ascii_case("true");
        }
        if let Ok(s) = std::env::var("GATEWAY_TOKEN") {
            if !s.is_empty() {
                c.token = Some(s);
            }
        }
        if let Ok(s) = std::env::var("GATEWAY_BACKUP_SECRET") {
            if !s.is_empty() {
                c.backup_secret = s;
            }
        }
        c
    }

    /// SQLite 数据库文件路径
    pub fn data_db(&self) -> PathBuf {
        self.data_dir.join("data.db")
    }

    /// 兼容：原 JSON 路径，可用于迁移或导出
    pub fn data_file(&self) -> PathBuf {
        self.data_dir.join("data.json")
    }

    /// 离线授权文件路径（license.dat，置于数据目录）
    pub fn license_path(&self) -> PathBuf {
        self.data_dir.join(crate::license::LICENSE_FILENAME)
    }
}
