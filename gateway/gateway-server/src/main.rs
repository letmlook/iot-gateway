//! 网关服务入口：插件注册（.so 动态加载 + 可选内置）、REST API、前端静态资源、离线授权、用户管理。

mod api;
mod backup;
mod config;
mod flow;
mod license;
mod logging;
mod state;
mod users;

use axum::Router;
use gateway_core::{persist_load, persist_load_json, persist_save, PluginLoader, Manager};
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
    if !db_path.exists() && json_path.exists() {
        if let Ok(Some(snap)) = persist_load_json(&json_path).await {
            if persist_save(&db_path, &snap).await.is_ok() {
                tracing::info!("migrated {} -> {}", json_path.display(), db_path.display());
            }
        }
    }
    match persist_load(&db_path).await {
        Ok(Some(snap)) => {
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

    let flow_store = crate::flow::FlowStore::new(&db_path)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))?;
    let flow_runtimes: Arc<tokio::sync::RwLock<std::collections::HashMap<uuid::Uuid, gateway_flow::FlowRuntime>>> = Arc::new(tokio::sync::RwLock::new(std::collections::HashMap::new()));
    
    let state = AppState::new(mgr, config.clone(), loader_opt, feature_manager, user_store, node_log_names, flow_store, flow_runtimes);
    state.sync_node_log_names();

    let app = Router::new()
        .nest("/api", api::router(state.clone()))
        .layer(TraceLayer::new_for_http())
        .layer(CorsLayer::permissive())
        .with_state(state);

    let serve = Router::new()
        .merge(app)
        .fallback(api::serve_static_or_index);

    let addr = std::net::SocketAddr::from(([0, 0, 0, 0], config.port));
    tracing::info!("gateway listening on {}", addr);
    let listener = tokio::net::TcpListener::bind(addr).await?;
    axum::serve(listener, serve).await?;
    Ok(())
}
