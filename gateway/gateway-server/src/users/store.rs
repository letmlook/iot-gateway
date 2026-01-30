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
/// 系统初始化时的默认管理员密码（首次登录后建议修改）
const DEFAULT_ADMIN_PASSWORD: &str = "admin123";

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
    tokens: RwLock<std::collections::HashSet<String>>,
}

impl UserStore {
    /// 禁用态：无 DB，用于 open 失败时保证服务仍能启动。
    pub fn empty() -> Self {
        Self {
            db_path: None,
            tokens: RwLock::new(std::collections::HashSet::new()),
        }
    }

    fn path(&self) -> Option<Arc<std::path::PathBuf>> {
        self.db_path.clone()
    }

    /// 打开或创建数据库并确保 users 表存在；若不存在用户名为 admin 则创建默认 admin/admin123。
    pub fn open(db_path: &Path) -> Result<Self, String> {
        let path = db_path.to_path_buf();
        let tokens = tokio::task::block_in_place(|| {
            let conn = rusqlite::Connection::open(&path).map_err(|e| e.to_string())?;
            conn.execute_batch(USERS_SCHEMA).map_err(|e| e.to_string())?;
            let mut stmt = conn.prepare("SELECT 1 FROM users WHERE username = ?1 LIMIT 1").map_err(|e| e.to_string())?;
            let has_admin = stmt.exists([DEFAULT_ADMIN_USERNAME]).map_err(|e| e.to_string())?;
            drop(stmt);
            if !has_admin {
                let id = Uuid::new_v4().to_string();
                let hash = hash_password(DEFAULT_ADMIN_PASSWORD)?;
                let now = now_iso();
                conn.execute(
                    "INSERT INTO users (id, username, password_hash, role, created_at, updated_at) VALUES (?1, ?2, ?3, ?4, ?5, ?6)",
                    params![&id, DEFAULT_ADMIN_USERNAME, &hash, "admin", &now, &now],
                ).map_err(|e| e.to_string())?;
                tracing::info!("default admin user created (username: {}, please change password after first login)", DEFAULT_ADMIN_USERNAME);
            } else if std::env::var("GATEWAY_RESET_ADMIN_PASSWORD").map(|s| s == "1" || s.eq_ignore_ascii_case("true")).unwrap_or(false) {
                let hash = hash_password(DEFAULT_ADMIN_PASSWORD)?;
                let now = now_iso();
                let n = conn.execute(
                    "UPDATE users SET password_hash = ?1, updated_at = ?2 WHERE username = ?3",
                    params![&hash, &now, DEFAULT_ADMIN_USERNAME],
                ).map_err(|e| e.to_string())?;
                if n > 0 {
                    tracing::info!("admin password reset to default (GATEWAY_RESET_ADMIN_PASSWORD is set; please unset after login)");
                }
            }
            let mut stmt = conn.prepare("SELECT token FROM users WHERE token IS NOT NULL AND token != ''").map_err(|e| e.to_string())?;
            let rows = stmt.query_map([], |r| r.get::<_, String>(0)).map_err(|e| e.to_string())?;
            let mut set = std::collections::HashSet::new();
            for row in rows {
                if let Ok(t) = row {
                    set.insert(t);
                }
            }
            Ok::<_, String>(set)
        })?;
        Ok(Self {
            db_path: Some(Arc::new(path)),
            tokens: RwLock::new(tokens),
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
        let password = password.to_string();
        tracing::info!("login attempt: username={}", username);
        let (token, user) = tokio::task::spawn_blocking(move || {
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
            let user = Self::row_to_user(&row);
            tracing::info!("login success: user '{}' (id={})", username, row.id);
            Ok::<_, String>((token, user))
        })
        .await
        .map_err(|e| e.to_string())??;
        self.tokens.write().await.insert(token.clone());
        Ok((token, user))
    }

    /// 校验 Bearer token 是否有效（内存缓存）
    pub fn token_valid(&self, token: &str) -> bool {
        self.tokens.try_read().map(|t| t.contains(token)).unwrap_or(false)
    }

    /// 根据 token 取用户信息
    pub async fn get_user_by_token(&self, token: &str) -> Option<User> {
        let path = match self.path() {
            Some(p) => p,
            None => return None,
        };
        let token = token.to_string();
        tokio::task::spawn_blocking(move || {
            let conn = rusqlite::Connection::open(path.as_path()).ok()?;
            let mut stmt = conn.prepare(
                "SELECT id, username, password_hash, role, token, created_at, updated_at FROM users WHERE token = ?1",
            ).ok()?;
            let row: UserRow = stmt.query_row([token.as_str()], |r| {
                Ok(UserRow {
                    id: r.get(0)?,
                    username: r.get(1)?,
                    password_hash: r.get(2)?,
                    role: r.get(3)?,
                    token: r.get(4)?,
                    created_at: r.get(5)?,
                    updated_at: r.get(6)?,
                })
            }).ok()?;
            Some(Self::row_to_user(&row))
        })
        .await
        .ok()?
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
        tokio::task::spawn_blocking(move || {
            let conn = rusqlite::Connection::open(path.as_path()).map_err(|e| e.to_string())?;
            let n = conn.execute(
                "UPDATE users SET password_hash = ?1, updated_at = ?2 WHERE id = ?3",
                params![&hash, &now, &id],
            ).map_err(|e| e.to_string())?;
            if n == 0 {
                return Err::<(), String>("user not found".into());
            }
            Ok(())
        })
        .await
        .map_err(|e| e.to_string())??;
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
