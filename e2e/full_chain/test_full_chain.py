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
