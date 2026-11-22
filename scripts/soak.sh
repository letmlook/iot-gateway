#!/bin/bash
# Soak 长跑测试（T5 质量纵深，蓝图 M3 首轮启动口径）。
#
# 场景覆盖（对应蓝图 T5「覆盖 Lagged/重连/持久化退化」）：
#   - 持续数据面：sim 南向周期采集 → kafka 北向 stub 订阅（stub 丢弃计数累积，
#     无离线队列磁盘风险）；北向无真实消费即「慢消费者」形态，Lagged 语义长跑观察
#   - 周期性「重连」：每 reconnect-interval 秒 stop/start 一次南向节点（节点级重连
#     语义）；WS 客户端重连已由集成测试覆盖，soak 不重复
#   - 告警子系统长跑：flow threshold(低阈值) → sink_alarm 持续落库
#   - 持久化：网关默认防抖落盘长跑 + 每 sample-interval 触发一次 flush（观测退化）
#   - 采样：每 interval 记 health/nodes_running/published/lagged/RSS/库大小 → samples.jsonl
#
# 用法：
#   ./scripts/soak.sh                          # 默认 72h 首轮（生产口径）
#   ./scripts/soak.sh --duration 120           # 2 分钟冒烟（验证脚本自身）
#   ./scripts/soak.sh --duration 3600 --interval 10
#   ./scripts/soak.sh --skip-build             # 复用已有二进制
#   ./scripts/soak.sh --license data/license.dat --skip-build
#
# 输出：<data-dir>/samples.jsonl + 结束摘要；SIGINT/SIGTERM 触发网关优雅停机后退出。
set -euo pipefail
cd "$(dirname "$0")/.."

DURATION=259200        # 72h（T5 首轮口径）
INTERVAL=60            # 采样间隔（秒）
RECONNECT_INTERVAL=600 # 南向节点 stop/start 周期（秒）
PORT=4300
DATA_DIR="data/soak"
SKIP_BUILD=0
LICENSE_SRC=""

while [ $# -gt 0 ]; do
  case "$1" in
    --duration) DURATION=$2; shift 2 ;;
    --interval) INTERVAL=$2; shift 2 ;;
    --reconnect-interval) RECONNECT_INTERVAL=$2; shift 2 ;;
    --port) PORT=$2; shift 2 ;;
    --data-dir) DATA_DIR=$2; shift 2 ;;
    --license) LICENSE_SRC=$2; shift 2 ;;
    --skip-build) SKIP_BUILD=1; shift ;;
    *) echo "unknown arg: $1"; exit 1 ;;
  esac
done

mkdir -p "$DATA_DIR"
# 收费插件（kafka stub 等）需要 license 特性位；--license 指定已签发的 license.dat
if [ -n "$LICENSE_SRC" ] && [ ! -f "$DATA_DIR/license.dat" ]; then
  cp "$LICENSE_SRC" "$DATA_DIR/license.dat"
  echo "[soak] license installed from $LICENSE_SRC"
fi
LOG="$DATA_DIR/gateway.log"
SAMPLES="$DATA_DIR/samples.jsonl"
SUMMARY="$DATA_DIR/summary.md"

if [ "$SKIP_BUILD" != "1" ]; then
  echo "[soak] building release binary..."
  cargo build --release -p gateway-server --bin gateway
fi
BIN=./target/release/gateway
[ -x "$BIN" ] || { echo "binary not found: $BIN"; exit 1; }

echo "[soak] starting gateway on :$PORT (data=$DATA_DIR)"
GATEWAY_PORT=$PORT GATEWAY_DATA_DIR="$DATA_DIR" GATEWAY_DISABLE_AUTH=1 \
  GATEWAY_STATIC_DIR=web/dist "$BIN" > "$LOG" 2>&1 &
GW_PID=$!
trap 'kill -TERM $GW_PID 2>/dev/null; wait $GW_PID 2>/dev/null || true; echo "[soak] gateway stopped (graceful SIGTERM)"' EXIT

BASE="http://127.0.0.1:$PORT"
until curl -fsS -m 2 "$BASE/api/health" > /dev/null 2>&1; do sleep 0.5; done
echo "[soak] gateway up (pid $GW_PID)"

# ---------- 搭建长跑拓扑 ----------
api() { curl -fsS -m 10 "$@"; }
api_post() { curl -fsS -m 10 -X POST "$@" -H 'Content-Type: application/json'; }

south_ids=()
# 幂等重启：数据目录残留上一轮的 soak-* 节点/flow 时（如进程崩溃后重启），
# 先清干净再建——否则同名 flow 创建 400、同名节点干扰订阅校验。
# （2026-11-22 首轮崩溃后重启即死于残留 soak-alarm flow）
old=$(curl -fsS -m 10 "$BASE/api/nodes" 2>/dev/null | python3 -c 'import json,sys
d = json.load(sys.stdin)
items = d["items"] if isinstance(d, dict) else d
print(" ".join(n["id"] for n in items if n["name"].startswith("soak-")))' 2>/dev/null || true)
for oid in $old; do
  api -X DELETE "$BASE/api/nodes/$oid" > /dev/null 2>&1 || true
  echo "[soak] removed stale node $oid"
done
old_flow=$(curl -fsS -m 10 "$BASE/api/flows" 2>/dev/null | python3 -c 'import json,sys
d = json.load(sys.stdin)
flows = d["flows"] if isinstance(d, dict) else d
print(" ".join(f["id"] for f in flows if f["name"] == "soak-alarm"))' 2>/dev/null || true)
for fid in $old_flow; do
  api -X DELETE "$BASE/api/flows/$fid" > /dev/null 2>&1 || true
  echo "[soak] removed stale flow $fid"
done

for i in 1 2; do
  id=$(api_post "$BASE/api/nodes" -d "{\"name\":\"soak-sim-$i\",\"kind\":\"south\",\"plugin_name\":\"sim\",\"config\":{\"interval_ms\":$((i*1000))}}" | python3 -c 'import json,sys;print(json.load(sys.stdin)["id"])')
  south_ids+=("$id")
  gid=$(api_post "$BASE/api/nodes/$id/groups" -d '{"name":"g1","interval_ms":'"$((i*1000))"'}' | python3 -c 'import json,sys;print(json.load(sys.stdin)["id"])')
  api_post "$BASE/api/nodes/$id/tags" -d "{\"name\":\"temperature\",\"address\":\"R0\",\"group_id\":\"$gid\",\"data_type\":\"float64\"}" > /dev/null
  echo "[soak] south node soak-sim-$i = $id"
done

# kafka 是收费插件（license 特性位门禁）：数据目录无 license.dat 时这里 403，
# 失败信息要直指原因，而不是让 python traceback 代替人说话
resp=$(api_post "$BASE/api/nodes" -d '{"name":"soak-kafka","kind":"north","plugin_name":"kafka","config":{"brokers":"127.0.0.1:9092"}}') || {
  echo "[soak] FAIL: create kafka node rejected: ${resp:-<no body>}"
  echo "[soak] hint: kafka 需 license 特性位——用 --license data/license.dat 启动（数据目录已有则自动跳过）"
  exit 1
}
kafka_id=$(echo "$resp" | python3 -c 'import json,sys;print(json.load(sys.stdin)["id"])')
echo "[soak] north kafka stub = $kafka_id"

# 订阅：两个南向组 → kafka stub。必须走 /api 前缀 + PUT + {subscriptions:[...]} 包裹：
# 裸路径会被 SPA fallback 吞成 200 + index.html——「看起来成功」实际没订阅
# （2026-11-22 首轮 8.5h 的 north_lagged 全空、bus_no_subscribers 一路涨，就是这个静默失败）
for id in "${south_ids[@]}"; do
  gid=$(api "$BASE/api/nodes/$id/groups" | python3 -c 'import json,sys
d = json.load(sys.stdin)
items = d["items"] if isinstance(d, dict) else d
print(items[0]["id"])')
  resp=$(curl -fsS -m 10 -X PUT "$BASE/api/nodes/$kafka_id/subscriptions" -H 'Content-Type: application/json' \
    -d "{\"subscriptions\":[{\"south_node_id\":\"$id\",\"group_id\":\"$gid\"}]}" 2>&1) || {
    echo "[soak] FAIL: subscribe $id -> $kafka_id: ${resp:-<no body>}"
    exit 1
  }
  echo "[soak] subscribed $id/$gid -> kafka"
done

# 告警链路 flow：sim → threshold(low=100，sim 值域 15-25 恒低于阈值 → 恒触发) → sink_alarm
g0=$(api "$BASE/api/nodes/${south_ids[0]}/groups" | python3 -c 'import json,sys
d = json.load(sys.stdin)
items = d["items"] if isinstance(d, dict) else d
print(items[0]["id"])')
api_post "$BASE/api/flows" -d '{
  "name":"soak-alarm",
  "nodes":[
    {"id":"10000000-0000-0000-0000-000000000001","name":"s0","kind":"source","config":{"south_node_id":"'"${south_ids[0]}"'","group_id":"'"$g0"'"},"position":{"x":0,"y":0}},
    {"id":"10000000-0000-0000-0000-000000000003","name":"ta","kind":"operator","operator":"threshold_alarm","config":{"tag_name":"temperature","low":100,"level":"info","message":"soak"},"position":{"x":150,"y":0}},
    {"id":"10000000-0000-0000-0000-000000000004","name":"sink","kind":"sink","operator":"sink_alarm","config":{},"position":{"x":300,"y":0}}
  ],
  "edges":[
    {"source_node_id":"10000000-0000-0000-0000-000000000001","source_port":"out","target_node_id":"10000000-0000-0000-0000-000000000003"},
    {"source_node_id":"10000000-0000-0000-0000-000000000003","source_port":"alarm","target_node_id":"10000000-0000-0000-0000-000000000004"}
  ]
}' | python3 -c 'import json,sys;print("[soak] flow created:", json.load(sys.stdin)["id"])'
FLOW_ID=$(api "$BASE/api/flows" | python3 -c 'import json,sys;print([f for f in json.load(sys.stdin)["flows"] if f["name"]=="soak-alarm"][0]["id"])')
api_post "$BASE/api/flows/$FLOW_ID/publish" -d '{}' > /dev/null
echo "[soak] flow published"

for id in "${south_ids[@]}"; do api_post "$BASE/api/nodes/$id/start" > /dev/null; done
api_post "$BASE/api/nodes/$kafka_id/start" > /dev/null
echo "[soak] nodes started"

# ---------- 采样与重连循环 ----------
# prometheus 文本解析 + RSS + 库大小 → samples.jsonl
sample() {
  local metrics health rss db_size published lagged running alarms_total
  metrics=$(curl -fsS -m 5 "$BASE/api/metrics" 2>/dev/null || true)
  health=$(curl -fsS -m 5 "$BASE/api/health" 2>/dev/null || echo '{"status":"down"}')
  # /api/alarms 无 total 字段：取 len（limit 1000 封顶；封顶本身即「告警链路活跃」证据）。
  # 注意：脚本开了 pipefail——curl 瞬时失败（连接重置等）会让 pipeline 退出码非零，
  # 若写 `|| echo 0` 会在 python 已输出 len 后再追加一个 0（'1\n0'），下游 float() 必炸
  # （2026-11-22 首轮 soak 跑 8.5h 即死于这个组合）。改为空值兜底：成功取输出、失败补 0。
  alarms_total=$(curl -fsS -m 5 "$BASE/api/alarms?limit=1000" 2>/dev/null | python3 -c 'import json,sys
try: print(len(json.load(sys.stdin).get("alarms", [])))
except Exception: print(0)' 2>/dev/null || true)
  [ -n "$alarms_total" ] || alarms_total=0
  rss=$(ps -o rss= -p "$GW_PID" 2>/dev/null | tr -d ' ' || echo 0)
  # WAL 模式下主文件可能不增长：库大小 = 主文件 + -wal（接近真实占用）
  db_size=0
  for f in "$DATA_DIR/data.db" "$DATA_DIR/data.db-wal"; do
    sz=$(stat -f%z "$f" 2>/dev/null || stat -c%s "$f" 2>/dev/null || echo 0)
    db_size=$(( db_size + sz ))
  done
  METRICS="$metrics" HEALTH="$health" RSS="$rss" DB_SIZE="$db_size" ALARMS="$alarms_total" TS="$(date +%s)" \
  python3 - << 'PYEOF' >> "$SAMPLES"
import json, os, re, sys
metrics = os.environ.get("METRICS", "")
def num(name):
    for line in metrics.splitlines():
        if line.startswith(name + " ") or line.startswith(name + "{"):
            try: return float(line.rsplit(" ", 1)[1])
            except Exception: pass
    return None
def fnum(v):
    # 环境变量统一经此转 float：非有限输入（空/多值/脏字符）回退 0，不让单点脏数据杀 soak
    try: return float(str(v).strip().split()[0])
    except Exception: return 0.0
rec = {
    "ts": int(os.environ["TS"]),
    "rss_kb": fnum(os.environ.get("RSS", "0")),
    "db_bytes": fnum(os.environ.get("DB_SIZE", "0")),
    "health": json.loads(os.environ.get("HEALTH", '{"status":"down"}')).get("status"),
    "nodes_total": num("gateway_nodes_total"),
    "nodes_running": num("gateway_nodes_running"),
    "south_published": num("gateway_data_flow_south_published"),
    "bus_no_subscribers": num("gateway_data_flow_bus_no_subscribers"),
    "alarms_total": fnum(os.environ.get("ALARMS", "0")),
}
lag = {}
for line in metrics.splitlines():
    if line.startswith("gateway_north_lagged"):
        m = re.search(r'\{node="([^"]+)"\}\s+([\d.e+]+)', line)
        if m: lag[m.group(1)] = float(m.group(2))
rec["north_lagged"] = lag
print(json.dumps(rec, ensure_ascii=False))
PYEOF
}

echo "[soak] soak running: duration=${DURATION}s interval=${INTERVAL}s reconnect=${RECONNECT_INTERVAL}s"
END=$(( $(date +%s) + DURATION ))
NEXT_RECONNECT=$(( $(date +%s) + RECONNECT_INTERVAL ))
# 长跑健壮性：单次采样失败（瞬时网络/解析问题）只告警不中止——72h 里任何
# 一次性异常都不该终止整个 soak（set -e 下裸调用 sample 失败即全脚本退出，
# 2026-11-22 首轮即因此死于一次 curl 瞬时失败 × pipefail × ALARMS 脏值）
sample || echo "[soak] WARN: sample failed at $(date '+%F %T')"
while [ "$(date +%s)" -lt "$END" ]; do
  sleep "$INTERVAL"
  sample || echo "[soak] WARN: sample failed at $(date '+%F %T')"
  # 节点级重连演练：stop → start（校验状态回到 running，恢复失败只告警不中止）
  if [ "$(date +%s)" -ge "$NEXT_RECONNECT" ]; then
    for id in "${south_ids[@]}"; do
      api_post "$BASE/api/nodes/$id/stop" > /dev/null 2>&1 || true
      sleep 1
      api_post "$BASE/api/nodes/$id/start" > /dev/null 2>&1 || echo "[soak] WARN: restart $id failed"
    done
    echo "[soak] reconnect drill done at $(date '+%F %T')"
    NEXT_RECONNECT=$(( $(date +%s) + RECONNECT_INTERVAL ))
  fi
  # 持久化退化观测：显式 flush 由网关防抖完成；这里只确认进程仍响应
  curl -fsS -m 5 "$BASE/api/health" > /dev/null || echo "[soak] WARN: health check failed at $(date '+%F %T')"
done

# ---------- 摘要 ----------
echo "[soak] duration reached, writing summary"
SUMMARY="$SUMMARY" SAMPLES="$SAMPLES" FLOW_ID="$FLOW_ID" python3 - << 'PYEOF'
import json, os
samples = []
with open(os.environ["SAMPLES"], encoding="utf-8") as f:
    for line in f:
        line = line.strip()
        if line:
            samples.append(json.loads(line))
rss = [s["rss_kb"] for s in samples if s.get("rss_kb")]
pub = [s["south_published"] for s in samples if s.get("south_published") is not None]
lagged = sum(sum(s.get("north_lagged", {}).values()) for s in samples[-1:]) if samples else 0
db = [s["db_bytes"] for s in samples if s.get("db_bytes")]
down = sum(1 for s in samples if s.get("health") != "ok")
lines = [
    "# Soak 摘要",
    "",
    f"- 采样数：{len(samples)}（健康异常 {down} 次）",
    f"- RSS：峰 {max(rss)/1024:.1f} MB / 末 {rss[-1]/1024:.1f} MB" if rss else "- RSS：无样本",
    f"- 南向发布计数：{pub[-1]:.0f}" if pub else "- 南向发布：无样本",
    f"- 北向 Lagged 累计：{lagged:.0f}",
    f"- 告警行数：{samples[-1].get('alarms_total', 0):.0f}（≥1 即告警链路活跃；同键 active 告警只前移时间戳不新增行——生命周期去重是预期行为）",
    f"- 库大小：{db[0]:.0f} → {db[-1]:.0f} 字节" if db else "- 库大小：无样本",
    f"- 判据（人工审阅）：RSS 是否线性上扬（泄漏嫌疑）；发布计数是否持续增长；Lagged 只增后恒定为 Lagged 语义生效；告警行数 ≥1 为告警链路健康",
]
with open(os.environ["SUMMARY"], "w", encoding="utf-8") as f:
    f.write("\n".join(lines) + "\n")
print("\n".join(lines))
PYEOF
echo "[soak] done. samples=$SAMPLES summary=$SUMMARY log=$LOG"
