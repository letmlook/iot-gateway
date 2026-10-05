#!/usr/bin/env bash
# 构建并打包发布产物。
#
#   ./scripts/package-release.sh                  # 完整流程（release 构建 + 前端 + 打包 + 校验和）
#   SKIP_BUILD=1 ./scripts/package-release.sh     # 跳过编译，只用现有 target/release 产物打包
#
# 产物（写入 dist/）：
#   iot-gateway-<版本>-<os>-<arch>.tar.gz
#   SHA256SUMS
#
# 包内布局：
#   gateway  gateway-plugin-host  plugins/*.so|dylib  web-dist/  config/  deploy/  VERSION  README.md
#
# 说明：
# - 版本号取 `git describe --tags --always --dirty`（有 tag 用 tag，无 tag 用短哈希）。
# - 插件必须带 --features ffi 构建，否则导不出 C ABI 符号（见 docs/部署与发布.md）。
# - 授权文件（license.dat）与私钥不进包：前者与机器码绑定，后者绝不能离开签发机。
set -euo pipefail
cd "$(dirname "$0")/.."

VERSION="$(git describe --tags --always --dirty 2>/dev/null || echo dev)"
OS="$(uname -s | tr '[:upper:]' '[:lower:]')"
ARCH="$(uname -m)"
case "$ARCH" in
  arm64) ARCH=aarch64 ;;
esac
PLUGIN_EXT="so"
[ "$OS" = "darwin" ] && PLUGIN_EXT="dylib"

NAME="iot-gateway-${VERSION}-${OS}-${ARCH}"
OUT="dist/${NAME}"

if [ "${SKIP_BUILD:-0}" != "1" ]; then
  echo "==> 构建网关与插件宿主（release）"
  # 分开构建：--bin 会过滤掉名字不匹配的包的默认目标（plugin-host 因此曾被漏掉）
  cargo build --release --locked -p gateway-server --bin gateway
  cargo build --release --locked -p gateway-plugin-host

  echo "==> 构建插件（--features ffi）"
  cargo build --release --locked \
    -p plugin-sim -p plugin-mqtt -p plugin-modbus-tcp \
    -p plugin-modbus-rtu -p plugin-opcua -p plugin-virb \
    -p plugin-http -p plugin-influxdb -p plugin-tdengine \
    --features ffi

  echo "==> 构建 Kafka 插件（可选，需要 CMake + rdkafka 依赖；构建失败不中断打包）"
  cargo build --release --locked -p plugin-kafka --features "ffi,kafka-client" || \
    echo "WARN: kafka plugin build failed (requires CMake + librdkafka), package will omit kafka plugin"

  echo "==> 构建前端"
  if [ -d web/node_modules ]; then
    ( cd web && npm run build )
  else
    ( cd web && npm ci --no-audit --no-fund && npm run build )
  fi
fi

echo "==> 组装 $OUT"
rm -rf "$OUT"
mkdir -p "$OUT/plugins"

cp target/release/gateway "$OUT/"
cp target/release/gateway-plugin-host "$OUT/"
# 只打包真正对外提供的插件；测试夹具（faulty/abi-mismatch）不进发布包
for p in sim mqtt modbus_tcp modbus_rtu opcua virb http influxdb tdengine; do
  src="target/release/libplugin_${p}.${PLUGIN_EXT}"
  [ -f "$src" ] && cp "$src" "$OUT/plugins/"
done
# kafka 单独构建，可能缺失（无 CMake 时）
if [ -f "target/release/libplugin_kafka.${PLUGIN_EXT}" ]; then
  cp "target/release/libplugin_kafka.${PLUGIN_EXT}" "$OUT/plugins/"
fi
cp -R web/dist "$OUT/web-dist"
cp -R config "$OUT/config"
# 授权文件与机器码绑定、私钥绝不能分发：从包里剔除（现场用 rotate/gen 脚本单独生成）
rm -f "$OUT/config/license.dat"
find "$OUT/config" -name "*.pem" -delete
# 授权文件与机器码绑定、私钥绝不能分发：从包里剔除（现场用 rotate/gen 脚本单独生成）
rm -f "$OUT/config/license.dat"
find "$OUT/config" -name "*.pem" -delete
cp -R deploy "$OUT/deploy"
[ -f README.md ] && cp README.md "$OUT/"

# VERSION 文件：让运行中的实例可以说清自己来自哪次构建
{
  echo "version=${VERSION}"
  echo "os=${OS}"
  echo "arch=${ARCH}"
  echo "built_at=$(date -u +%Y-%m-%dT%H:%M:%SZ)"
  echo "git_rev=$(git rev-parse --short HEAD 2>/dev/null || echo unknown)"
} > "$OUT/VERSION"

echo "==> 打包"
mkdir -p dist
tar -czf "${OUT}.tar.gz" -C dist "$NAME"

echo "==> 生成 SHA256SUMS"
( cd dist && shasum -a 256 "${NAME}.tar.gz" > SHA256SUMS.tmp && mv SHA256SUMS.tmp SHA256SUMS )

echo "==> 完成"
ls -la "dist/${NAME}.tar.gz" dist/SHA256SUMS
echo "包内清单："
tar -tzf "${OUT}.tar.gz" | head -20
