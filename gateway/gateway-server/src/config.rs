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
    /// 备份加密密钥（GATEWAY_BACKUP_SECRET）；未设置时随机生成，仅本进程可用
    pub backup_secret: String,
    /// 备份密钥是否为运行时随机生成（未配置 GATEWAY_BACKUP_SECRET）
    pub backup_secret_ephemeral: bool,
    /// CORS 允许的来源列表（GATEWAY_ALLOWED_ORIGINS，逗号分隔）；为空表示不允许任何跨域来源
    pub allowed_origins: Vec<String>,
    /// HTTP 监听地址（GATEWAY_BIND），默认 0.0.0.0
    pub bind: String,
    /// 敏感配置落盘加密密钥（GATEWAY_SECRET_KEY，或回退显式设置的 GATEWAY_BACKUP_SECRET）；None 表示明文存储并告警
    pub master_secret: Option<String>,
    /// 是否启用角色授权（RBAC）：默认开启；`GATEWAY_ENFORCE_ROLES=0` 可临时关闭以便灰度
    pub enforce_roles: bool,
    /// 持久化写合并窗口（毫秒）：窗口内的多次变更合并为一次 SQLite 事务；0 表示每次变更立即落盘
    pub persist_debounce_ms: u64,
    /// 消息总线容量（条）：慢消费者可积压的消息数，规模较大时可调高
    pub bus_capacity: usize,
    /// 全局采集并发上限：同时进行的 poll_group 次数，避免同一时刻同时打向设备
    pub max_concurrent_polls: usize,
    /// 插件加载模式：`inproc`（默认，进程内 FFI）| `process`（每个插件一个子进程，崩溃隔离）
    pub plugin_isolation: String,
    /// 进程隔离模式下 `gateway-plugin-host` 的路径；为空时按可执行文件同级目录查找
    pub plugin_host_bin: Option<String>,
    /// 是否启用本地历史存储（默认关闭：开启后会在数据目录写 history.db）
    pub history_enabled: bool,
    /// 历史保留时长（小时）
    pub history_retention_hours: u64,
    /// 历史落盘批间隔（毫秒）
    pub history_flush_ms: u64,
    /// 历史行数上限（超过后从最旧开始删除）
    pub history_max_rows: u64,
    /// 会话绝对过期秒数（GATEWAY_SESSION_TTL_SECS）：自登录签发起算，不因活动续期；0 表示永不过期
    pub session_ttl_secs: u64,
    /// 启动完整性检查模式（GATEWAY_DB_INTEGRITY=full|quick|off）：data.db 损坏时自动从 .bak 恢复
    pub db_integrity: String,
    /// 是否启用 OpenAPI 文档（Swagger UI + openapi.json）；默认关闭
    pub enable_docs: bool,
    /// WebSocket 最大并发连接数
    pub ws_max_clients: u32,
    /// WebSocket 节点状态快照推送间隔（毫秒）；0 = 关闭快照推送
    pub ws_snapshot_interval_ms: u64,
}

fn default_bind_str() -> String {
    "0.0.0.0".into()
}

/// 生成 32 字节随机密钥（hex），用于未显式配置时的备份加密
fn random_secret() -> String {
    use rand::Rng;
    let bytes: [u8; 32] = rand::thread_rng().gen();
    hex::encode(bytes)
}

/// 总线默认容量（与 gateway-core 的 BUS_CAPACITY 一致）
fn default_bus_capacity() -> usize {
    4096
}

/// 采集并发上限默认值（与 gateway-core 的 DEFAULT_MAX_CONCURRENT_POLLS 一致）
fn default_max_concurrent_polls() -> usize {
    32
}

/// 插件加载模式默认值：`inproc`
///
/// 默认保持进程内加载：延迟最低、部署最简单。需要跑**不可信或未充分验证**的插件、
/// 或插件里有段错误/abort 风险时，用 `GATEWAY_PLUGIN_ISOLATION=process` 打开进程隔离。
fn default_plugin_isolation() -> String {
    "inproc".to_string()
}

/// 持久化写合并窗口默认 300ms：足够合并一次页面上的连续操作，又不会让数据长时间只在内存里
fn default_persist_debounce_ms() -> u64 {
    300
}

/// 会话绝对过期默认 12 小时：自登录起算，到点失效；0 = 永不过期（完全恢复旧行为）
fn default_session_ttl_secs() -> u64 {
    43_200
}

/// 启动完整性检查默认 full：配置库体量小（MB 级以下），全量检查只在启动时执行一次
fn default_db_integrity() -> String {
    "full".into()
}

fn default_ws_max_clients() -> u32 {
    64
}

fn default_ws_snapshot_interval_ms() -> u64 {
    5000
}

fn parse_origins(s: &str) -> Vec<String> {
    s.split(',')
        .map(|x| x.trim().to_string())
        .filter(|x| !x.is_empty())
        .collect()
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
    #[serde(default)]
    backup_secret: String,
    #[serde(default)]
    allowed_origins: Vec<String>,
    #[serde(default = "default_bind_str")]
    bind: String,
    #[serde(default = "default_session_ttl_secs")]
    session_ttl_secs: u64,
    #[serde(default = "default_db_integrity")]
    db_integrity: String,
    #[serde(default)]
    enable_docs: bool,
    #[serde(default = "default_ws_max_clients")]
    ws_max_clients: u32,
    #[serde(default = "default_ws_snapshot_interval_ms")]
    ws_snapshot_interval_ms: u64,
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
            // 默认不提供密钥，from_env 会以随机值兜底并告警
            backup_secret: String::new(),
            backup_secret_ephemeral: true,
            allowed_origins: Vec::new(),
            bind: default_bind_str(),
            master_secret: None,
            enforce_roles: true,
            persist_debounce_ms: default_persist_debounce_ms(),
            bus_capacity: default_bus_capacity(),
            max_concurrent_polls: default_max_concurrent_polls(),
            plugin_isolation: default_plugin_isolation(),
            plugin_host_bin: None,
            history_enabled: false,
            history_retention_hours: 72,
            history_flush_ms: 1000,
            history_max_rows: 5_000_000,
            session_ttl_secs: default_session_ttl_secs(),
            db_integrity: default_db_integrity(),
            enable_docs: false,
            ws_max_clients: default_ws_max_clients(),
            ws_snapshot_interval_ms: default_ws_snapshot_interval_ms(),
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
            backup_secret_ephemeral: cf.backup_secret.is_empty(),
            backup_secret: cf.backup_secret,
            allowed_origins: cf.allowed_origins,
            bind: if cf.bind.is_empty() {
                default_bind_str()
            } else {
                cf.bind
            },
            master_secret: None,
            enforce_roles: true,
            persist_debounce_ms: default_persist_debounce_ms(),
            bus_capacity: default_bus_capacity(),
            max_concurrent_polls: default_max_concurrent_polls(),
            plugin_isolation: default_plugin_isolation(),
            plugin_host_bin: None,
            history_enabled: false,
            history_retention_hours: 72,
            history_flush_ms: 1000,
            history_max_rows: 5_000_000,
            session_ttl_secs: cf.session_ttl_secs,
            db_integrity: cf.db_integrity,
            enable_docs: cf.enable_docs,
            ws_max_clients: cf.ws_max_clients,
            ws_snapshot_interval_ms: cf.ws_snapshot_interval_ms,
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
            backup_secret: String::new(),
            allowed_origins: Vec::new(),
            bind: default_bind_str(),
            session_ttl_secs: default_session_ttl_secs(),
            db_integrity: default_db_integrity(),
            enable_docs: false,
            ws_max_clients: default_ws_max_clients(),
            ws_snapshot_interval_ms: default_ws_snapshot_interval_ms(),
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
            .unwrap_or_default();
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
                c.backup_secret_ephemeral = false;
            }
        }
        if let Ok(s) = std::env::var("GATEWAY_ALLOWED_ORIGINS") {
            c.allowed_origins = parse_origins(&s);
        }
        if let Ok(s) = std::env::var("GATEWAY_BIND") {
            if !s.is_empty() {
                c.bind = s;
            }
        }
        if let Ok(s) = std::env::var("GATEWAY_ENFORCE_ROLES") {
            c.enforce_roles = !(s == "0" || s.eq_ignore_ascii_case("false"));
        }
        if let Ok(s) = std::env::var("GATEWAY_BUS_CAPACITY") {
            if let Ok(n) = s.parse::<usize>() {
                c.bus_capacity = n;
            }
        }
        if let Ok(s) = std::env::var("GATEWAY_MAX_CONCURRENT_POLLS") {
            if let Ok(n) = s.parse::<usize>() {
                c.max_concurrent_polls = n;
            }
        }
        if let Ok(s) = std::env::var("GATEWAY_PLUGIN_ISOLATION") {
            if !s.is_empty() {
                c.plugin_isolation = s;
            }
        }
        if let Ok(s) = std::env::var("GATEWAY_PLUGIN_HOST_BIN") {
            if !s.is_empty() {
                c.plugin_host_bin = Some(s);
            }
        }
        if let Ok(s) = std::env::var("GATEWAY_HISTORY_ENABLED") {
            c.history_enabled = s == "1" || s.eq_ignore_ascii_case("true");
        }
        if let Ok(s) = std::env::var("GATEWAY_HISTORY_RETENTION_HOURS") {
            if let Ok(n) = s.parse::<u64>() {
                c.history_retention_hours = n;
            }
        }
        if let Ok(s) = std::env::var("GATEWAY_HISTORY_FLUSH_MS") {
            if let Ok(n) = s.parse::<u64>() {
                c.history_flush_ms = n;
            }
        }
        if let Ok(s) = std::env::var("GATEWAY_HISTORY_MAX_ROWS") {
            if let Ok(n) = s.parse::<u64>() {
                c.history_max_rows = n;
            }
        }
        if let Ok(s) = std::env::var("GATEWAY_PERSIST_DEBOUNCE_MS") {
            if let Ok(ms) = s.parse::<u64>() {
                c.persist_debounce_ms = ms;
            }
        }
        if let Ok(s) = std::env::var("GATEWAY_SESSION_TTL_SECS") {
            if let Ok(n) = s.parse::<u64>() {
                c.session_ttl_secs = n;
            }
        }
        if let Ok(s) = std::env::var("GATEWAY_DB_INTEGRITY") {
            if !s.is_empty() {
                c.db_integrity = s;
            }
        }
        if let Ok(s) = std::env::var("GATEWAY_ENABLE_DOCS") {
            c.enable_docs = s == "1" || s.eq_ignore_ascii_case("true");
        }
        if let Ok(s) = std::env::var("GATEWAY_WS_MAX_CLIENTS") {
            if let Ok(n) = s.parse::<u32>() {
                c.ws_max_clients = n;
            }
        }
        if let Ok(s) = std::env::var("GATEWAY_WS_SNAPSHOT_INTERVAL_MS") {
            if let Ok(n) = s.parse::<u64>() {
                c.ws_snapshot_interval_ms = n;
            }
        }
        // 未提供备份密钥时随机生成：保证不再有「写死在仓库中的默认密钥」
        if c.backup_secret.is_empty() {
            c.backup_secret = random_secret();
            c.backup_secret_ephemeral = true;
        }
        // 敏感配置落盘加密密钥：优先 GATEWAY_SECRET_KEY；回退显式设置的 GATEWAY_BACKUP_SECRET。
        // 随机（ephemeral）密钥不可用于加密落盘，否则重启后无法解密。
        c.master_secret = match std::env::var("GATEWAY_SECRET_KEY")
            .ok()
            .filter(|s| !s.is_empty())
        {
            Some(s) => Some(s),
            None => {
                if c.backup_secret_ephemeral {
                    None
                } else {
                    Some(c.backup_secret.clone())
                }
            }
        };
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

    /// 历史数据库文件路径（与配置库分开，便于单独备份/清理）
    pub fn history_db(&self) -> PathBuf {
        self.data_dir.join("history.db")
    }

    /// 组装历史存储配置
    pub fn history_cfg(&self) -> crate::history::HistoryConfig {
        crate::history::HistoryConfig {
            enabled: self.history_enabled,
            db_path: self.history_db(),
            retention_hours: self.history_retention_hours,
            flush_ms: self.history_flush_ms,
            batch_size: 500,
            max_rows: self.history_max_rows,
            channel_capacity: 8192,
        }
    }

    /// 离线授权文件路径（license.dat，置于数据目录）
    pub fn license_path(&self) -> PathBuf {
        self.data_dir.join(crate::license::LICENSE_FILENAME)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// from_file 逐字段硬拷贝：新字段必须显式带出（防漏配回归），缺省时回退默认 12 小时
    #[test]
    fn from_file_loads_session_ttl_and_defaults_apply() {
        let dir = std::env::temp_dir().join(format!("gw-cfg-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();

        let explicit = dir.join("explicit.json");
        std::fs::write(&explicit, r#"{"port": 3000, "session_ttl_secs": 3600}"#).unwrap();
        let c = Config::from_file(&explicit).expect("parse explicit");
        assert_eq!(c.session_ttl_secs, 3_600);

        let implicit = dir.join("implicit.json");
        std::fs::write(&implicit, r#"{"port": 3000}"#).unwrap();
        let c = Config::from_file(&implicit).expect("parse implicit");
        assert_eq!(c.session_ttl_secs, default_session_ttl_secs());
        assert_eq!(c.session_ttl_secs, 43_200);

        // db_integrity 同样必须被 from_file 显式带出（防漏配回归）
        let integrity = dir.join("integrity.json");
        std::fs::write(&integrity, r#"{"port": 3000, "db_integrity": "quick"}"#).unwrap();
        let c = Config::from_file(&integrity).expect("parse integrity");
        assert_eq!(c.db_integrity, "quick");
        let c = Config::from_file(&implicit).expect("parse implicit");
        assert_eq!(c.db_integrity, "full");

        let _ = std::fs::remove_dir_all(&dir);
    }
}
