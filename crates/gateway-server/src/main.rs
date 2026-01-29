//! 网关服务入口：插件注册（.so 动态加载 + 可选内置）、REST API、前端静态资源。

mod api;
mod config;
mod state;

use axum::Router;
use gateway_core::{persist_load, persist_load_json, persist_save, PluginLoader, Manager};
use std::sync::Arc;
use tower_http::cors::CorsLayer;
use tower_http::trace::TraceLayer;
use tracing_subscriber::{layer::SubscriberExt, util::SubscriberInitExt, EnvFilter};

use config::Config;
use state::AppState;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    let config = Config::from_env();
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| config.log_filter.clone().into()))
        .with(tracing_subscriber::fmt::layer())
        .init();

    let mut mgr = Manager::new();
    let mut loader_opt: Option<PluginLoader> = None;

    // 从 plugins_dir 加载 .so 插件（对标 Neuron）
    if config.plugins_dir.exists() {
        if let Ok(loader) = PluginLoader::load(&config.plugins_dir, &mut mgr) {
            loader_opt = Some(loader);
        } else {
            tracing::warn!("load plugins from {} failed, using built-in", config.plugins_dir.display());
            use gateway_plugin_mqtt::MqttPlugin;
            use gateway_plugin_sim::SimPlugin;
            mgr.register_south("sim", Arc::new(SimPlugin::new()));
            mgr.register_north("mqtt", Arc::new(MqttPlugin::new()));
        }
    } else {
        // 无 plugins 目录时使用内置插件（开发/兼容）
        use gateway_plugin_mqtt::MqttPlugin;
        use gateway_plugin_sim::SimPlugin;
        mgr.register_south("sim", Arc::new(SimPlugin::new()));
        mgr.register_north("mqtt", Arc::new(MqttPlugin::new()));
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
            mgr.apply_snapshot(&snap).await;
            tracing::info!("loaded snapshot from {}", db_path.display());
        }
        Ok(None) => {}
        Err(e) => tracing::warn!("persist load failed: {}, starting with empty config", e),
    }

    let _ = crate::config::STATIC_DIR.set(config.static_dir.clone());
    let state = AppState::new(mgr, config.clone(), loader_opt);

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
