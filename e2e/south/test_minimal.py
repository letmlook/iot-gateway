#!/usr/bin/env python3
"""Minimal asyncio-based Modbus TCP server - no external dependencies."""
import asyncio

MODBUS_PORT = 10502

MB_FUNCTION_READ_HOLDING = 0x03

class ModbusSession(asyncio.Protocol):
    """One session per TCP connection."""

    def __init__(self):
        # holding registers 0-5 with raw integer values
        # These get interpreted as float32 by the modbus-tcp plugin
        self.hr = {i: 100 * (i + 1) for i in range(6)}
        self.transport = None

    def connection_made(self, transport):
        self.transport = transport

    def data_received(self, data):
        if len(data) < 12:
            return

        tid = data[0] << 8 | data[1]
        proto = data[2] << 8 | data[3]
        length = data[4] << 8 | data[5]
        unit = data[6]
        func = data[7]

        if func == MB_FUNCTION_READ_HOLDING:
            start = data[8] << 8 | data[9]
            count = data[10] << 8 | data[11]
            values = [self.hr.get(start + i, 0) for i in range(count)]
            byte_count = count * 2
            resp = bytearray([
                (tid >> 8) & 0xFF, tid & 0xFF,
                (proto >> 8) & 0xFF, proto & 0xFF,
                0, 3 + byte_count,
                unit, func, byte_count,
            ])
            for v in values:
                resp.append((v >> 8) & 0xFF)
                resp.append(v & 0xFF)
            self.transport.write(bytes(resp))
        else:
            # Exception response for unsupported functions
            resp = bytearray([data[0], data[1], 0, 0, 0, 3, unit, func | 0x80, 0x01])
            self.transport.write(bytes(resp))

    def connection_lost(self, exc):
        pass


class ModbusSimulator:
    """Minimal asyncio Modbus TCP server as context manager."""

    def __init__(self, host: str = "127.0.0.1", port: int = MODBUS_PORT):
        self.host = host
        self.port = port
        self.server = None
        self.thread = None

    def start(self) -> None:
        import threading
        import asyncio

        async def serve():
            loop = asyncio.get_running_loop()
            self.server = await loop.create_server(lambda: ModbusSession(), self.host, self.port)
            print(f"Modbus TCP server listening on {self.host}:{self.port}")

        def run():
            asyncio.run(serve())

        self.thread = threading.Thread(target=run, daemon=True)
        self.thread.start()
        if not wait_for_port(self.host, self.port, timeout=5):
            raise RuntimeError(f"Modbus simulator failed to start on {self.host}:{self.port}")

    def stop(self) -> None:
        if self.server:
            self.server.close()


def wait_for_port(host: str, port: int, timeout: float = 10.0) -> bool:
    """Wait for a TCP port to become available."""
    import time
    import socket
    start = time.time()
    while time.time() - start < timeout:
        try:
            with socket.create_connection((host, port), timeout=0.5):
                return True
        except OSError:
            time.sleep(0.2)
    return False


GATEWAY_URL = "http://127.0.0.1:4001"
API_URL = f"{GATEWAY_URL}/api"


# ---- Pytest fixtures ----

import pytest
import requests

def get_auth_token() -> str:
    resp = requests.post(f"{API_URL}/auth/login", json={"username": "admin", "password": "admin123"}, timeout=5)
    resp.raise_for_status()
    token = resp.json().get("token")
    if not token:
        raise RuntimeError(f"No token in response: {resp.json()}")
    return token


def api_request(method: str, path: str, token: str, **kwargs) -> requests.Response:
    headers = {"Authorization": f"Bearer {token}"}
    if "json" in kwargs:
        headers["Content-Type"] = "application/json"
    kwargs.setdefault("timeout", 30)
    return requests.request(method, f"{API_URL}{path}", headers=headers, timeout=kwargs.pop("timeout"), **kwargs)


@pytest.fixture(scope="module")
def gateway_available():
    try:
        resp = requests.get(f"{GATEWAY_URL}/health", timeout=3)
        return resp.status_code == 200
    except Exception:
        return False


@pytest.fixture(scope="module")
def auth_token(gateway_available):
    if not gateway_available:
        pytest.skip("Gateway not available")
    return get_auth_token()


@pytest.fixture
def modbus_simulator(gateway_available):
    if not gateway_available:
        pytest.skip("Gateway not available")
    sim = ModbusSimulator()
    sim.start()
    yield sim
    sim.stop()


@pytest.fixture
def modbus_node(auth_token, modbus_simulator):
    node_payload = {
        "name": "test-modbus-tcp-e2e",
        "kind": "south",
        "plugin_name": "modbus-tcp",
        "config": {
            "host": "127.0.0.1",
            "port": MODBUS_PORT,
            "connection_mode": 1,
            "check_header": 1,
            "device_degrade": 1,
            "degrade_cycle": 60,
            "degrade_time": 300,
            "max_retry_times": 3,
            "retry_interval_ms": 100,
            "send_interval_ms": 1000,
            "connection_timeout_ms": 3000,
            "start_address": 1,
            "endianess": 1,
        },
    }
    resp = api_request("POST", "/nodes", auth_token, json=node_payload)
    resp.raise_for_status()
    node = resp.json()
    node_id = node["id"]

    # South nodes have a built-in default group when started.
    # We need to start the node to create the default group.
    api_request("POST", f"/nodes/{node_id}/start", auth_token)
    import time; time.sleep(1)
    resp = api_request("GET", f"/nodes/{node_id}/groups", auth_token)
    resp.raise_for_status()
    groups = resp.json()
    if not groups:
        api_request("POST", f"/nodes/{node_id}/stop", auth_token)
        raise RuntimeError(f"No groups found for node {node_id}")
    group_id = groups[0]["id"]
    node["group_id"] = group_id

    yield node

    try:
        api_request("POST", f"/nodes/{node_id}/stop", auth_token)
    except Exception:
        pass
    try:
        api_request("DELETE", f"/nodes/{node_id}", auth_token)
    except Exception:
        pass


@pytest.fixture
def modbus_tags(auth_token, modbus_node):
    group_id = modbus_node["group_id"]
    # Clean up any stale tags from previous crashed runs
    resp = api_request("GET", f"/nodes/{modbus_node['id']}/tags", auth_token)
    if resp.ok:
        for tag in resp.json():
            api_request("DELETE", f"/nodes/{modbus_node['id']}/tags/{tag['id']}", auth_token)
    tags_payload = [
        {"name": "holding_0", "address": "4x!0", "data_type": "float32", "group_id": group_id},
        {"name": "holding_1", "address": "4x!1", "data_type": "float32", "group_id": group_id},
        {"name": "holding_2", "address": "4x!2", "data_type": "float32", "group_id": group_id},
    ]
    created = []
    for tag_req in tags_payload:
        resp = api_request("POST", f"/nodes/{modbus_node['id']}/tags", auth_token, json=tag_req)
        if not resp.ok:
            raise RuntimeError(f"Tag creation failed: {resp.status_code} {resp.text}")
        resp.raise_for_status()
        created.append(resp.json())
    return created


# ---- Tests ----

def test_gateway_reachable(gateway_available):
    assert gateway_available, "Gateway not reachable at http://127.0.0.1:4001"


def test_create_modbus_node(auth_token, modbus_node):
    assert modbus_node["plugin_name"] == "modbus-tcp"
    assert modbus_node["kind"] == "south"


def test_start_node_and_poll(auth_token, modbus_node, modbus_tags, modbus_simulator):
    # modbus_simulator MUST come before modbus_node in params so it starts first
    # (pytest resolves fixtures in order of appearance in the function signature)
    node_id = modbus_node["id"]

    resp = api_request("POST", f"/nodes/{node_id}/start", auth_token)
    resp.raise_for_status()

    import time
    time.sleep(2.5)

    resp = api_request("POST", f"/nodes/{node_id}/read_tags", auth_token, json={
        "tag_ids": [tag["id"] for tag in modbus_tags]
    })
    resp.raise_for_status()
    values_payload = resp.json()

    assert isinstance(values_payload, list), f"Expected list, got: {values_payload}"
    assert len(values_payload) == 3, f"Expected 3 tag values, got: {values_payload}"

    # Values from our simulator: registers 0,1,2 = 100, 200, 300
    for tag_id, value in values_payload:
        idx = next((i for i, t in enumerate(modbus_tags) if t["id"] == tag_id), None)
        if idx is None:
            continue
        expected = [100.0, 200.0, 300.0][idx]
        actual = float(value)
        assert abs(actual - expected) < 0.01, \
            f"Tag {tag_id} value mismatch: expected ~{expected}, got {actual}"

    resp = api_request("POST", f"/nodes/{node_id}/stop", auth_token)
    resp.raise_for_status()