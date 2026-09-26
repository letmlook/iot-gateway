//! 网关服务入口：插件注册（.so 动态加载 + 可选内置）、REST API、前端静态资源、离线授权、用户管理。

mod api;
mod backup;
mod config;
mod license;
mod logging;
mod state;
mod users;

use axum::Router;
use gateway_core::{
    persist_load_json, persist_load_secret, persist_save_secret, Manager, PluginLoader,
};
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;

use config::Config;
use state::AppState;
use users::UserStore;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let config = Config::from_env();
    let node_log_names = config
        .log_dir_nodes
        .as_ref()
        .map(|_| std::sync::Arc::new(std::sync::RwLock::new(std::collections::HashMap::new())));
    logging::init_logging(&config, node_log_names.clone());

    let mut mgr = Manager::new();
    let mut loader_opt: Option<PluginLoader> = None;

    // 从 plugins_dir 加载动态库插件（Windows: .dll，Unix: .so）；若无目录或加载后无插件则使用内置
    fn register_builtin_plugins(mgr: &mut gateway_core::Manager) {
        use plugin_mqtt::MqttPlugin;
        use plugin_modbus_rtu::ModbusRtuPlugin;
        use plugin_modbus_tcp::ModbusTcpPlugin;
        use plugin_sim::SimPlugin;
        mgr.register_south("sim", Arc::new(SimPlugin::new()));
        mgr.register_south("modbus-tcp", Arc::new(ModbusTcpPlugin::new()));
        mgr.register_south("modbus-rtu", Arc::new(ModbusRtuPlugin::new()));
        mgr.register_north("mqtt", Arc::new(MqttPlugin::new()));
    }

    if config.plugins_dir.exists() {
        if let Ok(loader) = PluginLoader::load(&config.plugins_dir, &mut mgr) {
            loader_opt = Some(loader);
            // 若目录下未加载到任何插件（如空目录或仅有非插件库），则补充内置插件供管理页展示与使用
            if mgr.south_plugins().is_empty() && mgr.north_plugins().is_empty() {
                tracing::info!("no plugins loaded from {}, using built-in", config.plugins_dir.display());
                register_builtin_plugins(&mut mgr);
            }
        } else {
            tracing::warn!("load plugins from {} failed, using built-in", config.plugins_dir.display());
            register_builtin_plugins(&mut mgr);
        }
    } else {
        // 无 plugins 目录时使用内置插件（开发/兼容）
        register_builtin_plugins(&mut mgr);
    }

    let mgr = Arc::new(mgr);

    if let Some(parent) = config.data_db().parent() {
        tokio::fs::create_dir_all(parent).await?;
    }
    let db_path = config.data_db();
    let json_path = config.data_file();
    let secret = config.master_secret.as_deref();
    if secret.is_none() {
        tracing::warn!(
            "SECURITY: GATEWAY_SECRET_KEY (or a fixed GATEWAY_BACKUP_SECRET) is not set; \
node credentials such as MQTT passwords will be stored in PLAINTEXT in data.db. Set GATEWAY_SECRET_KEY to enable encryption at rest."
        );
    }
    if !db_path.exists() && json_path.exists() {
        if let Ok(Some(snap)) = persist_load_json(&json_path).await {
            if persist_save_secret(&db_path, &snap, secret).await.is_ok() {
                tracing::info!("migrated {} -> {}", json_path.display(), db_path.display());
            }
        }
    }
    match persist_load_secret(&db_path, secret).await {
        Ok(Some(snap)) => {
            // 历史数据可能仍是明文口令：启用密钥后首次启动把它们加密回写
            if secret.is_some() && gateway_core::persist_has_plaintext_secrets(&snap) {
                let _ = persist_save_secret(&db_path, &snap, secret).await;
                tracing::info!("encrypted existing plaintext credentials in {}", db_path.display());
            }
            // 在 apply_snapshot 前填充 node_log_names，确保节点启动后首次写日志即用节点名称
            if let Some(ref map) = node_log_names {
                if let Ok(mut m) = map.write() {
                    m.clear();
                    for n in &snap.nodes {
                        m.insert(n.id().0.to_string(), n.config.name.clone());
                    }
                }
            }
            mgr.apply_snapshot(&snap).await;
            tracing::info!("loaded snapshot from {}", db_path.display());
        }
        Ok(None) => {}
        Err(e) => tracing::warn!("persist load failed: {}, starting with empty config", e),
    }

    let _ = crate::config::STATIC_DIR.set(config.static_dir.clone());

    // 离线授权：启动时读取 license.dat，验签并校验机器码与到期时间
    let feature_manager = match license::load_and_verify_license(&config.license_path()) {
        Ok(payload) => {
            tracing::info!(
                expiry = %payload.expiry_date,
                features = ?payload.features,
                "license loaded and verified"
            );
            license::FeatureManager::with_license(payload)
        }
        Err(e) => {
            tracing::warn!("license not loaded (free mode): {}", e);
            license::FeatureManager::without_license()
        }
    };

    let user_store = match UserStore::open(&db_path) {
        Ok(s) => Arc::new(s),
        Err(e) => {
            tracing::warn!("user store open failed: {}, user management disabled", e);
            Arc::new(UserStore::empty())
        }
    };

    // 首次初始化生成的随机管理员口令：只在本次生成时输出一次，登录后应删除 .admin_initial_password 并改密
    if let Some(pwd) = user_store.take_initial_password() {
        tracing::warn!(
            "SECURITY: 已为初始管理员 admin 生成随机口令（同时写入数据目录 .admin_initial_password，权限 0600）。请首次登录后立即修改口令并删除该文件。"
        );
        tracing::warn!("SECURITY: initial admin password = {}", pwd);
    }

    // 安全基线：生产环境不允许关闭认证；确需关闭时必须显式设置环境变量并接受告警
    if config.disable_auth {
        tracing::warn!(
            "SECURITY WARNING: API authentication is DISABLED (disable_auth=true). \
All /api endpoints are accessible without credentials, including write_tags (PLC write), restore and user management. \
Set disable_auth=false or unset GATEWAY_DISABLE_AUTH for any non-local deployment."
        );
    }
    if config.backup_secret_ephemeral {
        tracing::warn!(
            "SECURITY: GATEWAY_BACKUP_SECRET is not set; a random key was generated for this process. \
Backups created now can only be restored by this running instance. Set a fixed strong secret to restore elsewhere."
        );
    }

    let state = AppState::new(mgr, config.clone(), loader_opt, feature_manager, user_store, node_log_names);
    state.sync_node_log_names();
    // 优雅退出时需要用到状态（state 随后会被 move 进 Router）
    let shutdown_state = state.clone();

    // 后台合并落盘：把去抖窗口内的多次变更合并成一次 SQLite 事务，
    // 避免「改一个点位就全量重写一次配置库」的写放大。
    if config.persist_debounce_ms > 0 {
        let flusher = state.clone();
        let tick_ms = (config.persist_debounce_ms / 2).max(50);
        tokio::spawn(async move {
            let mut ticker = tokio::time::interval(std::time::Duration::from_millis(tick_ms));
            ticker.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Delay);
            loop {
                ticker.tick().await;
                if flusher.persist_pending() {
                    flusher.flush().await;
                }
            }
        });
    }

    // CORS：默认拒绝所有跨域来源（前端由本服务同源提供）；允许的来源需显式配置 GATEWAY_ALLOWED_ORIGINS
    let cors = if config.allowed_origins.is_empty() {
        CorsLayer::new()
    } else {
        let origins: Vec<axum::http::HeaderValue> = config
            .allowed_origins
            .iter()
            .filter_map(|o| o.parse().ok())
            .collect();
        if origins.is_empty() {
            tracing::warn!("GATEWAY_ALLOWED_ORIGINS contains no valid origin; cross-origin requests stay blocked");
            CorsLayer::new()
        } else {
            tracing::info!("CORS allowed origins: {:?}", config.allowed_origins);
            CorsLayer::new()
                .allow_origin(tower_http::cors::AllowOrigin::list(origins))
                .allow_methods([
                    axum::http::Method::GET,
                    axum::http::Method::POST,
                    axum::http::Method::PUT,
                    axum::http::Method::DELETE,
                    axum::http::Method::OPTIONS,
                ])
                .allow_headers([
                    axum::http::header::AUTHORIZATION,
                    axum::http::header::CONTENT_TYPE,
                ])
        }
    };

    let app = Router::new()
        .nest("/api", api::router(state.clone()))
        .layer(TraceLayer::new_for_http())
        .layer(cors)
        .with_state(state);

    let serve = Router::new()
        .merge(app)
        .fallback(api::serve_static_or_index);

    let addr: std::net::SocketAddr = match config.bind.parse::<std::net::IpAddr>() {
        Ok(ip) => std::net::SocketAddr::from((ip, config.port)),
        Err(_) => {
            tracing::warn!("invalid bind address '{}', falling back to 0.0.0.0", config.bind);
            std::net::SocketAddr::from(([0, 0, 0, 0], config.port))
        }
    };
    tracing::info!("gateway listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, serve)
        .with_graceful_shutdown(shutdown_signal(shutdown_state))
        .await?;
    Ok(())
}

/// 优雅退出：收到 SIGINT/SIGTERM 后停止接收新请求，顺序停止所有运行中的节点，
/// 等待采集任务收敛并落盘，避免停机时丢数据或留下孤儿任务。
async fn shutdown_signal(state: AppState) {
    let manager = state.manager.clone();
    let ctrl_c = async {
        let _ = tokio::signal::ctrl_c().await;
    };
    #[cfg(unix)]
    let terminate = async {
        match tokio::signal::unix::signal(tokio::signal::unix::SignalKind::terminate()) {
            Ok(mut s) => {
                s.recv().await;
            }
            Err(e) => {
                tracing::warn!("install SIGTERM handler failed: {}", e);
                std::future::pending::<()>().await;
            }
        }
    };
    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => tracing::info!("SIGINT received, shutting down gracefully"),
        _ = terminate => tracing::info!("SIGTERM received, shutting down gracefully"),
    }

    let running: Vec<_> = manager
        .nodes_list()
        .into_iter()
        .filter(|n| n.state == gateway_sdk::NodeState::Running)
        .map(|n| n.id())
        .collect();
    if !running.is_empty() {
        tracing::info!("stopping {} running node(s) before exit", running.len());
        for id in running {
            if let Err(e) = manager.node_stop(id).await {
                tracing::warn!(node = %id.0, "stop node during shutdown failed: {}", e);
            }
        }
    }
    // 退出前强制落盘：去抖窗口内可能仍有未写入的变更
    state.flush().await;
    tracing::info!("gateway stopped");
}
