//! 用户存储：SQLite 表 users（与配置快照共用 data.db 的进程内单长连接，操作在 spawn_blocking 中经
//! `persist::Db::with` 串行执行，保证 AppState: Send），密码 argon2，登录 token 内存缓存（含绝对过期时刻）。

use argon2::password_hash::{
    rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString,
};
use argon2::{Algorithm, Argon2, Params, Version};
use gateway_core::{Db, PersistError};
use rusqlite::params;
use serde::{Deserialize, Serialize};
use std::path::Path;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::sync::RwLock;
use uuid::Uuid;

const USERS_SCHEMA: &str = r#"
CREATE TABLE IF NOT EXISTS users (
  id TEXT PRIMARY KEY,
  username TEXT UNIQUE NOT NULL,
  password_hash TEXT NOT NULL,
  role TEXT NOT NULL DEFAULT 'operator',
  token TEXT,
  created_at TEXT NOT NULL,
  updated_at TEXT NOT NULL,
  tenant_id TEXT NOT NULL DEFAULT 'default'
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_users_username ON users(username);
CREATE INDEX IF NOT EXISTS idx_users_token ON users(token);
CREATE TABLE IF NOT EXISTS tenants (
  id TEXT PRIMARY KEY,
  name TEXT NOT NULL,
  created_at TEXT NOT NULL
);
"#;

/// 系统初始化时的默认管理员：用户名
const DEFAULT_ADMIN_USERNAME: &str = "admin";
/// 连续登录失败达到该次数后临时锁定账号，防止暴力破解
const MAX_FAILED_LOGINS: u32 = 5;
/// 登录失败锁定时长（秒）
const LOGIN_LOCK_SECS: i64 = 300;

/// 生成高强度随机口令（20 位：大小写字母 + 数字 + 符号，已剔除易混淆字符）
fn generate_random_password() -> String {
    use rand::Rng;
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789!@#$%^&*-_";
    let mut rng = rand::thread_rng();
    (0..20)
        .map(|_| ALPHABET[rng.gen_range(0..ALPHABET.len())] as char)
        .collect()
}

/// 当前 UNIX 时间戳（秒）
fn now_secs() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as i64
}

/// 用户角色
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum UserRole {
    Admin,
    Operator,
    Viewer,
}

impl UserRole {
    pub fn as_str(&self) -> &'static str {
        match self {
            UserRole::Admin => "admin",
            UserRole::Operator => "operator",
            UserRole::Viewer => "viewer",
        }
    }
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s {
            "admin" => UserRole::Admin,
            "operator" => UserRole::Operator,
            "viewer" => UserRole::Viewer,
            _ => UserRole::Operator,
        }
    }
}

/// 对外返回的用户信息（不含密码、token）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub id: String,
    pub username: String,
    pub role: UserRole,
    pub tenant_id: String,
    pub created_at: String,
    pub updated_at: String,
}

/// 内存会话条目：角色 + 所属域 + 绝对过期时刻（自登录签发起算，不因活动续期）
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Session {
    pub role: UserRole,
    pub tenant_id: String,
    /// 绝对过期时刻（UNIX 秒）；None = 永不过期（仅 ttl=0 时出现）
    pub expires_at: Option<i64>,
}

impl Session {
    /// 过期判定：`now == expires_at` 即视为过期；`expires_at = None` 永不过期
    pub fn is_expired(&self, now: i64) -> bool {
        match self.expires_at {
            Some(e) => now >= e,
            None => false,
        }
    }
}

/// 内部行
struct UserRow {
    id: String,
    username: String,
    password_hash: String,
    role: String,
    token: Option<String>,
    tenant_id: String,
    created_at: String,
    updated_at: String,
}

fn now_iso() -> String {
    let t = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default();
    format!("{}", t.as_secs())
}

/// 创建与校验共用同一算法与参数，避免不一致（Argon2id v19，默认 m/t/p）
fn argon2_ctx() -> Argon2<'static> {
    Argon2::new(Algorithm::Argon2id, Version::V0x13, Params::default())
}

fn hash_password(password: &str) -> Result<String, String> {
    let salt = SaltString::generate(&mut OsRng);
    argon2_ctx()
        .hash_password(password.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|e| e.to_string())
}

fn verify_password(password: &str, hash: &str) -> Result<bool, String> {
    let hash = hash.trim();
    let parsed = PasswordHash::new(hash).map_err(|e| e.to_string())?;
    Ok(argon2_ctx()
        .verify_password(password.as_bytes(), &parsed)
        .is_ok())
}

/// users 层错误统一为 String：业务错误（Validation）保留原文，其余透传 Display
fn err_string(e: PersistError) -> String {
    match e {
        PersistError::Validation(m) => m,
        other => other.to_string(),
    }
}

/// 用户存储：持有与配置快照共享的 data.db 长连接（persist::Db），DB 操作在 spawn_blocking 中
/// 经 `Db::with` 串行执行，保证 Send。当 open 失败时使用 empty()，此时所有接口返回空/假/错误，不阻塞启动。
pub struct UserStore {
    db: Option<Arc<Db>>,
    tokens: RwLock<std::collections::HashMap<String, Session>>,
    /// 会话绝对过期秒数；0 = 永不过期
    session_ttl_secs: u64,
    /// 首次初始化时生成的随机管理员口令（供启动日志/文件输出，取走后清空）
    initial_password: std::sync::OnceLock<String>,
    /// 登录失败计数：username -> (失败次数, 最近失败时间戳秒)
    failed_logins: RwLock<std::collections::HashMap<String, (u32, i64)>>,
}

impl UserStore {
    /// 禁用态：无 DB，用于 open 失败时保证服务仍能启动。
    pub fn empty() -> Self {
        Self {
            db: None,
            tokens: RwLock::new(std::collections::HashMap::new()),
            session_ttl_secs: 0,
            initial_password: std::sync::OnceLock::new(),
            failed_logins: RwLock::new(std::collections::HashMap::new()),
        }
    }

    /// 取出首次初始化时生成的随机管理员口令（只在本次进程首次创建 admin 时有值）
    pub fn take_initial_password(&self) -> Option<String> {
        self.initial_password.get().cloned()
    }

    fn db(&self) -> Option<Arc<Db>> {
        self.db.clone()
    }

    /// 把随机初始口令写入数据目录下的 `.admin_initial_password`（0600），失败仅告警不影响启动。
    fn write_initial_password_file(db_path: &Path, pwd: &str) {
        let Some(dir) = db_path.parent() else { return };
        let file = dir.join(".admin_initial_password");
        if let Err(e) = std::fs::write(&file, pwd) {
            tracing::warn!("write {} failed: {}", file.display(), e);
        } else {
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600));
            }
            #[cfg(not(unix))]
            {
                let _ = &file; // 非 Unix 平台暂无 0600 等价处理
            }
        }
    }

    /// 挂接到与配置快照共享的 data.db 长连接；若不存在 admin 用户则创建，**口令为随机生成**
    /// （写入数据目录 `.admin_initial_password`，权限 0600，首次登录后应删除并修改口令）。
    ///
    /// `session_ttl_secs` 为会话绝对过期秒数（自登录起算）；0 表示永不过期（完全恢复旧行为）。
    pub fn open(db: Arc<Db>, session_ttl_secs: u64) -> Result<Self, String> {
        let db_path = db.path().to_path_buf();
        let (tokens, generated) = tokio::task::block_in_place(|| {
            db.with(|conn| {
                conn.execute_batch(USERS_SCHEMA)?;

                // 增量迁移：users 表加 token_expiry 列（UNIX 秒；NULL = 未记录，如升级前签发的旧 token）。
                // 已有列时 ALTER 必然失败——「执行失败即视为列已存在」，不做脆弱的字符串精确匹配。
                if let Err(e) =
                    conn.execute_batch("ALTER TABLE users ADD COLUMN token_expiry INTEGER;")
                {
                    tracing::warn!(
                        "users: token_expiry column migration skipped (already applied?): {}",
                        e
                    );
                }

                // v2 迁移：users 表加 tenant_id 列（幂等检查——遍历所有列而非只看第一行）
                let has_tenant_id: bool = {
                    let mut stmt = conn.prepare("PRAGMA table_info(users)")?;
                    let mut rows = stmt.query([])?;
                    let mut found = false;
                    while let Some(row) = rows.next()? {
                        let col: String = row.get(1)?;
                        if col == "tenant_id" {
                            found = true;
                            break;
                        }
                    }
                    found
                };
                if !has_tenant_id {
                    if let Err(e) = conn.execute_batch(
                        "ALTER TABLE users ADD COLUMN tenant_id TEXT NOT NULL DEFAULT 'default';",
                    ) {
                        return Err(PersistError::Validation(format!(
                            "users tenant_id migration failed: {}",
                            e
                        )));
                    }
                }

                // 确保内置 'default' 租户存在
                let now = now_iso();
                conn.execute(
                    "INSERT OR IGNORE INTO tenants (id, name, created_at) VALUES ('default', 'Default', ?1)",
                    params![&now],
                )?;

                let mut stmt =
                    conn.prepare("SELECT 1 FROM users WHERE username = ?1 LIMIT 1")?;
                let has_admin = stmt.exists([DEFAULT_ADMIN_USERNAME])?;
                drop(stmt);
                let mut generated: Option<String> = None;
                if !has_admin {
                    let pwd = generate_random_password();
                    let id = Uuid::new_v4().to_string();
                    let hash =
                        hash_password(&pwd).map_err(PersistError::Validation)?;
                    let now = now_iso();
                    conn.execute(
                        "INSERT INTO users (id, username, password_hash, role, created_at, updated_at, tenant_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, 'default')",
                        params![&id, DEFAULT_ADMIN_USERNAME, &hash, "admin", &now, &now],
                    )?;
                    Self::write_initial_password_file(&db_path, &pwd);
                    tracing::info!("default admin created with a RANDOM password (see .admin_initial_password in data dir); change it after first login");
                    generated = Some(pwd);
                } else if std::env::var("GATEWAY_RESET_ADMIN_PASSWORD")
                    .map(|s| s == "1" || s.eq_ignore_ascii_case("true"))
                    .unwrap_or(false)
                {
                    let pwd = generate_random_password();
                    let hash =
                        hash_password(&pwd).map_err(PersistError::Validation)?;
                    let now = now_iso();
                    let n = conn.execute(
                        "UPDATE users SET password_hash = ?1, token = NULL, token_expiry = NULL, updated_at = ?2 WHERE username = ?3",
                        params![&hash, &now, DEFAULT_ADMIN_USERNAME],
                    )?;
                    if n > 0 {
                        Self::write_initial_password_file(&db_path, &pwd);
                        tracing::info!("admin password reset to a RANDOM value (unset GATEWAY_RESET_ADMIN_PASSWORD after login)");
                        generated = Some(pwd);
                    }
                }
                let mut map = std::collections::HashMap::new();
                if session_ttl_secs == 0 {
                    // ttl=0：永不过期，与旧行为一致——全部灌内存，expires_at = None
                    let mut stmt = conn
                        .prepare(
                            "SELECT token, role, tenant_id FROM users WHERE token IS NOT NULL AND token != ''",
                        )?;
                    let rows = stmt.query_map([], |r| -> rusqlite::Result<(String, String, String)> {
                        Ok((r.get(0)?, r.get(1)?, r.get(2)?))
                    })?;
                    for (t, role, tenant_id) in rows.flatten() {
                        map.insert(
                            t,
                            Session {
                                role: UserRole::from_str(&role),
                                tenant_id,
                                expires_at: None,
                            },
                        );
                    }
                } else {
                    // ttl>0：只加载尚未过期的会话；token_expiry 为 NULL 的存量旧 token（升级前签发）
                    // 一律视为已过期、不灌内存——升级后一次性强制重登录，安全优先。
                    // 内存 expires_at 直接沿用 DB 值，不是「再加一次 ttl」。
                    let now = now_secs();
                    let mut stmt = conn.prepare(
                        "SELECT token, role, tenant_id, token_expiry FROM users \
                         WHERE token IS NOT NULL AND token != '' \
                           AND token_expiry IS NOT NULL AND token_expiry > ?1",
                    )?;
                    let rows = stmt
                        .query_map([now], |r| -> rusqlite::Result<(String, String, String, i64)> {
                            Ok((r.get(0)?, r.get(1)?, r.get(2)?, r.get(3)?))
                        })?;
                    for (t, role, tenant_id, expiry) in rows.flatten() {
                        map.insert(
                            t,
                            Session {
                                role: UserRole::from_str(&role),
                                tenant_id,
                                expires_at: Some(expiry),
                            },
                        );
                    }
                }
                Ok((map, generated))
            })
            .map_err(err_string)
        })?;
        let initial_password = std::sync::OnceLock::new();
        if let Some(p) = generated {
            let _ = initial_password.set(p);
        }
        Ok(Self {
            db: Some(db),
            tokens: RwLock::new(tokens),
            session_ttl_secs,
            initial_password,
            failed_logins: RwLock::new(std::collections::HashMap::new()),
        })
    }

    fn row_to_user(r: &UserRow) -> User {
        User {
            id: r.id.clone(),
            username: r.username.clone(),
            role: UserRole::from_str(&r.role),
            tenant_id: r.tenant_id.clone(),
            created_at: r.created_at.clone(),
            updated_at: r.updated_at.clone(),
        }
    }

    /// 登录：校验用户名密码，若成功则更新 token 与绝对过期时刻并返回 (token, expires_at, user)
    pub async fn login(
        &self,
        username: &str,
        password: &str,
    ) -> Result<(String, Option<i64>, User), String> {
        let db = match self.db() {
            Some(d) => d,
            None => {
                tracing::warn!("login failed: user management disabled (no db path)");
                return Err("user management disabled".into());
            }
        };
        let username = username.trim().to_string();
        if username.is_empty() {
            tracing::warn!("login failed: username is empty");
            return Err("username required".into());
        }
        // 登录失败限流：连续失败达到阈值后临时锁定，防止暴力破解
        {
            let now = now_secs();
            let fl = self.failed_logins.read().await;
            if let Some((count, last)) = fl.get(username.as_str()) {
                if *count >= MAX_FAILED_LOGINS && now - *last < LOGIN_LOCK_SECS {
                    let left = LOGIN_LOCK_SECS - (now - *last);
                    tracing::warn!(
                        "login blocked: account '{}' locked ({}s left)",
                        username,
                        left
                    );
                    return Err(format!("account temporarily locked, retry after {}s", left));
                }
            }
        }
        let password = password.to_string();
        let self_ttl = self.session_ttl_secs;
        tracing::info!("login attempt: username={}", username);
        let user_key = username.clone();
        let result = tokio::task::spawn_blocking(move || {
            db.with(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, username, password_hash, role, token, tenant_id, created_at, updated_at FROM users WHERE username = ?1",
                )?;
                let row = stmt.query_row([username.as_str()], |r| {
                    Ok(UserRow {
                        id: r.get(0)?,
                        username: r.get(1)?,
                        password_hash: r.get(2)?,
                        role: r.get(3)?,
                        token: r.get(4)?,
                        tenant_id: r.get(5)?,
                        created_at: r.get(6)?,
                        updated_at: r.get(7)?,
                    })
                });
                let row: UserRow = match row {
                    Ok(r) => r,
                    Err(e) => {
                        tracing::warn!("login failed: user '{}' not found: {}", username, e);
                        return Err(PersistError::Validation(
                            "invalid username or password".into(),
                        ));
                    }
                };
                tracing::info!("login: found user id={}, verifying password", row.id);
                let ok = match verify_password(&password, &row.password_hash) {
                    Ok(v) => v,
                    Err(e) => {
                        tracing::error!(
                            "login: password verify error for user '{}': {}",
                            username,
                            e
                        );
                        return Err(PersistError::Validation(format!("verify: {}", e)));
                    }
                };
                if !ok {
                    tracing::warn!("login failed: wrong password for user '{}'", username);
                    return Err(PersistError::Validation(
                        "invalid username or password".into(),
                    ));
                }
                let token = Uuid::new_v4().to_string();
                let now = now_iso();
                // 绝对过期时刻自签发起算：ttl=0 表示永不过期（DB 落 NULL）
                let expires_at = if self_ttl == 0 {
                    None
                } else {
                    Some(now_secs() + self_ttl as i64)
                };
                conn.execute(
                    "UPDATE users SET token = ?1, token_expiry = ?2, updated_at = ?3 WHERE id = ?4",
                    params![&token, &expires_at, &now, &row.id],
                )?;
                let old_token = row.token.clone();
                let user = Self::row_to_user(&row);
                tracing::info!("login success: user '{}' (id={})", username, row.id);
                Ok((token, expires_at, user, old_token))
            })
            .map_err(err_string)
        })
        .await
        .map_err(|e| e.to_string())?;
        match result {
            Ok((token, expires_at, user, old_token)) => {
                let mut tokens = self.tokens.write().await;
                if let Some(t) = old_token {
                    if !t.is_empty() {
                        tokens.remove(&t);
                    }
                }
                // 顶号：旧 token 已移除；新 token 按登录时刻签发绝对过期
                tokens.insert(
                    token.clone(),
                    Session {
                        role: user.role,
                        tenant_id: user.tenant_id.clone(),
                        expires_at,
                    },
                );
                drop(tokens);
                self.failed_logins.write().await.remove(&user_key);
                Ok((token, expires_at, user))
            }
            Err(e) => {
                let now = now_secs();
                let mut fl = self.failed_logins.write().await;
                let entry = fl.entry(user_key.clone()).or_insert((0, now));
                entry.0 += 1;
                entry.1 = now;
                tracing::warn!(
                    "login failed for '{}': {} consecutive failure(s), {} left before lock",
                    user_key,
                    entry.0,
                    MAX_FAILED_LOGINS.saturating_sub(entry.0)
                );
                Err(e)
            }
        }
    }

    /// 登出：使该 token 立即失效（清空 DB 中的 token 并移出内存缓存）
    pub async fn logout(&self, token: &str) -> Result<(), String> {
        self.tokens.write().await.remove(token);
        let db = match self.db() {
            Some(d) => d,
            None => return Ok(()),
        };
        let token = token.to_string();
        tokio::task::spawn_blocking(move || {
            db.with(|conn| {
                conn.execute(
                    "UPDATE users SET token = NULL, token_expiry = NULL WHERE token = ?1",
                    params![&token],
                )?;
                Ok(())
            })
            .map_err(err_string)
        })
        .await
        .map_err(|e| e.to_string())??;
        Ok(())
    }

    /// 取 token 对应用户的角色与租户，供授权（RBAC）与租户判定使用。
    /// 过期判定收敛在此处：过期会话按未知 token 处理（认证中间件据此返回 401），
    /// 过期条目不在此处删除（需要写锁；条目数 = 用户数，每用户单 token，由下次登录顶号时惰性清出）。
    pub fn auth_of(&self, token: &str) -> Option<(UserRole, String)> {
        self.tokens.try_read().ok().and_then(|t| {
            let s = t.get(token)?;
            if s.is_expired(now_secs()) {
                None
            } else {
                Some((s.role, s.tenant_id.clone()))
            }
        })
    }

    /// 兼容性别名：仅返回角色（不含租户），供不需要租户的调用方使用。
    #[allow(dead_code)]
    pub fn role_of(&self, token: &str) -> Option<UserRole> {
        self.auth_of(token).map(|(role, _)| role)
    }

    pub async fn list(&self) -> Result<Vec<User>, String> {
        let db = match self.db() {
            Some(d) => d,
            None => return Ok(Vec::new()),
        };
        tokio::task::spawn_blocking(move || {
            db.with(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, username, password_hash, role, token, tenant_id, created_at, updated_at FROM users ORDER BY created_at",
                )?;
                let rows = stmt.query_map([], |r| {
                    Ok(UserRow {
                        id: r.get(0)?,
                        username: r.get(1)?,
                        password_hash: r.get(2)?,
                        role: r.get(3)?,
                        token: r.get(4)?,
                        tenant_id: r.get(5)?,
                        created_at: r.get(6)?,
                        updated_at: r.get(7)?,
                    })
                })?;
                let mut list = Vec::new();
                for row in rows {
                    list.push(Self::row_to_user(&row?));
                }
                Ok(list)
            })
            .map_err(err_string)
        })
        .await
        .map_err(|e| e.to_string())?
    }

    pub async fn get(&self, id: &str) -> Result<Option<User>, String> {
        let db = match self.db() {
            Some(d) => d,
            None => return Ok(None),
        };
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            db.with(|conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, username, password_hash, role, token, tenant_id, created_at, updated_at FROM users WHERE id = ?1",
                )?;
                let row = stmt.query_row([id.as_str()], |r| {
                    Ok(UserRow {
                        id: r.get(0)?,
                        username: r.get(1)?,
                        password_hash: r.get(2)?,
                        role: r.get(3)?,
                        token: r.get(4)?,
                        tenant_id: r.get(5)?,
                        created_at: r.get(6)?,
                        updated_at: r.get(7)?,
                    })
                });
                match row {
                    Ok(r) => Ok(Some(Self::row_to_user(&r))),
                    Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                    Err(e) => Err(PersistError::from(e)),
                }
            })
            .map_err(err_string)
        })
        .await
        .map_err(|e| e.to_string())?
    }

    pub async fn create(
        &self,
        username: &str,
        password: &str,
        role: UserRole,
        tenant_id: &str,
    ) -> Result<User, String> {
        let db = match self.db() {
            Some(d) => d,
            None => return Err("user management disabled".into()),
        };
        let username = username.trim().to_string();
        if password.is_empty() {
            return Err("password required".into());
        }
        if username.is_empty() {
            return Err("username required".into());
        }
        let password = password.to_string();
        let role_str = role.as_str().to_string();
        let tenant_id = tenant_id.to_string();
        let id = tokio::task::spawn_blocking(move || {
            db.with(move |conn| {
                let id = Uuid::new_v4().to_string();
                let hash = hash_password(&password).map_err(PersistError::Validation)?;
                let now = now_iso();
                conn.execute(
                    "INSERT INTO users (id, username, password_hash, role, created_at, updated_at, tenant_id) VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7)",
                    params![&id, &username, &hash, &role_str, &now, &now, &tenant_id],
                )?;
                Ok(id)
            })
            .map_err(err_string)
        })
        .await
        .map_err(|e| e.to_string())??;
        self.get(&id)
            .await
            .and_then(|o| o.ok_or_else(|| "user not found".into()))
    }

    pub async fn update_simple(
        &self,
        id: &str,
        username: Option<&str>,
        role: Option<UserRole>,
        tenant_id: Option<&str>,
    ) -> Result<User, String> {
        let db = match self.db() {
            Some(d) => d,
            None => return Err("user management disabled".into()),
        };
        let id = id.to_string();
        let id_clone = id.clone();
        let username = username
            .map(|s| s.trim().to_string())
            .filter(|s| !s.is_empty());
        let role_str = role.map(|r| r.as_str().to_string());
        let tenant_id = tenant_id.map(|s| s.to_string());
        tokio::task::spawn_blocking(move || {
            db.with(move |conn| {
                let mut stmt = conn.prepare(
                    "SELECT id, username, password_hash, role, token, tenant_id, created_at, updated_at FROM users WHERE id = ?1",
                )?;
                let current: UserRow = stmt.query_row([id_clone.as_str()], |r| {
                    Ok(UserRow {
                        id: r.get(0)?,
                        username: r.get(1)?,
                        password_hash: r.get(2)?,
                        role: r.get(3)?,
                        token: r.get(4)?,
                        tenant_id: r.get(5)?,
                        created_at: r.get(6)?,
                        updated_at: r.get(7)?,
                    })
                }).map_err(|_| {
                    PersistError::Validation("user not found".into())
                })?;
                let new_username = username.as_deref().unwrap_or(current.username.as_str());
                let new_role = role_str.as_deref().unwrap_or(current.role.as_str());
                let new_tenant = tenant_id.as_deref().unwrap_or(current.tenant_id.as_str());
                let now = now_iso();

                // 变更租户时必须使旧 token 失效（强制重新登录）
                let token_changed = tenant_id.as_ref().is_some_and(|t| *t != current.tenant_id);
                let sql = if token_changed {
                    "UPDATE users SET username = ?1, role = ?2, tenant_id = ?3, token = NULL, token_expiry = NULL, updated_at = ?4 WHERE id = ?5"
                } else {
                    "UPDATE users SET username = ?1, role = ?2, tenant_id = ?3, updated_at = ?4 WHERE id = ?5"
                };
                conn.execute(
                    sql,
                    params![new_username, new_role, new_tenant, &now, &id_clone],
                )?;
                Ok(())
            })
            .map_err(err_string)
        })
        .await
        .map_err(|e| e.to_string())??;
        self.get(&id)
            .await
            .and_then(|o| o.ok_or_else(|| "user not found".into()))
    }

    pub async fn delete(&self, id: &str) -> Result<bool, String> {
        let db = match self.db() {
            Some(d) => d,
            None => return Ok(false),
        };
        let id = id.to_string();
        let (old_token, deleted) = tokio::task::spawn_blocking(move || {
            db.with(move |conn| {
                let old_token: Option<String> = conn
                    .query_row(
                        "SELECT token FROM users WHERE id = ?1",
                        [id.as_str()],
                        |r| r.get(0),
                    )
                    .ok();
                let n = conn.execute("DELETE FROM users WHERE id = ?1", [id.as_str()])?;
                Ok((old_token, n > 0))
            })
            .map_err(err_string)
        })
        .await
        .map_err(|e| e.to_string())??;
        if let Some(t) = old_token {
            if !t.is_empty() {
                self.tokens.write().await.remove(&t);
            }
        }
        Ok(deleted)
    }

    pub async fn set_password(&self, id: &str, new_password: &str) -> Result<(), String> {
        let db = match self.db() {
            Some(d) => d,
            None => return Err("user management disabled".into()),
        };
        if new_password.is_empty() {
            return Err("password required".into());
        }
        let id = id.to_string();
        let hash = hash_password(new_password)?;
        let now = now_iso();
        // 改密后原有会话必须立即失效：清空该用户的 token
        let old_token = tokio::task::spawn_blocking(move || {
            db.with(move |conn| {
                let old: Option<String> = conn
                    .query_row("SELECT token FROM users WHERE id = ?1", [id.as_str()], |r| {
                        r.get(0)
                    })
                    .ok()
                    .flatten();
                let n = conn.execute(
                    "UPDATE users SET password_hash = ?1, token = NULL, token_expiry = NULL, updated_at = ?2 WHERE id = ?3",
                    params![&hash, &now, &id],
                )?;
                if n == 0 {
                    return Err(PersistError::Validation("user not found".into()));
                }
                Ok(old)
            })
            .map_err(err_string)
        })
        .await
        .map_err(|e| e.to_string())??;
        if let Some(t) = old_token {
            if !t.is_empty() {
                self.tokens.write().await.remove(&t);
            }
        }
        Ok(())
    }

    pub async fn has_any_user(&self) -> Result<bool, String> {
        let db = match self.db() {
            Some(d) => d,
            None => return Ok(false),
        };
        tokio::task::spawn_blocking(move || {
            db.with(|conn| {
                let n: i64 = conn.query_row("SELECT COUNT(1) FROM users", [], |r| r.get(0))?;
                Ok(n > 0)
            })
            .map_err(err_string)
        })
        .await
        .map_err(|e| e.to_string())?
    }
}

/// 租户行（供 API 层使用）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TenantRow {
    pub id: String,
    pub name: String,
    pub created_at: String,
}

impl UserStore {
    /// 列出所有租户
    pub async fn list_tenants(&self) -> Result<Vec<TenantRow>, String> {
        let db = match self.db() {
            Some(d) => d,
            None => return Ok(Vec::new()),
        };
        tokio::task::spawn_blocking(move || {
            db.with(|conn| {
                let mut stmt =
                    conn.prepare("SELECT id, name, created_at FROM tenants ORDER BY created_at")?;
                let rows = stmt.query_map([], |r| {
                    Ok(TenantRow {
                        id: r.get(0)?,
                        name: r.get(1)?,
                        created_at: r.get(2)?,
                    })
                })?;
                let mut list = Vec::new();
                for row in rows {
                    list.push(row?);
                }
                Ok(list)
            })
            .map_err(err_string)
        })
        .await
        .map_err(|e| e.to_string())?
    }

    /// 创建租户
    pub async fn create_tenant(&self, id: &str, name: &str) -> Result<(), String> {
        let db = match self.db() {
            Some(d) => d,
            None => return Err("user management disabled".into()),
        };
        let id = id.trim().to_string();
        let name = name.trim().to_string();
        if id.is_empty() || id.len() > 64 {
            return Err("tenant id must be 1-64 characters".into());
        }
        if !id
            .chars()
            .all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
        {
            return Err("tenant id must be lowercase letters, digits and hyphens only".into());
        }
        if name.is_empty() {
            return Err("tenant name required".into());
        }
        let now = now_iso();
        tokio::task::spawn_blocking(move || {
            db.with(move |conn| {
                conn.execute(
                    "INSERT INTO tenants (id, name, created_at) VALUES (?1, ?2, ?3)",
                    params![&id, &name, &now],
                )
                .map_err(|e| {
                    if e.to_string().contains("UNIQUE constraint") {
                        PersistError::Validation("tenant id already exists".into())
                    } else {
                        PersistError::Validation(e.to_string())
                    }
                })?;
                Ok(())
            })
            .map_err(err_string)
        })
        .await
        .map_err(|e| e.to_string())?
    }

    /// 删除租户（必须先确保无节点和用户引用）
    pub async fn delete_tenant(&self, id: &str) -> Result<(), String> {
        let db = match self.db() {
            Some(d) => d,
            None => return Err("user management disabled".into()),
        };
        if id == "default" {
            return Err("cannot delete the default tenant".into());
        }
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            db.with(move |conn| {
                // 检查节点引用
                let node_count: i64 = conn.query_row(
                    "SELECT COUNT(1) FROM nodes WHERE tenant_id = ?1",
                    [&id],
                    |r| r.get(0),
                )?;
                if node_count > 0 {
                    return Err(PersistError::Validation(format!(
                        "tenant has {} node(s), remove them first",
                        node_count
                    )));
                }
                // 检查用户引用
                let user_count: i64 = conn.query_row(
                    "SELECT COUNT(1) FROM users WHERE tenant_id = ?1",
                    [&id],
                    |r| r.get(0),
                )?;
                if user_count > 0 {
                    return Err(PersistError::Validation(format!(
                        "tenant has {} user(s), delete them first",
                        user_count
                    )));
                }
                conn.execute("DELETE FROM tenants WHERE id = ?1", [&id])
                    .map_err(|e| PersistError::Validation(e.to_string()))?;
                Ok(())
            })
            .map_err(err_string)
        })
        .await
        .map_err(|e| e.to_string())?
    }

    /// 检查租户是否存在
    #[allow(dead_code)]
    pub async fn tenant_exists(&self, id: &str) -> Result<bool, String> {
        let db = match self.db() {
            Some(d) => d,
            None => return Ok(false),
        };
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            db.with(|conn| {
                let n: i64 =
                    conn.query_row("SELECT COUNT(1) FROM tenants WHERE id = ?1", [&id], |r| {
                        r.get(0)
                    })?;
                Ok(n > 0)
            })
            .map_err(err_string)
        })
        .await
        .map_err(|e| e.to_string())?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use gateway_core::IntegrityMode;
    use rusqlite::Connection;

    fn tmp_dir(tag: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("gw-users-{}-{}", tag, Uuid::new_v4()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    /// 打开与 data.db 共享的长连接（Off：users 测试不关心 integrity 门禁）
    fn open_db(path: &Path) -> Arc<Db> {
        Db::open(path, IntegrityMode::Off).expect("db open")
    }

    // ---------- Session::is_expired 边界 ----------

    #[test]
    fn session_expiry_predicate_boundaries() {
        let never = Session {
            role: UserRole::Viewer,
            tenant_id: "default".to_string(),
            expires_at: None,
        };
        assert!(!never.is_expired(0), "expires_at=None 永不过期");
        assert!(!never.is_expired(i64::MAX));

        let s = Session {
            role: UserRole::Operator,
            tenant_id: "default".to_string(),
            expires_at: Some(1_000),
        };
        assert!(!s.is_expired(999), "now < expires_at 未过期");
        assert!(s.is_expired(1_000), "now == expires_at 视为过期");
        assert!(s.is_expired(1_001));
    }

    // ---------- 登录签发：DB token_expiry 与内存 expires_at 一致 ----------

    #[tokio::test(flavor = "multi_thread")]
    async fn login_issues_absolute_expiry_in_db_and_memory() {
        let dir = tmp_dir("ttl-login");
        let path = dir.join("data.db");
        let store = UserStore::open(open_db(&path), 3_600).expect("open");
        store
            .create("alice", "pw-123456", UserRole::Viewer, "default")
            .await
            .expect("create");

        let before = now_secs();
        let (token, expires_at, user) = store.login("alice", "pw-123456").await.expect("login");
        let after = now_secs();
        assert_eq!(user.username, "alice");
        let exp = expires_at.expect("ttl>0 必须产生过期时刻");
        assert!(
            exp >= before + 3_600 && exp <= after + 3_600,
            "expires_at 应自签发起算 3600s，实际 {}（now ∈ [{}, {}]）",
            exp,
            before,
            after
        );

        // 内存条目与 DB 列一致
        let mem = store
            .tokens
            .read()
            .await
            .get(&token)
            .expect("session cached")
            .clone();
        assert_eq!(mem.role, UserRole::Viewer);
        assert_eq!(mem.expires_at, Some(exp));
        let db: Option<i64> = Connection::open(&path)
            .unwrap()
            .query_row(
                "SELECT token_expiry FROM users WHERE username = 'alice'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(db, Some(exp), "DB token_expiry 与内存 expires_at 一致");
        assert_eq!(store.role_of(&token), Some(UserRole::Viewer));

        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn login_with_zero_ttl_never_expires() {
        let dir = tmp_dir("ttl-zero");
        let path = dir.join("data.db");
        let store = UserStore::open(open_db(&path), 0).expect("open");
        store
            .create("bob", "pw-123456", UserRole::Operator, "default")
            .await
            .expect("create");

        let (token, expires_at, _) = store.login("bob", "pw-123456").await.expect("login");
        assert_eq!(expires_at, None, "ttl=0 永不过期");
        let db: Option<i64> = Connection::open(&path)
            .unwrap()
            .query_row(
                "SELECT token_expiry FROM users WHERE username = 'bob'",
                [],
                |r| r.get(0),
            )
            .unwrap();
        assert_eq!(db, None, "ttl=0 时 DB 落 NULL");
        assert_eq!(store.role_of(&token), Some(UserRole::Operator));

        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---------- 启动加载过滤：过期与 NULL（存量旧 token）不灌内存 ----------

    #[tokio::test(flavor = "multi_thread")]
    async fn open_loads_only_unexpired_sessions() {
        let dir = tmp_dir("ttl-load");
        let path = dir.join("data.db");
        let now = now_secs();
        // 先用共享连接建库并预置三种会话行（过去 / 未来 / NULL = 升级前旧 token）
        let seed_db = open_db(&path);
        seed_db
            .with(|conn| {
                conn.execute_batch(USERS_SCHEMA)?;
                let _ = conn.execute_batch("ALTER TABLE users ADD COLUMN token_expiry INTEGER;");
                for (name, token, expiry) in [
                    ("old", "tok-past", Some(now - 100)),
                    ("cur", "tok-future", Some(now + 3_600)),
                    ("legacy", "tok-null", None),
                ] {
                    conn.execute(
                        "INSERT INTO users (id, username, password_hash, role, token, tenant_id, created_at, updated_at, token_expiry) \
                         VALUES (?1, ?2, 'x', 'viewer', ?3, 'default', '0', '0', ?4)",
                        params![Uuid::new_v4().to_string(), name, token, expiry],
                    )?;
                }
                Ok(())
            })
            .expect("seed");
        drop(seed_db);

        // ttl>0：仅加载未过期会话；token_expiry 为 NULL 的存量旧 token 视为已过期不灌内存
        //（open 未发现 admin 用户会创建默认 admin，随机口令写入临时目录，不影响断言）
        let store = UserStore::open(open_db(&path), 3_600).expect("open");
        assert_eq!(store.role_of("tok-future"), Some(UserRole::Viewer));
        assert_eq!(store.role_of("tok-past"), None, "过期 token 不灌内存");
        assert_eq!(
            store.role_of("tok-null"),
            None,
            "token_expiry 为 NULL 的存量旧 token 视为已过期（安全优先）"
        );
        let mem = store
            .tokens
            .read()
            .await
            .get("tok-future")
            .expect("loaded")
            .clone();
        assert_eq!(
            mem.expires_at,
            Some(now + 3_600),
            "内存沿用 DB 值，不再叠加 ttl"
        );

        // ttl=0：与旧行为一致，全部加载且永不过期
        let store0 = UserStore::open(open_db(&path), 0).expect("open ttl=0");
        assert_eq!(store0.role_of("tok-future"), Some(UserRole::Viewer));
        assert_eq!(store0.role_of("tok-past"), Some(UserRole::Viewer));
        assert_eq!(store0.role_of("tok-null"), Some(UserRole::Viewer));

        drop(store0);
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    // ---------- 顶号 / 登出 / 改密 / 删除用户与过期共存（回归） ----------

    #[tokio::test(flavor = "multi_thread")]
    async fn logout_password_change_relogin_and_delete_invalidate_sessions() {
        let dir = tmp_dir("ttl-invalidate");
        let path = dir.join("data.db");
        let store = UserStore::open(open_db(&path), 3_600).expect("open");
        let user = store
            .create("bob", "pw-old-123", UserRole::Operator, "default")
            .await
            .expect("create");

        // 顶号：同用户新登录使旧 token 失效（过期条目由此惰性清出内存）
        let (t1, e1, _) = store.login("bob", "pw-old-123").await.expect("login1");
        assert!(e1.is_some(), "ttl>0 登录必须产生过期时刻");
        let (t2, _, _) = store.login("bob", "pw-old-123").await.expect("login2");
        assert_eq!(store.role_of(&t2), Some(UserRole::Operator));
        assert_eq!(store.role_of(&t1), None, "顶号后旧 token 立即失效");

        // 登出：内存 + DB（token 与 token_expiry 一并置空）
        store.logout(&t2).await.expect("logout");
        assert_eq!(store.role_of(&t2), None);
        let (db_token, db_expiry): (Option<String>, Option<i64>) = Connection::open(&path)
            .unwrap()
            .query_row(
                "SELECT token, token_expiry FROM users WHERE username = 'bob'",
                [],
                |r| Ok((r.get(0)?, r.get(1)?)),
            )
            .unwrap();
        assert_eq!(db_token, None);
        assert_eq!(db_expiry, None);

        // 改密：原有会话立即失效
        let (t3, _, _) = store.login("bob", "pw-old-123").await.expect("login3");
        store
            .set_password(&user.id, "pw-new-456")
            .await
            .expect("set_password");
        assert_eq!(store.role_of(&t3), None, "改密后旧会话立即失效");
        let (t4, _, _) = store.login("bob", "pw-new-456").await.expect("login4");
        assert_eq!(store.role_of(&t4), Some(UserRole::Operator));

        // 删除用户：会话随之消失
        store.delete(&user.id).await.expect("delete");
        assert_eq!(store.role_of(&t4), None);

        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
