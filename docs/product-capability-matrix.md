# IoT Gateway Product Capability Matrix

Date: 2026-05-29

This matrix freezes the current production-readiness scope for the IoT Gateway roadmap. It is intentionally conservative: a feature is marked GA/Beta only when the implementation is real enough to demonstrate or field-test.

## Status Legend

| Status | Meaning |
|---|---|
| GA | Supported product capability for the current target scope. |
| Beta | Real implementation, usable for demos/field trials, still needs hardening. |
| Experimental | Prototype or simulated behavior; not production-ready. |
| Stub | API/plugin shell exists but behavior is partial or placeholder. |
| Missing | Planned but not implemented yet. |

## Protocol Stack Legend

| Stack | Meaning |
|---|---|
| Real | Uses an actual protocol/client stack or realistic protocol implementation. |
| Simulated | Generates/accepts simulated data only. |
| Partial | Some protocol-facing behavior exists but not a complete production path. |
| None | No protocol behavior yet. |

## South Plugins

| Plugin | Status | Protocol Stack | Capabilities | Notes |
|---|---|---|---|---|
| sim | GA | Simulated | read | Primary demo and test source. |
| modbus-tcp | Beta | Real | read, write | Real protocol path; continue integration/stability tests. |
| modbus-rtu | Beta | Real | read, write | Real protocol path; serial environment coverage still required. |
| opcua | Beta | Real | read, write, browse | Real client stack; needs broader server compatibility testing. |
| virb | Beta | Real | read, write | Real implementation for supported devices; field validation needed. |
| bacnet | Experimental | Simulated | read | Treat as simulated/prototype until real stack is verified. |
| s7 | Experimental | Simulated | read | Treat as simulated/prototype until real stack is verified. |
| dlt645 | Experimental | Simulated | read | Treat as simulated/prototype until real stack is verified. |
| iec61850 | Experimental | Simulated | read | Treat as simulated/prototype until real stack is verified. |
| ethernet-ip | Experimental | Simulated | read | Treat as simulated/prototype until real stack is verified. |
| mitsubishi-mc | Experimental | Simulated | read | Treat as simulated/prototype until real stack is verified. |
| profinet | Experimental | Simulated | read | Treat as simulated/prototype until real stack is verified. |
| snmp | Experimental | Simulated | read | Treat as simulated/prototype until real stack is verified. |
| omron-fins | Experimental | Simulated | read | Treat as simulated/prototype until real stack is verified. |

## North Plugins

| Plugin | Status | Protocol Stack | Capabilities | Notes |
|---|---|---|---|---|
| mqtt | Beta | Real | publish | Main real northbound path. |
| http | Stub | Partial | publish | FFI-style plugin shell exists; migrate to `NorthPlugin` lifecycle. |
| kafka | Stub | Partial | publish | FFI-style plugin shell exists; needs real lifecycle/integration hardening. |
| influxdb | Experimental | Partial | publish | Not part of the v0.6 closed-loop target. |
| tdengine | Experimental | Partial | publish | Not part of the v0.6 closed-loop target. |
| websocket | Experimental | Partial | publish | Do not present as production northbound protocol yet. |
| grpc | Missing | None | none | Planned only. |
| sparkplug | Experimental | Partial | publish | Needs protocol conformance validation. |

## Flow Operators

| Operator | Status | Notes |
|---|---|---|
| filter | Beta | Usable for core Flow demos. |
| transform | Beta | Usable for core Flow demos. |
| deadband | Beta | Usable for core Flow demos. |
| alarm | Experimental | Rule evaluation exists, but persisted lifecycle is not complete yet. |
| throttle | Experimental | Needs runtime/load validation. |
| aggregate | Stub | Count-window implementation is planned. |
| buffer | Stub | Batch flush implementation is planned. |
| batch/change/convert/dedup/formula/join/json_path/xml_path | Experimental | Useful for experimentation; validate per scenario before demo claims. |

## Product Areas

| Area | Status | Notes |
|---|---|---|
| South -> Bus -> North | Beta | Core Manager/Bus/NorthPlugin path exists. |
| South -> Flow -> North | Experimental | Flow runtime exists but must be integrated into the main data path. |
| Flow CRUD/lifecycle | Beta | Flow store, deploy/start/pause/stop APIs exist. |
| Flow preview | Missing | Replace mock preview with real API. |
| Live monitor | Beta | WebSocket/live UI path exists; continue stability testing. |
| Alarm events | Beta | Basic persisted lifecycle exists: create/list/ack/resolve plus standalone UI visibility. Query filters, dedupe/recovery semantics, and WebSocket auth hardening remain future work. |
| Plugin status metadata | Missing | SDK/API/UI metadata extension planned. |
| Prometheus metrics | Experimental | Basic metrics exist; production metric names/runtime paths planned. |
| Audit logs | Missing | Store/API and critical-operation recording planned. |
| RBAC | Experimental | Roles exist; route-level enforcement planned. |
| Backup/restore | Beta | Existing API present; production verification still required. |
| License | Beta | Offline license path exists; commercial policy hardening continues. |

## v0.6 Closed-Loop Acceptance Target

The first production-readiness target is a reliable demo/internal acceptance path:

```text
Sim or Modbus TCP/RTU -> Flow filter/transform/deadband/alarm -> MQTT -> Live Monitor + basic Alarm visibility
```

Required before marking v0.6 complete:

- API/UI parity routes return non-404 responses.
- Flow preview uses real runtime execution.
- A bound Flow can process south group data before north publishing.
- MQTT receives processed values.
- Alarm events have at least a basic persisted lifecycle and standalone UI visibility.
- Plugin capability/status metadata is visible in the UI.
