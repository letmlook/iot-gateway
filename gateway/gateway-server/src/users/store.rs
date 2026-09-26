//! 用户存储：SQLite 表 users（每次操作在 spawn_blocking 中打开连接，保证 AppState: Send），密码 argon2，登录 token 内存缓存。

use argon2::password_hash::{rand_core::OsRng, PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::{Algorithm, Argon2, Params, Version};
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
  updated_at TEXT NOT NULL
);
CREATE UNIQUE INDEX IF NOT EXISTS idx_users_username ON users(username);
CREATE INDEX IF NOT EXISTS idx_users_token ON users(token);
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
    const ALPHABET: &[u8] =
        b"ABCDEFGHJKLMNPQRSTUVWXYZabcdefghijkmnopqrstuvwxyz23456789!@#$%^&*-_";
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
    pub created_at: String,
    pub updated_at: String,
}

/// 内部行
struct UserRow {
    id: String,
    username: String,
    password_hash: String,
    role: String,
    token: Option<String>,
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

/// 用户存储：仅保存 DB 路径与 token 缓存，DB 操作在 spawn_blocking 中执行，保证 Send。
/// 当 open 失败时使用 empty()，此时所有接口返回空/假/错误，不阻塞启动。
pub struct UserStore {
    db_path: Option<Arc<std::path::PathBuf>>,
    tokens: RwLock<std::collections::HashMap<String, UserRole>>,
    /// 首次初始化时生成的随机管理员口令（供启动日志/文件输出，取走后清空）
    initial_password: std::sync::OnceLock<String>,
    /// 登录失败计数：username -> (失败次数, 最近失败时间戳秒)
    failed_logins: RwLock<std::collections::HashMap<String, (u32, i64)>>,
}

impl UserStore {
    /// 禁用态：无 DB，用于 open 失败时保证服务仍能启动。
    pub fn empty() -> Self {
        Self {
            db_path: None,
            tokens: RwLock::new(std::collections::HashMap::new()),
            initial_password: std::sync::OnceLock::new(),
            failed_logins: RwLock::new(std::collections::HashMap::new()),
        }
    }

    /// 取出首次初始化时生成的随机管理员口令（只在本次进程首次创建 admin 时有值）
    pub fn take_initial_password(&self) -> Option<String> {
        self.initial_password.get().cloned()
    }

    fn path(&self) -> Option<Arc<std::path::PathBuf>> {
        self.db_path.clone()
    }

/// 把随机初始口令写入数据目录下的 `.admin_initial_password`（0600），失败仅告警不影响启动。
fn write_initial_password_file(db_path: &Path, pwd: &str) {
    let Some(dir) = db_path.parent() else { return };
    let file = dir.join(".admin_initial_password");
    if let Err(e) = std::fs::write(&file, pwd) {
        tracing::warn!("write {} failed: {}", file.display(), e);
        return;
    }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o600));
    }
}

/// 打开或创建数据库并确保 users 表存在；若不存在 admin 用户则创建，**口令为随机生成**
    /// （写入数据目录 `.admin_initial_password`，权限 0600，首次登录后应删除并修改口令）。
    pub fn open(db_path: &Path) -> Result<Self, String> {
        let path = db_path.to_path_buf();
        let (tokens, generated) = tokio::task::block_in_place(|| {
            let conn = rusqlite::Connection::open(&path).map_err(|e| e.to_string())?;
            conn.execute_batch(USERS_SCHEMA).map_err(|e| e.to_string())?;
            let mut stmt = conn.prepare("SELECT 1 FROM users WHERE username = ?1 LIMIT 1").map_err(|e| e.to_string())?;
            let has_admin = stmt.exists([DEFAULT_ADMIN_USERNAME]).map_err(|e| e.to_string())?;
            drop(stmt);
            let mut generated: Option<String> = None;
            if !has_admin {
                let pwd = generate_random_password();
                let id = Uuid::new_v4().to_string();
                let hash = hash_password(&pwd)?;
                let now = now_iso();
                conn.execute(
                    "INSERT INTO users (id, username, password_hash, role, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![&id, DEFAULT_ADMIN_USERNAME, &hash, "admin", &now, &now],
                ).map_err(|e| e.to_string())?;
                Self::write_initial_password_file(&path, &pwd);
                tracing::info!("default admin created with a RANDOM password (see .admin_initial_password in data dir); change it after first login");
                generated = Some(pwd);
            } else if std::env::var("GATEWAY_RESET_ADMIN_PASSWORD").map(|s| s == "1" || s.eq_ignore_ascii_case("true")).unwrap_or(false) {
                let pwd = generate_random_password();
                let hash = hash_password(&pwd)?;
                let now = now_iso();
                let n = conn.execute(
                    "UPDATE users SET password_hash = ?1, token = NULL, updated_at = ?2 WHERE username = ?3",
                    params![&hash, &now, DEFAULT_ADMIN_USERNAME],
                ).map_err(|e| e.to_string())?;
                if n > 0 {
                    Self::write_initial_password_file(&path, &pwd);
                    tracing::info!("admin password reset to a RANDOM value (unset GATEWAY_RESET_ADMIN_PASSWORD after login)");
                    generated = Some(pwd);
                }
            }
            let mut stmt = conn.prepare("SELECT token, role FROM users WHERE token IS NOT NULL AND token != ''").map_err(|e| e.to_string())?;
            let rows = stmt.query_map([], |r| {
                Ok((r.get::<_, String>(0)?, r.get::<_, String>(1)?))
            }).map_err(|e| e.to_string())?;
            let mut map = std::collections::HashMap::new();
            for (t, role) in rows.flatten() {
                map.insert(t, UserRole::from_str(&role));
            }
            Ok::<_, String>((map, generated))
        })?;
        let initial_password = std::sync::OnceLock::new();
        if let Some(p) = generated {
            let _ = initial_password.set(p);
        }
        Ok(Self {
            db_path: Some(Arc::new(path)),
            tokens: RwLock::new(tokens),
            initial_password,
            failed_logins: RwLock::new(std::collections::HashMap::new()),
        })
    }


    fn row_to_user(r: &UserRow) -> User {
        User {
            id: r.id.clone(),
            username: r.username.clone(),
            role: UserRole::from_str(&r.role),
            created_at: r.created_at.clone(),
            updated_at: r.updated_at.clone(),
        }
    }

    /// 登录：校验用户名密码，若成功则更新 token 并返回 (token, user)
    pub async fn login(&self, username: &str, password: &str) -> Result<(String, User), String> {
        let path = match self.path() {
            Some(p) => p,
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
                    tracing::warn!("login blocked: account '{}' locked ({}s left)", username, left);
                    return Err(format!("account temporarily locked, retry after {}s", left));
                }
            }
        }
        let password = password.to_string();
        tracing::info!("login attempt: username={}", username);
        let user_key = username.clone();
        let result = tokio::task::spawn_blocking(move || {
            let conn = rusqlite::Connection::open(path.as_path()).map_err(|e| {
                tracing::error!("login: db open failed: {}", e);
                e.to_string()
            })?;
            let mut stmt = conn.prepare(
                "SELECT id, username, password_hash, role, token, created_at, updated_at FROM users WHERE username = ?1",
            ).map_err(|e| {
                tracing::error!("login: prepare failed: {}", e);
                e.to_string()
            })?;
            let row = stmt.query_row([username.as_str()], |r| {
                Ok(UserRow {
                    id: r.get(0)?,
                    username: r.get(1)?,
                    password_hash: r.get(2)?,
                    role: r.get(3)?,
                    token: r.get(4)?,
                    created_at: r.get(5)?,
                    updated_at: r.get(6)?,
                })
            });
            let row: UserRow = match row {
                Ok(r) => r,
                Err(e) => {
                    tracing::warn!("login failed: user '{}' not found: {}", username, e);
                    return Err("invalid username or password".into());
                }
            };
            tracing::info!("login: found user id={}, verifying password", row.id);
            let ok = match verify_password(&password, &row.password_hash) {
                Ok(v) => v,
                Err(e) => {
                    tracing::error!("login: password verify error for user '{}': {}", username, e);
                    return Err(format!("verify: {}", e));
                }
            };
            if !ok {
                tracing::warn!("login failed: wrong password for user '{}'", username);
                return Err("invalid username or password".into());
            }
            let token = Uuid::new_v4().to_string();
            let now = now_iso();
            conn.execute(
                "UPDATE users SET token = ?1, updated_at = ?2 WHERE id = ?3",
                params![&token, &now, &row.id],
            ).map_err(|e| e.to_string())?;
            let old_token = row.token.clone();
            let user = Self::row_to_user(&row);
            tracing::info!("login success: user '{}' (id={})", username, row.id);
            Ok::<_, String>((token, user, old_token))
        })
        .await
        .map_err(|e| e.to_string())?;
        match result {
            Ok((token, user, old_token)) => {
                let mut tokens = self.tokens.write().await;
                if let Some(t) = old_token {
                    if !t.is_empty() {
                        tokens.remove(&t);
                    }
                }
                tokens.insert(token.clone(), user.role);
                drop(tokens);
                self.failed_logins.write().await.remove(&user_key);
                Ok((token, user))
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
        let path = match self.path() {
            Some(p) => p,
            None => return Ok(()),
        };
        let token = token.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = rusqlite::Connection::open(path.as_path()).map_err(|e| e.to_string())?;
            conn.execute(
                "UPDATE users SET token = NULL WHERE token = ?1",
                params![&token],
            )
            .map_err(|e| e.to_string())?;
            Ok::<_, String>(())
        })
        .await
        .map_err(|e| e.to_string())??;
        Ok(())
    }

    /// 取 token 对应用户的角色，供授权（RBAC）判定使用
    pub fn role_of(&self, token: &str) -> Option<UserRole> {
        self.tokens.try_read().ok().and_then(|t| t.get(token).copied())
    }

    pub async fn list(&self) -> Result<Vec<User>, String> {
        let path = match self.path() {
            Some(p) => p,
            None => return Ok(Vec::new()),
        };
        tokio::task::spawn_blocking(move || {
            let conn = rusqlite::Connection::open(path.as_path()).map_err(|e| e.to_string())?;
            let mut stmt = conn.prepare(
                "SELECT id, username, password_hash, role, token, created_at, updated_at FROM users ORDER BY created_at",
            ).map_err(|e| e.to_string())?;
            let rows = stmt.query_map([], |r| {
                Ok(UserRow {
                    id: r.get(0)?,
                    username: r.get(1)?,
                    password_hash: r.get(2)?,
                    role: r.get(3)?,
                    token: r.get(4)?,
                    created_at: r.get(5)?,
                    updated_at: r.get(6)?,
                })
            }).map_err(|e| e.to_string())?;
            let mut list = Vec::new();
            for row in rows {
                list.push(Self::row_to_user(&row.map_err(|e| e.to_string())?));
            }
            Ok(list)
        })
        .await
        .map_err(|e| e.to_string())?
    }

    pub async fn get(&self, id: &str) -> Result<Option<User>, String> {
        let path = match self.path() {
            Some(p) => p,
            None => return Ok(None),
        };
        let id = id.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = rusqlite::Connection::open(path.as_path()).map_err(|e| e.to_string())?;
            let mut stmt = conn.prepare(
                "SELECT id, username, password_hash, role, token, created_at, updated_at FROM users WHERE id = ?1",
            ).map_err(|e| e.to_string())?;
            let row = stmt.query_row([id.as_str()], |r| {
                Ok(UserRow {
                    id: r.get(0)?,
                    username: r.get(1)?,
                    password_hash: r.get(2)?,
                    role: r.get(3)?,
                    token: r.get(4)?,
                    created_at: r.get(5)?,
                    updated_at: r.get(6)?,
                })
            });
            match row {
                Ok(r) => Ok(Some(Self::row_to_user(&r))),
                Err(rusqlite::Error::QueryReturnedNoRows) => Ok(None),
                Err(e) => Err(e.to_string()),
            }
        })
        .await
        .map_err(|e| e.to_string())?
    }

    pub async fn create(&self, username: &str, password: &str, role: UserRole) -> Result<User, String> {
        let path = match self.path() {
            Some(p) => p,
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
        let id = tokio::task::spawn_blocking(move || {
            let conn = rusqlite::Connection::open(path.as_path()).map_err(|e| e.to_string())?;
            let id = Uuid::new_v4().to_string();
            let hash = hash_password(&password)?;
            let now = now_iso();
            conn.execute(
                "INSERT INTO users (id, username, password_hash, role, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                params![&id, &username, &hash, &role_str, &now, &now],
            ).map_err(|e| e.to_string())?;
            Ok::<_, String>(id)
        })
        .await
        .map_err(|e| e.to_string())??;
        self.get(&id).await.and_then(|o| o.ok_or_else(|| "user not found".into()))
    }

    pub async fn update_simple(&self, id: &str, username: Option<&str>, role: Option<UserRole>) -> Result<User, String> {
        let path = match self.path() {
            Some(p) => p,
            None => return Err("user management disabled".into()),
        };
        let id = id.to_string();
        let id_clone = id.clone();
        let username = username.map(|s| s.trim().to_string()).filter(|s| !s.is_empty());
        let role_str = role.map(|r| r.as_str().to_string());
        tokio::task::spawn_blocking(move || {
            let conn = rusqlite::Connection::open(path.as_path()).map_err(|e| e.to_string())?;
            let mut stmt = conn.prepare(
                "SELECT id, username, password_hash, role, token, created_at, updated_at FROM users WHERE id = ?1",
            ).map_err(|e| e.to_string())?;
            let current: UserRow = stmt.query_row([id_clone.as_str()], |r| {
                Ok(UserRow {
                    id: r.get(0)?,
                    username: r.get(1)?,
                    password_hash: r.get(2)?,
                    role: r.get(3)?,
                    token: r.get(4)?,
                    created_at: r.get(5)?,
                    updated_at: r.get(6)?,
                })
            }).map_err(|_| "user not found".to_string())?;
            let new_username = username.as_deref().unwrap_or(current.username.as_str());
            let new_role = role_str.as_deref().unwrap_or(current.role.as_str());
            let now = now_iso();
            conn.execute(
                "UPDATE users SET username = ?1, role = ?2, updated_at = ?3 WHERE id = ?4",
                params![new_username, new_role, &now, &id_clone],
            ).map_err(|e| e.to_string())?;
            Ok::<_, String>(())
        })
        .await
        .map_err(|e| e.to_string())??;
        self.get(&id).await.and_then(|o| o.ok_or_else(|| "user not found".into()))
    }

    pub async fn delete(&self, id: &str) -> Result<bool, String> {
        let path = match self.path() {
            Some(p) => p,
            None => return Ok(false),
        };
        let id = id.to_string();
        let (old_token, deleted) = tokio::task::spawn_blocking({
            let path = Arc::clone(&path);
            move || {
                let conn = rusqlite::Connection::open(path.as_path()).map_err(|e| e.to_string())?;
                let old_token: Option<String> = conn.query_row("SELECT token FROM users WHERE id = ?1", [id.as_str()], |r| r.get(0)).ok();
                let n = conn.execute("DELETE FROM users WHERE id = ?1", [id.as_str()]).map_err(|e| e.to_string())?;
                Ok::<_, String>((old_token, n > 0))
            }
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
        let path = match self.path() {
            Some(p) => p,
            None => return Err("user management disabled".into()),
        };
        if new_password.is_empty() {
            return Err("password required".into());
        }
        let id = id.to_string();
        let hash = hash_password(new_password)?;
        let now = now_iso();
        // 改密后原有会话必须立即失效：清空该用户的 token
        let old_token = tokio::task::spawn_blocking({
            let path = Arc::clone(&path);
            let id = id.clone();
            move || {
                let conn = rusqlite::Connection::open(path.as_path()).map_err(|e| e.to_string())?;
                let old: Option<String> = conn
                    .query_row("SELECT token FROM users WHERE id = ?1", [id.as_str()], |r| r.get(0))
                    .ok()
                    .flatten();
                let n = conn.execute(
                    "UPDATE users SET password_hash = ?1, token = NULL, updated_at = ?2 WHERE id = ?3",
                    params![&hash, &now, &id],
                ).map_err(|e| e.to_string())?;
                if n == 0 {
                    return Err::<Option<String>, String>("user not found".into());
                }
                Ok(old)
            }
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
        let path = match self.path() {
            Some(p) => p,
            None => return Ok(false),
        };
        tokio::task::spawn_blocking(move || {
            let conn = rusqlite::Connection::open(path.as_path()).map_err(|e| e.to_string())?;
            let n: i64 = conn.query_row("SELECT COUNT(1) FROM users", [], |r| r.get(0)).map_err(|e| e.to_string())?;
            Ok(n > 0)
        })
        .await
        .map_err(|e| e.to_string())?
    }
}
