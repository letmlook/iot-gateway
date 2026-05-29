# Full-Chain E2E Design

Date: 2026-05-30

## Goal

Add a process-level end-to-end acceptance test that proves the core gateway product loop works with real runtime components:

```text
Sim south plugin
  -> gateway manager / bus
  -> flow processor
  -> MQTT north plugin
  -> test MQTT subscriber

and, in the same run:
  -> WebSocket live-monitor events
  -> alarm event trigger and REST query
```

The test should be runnable locally and in CI as an explicit heavy E2E command. It should not be part of the default fast test suite.

## Scope

In scope:

- Start a real gateway process with isolated test configuration.
- Use the built-in Sim plugin as the south data source.
- Use the MQTT north plugin as the north output target.
- Configure nodes, groups, tags, flow binding, MQTT subscription, and alarm rule through REST APIs.
- Observe the runtime through MQTT, WebSocket, and Alarm REST APIs.
- Use temporary test data and unique resource names so the test is repeatable.
- Provide clear diagnostics and cleanup behavior.

Out of scope for this increment:

- UI-driven Playwright configuration flows.
- Modbus TCP or other real south protocol coverage.
- Broad plugin maturity work.
- Large API or WebSocket protocol redesigns.

## Recommended Approach

Implement one full-chain process-level E2E test, internally structured as staged helpers. The final test should validate the complete chain in one run, while each stage logs enough detail to identify whether a failure happened during gateway startup, REST configuration, Sim polling, flow processing, MQTT output, WebSocket observation, or alarm persistence.

This combines the value of a true product-level acceptance test with the debuggability of segmented setup and assertions.

## Files

Add:

```text
scripts/e2e_full_chain.sh
e2e/full_chain/test_full_chain.py
e2e/full_chain/requirements.txt
```

Update documentation as needed:

```text
README.md
docs/e2e-acceptance-scenarios.md
```

If the existing shared Python dependency file is sufficient, `e2e/full_chain/requirements.txt` may point users to the shared requirements instead of duplicating dependencies.

## Runtime Shape

### Shell wrapper

`scripts/e2e_full_chain.sh` is the stable entry point. It should:

1. Check for `python3`.
2. Check that the gateway binary exists, or clearly instruct the user to run `cargo build` first.
3. Export default E2E environment variables when they are not already set.
4. Invoke the Python E2E runner.
5. Preserve the Python runner exit code.

The wrapper should avoid hiding errors behind broad shell cleanup. Cleanup that needs state should live in the Python runner.

### Python runner

`e2e/full_chain/test_full_chain.py` owns the acceptance flow:

1. Allocate a unique `run_id`, such as `e2e_full_chain_<timestamp>`.
2. Choose a free gateway port.
3. Create a temporary `GATEWAY_DATA_DIR`.
4. Start the gateway process with:

   ```text
   GATEWAY_PORT=<free_port>
   GATEWAY_DATA_DIR=<tmp_dir>
   GATEWAY_DISABLE_AUTH=true
   RUST_LOG=info,gateway_server=debug,gateway_core=debug
   ```

5. Wait for `/api/health` to become ready.
6. Connect to a configured MQTT broker, or start a temporary local broker when possible.
7. Configure test resources through REST APIs.
8. Open WebSocket observation before starting the data flow.
9. Start the south and north nodes, and enable the flow if the flow lifecycle requires it.
10. Wait for MQTT, WebSocket, and Alarm REST assertions.
11. Clean up test resources, stop child processes, and remove temporary data unless artifact retention is requested.

## Test Resources

All created resources should include the unique `run_id` prefix.

```text
South node:      <run_id>_sim
South group:     <run_id>_group
Tags:            temperature, pressure
Flow:            <run_id>_flow
MQTT north node: <run_id>_mqtt
MQTT topic:      iot-gateway/e2e/<run_id>/data
Alarm rule:      <run_id>_high_temperature
```

The flow should perform a deterministic transformation that can be distinguished from raw Sim data, for example deriving a transformed temperature from the incoming `temperature` tag. The MQTT assertion must prove the output came from the flow result rather than only from unprocessed Sim data.

The alarm rule should use a condition that is guaranteed to trigger with the test's generated or transformed data. If the existing alarm model requires a threshold over a tag value, choose a threshold below the deterministic test value.

## REST Configuration Flow

The test should use the existing public API surface where possible:

```text
1. Verify health.
2. Create Sim south node.
3. Create group.
4. Create tags.
5. Create flow.
6. Bind or enable the flow for the test south group.
7. Create MQTT north node.
8. Create north subscription to the test group.
9. Create alarm rule.
10. Open WebSocket listener.
11. Start south node.
12. Start north node.
13. Start or enable flow if required.
14. Wait for runtime observations.
15. Query Alarm REST API.
16. Clean up resources.
```

If an existing API is awkward but usable, the E2E should adapt to it instead of changing product APIs. Product code should only be patched when the current implementation has a clear bug or cannot express the required chain.

## MQTT Broker Strategy

The runner should support explicit broker configuration:

```text
E2E_MQTT_HOST=127.0.0.1
E2E_MQTT_PORT=1883
```

When no broker is configured, the runner should try to locate `mosquitto` and start a temporary broker on an available port. If no broker can be used, the test should exit with a clear skip message instead of reporting a product failure.

CI can provide a known broker through environment variables. Local developers can either install `mosquitto` or point the test at an existing broker.

## Observability and Assertions

The test passes only when all of these are true:

1. Gateway health becomes ready within the startup timeout.
2. MQTT subscriber receives at least one message on the `run_id` topic.
3. The MQTT payload proves that flow processing occurred.
4. WebSocket receives a group data event associated with the test node/group.
5. WebSocket receives an alarm event, or the Alarm REST API observes the event after the trigger.
6. Alarm REST query returns an event associated with the test rule/node/group/tag.
7. Cleanup completes or reports exactly which resource could not be removed.

WebSocket assertions should tolerate the existing catch-all event shape if that is how the server currently publishes events. The test should inspect message type and payload fields defensively, while still requiring enough identifiers to avoid matching unrelated events.

## Stability Strategy

Use bounded waits rather than fixed sleeps:

```text
health ready:             20 seconds
MQTT message received:    30 seconds
WebSocket group event:    30 seconds
Alarm event persisted:    30 seconds
cleanup requests:         bounded per request
```

Timeout errors should include:

- The condition that was being waited on.
- Counts of MQTT messages and WebSocket messages observed.
- The last relevant REST response summary.
- Tail output from gateway stdout/stderr.
- MQTT broker startup or connection status.

## Cleanup and Artifacts

Normal completion should:

- Delete or stop test-created resources where APIs support it.
- Stop gateway and temporary broker processes.
- Remove the temporary data directory.

Failure completion should:

- Stop child processes.
- Preserve enough logs for diagnosis.
- Respect `E2E_KEEP_ARTIFACTS=1` to keep the temporary data directory and logs.

The test must not use the default development `data/` directory.

## Minimal Product Patches Allowed

Implementation may include small product fixes when necessary for this chain:

- Correcting a broken flow binding or lifecycle call.
- Making MQTT output parseable according to the existing intended format.
- Fixing alarm rule creation or event persistence bugs that block the chain.
- Making WebSocket event emission include already-modeled identifiers needed for reliable matching.

Avoid broad refactors, UI changes, and protocol redesigns in this increment.

## Validation Plan

After implementation, run:

```bash
cargo test --workspace
./scripts/e2e_full_chain.sh
```

If the environment lacks an MQTT broker and `mosquitto`, the E2E command may skip with a clear dependency message. In a configured environment, the E2E command must run the full chain and fail on missing MQTT, WebSocket, or Alarm observations.

## Acceptance Criteria

- A documented one-command full-chain E2E entry point exists.
- The test starts gateway with isolated data and auth disabled.
- The test uses Sim as south input and MQTT as north output.
- The test configures resources through REST APIs.
- The test verifies flow-processed MQTT output.
- The test verifies WebSocket live data and alarm visibility.
- The test verifies persisted alarm data through REST.
- The test logs staged diagnostics and supports artifact retention.
- The test does not pollute local development data.
