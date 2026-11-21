# 语法：多阶段构建。前端与后端分别在独立阶段产出，运行镜像只带二进制与静态资源。
# syntax=docker/dockerfile:1

# ---------- 阶段 1：前端 ----------
FROM node:22-slim AS web
WORKDIR /src/web
COPY web/package.json web/package-lock.json ./
# 依赖清单不变时复用缓存
RUN npm ci --no-audit --no-fund
COPY web/ ./
RUN npm run build

# ---------- 阶段 2：后端 ----------
FROM rust:1-slim-bookworm AS backend
WORKDIR /src

# 先只拷清单，跑一次依赖解析；改代码时不必重装依赖
# （workspace 全成员清单：新增 crate/插件时这里必须同步，否则 fetch --locked 失败）
COPY Cargo.toml Cargo.lock ./
COPY gateway/gateway-sdk/Cargo.toml gateway/gateway-sdk/
COPY gateway/gateway-core/Cargo.toml gateway/gateway-core/
COPY gateway/gateway-server/Cargo.toml gateway/gateway-server/
COPY gateway/gateway-plugin-host/Cargo.toml gateway/gateway-plugin-host/
COPY gateway/gateway-bench/Cargo.toml gateway/gateway-bench/
# 流程编排 v2（蓝图 T1）与北向插件共享基础设施
COPY gateway/gateway-flow/Cargo.toml gateway/gateway-flow/
COPY gateway/gateway-plugin-common/Cargo.toml gateway/gateway-plugin-common/
COPY gateway/gateway-plugins/plugin-sim/Cargo.toml gateway/gateway-plugins/plugin-sim/
COPY gateway/gateway-plugins/plugin-mqtt/Cargo.toml gateway/gateway-plugins/plugin-mqtt/
COPY gateway/gateway-plugins/plugin-modbus-tcp/Cargo.toml gateway/gateway-plugins/plugin-modbus-tcp/
COPY gateway/gateway-plugins/plugin-modbus-rtu/Cargo.toml gateway/gateway-plugins/plugin-modbus-rtu/
COPY gateway/gateway-plugins/plugin-opcua/Cargo.toml gateway/gateway-plugins/plugin-opcua/
COPY gateway/gateway-plugins/plugin-virb/Cargo.toml gateway/gateway-plugins/plugin-virb/
COPY gateway/gateway-plugins/plugin-s7/Cargo.toml gateway/gateway-plugins/plugin-s7/
COPY gateway/gateway-plugins/plugin-bacnet/Cargo.toml gateway/gateway-plugins/plugin-bacnet/
COPY gateway/gateway-plugins/plugin-ethernet-ip/Cargo.toml gateway/gateway-plugins/plugin-ethernet-ip/
COPY gateway/gateway-plugins/plugin-iec104/Cargo.toml gateway/gateway-plugins/plugin-iec104/
COPY gateway/gateway-plugins/plugin-http/Cargo.toml gateway/gateway-plugins/plugin-http/
COPY gateway/gateway-plugins/plugin-influxdb/Cargo.toml gateway/gateway-plugins/plugin-influxdb/
COPY gateway/gateway-plugins/plugin-tdengine/Cargo.toml gateway/gateway-plugins/plugin-tdengine/
COPY gateway/gateway-plugins/plugin-kafka/Cargo.toml gateway/gateway-plugins/plugin-kafka/
COPY gateway/gateway-plugins/plugin-smtp/Cargo.toml gateway/gateway-plugins/plugin-smtp/
# 测试夹具不进镜像，但 workspace 成员清单需要它们的 Cargo.toml
COPY gateway/gateway-plugins/plugin-faulty/Cargo.toml gateway/gateway-plugins/plugin-faulty/
COPY gateway/gateway-plugins/plugin-abi-mismatch/Cargo.toml gateway/gateway-plugins/plugin-abi-mismatch/
RUN cargo fetch --locked

COPY gateway/ gateway/
COPY config/ config/

# 网关本体 + 插件宿主进程（进程级隔离模式需要）
RUN cargo build --release --locked -p gateway-server --bin gateway -p gateway-plugin-host

# 插件编译为 cdylib（--features ffi）。发布镜像带全部对外插件；
# kafka / smtp 为 stub 构建（重客户端依赖默认不编译，schema 明确标注），
# 测试夹具（plugin-faulty / plugin-abi-mismatch）与压测程序不进镜像。
RUN set -eux; \
    for p in sim mqtt modbus-tcp modbus-rtu opcua virb s7 bacnet ethernet-ip iec104 http influxdb tdengine kafka smtp; do \
      cargo build --release --locked -p "plugin-$p" --features ffi; \
    done

# ---------- 阶段 3：运行 ----------
FROM debian:bookworm-slim AS runtime

RUN apt-get update \
 && apt-get install -y --no-install-recommends ca-certificates curl \
 && rm -rf /var/lib/apt/lists/* \
 && useradd --system --uid 10001 --create-home --home-dir /home/gateway gateway

WORKDIR /app

COPY --from=backend /src/target/release/gateway /app/gateway
COPY --from=backend /src/target/release/gateway-plugin-host /app/gateway-plugin-host
COPY --from=backend /src/target/release/libplugin_*.so /app/plugins/
COPY --from=web    /src/web/dist /app/web/dist
COPY config/ /app/config/

# data 目录放配置库、授权文件、离线队列与日志；以卷方式挂出才能持久化
RUN mkdir -p /app/data /app/logs && chown -R gateway:gateway /app

USER gateway:gateway

ENV GATEWAY_PORT=3000 \
    GATEWAY_DATA_DIR=/app/data \
    GATEWAY_STATIC_DIR=/app/web/dist \
    GATEWAY_PLUGINS_DIR=/app/plugins \
    RUST_LOG=info

EXPOSE 3000
VOLUME ["/app/data"]

# /api/health 是免认证的白名单接口，适合做容器探针
HEALTHCHECK --interval=30s --timeout=3s --start-period=15s --retries=3 \
  CMD curl -fsS "http://127.0.0.1:${GATEWAY_PORT}/api/health" || exit 1

ENTRYPOINT ["/app/gateway"]
