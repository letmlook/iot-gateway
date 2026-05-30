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
import tempfile
import threading
import time
import uuid
from collections import deque
from dataclasses import dataclass, field
from datetime import datetime, timezone
from pathlib import Path
from typing import Any

try:
    import aiohttp
except ModuleNotFoundError:
    aiohttp = None  # type: ignore[assignment]

try:
    import paho.mqtt.client as mqtt
except ModuleNotFoundError:
    mqtt = None  # type: ignore[assignment]

try:
    import websockets
except ModuleNotFoundError:
    websockets = None  # type: ignore[assignment]


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


def missing_python_dependencies() -> list[str]:
    missing = []
    if aiohttp is None:
        missing.append("aiohttp")
    if mqtt is None:
        missing.append("paho-mqtt")
    if websockets is None:
        missing.append("websockets")
    return missing


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
    assert aiohttp is not None
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
        self.session: Any | None = None

    async def __aenter__(self) -> "RestClient":
        assert aiohttp is not None
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
        assert mqtt is not None
        self.host = host
        self.port = port
        self.topic = topic
        self.loop = loop
        self.queue: asyncio.Queue[tuple[str, bytes]] = asyncio.Queue()
        self.connected = asyncio.Event()
        self.client = mqtt.Client(client_id=f"iot-gateway-e2e-subscriber-{uuid.uuid4()}")
        self.client.on_connect = self._on_connect
        self.client.on_message = self._on_message

    def _on_connect(self, client: Any, userdata: Any, flags: Any, rc: int, properties: Any = None) -> None:
        if rc == 0:
            client.subscribe(self.topic, qos=1)
            self.loop.call_soon_threadsafe(self.connected.set)

    def _on_message(self, client: Any, userdata: Any, msg: Any) -> None:
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
    assert websockets is not None
    url = f"{state.flow_ws_base}/{flow_id}/live"
    async with websockets.connect(url) as ws:
        print_step(f"[observe] flow websocket connected {url}")
        async for raw in ws:
            event = json.loads(raw)
            if event.get("type") == "data_processed" and event.get("group_id") == expected_group_id and event.get("group_name") == expected_group_name:
                await queue.put(event)


async def alarm_ws_reader(state: RuntimeState, expected_rule_id: str, queue: asyncio.Queue[dict[str, Any]]) -> None:
    assert websockets is not None
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

    try:
        missing = missing_python_dependencies()
        if missing:
            raise SkipE2E("missing Python dependencies: " + ", ".join(missing) + "; run `python3 -m pip install -r e2e/full_chain/requirements.txt`")
        config = RuntimeConfig(
            gateway_port=args.gateway_port or free_port(),
            gateway_bin=resolve_gateway_bin(args.gateway_bin),
            keep_artifacts=os.environ.get("E2E_KEEP_ARTIFACTS") == "1",
            timeout=args.timeout,
        )
        return asyncio.run(run_full_chain(config))
    except SkipE2E as exc:
        print_step(f"[skip] {exc}")
        return 77
    except Exception as exc:
        print_step(f"[fail] {exc}")
        return 1


if __name__ == "__main__":
    raise SystemExit(main())
