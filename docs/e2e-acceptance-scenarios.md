# IoT Gateway E2E Acceptance Scenarios

Date: 2026-05-29

These scenarios define the minimum vertical acceptance checks for the production-readiness roadmap. They are written for manual smoke testing and future automation.

## Common Setup

1. Build the backend and frontend:

   ```bash
   cargo build
   cd web && npm run build
   ```

2. Start the gateway with authentication disabled for local acceptance testing:

   ```bash
   GATEWAY_DISABLE_AUTH=1 cargo run -p gateway-server
   ```

3. In another shell, start the frontend development server when UI testing is needed:

   ```bash
   cd web && npm run dev
   ```

4. Confirm baseline health:

   ```bash
   curl http://127.0.0.1:4000/api/health
   curl http://127.0.0.1:4000/api/version
   ```

If the configured port differs, use the value from `GATEWAY_PORT` or `config/gateway.json`.

## Scenario 1: Sim -> Flow -> MQTT -> Live Monitor

### Goal

Verify the primary demo path using the simulated south plugin and MQTT north plugin.

### Setup

1. Create a south node using plugin `sim`.
2. Create or confirm a polling group with at least one numeric tag.
3. Create a north node using plugin `mqtt` and configure a reachable broker.
4. Subscribe the MQTT north node to the simulated south group.
5. Create a Flow with a simple operator chain, for example:

   ```text
   source -> filter or transform -> sink
   ```

6. Bind the Flow to the simulated south group once Flow binding is implemented.

### Expected Results

- The south node starts successfully.
- The Flow can be deployed and started.
- Live Monitor shows incoming group/tag values.
- MQTT receives processed values when Flow binding is enabled.
- If Flow execution fails under the initial fail-open policy, raw data is still published and a warning is logged.

### Current Roadmap Status

- South -> Bus -> MQTT path: Beta.
- Flow binding into Manager publish path: planned.
- Real Flow preview: planned.

## Scenario 2: Modbus TCP -> Flow -> MQTT

### Goal

Verify a real south protocol path through Flow and north publishing.

### Setup

1. Start a Modbus TCP simulator or real test server.
   - Reuse the standard-library simulator pattern in `e2e/south/test_minimal.py`.
2. Create a south node using plugin `modbus-tcp`.
3. Configure host, port, unit id, and polling group.
4. Add numeric tags that map to known simulator registers.
5. Create an MQTT north node and subscribe it to the Modbus group.
6. Bind a Flow with `filter`, `transform`, or `deadband` to the Modbus group.

### Expected Results

- The Modbus node can start and read configured tags.
- Values are transformed or filtered by Flow before publishing.
- MQTT receives the processed payload.
- Live Monitor and metrics show traffic through the pipeline.

### Suggested Automation Command

```bash
pytest e2e/south/test_minimal.py
```

### Current Roadmap Status

- Modbus TCP plugin: Beta/real.
- Flow main-path execution: planned.

## Scenario 3: Alarm Lifecycle

### Goal

Verify alarm trigger, persistence, query, acknowledgement, resolution, and realtime update.

### Setup

1. Create a south node and numeric tag that can cross a threshold.
2. Create a Flow with an alarm operator configured with a threshold rule.
3. Bind the Flow to the source group.
4. Start the source node and Flow.
5. Open the Alarm UI component or call the alarm API.

### Expected Results

- When the threshold is exceeded, an alarm event is created.
- `GET /api/alarm/events` returns the alarm in an `items` array.
- `POST /api/alarm/events/:id/ack` changes status to `acknowledged`.
- `POST /api/alarm/events/:id/resolve` changes status to `resolved`.
- `/ws/alarms` broadcasts `alarm.created`, `alarm.acknowledged`, and `alarm.resolved` events.
- The UI updates without manual refresh.

### Current Roadmap Status

- Alarm operator: Experimental metadata-based behavior.
- Persisted alarm lifecycle/API/WebSocket: planned.
- During API parity work, `GET /api/alarm/events` may return an empty compatibility response until the real lifecycle is implemented.

## Scenario 4: API/UI Parity Smoke Test

### Goal

Ensure frontend-visible API routes do not 404 and return documented shapes.

### Routes

The following routes must return non-404 responses:

| Route | Expected Shape |
|---|---|
| `GET /api/hardware` | Object with `hardware_id`, `arch`, `os_version`, `uptime_seconds`. |
| `GET /api/logs/download?type=system` | File/text response with `Content-Disposition`. |
| `GET /api/logs/config` | Object with `level`, `filter`, `dynamic_reload`, `restart_required`. |
| `PUT /api/logs/config` | Echo/validated log config with `restart_required: true`. |
| `GET /api/system/config` | Object with port, data/static/plugins dirs, auth and log fields. |
| `PUT /api/system/config` | Validated config response with `restart_required: true`. |
| `GET /api/alarm/events` | Object with `items` array; during transition also `events`. |
| `POST /api/alarm/events/:id/ack` | Updates status once real alarm lifecycle exists. |
| `POST /api/alarm/events/:id/resolve` | Updates status once real alarm lifecycle exists. |

### Suggested Smoke Commands

```bash
curl -i http://127.0.0.1:4000/api/hardware
curl -i http://127.0.0.1:4000/api/logs/config
curl -i http://127.0.0.1:4000/api/logs/download?type=system
curl -i http://127.0.0.1:4000/api/system/config
curl -i http://127.0.0.1:4000/api/alarm/events
```

### Expected Results

- No listed GET route returns 404.
- JSON routes return valid JSON.
- Download route returns a body and `Content-Disposition`.
- Frontend pages depending on these APIs can render without unhandled request errors.

## Scenario 5: Metrics Smoke Test

### Goal

Confirm observability output is available and grows toward the roadmap metric set.

### Steps

1. Start a south node and north node.
2. Let at least one poll/publish cycle complete.
3. Fetch metrics:

   ```bash
   curl http://127.0.0.1:4000/api/metrics
   ```

### Expected Results

- Metrics endpoint returns Prometheus text format.
- Existing data-flow counters are present.
- After production-hardening work, the endpoint includes:
  - `gateway_south_poll_total`
  - `gateway_flow_exec_total`
  - `gateway_north_publish_total`
  - `gateway_alarm_active_total`

## Final Acceptance Checklist

Before marking a roadmap milestone complete, run:

```bash
cargo test
cd web && npm run build
```

For Flow-specific milestones, also run:

```bash
cargo test -p gateway-flow
cargo test -p gateway-core flow
```

For API/UI parity milestones, run:

```bash
cargo test -p gateway-server
cd web && npm run build
```

For real protocol milestones, run:

```bash
cargo test -p plugin-modbus-tcp
cargo test -p plugin-modbus-rtu
cargo test -p plugin-mqtt
```

## Full-chain process E2E

The full-chain E2E verifies the product runtime loop with real gateway components:

```text
Sim south plugin -> Flow range/alarm operators -> MQTT north plugin
                                      |-> flow WebSocket data_processed event
                                      |-> alarm WebSocket / Alarm REST event
```

Run it explicitly because it starts processes and needs an MQTT broker:

```bash
cargo build -p gateway-server
python3 -m pip install -r e2e/full_chain/requirements.txt
./scripts/e2e_full_chain.sh
```

The runner starts gateway with an isolated temporary data directory and `GATEWAY_DISABLE_AUTH=true`. It does not use the development `data/` directory.

MQTT broker behavior:

- If `E2E_MQTT_HOST` and `E2E_MQTT_PORT` are set, the runner uses that broker.
- If no broker is configured, the runner tries to start a temporary `mosquitto` broker.
- If neither path is available, the command exits with code `77` and prints a skip message.

Useful environment variables:

```bash
E2E_MQTT_HOST=127.0.0.1
E2E_MQTT_PORT=1883
E2E_KEEP_ARTIFACTS=1
E2E_VERBOSE=1
GATEWAY_BIN=./target/debug/gateway
```

Success requires all of these observations in one run:

- MQTT receives a payload on `iot-gateway/e2e/<run_id>/#`.
- The MQTT `temperature` value is greater than `100`, proving the Flow `range` operator processed the Sim value.
- The flow WebSocket receives a `data_processed` event for the test group.
- The alarm WebSocket or Alarm REST API observes the test rule event.
- The Alarm REST API returns the persisted event for the test rule.
