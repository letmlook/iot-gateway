# Full-Chain E2E Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Add a one-command process-level E2E test that proves Sim south data is processed by a bound Flow, published through MQTT north, visible over WebSocket, and persisted as an Alarm event.

**Architecture:** The E2E runner is a Python script that owns the whole acceptance lifecycle: starts an isolated gateway process, starts or connects to an MQTT broker, configures resources through REST APIs, observes MQTT and WebSocket channels, verifies Alarm REST persistence, and cleans up. A small server patch adds a `data_processed` flow WebSocket event from the existing `FlowGroupDataProcessor`, because the current flow WebSocket only emits connection, metrics, status, and node events.

**Tech Stack:** Rust/Axum gateway, gateway-flow operators (`range`, `alarm`), Python 3.10+, `aiohttp`, `websockets`, `paho-mqtt`, `mosquitto` when no broker is configured.

---

## File Structure

Create:

- `e2e/full_chain/requirements.txt` — Python dependencies for this E2E suite.
- `e2e/full_chain/test_full_chain.py` — executable E2E runner and importable helper functions.
- `e2e/full_chain/test_full_chain_unit.py` — fast unit tests for helper functions used by the runner.
- `scripts/e2e_full_chain.sh` — stable shell entry point.

Modify:

- `gateway/gateway-server/src/websocket/mod.rs` — add serializable `FlowLiveEvent::DataProcessed` and a unit test for its JSON shape.
- `gateway/gateway-server/src/flow/processor.rs` — broadcast `DataProcessed` when a bound flow transforms group data.
- `docs/e2e-acceptance-scenarios.md` — document the new full-chain E2E command, dependencies, and skip behavior.

Do not modify UI files for this plan.

---

### Task 1: Add Python E2E Dependencies and Helper Unit Tests

**Files:**
- Create: `e2e/full_chain/requirements.txt`
- Create: `e2e/full_chain/test_full_chain_unit.py`
- Create: `e2e/full_chain/.gitignore`

- [ ] **Step 1: Create the Python dependency manifest**

Create `e2e/full_chain/requirements.txt` with exactly:

```text
aiohttp>=3.9.0
websockets>=12.0
paho-mqtt>=1.6.1
```

- [ ] **Step 2: Ignore local Python cache files for this suite**

Create `e2e/full_chain/.gitignore` with exactly:

```text
__pycache__/
*.pyc
.venv/
```

- [ ] **Step 3: Write failing unit tests for runner helpers**

Create `e2e/full_chain/test_full_chain_unit.py` with exactly:

```python
#!/usr/bin/env python3
import json
import socket
import unittest

import test_full_chain as full_chain


class FullChainHelperTests(unittest.TestCase):
    def test_free_port_returns_bindable_port(self):
        port = full_chain.free_port()
        self.assertIsInstance(port, int)
        self.assertGreater(port, 0)

        with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
            sock.bind(("127.0.0.1", port))

    def test_datavalue_to_python_converts_gateway_values(self):
        self.assertEqual(full_chain.datavalue_to_python({"type": "Bool", "value": True}), True)
        self.assertEqual(full_chain.datavalue_to_python({"type": "Int64", "value": 42}), 42)
        self.assertEqual(full_chain.datavalue_to_python({"type": "Float64", "value": 12.5}), 12.5)
        self.assertEqual(full_chain.datavalue_to_python({"type": "String", "value": "ok"}), "ok")
        self.assertEqual(full_chain.datavalue_to_python({"type": "Bytes", "value": [1, 2]}), [1, 2])
        self.assertIsNone(full_chain.datavalue_to_python({"type": "Missing"}))

    def test_mqtt_payload_proves_flow_processing(self):
        payload = {
            "timestamp": 1710000000000,
            "node": "e2e_sim",
            "group": "e2e_group",
            "values": {
                "temperature": 123.4,
                "humidity": 55.0,
            },
            "errors": {},
            "metas": {},
        }

        ok, detail = full_chain.mqtt_payload_proves_flow(json.dumps(payload).encode("utf-8"), "e2e_group")
        self.assertTrue(ok, detail)
        self.assertIn("temperature=123.4", detail)

    def test_mqtt_payload_rejects_unprocessed_temperature(self):
        payload = {
            "timestamp": 1710000000000,
            "node": "e2e_sim",
            "group": "e2e_group",
            "values": {
                "temperature": 23.4,
                "humidity": 55.0,
            },
            "errors": {},
            "metas": {},
        }

        ok, detail = full_chain.mqtt_payload_proves_flow(json.dumps(payload).encode("utf-8"), "e2e_group")
        self.assertFalse(ok)
        self.assertIn("temperature <= 100", detail)

    def test_build_flow_contains_range_and_alarm_nodes(self):
        flow = full_chain.build_flow_definition("e2e_run", "temperature")
        self.assertEqual(flow["name"], "e2e_run_flow")
        self.assertEqual(flow["status"], "draft")
        self.assertEqual(len(flow["nodes"]), 4)
        self.assertEqual(len(flow["edges"]), 3)

        operator_names = [node.get("operator_name") for node in flow["nodes"]]
        self.assertIn("range", operator_names)
        self.assertIn("alarm", operator_names)

        range_node = next(node for node in flow["nodes"] if node.get("operator_name") == "range")
        self.assertEqual(range_node["config"], {
            "tag": "temperature",
            "in_min": 0.0,
            "in_max": 100.0,
            "out_min": 100.0,
            "out_max": 200.0,
        })

        alarm_node = next(node for node in flow["nodes"] if node.get("operator_name") == "alarm")
        self.assertEqual(alarm_node["config"]["rules"][0]["id"], "e2e_run_high_temperature")
        self.assertEqual(alarm_node["config"]["rules"][0]["high"], 100.0)


if __name__ == "__main__":
    unittest.main()
```

- [ ] **Step 4: Run the unit tests and verify they fail because the runner module does not exist**

Run:

```bash
python3 -m unittest e2e/full_chain/test_full_chain_unit.py -v
```

Expected: failure with this import error:

```text
ModuleNotFoundError: No module named 'test_full_chain'
```

- [ ] **Step 5: Commit the failing tests and dependency manifest**

Run:

```bash
git add e2e/full_chain/requirements.txt e2e/full_chain/.gitignore e2e/full_chain/test_full_chain_unit.py
git commit -m "test(e2e): add full-chain helper unit tests" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 2: Implement Runner Helper Functions

**Files:**
- Create: `e2e/full_chain/test_full_chain.py`
- Test: `e2e/full_chain/test_full_chain_unit.py`

- [ ] **Step 1: Create the initial helper implementation**

Create `e2e/full_chain/test_full_chain.py` with exactly:

```python
#!/usr/bin/env python3
"""Process-level full-chain E2E runner for iot-gateway.

This module is both executable and importable. Unit tests cover pure helper
functions; the executable path starts gateway, MQTT, REST, and WebSocket checks.
"""

from __future__ import annotations

import argparse
import asyncio
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import time
import uuid
from collections import deque
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Any


REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_GATEWAY_HOST = "127.0.0.1"
DEFAULT_FLOW_TIMEOUT_SECONDS = 30.0


class SkipE2E(RuntimeError):
    """Raised when the environment cannot run this E2E suite."""


def utc_now_rfc3339() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def wait_for_tcp(host: str, port: int, timeout: float) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            with socket.create_connection((host, port), timeout=0.5):
                return True
        except OSError:
            time.sleep(0.2)
    return False


def datavalue_to_python(value: Any) -> Any:
    if not isinstance(value, dict):
        return value
    value_type = value.get("type")
    raw = value.get("value")
    if value_type in {
        "Bool",
        "Int8",
        "Int16",
        "Int32",
        "Int64",
        "UInt8",
        "UInt16",
        "UInt32",
        "UInt64",
        "Float32",
        "Float64",
        "String",
        "Bytes",
    }:
        return raw
    return None


def mqtt_payload_proves_flow(payload: bytes, expected_group: str) -> tuple[bool, str]:
    try:
        decoded = json.loads(payload.decode("utf-8"))
    except Exception as exc:
        return False, f"payload is not JSON: {exc}"

    if decoded.get("group") != expected_group:
        return False, f"group mismatch: expected {expected_group}, got {decoded.get('group')}"

    values = decoded.get("values")
    if not isinstance(values, dict):
        return False, f"payload has no values object: {decoded}"

    temperature = values.get("temperature")
    if not isinstance(temperature, (int, float)):
        return False, f"temperature is not numeric: {temperature!r}"

    if temperature <= 100.0:
        return False, f"temperature <= 100, so flow range transform is not proven: temperature={temperature}"

    return True, f"temperature={temperature} proves range flow transform"


def port(port_id: str, name: str, required: bool) -> dict[str, Any]:
    return {
        "id": port_id,
        "name": name,
        "port_type": "data",
        "required": required,
    }


def build_flow_definition(run_id: str, alarm_tag: str) -> dict[str, Any]:
    now = utc_now_rfc3339()
    source_id = str(uuid.uuid4())
    range_id = str(uuid.uuid4())
    alarm_id = str(uuid.uuid4())
    sink_id = str(uuid.uuid4())

    return {
        "id": str(uuid.uuid4()),
        "name": f"{run_id}_flow",
        "description": "Full-chain E2E flow: range transform then threshold alarm",
        "status": "draft",
        "created_at": now,
        "updated_at": now,
        "nodes": [
            {
                "id": source_id,
                "name": "source",
                "kind": "south",
                "operator_name": None,
                "config": {},
                "input_ports": [],
                "output_ports": [port("out", "out", False)],
            },
            {
                "id": range_id,
                "name": "range_temperature",
                "kind": "operator",
                "operator_name": "range",
                "config": {
                    "tag": alarm_tag,
                    "in_min": 0.0,
                    "in_max": 100.0,
                    "out_min": 100.0,
                    "out_max": 200.0,
                },
                "input_ports": [port("in", "in", True)],
                "output_ports": [port("out", "out", False)],
            },
            {
                "id": alarm_id,
                "name": "alarm_temperature",
                "kind": "operator",
                "operator_name": "alarm",
                "config": {
                    "rules": [
                        {
                            "id": f"{run_id}_high_temperature",
                            "type": "threshold",
                            "tag": alarm_tag,
                            "high": 100.0,
                            "low": None,
                            "level": "high",
                            "message": f"{run_id} transformed temperature is high",
                        }
                    ]
                },
                "input_ports": [port("in", "in", True)],
                "output_ports": [port("out", "out", False)],
            },
            {
                "id": sink_id,
                "name": "sink",
                "kind": "north",
                "operator_name": None,
                "config": {},
                "input_ports": [port("in", "in", True)],
                "output_ports": [],
            },
        ],
        "edges": [
            {
                "source_node_id": source_id,
                "source_port": "out",
                "target_node_id": range_id,
                "target_port": "in",
            },
            {
                "source_node_id": range_id,
                "source_port": "out",
                "target_node_id": alarm_id,
                "target_port": "in",
            },
            {
                "source_node_id": alarm_id,
                "source_port": "out",
                "target_node_id": sink_id,
                "target_port": "in",
            },
        ],
        "bindings": [],
        "version": 1,
    }


@dataclass
class RuntimeConfig:
    gateway_host: str = DEFAULT_GATEWAY_HOST
    gateway_port: int = 0
    gateway_bin: Path | None = None
    mqtt_host: str = "127.0.0.1"
    mqtt_port: int = 0
    keep_artifacts: bool = False
    timeout: float = DEFAULT_FLOW_TIMEOUT_SECONDS


@dataclass
class RuntimeState:
    run_id: str
    config: RuntimeConfig
    temp_dir: Path
    gateway_process: subprocess.Popen[str] | None = None
    broker_process: subprocess.Popen[str] | None = None
    gateway_log: deque[str] = field(default_factory=lambda: deque(maxlen=120))
    broker_log: deque[str] = field(default_factory=lambda: deque(maxlen=80))

    @property
    def gateway_url(self) -> str:
        return f"http://{self.config.gateway_host}:{self.config.gateway_port}"

    @property
    def api_url(self) -> str:
        return f"{self.gateway_url}/api"

    @property
    def flow_ws_base(self) -> str:
        return f"ws://{self.config.gateway_host}:{self.config.gateway_port}/api/ws/flows"

    @property
    def alarm_ws_url(self) -> str:
        return f"ws://{self.config.gateway_host}:{self.config.gateway_port}/api/ws/alarms"


def print_step(message: str) -> None:
    print(message, flush=True)


def main(argv: list[str] | None = None) -> int:
    parser = argparse.ArgumentParser(description="Run the iot-gateway full-chain E2E test")
    parser.add_argument("--unit-helper-check", action="store_true", help="only verify helper import path")
    args = parser.parse_args(argv)
    if args.unit_helper_check:
        print_step("[setup] helper module import ok")
        return 0
    print_step("[setup] full-chain runner helpers are installed; execute Task 4 to enable the full E2E flow")
    return 2


if __name__ == "__main__":
    raise SystemExit(main())
```

- [ ] **Step 2: Run the helper unit tests and verify they pass**

Run:

```bash
python3 -m unittest e2e/full_chain/test_full_chain_unit.py -v
```

Expected: all five tests pass:

```text
Ran 5 tests

OK
```

- [ ] **Step 3: Run the helper import check**

Run:

```bash
python3 e2e/full_chain/test_full_chain.py --unit-helper-check
```

Expected:

```text
[setup] helper module import ok
```

- [ ] **Step 4: Commit helper implementation**

Run:

```bash
git add e2e/full_chain/test_full_chain.py e2e/full_chain/test_full_chain_unit.py
git commit -m "test(e2e): add full-chain runner helpers" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 3: Add Flow Data WebSocket Event from the Processor

**Files:**
- Modify: `gateway/gateway-server/src/websocket/mod.rs`
- Modify: `gateway/gateway-server/src/flow/processor.rs`
- Test: `gateway/gateway-server/src/websocket/mod.rs`

- [ ] **Step 1: Add a failing serialization test for the new WebSocket event**

Append this test module to the end of `gateway/gateway-server/src/websocket/mod.rs`:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn data_processed_event_serializes_with_source_identifiers() {
        let event = FlowLiveEvent::DataProcessed {
            flow_id: "flow-1".to_string(),
            south_node_id: "south-1".to_string(),
            group_id: "group-1".to_string(),
            node_name: Some("e2e_sim".to_string()),
            group_name: Some("e2e_group".to_string()),
            values: serde_json::json!({
                "temperature": 123.4,
                "humidity": 55.0,
            }),
        };

        let json = serde_json::to_value(event).unwrap();
        assert_eq!(json["type"], "data_processed");
        assert_eq!(json["flow_id"], "flow-1");
        assert_eq!(json["south_node_id"], "south-1");
        assert_eq!(json["group_id"], "group-1");
        assert_eq!(json["node_name"], "e2e_sim");
        assert_eq!(json["group_name"], "e2e_group");
        assert_eq!(json["values"]["temperature"], 123.4);
    }
}
```

- [ ] **Step 2: Run the targeted Rust test and verify it fails**

Run:

```bash
cargo test -p gateway-server websocket::tests::data_processed_event_serializes_with_source_identifiers
```

Expected: compile failure containing:

```text
no variant named `DataProcessed` found for enum `FlowLiveEvent`
```

- [ ] **Step 3: Add the `DataProcessed` event variant**

In `gateway/gateway-server/src/websocket/mod.rs`, replace the current `FlowLiveEvent` enum with:

```rust
/// Events that can be sent over the WebSocket live channel.
#[derive(Debug, Clone, serde::Serialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum FlowLiveEvent {
    /// Periodic metrics snapshot (every ~1s when connected)
    Metrics {
        flow_id: String,
        flow_name: String,
        status: String,
        metrics: Vec<gateway_sdk::OperatorMetrics>,
    },
    /// Flow status changed (deployed, running, paused, stopped, error)
    StatusChange { flow_id: String, status: String },
    /// Node-level event (e.g., error, warning)
    NodeEvent {
        flow_id: String,
        node_id: String,
        node_name: String,
        event: String,
        detail: Option<String>,
    },
    /// A bound flow processed south group data in the main gateway data path.
    DataProcessed {
        flow_id: String,
        south_node_id: String,
        group_id: String,
        node_name: Option<String>,
        group_name: Option<String>,
        values: serde_json::Value,
    },
    /// Heartbeat / keepalive
    Ping { ts: String },
}
```

- [ ] **Step 4: Broadcast processed flow data from the flow processor**

In `gateway/gateway-server/src/flow/processor.rs`, change the import at the top from:

```rust
use crate::websocket::{AlarmLiveEvent, WsHub};
```

to:

```rust
use crate::websocket::{AlarmLiveEvent, FlowLiveEvent, WsHub};
```

Add this helper method inside `impl FlowGroupDataProcessor`, after `pipeline_to_group_data`:

```rust
    fn group_data_values_json(data: &GroupData) -> serde_json::Value {
        let mut values = serde_json::Map::new();
        for (tag_id, value) in &data.values {
            let key = data
                .tag_names
                .as_ref()
                .and_then(|names| names.get(tag_id))
                .cloned()
                .unwrap_or_else(|| tag_id.0.to_string());
            values.insert(key, serde_json::to_value(value).unwrap_or(serde_json::Value::Null));
        }
        serde_json::Value::Object(values)
    }

    async fn broadcast_processed_data(&self, flow_id: Uuid, data: &GroupData) {
        self.ws_hub
            .broadcast(
                flow_id,
                FlowLiveEvent::DataProcessed {
                    flow_id: flow_id.to_string(),
                    south_node_id: data.node_id.0.to_string(),
                    group_id: data.group_id.0.to_string(),
                    node_name: data.node_name.clone(),
                    group_name: data.group_name.clone(),
                    values: Self::group_data_values_json(data),
                },
            )
            .await;
    }
```

Then replace this block in `process_group_data`:

```rust
                let processed = Self::pipeline_to_group_data(&data, first);
                ProcessDecision::Publish(Arc::new(processed))
```

with:

```rust
                let processed = Self::pipeline_to_group_data(&data, first);
                self.broadcast_processed_data(binding.flow_id, &processed).await;
                ProcessDecision::Publish(Arc::new(processed))
```

- [ ] **Step 5: Run the targeted Rust test and verify it passes**

Run:

```bash
cargo test -p gateway-server websocket::tests::data_processed_event_serializes_with_source_identifiers
```

Expected:

```text
test websocket::tests::data_processed_event_serializes_with_source_identifiers ... ok
```

- [ ] **Step 6: Run server tests to catch compile errors**

Run:

```bash
cargo test -p gateway-server
```

Expected: all `gateway-server` tests pass.

- [ ] **Step 7: Commit the WebSocket processor patch**

Run:

```bash
git add gateway/gateway-server/src/websocket/mod.rs gateway/gateway-server/src/flow/processor.rs
git commit -m "feat(e2e): emit flow processed websocket events" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 4: Implement the Full-Chain E2E Runner

**Files:**
- Modify: `e2e/full_chain/test_full_chain.py`
- Test: `e2e/full_chain/test_full_chain_unit.py`

- [ ] **Step 1: Replace the helper-only runner with the full E2E runner**

Replace the full contents of `e2e/full_chain/test_full_chain.py` with:

```python
#!/usr/bin/env python3
"""Process-level full-chain E2E runner for iot-gateway."""

from __future__ import annotations

import argparse
import asyncio
import json
import os
import shutil
import socket
import subprocess
import sys
import tempfile
import threading
import time
import uuid
from collections import deque
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

import aiohttp
import paho.mqtt.client as mqtt
import websockets


REPO_ROOT = Path(__file__).resolve().parents[2]
DEFAULT_GATEWAY_HOST = "127.0.0.1"
DEFAULT_FLOW_TIMEOUT_SECONDS = 30.0


class SkipE2E(RuntimeError):
    """Raised when the environment cannot run this E2E suite."""


class E2EFailure(RuntimeError):
    """Raised when the product chain fails its acceptance criteria."""


def utc_now_rfc3339() -> str:
    return datetime.now(timezone.utc).isoformat().replace("+00:00", "Z")


def free_port() -> int:
    with socket.socket(socket.AF_INET, socket.SOCK_STREAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return int(sock.getsockname()[1])


def wait_for_tcp(host: str, port: int, timeout: float) -> bool:
    deadline = time.monotonic() + timeout
    while time.monotonic() < deadline:
        try:
            with socket.create_connection((host, port), timeout=0.5):
                return True
        except OSError:
            time.sleep(0.2)
    return False


def datavalue_to_python(value: Any) -> Any:
    if not isinstance(value, dict):
        return value
    value_type = value.get("type")
    raw = value.get("value")
    if value_type in {
        "Bool",
        "Int8",
        "Int16",
        "Int32",
        "Int64",
        "UInt8",
        "UInt16",
        "UInt32",
        "UInt64",
        "Float32",
        "Float64",
        "String",
        "Bytes",
    }:
        return raw
    return None


def mqtt_payload_proves_flow(payload: bytes, expected_group: str) -> tuple[bool, str]:
    try:
        decoded = json.loads(payload.decode("utf-8"))
    except Exception as exc:
        return False, f"payload is not JSON: {exc}"

    if decoded.get("group") != expected_group:
        return False, f"group mismatch: expected {expected_group}, got {decoded.get('group')}"

    values = decoded.get("values")
    if not isinstance(values, dict):
        return False, f"payload has no values object: {decoded}"

    temperature = values.get("temperature")
    if not isinstance(temperature, (int, float)):
        return False, f"temperature is not numeric: {temperature!r}"

    if temperature <= 100.0:
        return False, f"temperature <= 100, so flow range transform is not proven: temperature={temperature}"

    return True, f"temperature={temperature} proves range flow transform"


def port(port_id: str, name: str, required: bool) -> dict[str, Any]:
    return {
        "id": port_id,
        "name": name,
        "port_type": "data",
        "required": required,
    }


def build_flow_definition(run_id: str, alarm_tag: str) -> dict[str, Any]:
    now = utc_now_rfc3339()
    source_id = str(uuid.uuid4())
    range_id = str(uuid.uuid4())
    alarm_id = str(uuid.uuid4())
    sink_id = str(uuid.uuid4())

    return {
        "id": str(uuid.uuid4()),
        "name": f"{run_id}_flow",
        "description": "Full-chain E2E flow: range transform then threshold alarm",
        "status": "draft",
        "created_at": now,
        "updated_at": now,
        "nodes": [
            {
                "id": source_id,
                "name": "source",
                "kind": "south",
                "operator_name": None,
                "config": {},
                "input_ports": [],
                "output_ports": [port("out", "out", False)],
            },
            {
                "id": range_id,
                "name": "range_temperature",
                "kind": "operator",
                "operator_name": "range",
                "config": {
                    "tag": alarm_tag,
                    "in_min": 0.0,
                    "in_max": 100.0,
                    "out_min": 100.0,
                    "out_max": 200.0,
                },
                "input_ports": [port("in", "in", True)],
                "output_ports": [port("out", "out", False)],
            },
            {
                "id": alarm_id,
                "name": "alarm_temperature",
                "kind": "operator",
                "operator_name": "alarm",
                "config": {
                    "rules": [
                        {
                            "id": f"{run_id}_high_temperature",
                            "type": "threshold",
                            "tag": alarm_tag,
                            "high": 100.0,
                            "low": None,
                            "level": "high",
                            "message": f"{run_id} transformed temperature is high",
                        }
                    ]
                },
                "input_ports": [port("in", "in", True)],
                "output_ports": [port("out", "out", False)],
            },
            {
                "id": sink_id,
                "name": "sink",
                "kind": "north",
                "operator_name": None,
                "config": {},
                "input_ports": [port("in", "in", True)],
                "output_ports": [],
            },
        ],
        "edges": [
            {"source_node_id": source_id, "source_port": "out", "target_node_id": range_id, "target_port": "in"},
            {"source_node_id": range_id, "source_port": "out", "target_node_id": alarm_id, "target_port": "in"},
            {"source_node_id": alarm_id, "source_port": "out", "target_node_id": sink_id, "target_port": "in"},
        ],
        "bindings": [],
        "version": 1,
    }


@dataclass
class RuntimeConfig:
    gateway_host: str = DEFAULT_GATEWAY_HOST
    gateway_port: int = 0
    gateway_bin: Path | None = None
    mqtt_host: str = "127.0.0.1"
    mqtt_port: int = 0
    keep_artifacts: bool = False
    timeout: float = DEFAULT_FLOW_TIMEOUT_SECONDS


@dataclass
class RuntimeState:
    run_id: str
    config: RuntimeConfig
    temp_dir: Path
    gateway_process: subprocess.Popen[str] | None = None
    broker_process: subprocess.Popen[str] | None = None
    gateway_log: deque[str] = field(default_factory=lambda: deque(maxlen=120))
    broker_log: deque[str] = field(default_factory=lambda: deque(maxlen=80))
    created_flow_id: str | None = None
    south_node_id: str | None = None
    north_node_id: str | None = None

    @property
    def gateway_url(self) -> str:
        return f"http://{self.config.gateway_host}:{self.config.gateway_port}"

    @property
    def api_url(self) -> str:
        return f"{self.gateway_url}/api"

    @property
    def flow_ws_base(self) -> str:
        return f"ws://{self.config.gateway_host}:{self.config.gateway_port}/api/ws/flows"

    @property
    def alarm_ws_url(self) -> str:
        return f"ws://{self.config.gateway_host}:{self.config.gateway_port}/api/ws/alarms"


def print_step(message: str) -> None:
    print(message, flush=True)


def resolve_gateway_bin(explicit: str | None) -> Path:
    candidates = []
    if explicit:
        candidates.append(Path(explicit))
    env_bin = os.environ.get("GATEWAY_BIN")
    if env_bin:
        candidates.append(Path(env_bin))
    candidates.extend([
        REPO_ROOT / "target" / "debug" / "gateway",
        REPO_ROOT / "target" / "release" / "gateway",
    ])
    for candidate in candidates:
        if candidate.exists() and os.access(candidate, os.X_OK):
            return candidate
    raise SkipE2E("gateway binary not found; run `cargo build -p gateway-server` or set GATEWAY_BIN=/path/to/gateway")


def pump_output(process: subprocess.Popen[str], target: deque[str], label: str) -> threading.Thread:
    def run() -> None:
        if process.stdout is None:
            return
        for line in process.stdout:
            line = line.rstrip()
            target.append(line)
            if os.environ.get("E2E_VERBOSE") == "1":
                print(f"[{label}] {line}", flush=True)

    thread = threading.Thread(target=run, daemon=True)
    thread.start()
    return thread


async def wait_for_health(state: RuntimeState, timeout: float) -> None:
    deadline = time.monotonic() + timeout
    last_error = "no request attempted"
    async with aiohttp.ClientSession() as session:
        while time.monotonic() < deadline:
            try:
                async with session.get(f"{state.api_url}/health", timeout=2) as resp:
                    text = await resp.text()
                    if resp.status == 200:
                        print_step(f"[setup] gateway health ok: {text}")
                        return
                    last_error = f"HTTP {resp.status}: {text}"
            except Exception as exc:
                last_error = repr(exc)
            await asyncio.sleep(0.3)
    raise E2EFailure(f"gateway health did not become ready: {last_error}\nGateway log tail:\n" + "\n".join(state.gateway_log))


def start_gateway(state: RuntimeState) -> None:
    data_dir = state.temp_dir / "gateway-data"
    data_dir.mkdir(parents=True, exist_ok=True)
    env = os.environ.copy()
    env.update({
        "GATEWAY_PORT": str(state.config.gateway_port),
        "GATEWAY_DATA_DIR": str(data_dir),
        "GATEWAY_DISABLE_AUTH": "true",
        "RUST_LOG": env.get("RUST_LOG", "info,gateway_server=debug,gateway_core=debug"),
    })
    state.gateway_process = subprocess.Popen(
        [str(state.config.gateway_bin)],
        cwd=str(REPO_ROOT),
        env=env,
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        bufsize=1,
    )
    pump_output(state.gateway_process, state.gateway_log, "gateway")
    print_step(f"[setup] gateway started pid={state.gateway_process.pid} port={state.config.gateway_port} data_dir={data_dir}")


def start_or_configure_broker(state: RuntimeState) -> None:
    configured_host = os.environ.get("E2E_MQTT_HOST")
    configured_port = os.environ.get("E2E_MQTT_PORT")
    if configured_host and configured_port:
        state.config.mqtt_host = configured_host
        state.config.mqtt_port = int(configured_port)
        if not wait_for_tcp(state.config.mqtt_host, state.config.mqtt_port, timeout=5):
            raise SkipE2E(f"configured MQTT broker is unreachable at {state.config.mqtt_host}:{state.config.mqtt_port}")
        print_step(f"[setup] using configured MQTT broker {state.config.mqtt_host}:{state.config.mqtt_port}")
        return

    mosquitto = shutil.which("mosquitto")
    if not mosquitto:
        raise SkipE2E("MQTT broker unavailable; set E2E_MQTT_HOST/E2E_MQTT_PORT or install mosquitto")

    state.config.mqtt_host = "127.0.0.1"
    state.config.mqtt_port = free_port()
    config_path = state.temp_dir / "mosquitto.conf"
    config_path.write_text(
        f"listener {state.config.mqtt_port} {state.config.mqtt_host}\nallow_anonymous true\npersistence false\n",
        encoding="utf-8",
    )
    state.broker_process = subprocess.Popen(
        [mosquitto, "-c", str(config_path)],
        stdout=subprocess.PIPE,
        stderr=subprocess.STDOUT,
        text=True,
        bufsize=1,
    )
    pump_output(state.broker_process, state.broker_log, "mqtt")
    if not wait_for_tcp(state.config.mqtt_host, state.config.mqtt_port, timeout=5):
        raise SkipE2E("temporary mosquitto broker did not become reachable\nBroker log tail:\n" + "\n".join(state.broker_log))
    print_step(f"[setup] temporary MQTT broker ready {state.config.mqtt_host}:{state.config.mqtt_port}")


class RestClient:
    def __init__(self, state: RuntimeState):
        self.state = state
        self.session: aiohttp.ClientSession | None = None

    async def __aenter__(self) -> "RestClient":
        self.session = aiohttp.ClientSession()
        return self

    async def __aexit__(self, exc_type: Any, exc: Any, tb: Any) -> None:
        if self.session:
            await self.session.close()

    async def request(self, method: str, path: str, json_body: Any | None = None, ok: set[int] | None = None) -> Any:
        assert self.session is not None
        expected = ok or {200, 201, 204}
        url = f"{self.state.api_url}{path}"
        async with self.session.request(method, url, json=json_body, timeout=30) as resp:
            text = await resp.text()
            if resp.status not in expected:
                raise E2EFailure(f"{method} {path} failed: HTTP {resp.status}: {text}")
            if not text:
                return None
            try:
                return json.loads(text)
            except json.JSONDecodeError:
                return text


class MqttObserver:
    def __init__(self, host: str, port: int, topic: str, loop: asyncio.AbstractEventLoop):
        self.host = host
        self.port = port
        self.topic = topic
        self.loop = loop
        self.queue: asyncio.Queue[tuple[str, bytes]] = asyncio.Queue()
        self.connected = asyncio.Event()
        self.client = mqtt.Client(client_id=f"iot-gateway-e2e-subscriber-{uuid.uuid4()}")
        self.client.on_connect = self._on_connect
        self.client.on_message = self._on_message

    def _on_connect(self, client: mqtt.Client, userdata: Any, flags: Any, rc: int, properties: Any = None) -> None:
        if rc == 0:
            client.subscribe(self.topic, qos=1)
            self.loop.call_soon_threadsafe(self.connected.set)

    def _on_message(self, client: mqtt.Client, userdata: Any, msg: Any) -> None:
        self.loop.call_soon_threadsafe(self.queue.put_nowait, (msg.topic, bytes(msg.payload)))

    async def __aenter__(self) -> "MqttObserver":
        self.client.connect(self.host, self.port, keepalive=30)
        self.client.loop_start()
        await asyncio.wait_for(self.connected.wait(), timeout=10)
        print_step(f"[observe] MQTT subscribed topic={self.topic}")
        return self

    async def __aexit__(self, exc_type: Any, exc: Any, tb: Any) -> None:
        self.client.loop_stop()
        self.client.disconnect()

    async def wait_for_processed_payload(self, expected_group: str, timeout: float) -> tuple[str, bytes, str]:
        deadline = time.monotonic() + timeout
        seen = 0
        last_detail = "no MQTT messages observed"
        while time.monotonic() < deadline:
            remaining = max(0.1, deadline - time.monotonic())
            try:
                topic, payload = await asyncio.wait_for(self.queue.get(), timeout=min(1.0, remaining))
            except asyncio.TimeoutError:
                continue
            seen += 1
            ok, detail = mqtt_payload_proves_flow(payload, expected_group)
            last_detail = detail
            if ok:
                return topic, payload, detail
        raise E2EFailure(f"timed out waiting for processed MQTT payload; seen={seen}; last={last_detail}")


async def flow_ws_reader(state: RuntimeState, flow_id: str, expected_group_id: str, expected_group_name: str, queue: asyncio.Queue[dict[str, Any]]) -> None:
    url = f"{state.flow_ws_base}/{flow_id}/live"
    async with websockets.connect(url) as ws:
        print_step(f"[observe] flow websocket connected {url}")
        async for raw in ws:
            event = json.loads(raw)
            if event.get("type") == "data_processed" and event.get("group_id") == expected_group_id and event.get("group_name") == expected_group_name:
                await queue.put(event)


async def alarm_ws_reader(state: RuntimeState, expected_rule_id: str, queue: asyncio.Queue[dict[str, Any]]) -> None:
    async with websockets.connect(state.alarm_ws_url) as ws:
        print_step(f"[observe] alarm websocket connected {state.alarm_ws_url}")
        async for raw in ws:
            event = json.loads(raw)
            alarm = event.get("event") if isinstance(event, dict) else None
            if event.get("type") == "alarm.created" and isinstance(alarm, dict) and alarm.get("rule_id") == expected_rule_id:
                await queue.put(event)


async def configure_resources(rest: RestClient, state: RuntimeState) -> dict[str, Any]:
    run_id = state.run_id
    south_name = f"{run_id}_sim"
    group_name = f"{run_id}_group"
    north_name = f"{run_id}_mqtt"

    south = await rest.request("POST", "/nodes", {
        "name": south_name,
        "kind": "south",
        "plugin_name": "sim",
        "config": {"poll_base_ms": 500},
    })
    state.south_node_id = south["id"]
    print_step(f"[config] created south node {south['id']}")

    group = await rest.request("POST", f"/nodes/{south['id']}/groups", {
        "name": group_name,
        "interval_ms": 500,
        "description": "full-chain E2E group",
    })
    print_step(f"[config] created group {group['id']}")

    tags = await rest.request("POST", f"/nodes/{south['id']}/tags/batch", {
        "tags": [
            {"name": "temperature", "address": "0", "group_id": group["id"], "attr": "read", "data_type": "float64"},
            {"name": "humidity", "address": "1", "group_id": group["id"], "attr": "read", "data_type": "float64"},
        ]
    })
    print_step(f"[config] created tags {[tag['name'] for tag in tags]}")

    flow = build_flow_definition(run_id, "temperature")
    created_flow = await rest.request("POST", "/flows", flow)
    flow = created_flow["flow"]
    state.created_flow_id = flow["id"]
    rule_id = f"{run_id}_high_temperature"
    print_step(f"[config] created flow {flow['id']}")

    await rest.request("PUT", f"/flows/{flow['id']}/bindings", {
        "bindings": [
            {
                "flow_id": flow["id"],
                "south_node_id": south["id"],
                "group_id": group["id"],
                "enabled": True,
                "failure_policy": "fail_closed",
            }
        ]
    })
    print_step("[config] bound flow to south group")

    north = await rest.request("POST", "/nodes", {
        "name": north_name,
        "kind": "north",
        "plugin_name": "mqtt",
        "config": {
            "host": state.config.mqtt_host,
            "port": state.config.mqtt_port,
            "client_id": f"{run_id}_publisher",
            "topic_template": f"iot-gateway/e2e/{run_id}/${{node_name}}/${{group_name}}",
            "qos": 1,
            "retain": False,
            "upload_format": "values_format",
            "keep_alive_secs": 30,
            "cache_memory_size": 100,
            "cache_sync_interval_ms": 100,
            "ssl": False,
        },
    })
    state.north_node_id = north["id"]
    print_step(f"[config] created MQTT north node {north['id']}")

    await rest.request("PUT", f"/nodes/{north['id']}/subscriptions", {
        "subscriptions": [
            {"south_node_id": south["id"], "group_id": group["id"]}
        ]
    })
    print_step("[config] configured MQTT north subscription")

    await rest.request("POST", f"/flows/{flow['id']}/deploy")
    await rest.request("POST", f"/flows/{flow['id']}/start")
    print_step("[config] deployed and started flow")

    return {
        "south": south,
        "group": group,
        "tags": tags,
        "flow": flow,
        "north": north,
        "rule_id": rule_id,
    }


async def wait_for_alarm_rest(rest: RestClient, rule_id: str, timeout: float) -> dict[str, Any]:
    deadline = time.monotonic() + timeout
    last_count = 0
    while time.monotonic() < deadline:
        payload = await rest.request("GET", "/alarm/events")
        events = payload.get("items") or payload.get("events") or []
        last_count = len(events)
        for event in events:
            if event.get("rule_id") == rule_id:
                return event
        await asyncio.sleep(0.5)
    raise E2EFailure(f"timed out waiting for alarm REST event rule_id={rule_id}; events_seen={last_count}")


async def cleanup(rest: RestClient, state: RuntimeState) -> None:
    print_step("[cleanup] cleaning gateway resources")
    if state.south_node_id:
        try:
            await rest.request("POST", f"/nodes/{state.south_node_id}/stop", ok={200, 204, 404})
        except Exception as exc:
            print_step(f"[cleanup] south stop failed: {exc}")
    if state.north_node_id:
        try:
            await rest.request("POST", f"/nodes/{state.north_node_id}/stop", ok={200, 204, 404})
        except Exception as exc:
            print_step(f"[cleanup] north stop failed: {exc}")
    if state.created_flow_id:
        try:
            await rest.request("POST", f"/flows/{state.created_flow_id}/stop", ok={200, 204, 404})
        except Exception as exc:
            print_step(f"[cleanup] flow stop failed: {exc}")
        try:
            await rest.request("DELETE", f"/flows/{state.created_flow_id}", ok={200, 204, 404})
        except Exception as exc:
            print_step(f"[cleanup] flow delete failed: {exc}")
    if state.north_node_id:
        try:
            await rest.request("DELETE", f"/nodes/{state.north_node_id}", ok={200, 204, 404})
        except Exception as exc:
            print_step(f"[cleanup] north delete failed: {exc}")
    if state.south_node_id:
        try:
            await rest.request("DELETE", f"/nodes/{state.south_node_id}", ok={200, 204, 404})
        except Exception as exc:
            print_step(f"[cleanup] south delete failed: {exc}")


def terminate_process(process: subprocess.Popen[str] | None, label: str) -> None:
    if process is None:
        return
    if process.poll() is not None:
        return
    process.terminate()
    try:
        process.wait(timeout=5)
    except subprocess.TimeoutExpired:
        print_step(f"[cleanup] killing {label} pid={process.pid}")
        process.kill()
        process.wait(timeout=5)


async def run_full_chain(config: RuntimeConfig) -> int:
    run_id = f"e2e_full_chain_{datetime.now(timezone.utc).strftime('%Y%m%d_%H%M%S')}_{uuid.uuid4().hex[:8]}"
    temp_dir = Path(tempfile.mkdtemp(prefix=f"{run_id}_"))
    state = RuntimeState(run_id=run_id, config=config, temp_dir=temp_dir)
    flow_queue: asyncio.Queue[dict[str, Any]] = asyncio.Queue()
    alarm_queue: asyncio.Queue[dict[str, Any]] = asyncio.Queue()
    flow_task: asyncio.Task[Any] | None = None
    alarm_task: asyncio.Task[Any] | None = None

    try:
        start_or_configure_broker(state)
        start_gateway(state)
        await wait_for_health(state, timeout=20)

        async with RestClient(state) as rest:
            resources = await configure_resources(rest, state)
            flow_id = resources["flow"]["id"]
            group = resources["group"]
            rule_id = resources["rule_id"]
            mqtt_topic = f"iot-gateway/e2e/{run_id}/#"

            flow_task = asyncio.create_task(flow_ws_reader(state, flow_id, group["id"], group["name"], flow_queue))
            alarm_task = asyncio.create_task(alarm_ws_reader(state, rule_id, alarm_queue))

            async with MqttObserver(state.config.mqtt_host, state.config.mqtt_port, mqtt_topic, asyncio.get_running_loop()) as mqtt_observer:
                await rest.request("POST", f"/nodes/{resources['north']['id']}/start")
                print_step("[run] started MQTT north node")
                await rest.request("POST", f"/nodes/{resources['south']['id']}/start")
                print_step("[run] started Sim south node")

                topic, payload, detail = await mqtt_observer.wait_for_processed_payload(group["name"], config.timeout)
                print_step(f"[assert] MQTT processed payload ok topic={topic} {detail}")

                flow_event = await asyncio.wait_for(flow_queue.get(), timeout=config.timeout)
                print_step(f"[assert] flow websocket data_processed ok group={flow_event.get('group_name')}")

                try:
                    alarm_event = await asyncio.wait_for(alarm_queue.get(), timeout=5)
                    print_step(f"[assert] alarm websocket event ok rule={alarm_event['event']['rule_id']}")
                except asyncio.TimeoutError:
                    print_step("[assert] alarm websocket event not observed within 5s; checking REST persistence")

                rest_alarm = await wait_for_alarm_rest(rest, rule_id, timeout=config.timeout)
                print_step(f"[assert] alarm REST event persisted id={rest_alarm.get('id')} rule={rest_alarm.get('rule_id')}")

            await cleanup(rest, state)

        print_step("[result] full-chain E2E passed")
        return 0
    finally:
        if flow_task:
            flow_task.cancel()
        if alarm_task:
            alarm_task.cancel()
        terminate_process(state.gateway_process, "gateway")
        terminate_process(state.broker_process, "mqtt broker")
        if config.keep_artifacts:
            print_step(f"[cleanup] keeping artifacts at {state.temp_dir}")
            print_step("[cleanup] gateway log tail:\n" + "\n".join(state.gateway_log))
            if state.broker_log:
                print_step("[cleanup] broker log tail:\n" + "\n".join(state.broker_log))
        else:
            shutil.rmtree(state.temp_dir, ignore_errors=True)


def parse_args(argv: list[str] | None) -> argparse.Namespace:
    parser = argparse.ArgumentParser(description="Run the iot-gateway full-chain E2E test")
    parser.add_argument("--gateway-bin", default=None, help="path to gateway binary; defaults to GATEWAY_BIN, target/debug/gateway, target/release/gateway")
    parser.add_argument("--gateway-port", type=int, default=0, help="gateway port; 0 chooses a free port")
    parser.add_argument("--timeout", type=float, default=float(os.environ.get("E2E_TIMEOUT", DEFAULT_FLOW_TIMEOUT_SECONDS)))
    parser.add_argument("--unit-helper-check", action="store_true", help="only verify helper import path")
    return parser.parse_args(argv)


def main(argv: list[str] | None = None) -> int:
    args = parse_args(argv)
    if args.unit_helper_check:
        print_step("[setup] helper module import ok")
        return 0

    config = RuntimeConfig(
        gateway_port=args.gateway_port or free_port(),
        gateway_bin=resolve_gateway_bin(args.gateway_bin),
        keep_artifacts=os.environ.get("E2E_KEEP_ARTIFACTS") == "1",
        timeout=args.timeout,
    )

    try:
        return asyncio.run(run_full_chain(config))
    except SkipE2E as exc:
        print_step(f"[skip] {exc}")
        return 77
    except Exception as exc:
        print_step(f"[fail] {exc}")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
```

- [ ] **Step 2: Run helper unit tests after the full runner replacement**

Run:

```bash
python3 -m unittest e2e/full_chain/test_full_chain_unit.py -v
```

Expected:

```text
Ran 5 tests

OK
```

- [ ] **Step 3: Run Python syntax checks**

Run:

```bash
python3 -m py_compile e2e/full_chain/test_full_chain.py e2e/full_chain/test_full_chain_unit.py
```

Expected: command exits with status `0` and no output.

- [ ] **Step 4: Run the full-chain E2E command directly**

If dependencies are not installed, first run:

```bash
python3 -m pip install -r e2e/full_chain/requirements.txt
```

Then run:

```bash
python3 e2e/full_chain/test_full_chain.py
```

Expected with a built gateway binary and available MQTT broker or `mosquitto`:

```text
[setup] temporary MQTT broker ready ...
[setup] gateway started ...
[setup] gateway health ok ...
[config] created south node ...
[config] created group ...
[config] created tags ...
[config] created flow ...
[config] bound flow to south group
[config] created MQTT north node ...
[config] configured MQTT north subscription
[config] deployed and started flow
[observe] flow websocket connected ...
[observe] alarm websocket connected ...
[observe] MQTT subscribed topic=...
[run] started MQTT north node
[run] started Sim south node
[assert] MQTT processed payload ok ...
[assert] flow websocket data_processed ok ...
[assert] alarm REST event persisted ...
[result] full-chain E2E passed
```

Expected without MQTT broker and without `mosquitto`:

```text
[skip] MQTT broker unavailable; set E2E_MQTT_HOST/E2E_MQTT_PORT or install mosquitto
```

The skip exit code is `77`.

- [ ] **Step 5: Commit the full runner**

Run:

```bash
git add e2e/full_chain/test_full_chain.py e2e/full_chain/test_full_chain_unit.py
git commit -m "test(e2e): implement full-chain process runner" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 5: Add Shell Entry Point and Documentation

**Files:**
- Create: `scripts/e2e_full_chain.sh`
- Modify: `docs/e2e-acceptance-scenarios.md`

- [ ] **Step 1: Create the shell entry point**

Create `scripts/e2e_full_chain.sh` with exactly:

```bash
#!/usr/bin/env bash
set -euo pipefail

ROOT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
PYTHON_BIN="${PYTHON_BIN:-python3}"
REQUIREMENTS="$ROOT_DIR/e2e/full_chain/requirements.txt"
RUNNER="$ROOT_DIR/e2e/full_chain/test_full_chain.py"

if ! command -v "$PYTHON_BIN" >/dev/null 2>&1; then
  echo "[fail] python3 is required; set PYTHON_BIN=/path/to/python if needed" >&2
  exit 1
fi

if [[ ! -x "${GATEWAY_BIN:-}" ]]; then
  if [[ -x "$ROOT_DIR/target/debug/gateway" ]]; then
    export GATEWAY_BIN="$ROOT_DIR/target/debug/gateway"
  elif [[ -x "$ROOT_DIR/target/release/gateway" ]]; then
    export GATEWAY_BIN="$ROOT_DIR/target/release/gateway"
  else
    echo "[skip] gateway binary not found; run: cargo build -p gateway-server" >&2
    exit 77
  fi
fi

if ! "$PYTHON_BIN" - <<'PY' >/dev/null 2>&1
import aiohttp
import websockets
import paho.mqtt.client
PY
then
  echo "[fail] missing Python dependencies; run:" >&2
  echo "       $PYTHON_BIN -m pip install -r $REQUIREMENTS" >&2
  exit 1
fi

cd "$ROOT_DIR"
exec "$PYTHON_BIN" "$RUNNER" "$@"
```

- [ ] **Step 2: Make the shell entry point executable**

Run:

```bash
chmod +x scripts/e2e_full_chain.sh
```

- [ ] **Step 3: Verify shell syntax**

Run:

```bash
bash -n scripts/e2e_full_chain.sh
```

Expected: command exits with status `0` and no output.

- [ ] **Step 4: Add documentation for the new E2E command**

Append this section to `docs/e2e-acceptance-scenarios.md`:

```markdown

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
```

- [ ] **Step 5: Run wrapper checks**

Run:

```bash
bash -n scripts/e2e_full_chain.sh
python3 e2e/full_chain/test_full_chain.py --unit-helper-check
```

Expected:

```text
[setup] helper module import ok
```

- [ ] **Step 6: Commit the shell wrapper and docs**

Run:

```bash
git add scripts/e2e_full_chain.sh docs/e2e-acceptance-scenarios.md
git commit -m "docs(e2e): document full-chain runner" -m "Co-Authored-By: Claude Opus 4.8 <noreply@anthropic.com>"
```

---

### Task 6: Final Validation

**Files:**
- Validate only; no file edits expected.

- [ ] **Step 1: Run Python helper tests**

Run:

```bash
python3 -m unittest e2e/full_chain/test_full_chain_unit.py -v
```

Expected:

```text
Ran 5 tests

OK
```

- [ ] **Step 2: Run Python syntax checks**

Run:

```bash
python3 -m py_compile e2e/full_chain/test_full_chain.py e2e/full_chain/test_full_chain_unit.py
```

Expected: command exits with status `0` and no output.

- [ ] **Step 3: Run Rust tests for touched server behavior**

Run:

```bash
cargo test -p gateway-server websocket::tests::data_processed_event_serializes_with_source_identifiers
cargo test -p gateway-server
```

Expected: both commands pass.

- [ ] **Step 4: Build the gateway binary used by the E2E**

Run:

```bash
cargo build -p gateway-server
```

Expected: command succeeds and creates `target/debug/gateway`.

- [ ] **Step 5: Run the full-chain E2E wrapper**

If dependencies are missing, run:

```bash
python3 -m pip install -r e2e/full_chain/requirements.txt
```

Then run:

```bash
./scripts/e2e_full_chain.sh
```

Expected in a fully provisioned environment:

```text
[result] full-chain E2E passed
```

Expected in an environment with no MQTT broker and no `mosquitto`:

```text
[skip] MQTT broker unavailable; set E2E_MQTT_HOST/E2E_MQTT_PORT or install mosquitto
```

A skip exits with code `77` and is not a product failure.

- [ ] **Step 6: Run the workspace test suite**

Run:

```bash
cargo test --workspace
```

Expected: all workspace tests pass. If pre-existing unrelated failures appear, capture the failing test names and output before deciding whether they are in scope.

- [ ] **Step 7: Inspect git status**

Run:

```bash
git status --short
```

Expected: no uncommitted changes. If validation generated ignored cache files only, leave them ignored.

---

## Self-Review Against Spec

Spec coverage:

- Starts real gateway process with isolated config: Task 4 `start_gateway`, Task 6 validation.
- Uses Sim south plugin: Task 4 REST configuration.
- Uses MQTT north plugin: Task 4 REST configuration and `MqttObserver`.
- Configures nodes, group, tags, flow binding, subscription, and alarm rule through REST: Task 4 `configure_resources`.
- Observes MQTT, WebSocket, and Alarm REST: Task 4 `MqttObserver`, `flow_ws_reader`, `alarm_ws_reader`, `wait_for_alarm_rest`.
- Unique resource names: Task 4 `run_id` naming.
- Clear diagnostics and cleanup: Task 4 staged log lines, `cleanup`, process termination, artifact retention.
- One-command entry point: Task 5 `scripts/e2e_full_chain.sh`.
- MQTT broker skip behavior: Task 4 `start_or_configure_broker`, Task 5 docs.
- Minimal product patch only: Task 3 adds one event and one broadcast at the existing processor seam.

Type consistency:

- `FlowLiveEvent::DataProcessed` serializes as `data_processed`, matching the Python WebSocket reader.
- Flow binding uses `failure_policy: "fail_closed"`, which matches `FlowFailurePolicy` snake_case serialization.
- Tag attributes use `"read"`, matching `TagAttr` serialization.
- MQTT output uses `values_format`, matching `mqtt_payload_proves_flow` expectations.
- Flow uses `range` then `alarm`; `range` maps Sim `temperature` above `100`, and the alarm threshold is `100`.
