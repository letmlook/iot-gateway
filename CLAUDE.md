# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Build & Run

```bash
# One-click: build frontend + backend, then run
./scripts/build_and_run.sh                  # production build & run
./scripts/build_and_run.sh --dev            # dev mode (Vite HMR + backend)

# Or step by step:
# Frontend
cd web && npm install && npm run build

# Backend (workspace root)
cargo build --release
./target/release/gateway                    # default http://0.0.0.0:4000

# Build individual plugin as .so/.dll
cargo build -p plugin-mqtt --features ffi --release
# See scripts/build-plugins.sh / build-plugins.ps1

# Frontend (serve from web/dist after build)
cd web && npm install && npm run build

# Dev: frontend with Vite dev server (proxies /api to :3000)
cd web && npm run dev
```

## Commands

```bash
# Run individual test
cargo test -p gateway-core -- test_name

# Build with license embedded
./scripts/build_with_license.sh [--release]   # macOS/Linux
.\scripts\build_with_license.ps1 [-Release]    # Windows

# Generate license for current machine
python3 scripts/gen_license_auto.py
```

## Key Environment Variables

Priority: env var > `config/gateway.json` > defaults.

| Variable | Default | Purpose |
|---|---|---|
| `GATEWAY_PORT` | `4000` | HTTP listen port |
| `GATEWAY_DATA_DIR` | `data` | SQLite, backups, license |
| `GATEWAY_STATIC_DIR` | `web/dist` | Frontend static assets |
| `GATEWAY_PLUGINS_DIR` | `plugins` | .so/.dll scan directory |
| `GATEWAY_TOKEN` | none | API Bearer token |
| `GATEWAY_DISABLE_AUTH` | `false` | `1` or `true` to skip auth |
| `RUST_LOG` | `info,tower_http=debug` | Tracing filter |
| `GATEWAY_CONFIG` | `config/gateway.json` | Alternate config path |

## Architecture

### Crate Layering

```
gateway-sdk          → shared types, SouthPlugin/NorthPlugin traits, FFI ABI
gateway-core         → Bus, Manager, Store, persistence, .so loader
gateway-server       → Axum HTTP, config, license, users, main binary
gateway-plugins/*    → South (sim, modbus-tcp, modbus-rtu, opcua, virb) + North (mqtt)
```

### Data Flow: South → Core → North

```
SouthPlugin.poll_group() → GroupData → Bus::publish() → broadcast::channel
                                                              ↓
                                          NorthPlugin subscription filter
                                                              ↓
                                          NorthPlugin.on_group_data()
```

- **Bus** (`gateway-core/src/bus.rs`): `tokio::sync::broadcast` with 4096 capacity. Slow consumers get `Lagged(n)`.
- **Manager** (`gateway-core/src/manager.rs`): Central router — node lifecycle, subscriptions, read/write tags. Each south group polls on its own tokio task; each north node consumes the bus in its own task.
- **SubscriptionTable**: `HashMap<NorthNodeId, Vec<GroupSubscription>>` — maps north node → which (south_node, group) pairs to forward.

### Plugin System

Both `SouthPlugin` and `NorthPlugin` traits (`gateway-sdk/src/plugin.rs`) follow the same lifecycle:
**open → init → start ⇄ stop → uninit → close**

Plus `setting()` for runtime config changes without delete/recreate.

Plugins can be:
1. **Statically linked** — via `Cargo.toml` dependency, registered in `main.rs` `register_builtin_plugins()`
2. **Dynamically loaded** — compiled as `cdylib` with `ffi` feature, scanned from `plugins_dir` at startup via `PluginLoader` (`gateway-core/src/loader.rs`). FFI uses JSON serialization over C ABI via `libloading`.

Key trait methods:
- **South**: `poll_group()`, `write_tags()`, `validate_tag()`, `list_groups()`, `list_tags()`
- **North**: `set_subscriptions()`, `on_group_data()`, `connection_status()`
- Both: `config_schema()` and `tag_schema()` return JSON Schema for UI form generation

### Store & Persistence

- **Store** (`gateway-core/src/store.rs`): In-memory `DashMap` + SQLite backing. All mutations write to both. On startup loads from SQLite.
- **Snapshot** (`gateway-core/src/persist.rs`): Full export/import of nodes, groups, tags, subscriptions. Auto-backup to `data.db.bak` before save.
- Legacy `data.json` is auto-migrated to SQLite.

### Node State Machine

`NodeState`: `Stopped` ↔ `Running` (with `Error` variant for display). Manager orchestrates plugin lifecycle calls matching state transitions.

### License System

Offline RSA-signed license (`data/license.dat`). `FeatureManager` gates features. Exempt endpoints: `health`, `metrics`, `version`, `license/machine-id`, `license/status`, `auth/login`.

### REST API (`gateway-server/src/api/mod.rs`)

Auth middleware skips whitelisted paths. All `/api/*` paths. SPA fallback serves `index.html` for unmatched routes.

### Web Frontend

Vue 3 + Vite + Element Plus + Vue Router + vue-i18n (zh/en). Views map to the domain: Dashboard, SouthDevices, NorthApps, NodeDetail, GroupDetail, DataMonitor, DataFlowMetrics, Plugins, SystemConfig, License, Users, Logs, Login.

### Plugin Feature Flags

Each plugin has a `default` feature for its client dependency and an `ffi` feature for cdylib builds:
- `plugin-modbus-tcp`: `modbus-client` (tokio-modbus), `ffi`
- `plugin-modbus-rtu`: `modbus-client` (tokio-modbus + tokio-serial), `ffi`
- `plugin-opcua`: `opcua-client`, `ffi`
- `plugin-mqtt`: `mqtt-client` (rumqttc), `ffi`
- `plugin-sim`: `ffi` only
- `plugin-virb`: `ffi` only
