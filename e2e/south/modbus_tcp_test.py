"""
End-to-end test for Modbus TCP south driver.
Requires gateway running on http://127.0.0.1:4001 and pymodbus installed.
"""

import json
import socket
import subprocess
import sys
import time
from typing import Any, Dict, List, Optional

import pytest
import requests

GATEWAY_URL = "http://127.0.0.1:4001"
API_URL = f"{GATEWAY_URL}/api"
MODBUS_PORT = 10502

# Expected register values for validation
EXPECTED_REGISTERS = {
    0: 100.0,   # holding register 0 -> float32
    1: 200.0,   # holding register 1 -> float32
    2: 300.0,   # holding register 2 -> float32
}


def wait_for_port(host: str, port: int, timeout: float = 10.0) -> bool:
    """Wait for a TCP port to become available."""
    start = time.time()
    while time.time() - start < timeout:
        try:
            with socket.create_connection((host, port), timeout=0.5):
                return True
        except OSError:
            time.sleep(0.2)
    return False


def get_auth_token() -> str:
    """Login and get bearer token."""
    resp = requests.post(
        f"{API_URL}/auth/login",
        json={"username": "admin", "password": "admin123"},
        timeout=5,
    )
    resp.raise_for_status()
    data = resp.json()
    token = data.get("token")
    if not token:
        raise RuntimeError(f"No token in response: {data}")
    return token


def api_request(method: str, path: str, token: str, **kwargs) -> requests.Response:
    """Make an authenticated API request."""
    headers = {"Authorization": f"Bearer {token}"}
    if "json" in kwargs:
        headers["Content-Type"] = "application/json"
    resp = requests.request(method, f"{API_URL}{path}", headers=headers, timeout=10, **kwargs)
    return resp


class ModbusSimulator:
    """Wraps pymodbus ModbusTcpServer as a context manager."""

    def __init__(self, host: str = "127.0.0.1", port: int = MODBUS_PORT):
        self.host = host
        self.port = port
        self.process: Optional[subprocess.Popen] = None

    def start(self) -> None:
        import threading
        from pymodbus.datastore import ModbusSequentialDataBlock, ModbusDeviceContext
        from pymodbus.server import ModbusTcpServer

        # Create datastore with preset holding register values (addresses 1-6)
        # Holding registers 1-6: [100], [200], [300], [400], [500], [600]
        block = ModbusSequentialDataBlock(1, [100, 200, 300, 400, 500, 600])
        device = ModbusDeviceContext(hr=block)
        self.datastore = {0: device}  # slave_id=0

        self.server = ModbusTcpServer(
            self.datastore,
            address=(self.host, self.port),
        )
        self.thread = threading.Thread(target=self._serve, daemon=True)
        self.thread.start()
        if not wait_for_port(self.host, self.port, timeout=5):
            raise RuntimeError(f"Modbus simulator failed to start on {self.host}:{self.port}")

    def _serve(self) -> None:
        import asyncio
        loop = asyncio.new_event_loop()
        asyncio.set_event_loop(loop)
        loop.run_until_complete(self.server.serve_forever())

    def stop(self) -> None:
        if hasattr(self, 'server'):
            self.server.server_close()


@pytest.fixture(scope="module")
def gateway_available():
    """Check if gateway is running."""
    try:
        resp = requests.get(f"{GATEWAY_URL}/health", timeout=3)
        return resp.status_code == 200
    except Exception:
        return False


@pytest.fixture(scope="module")
def auth_token(gateway_available):
    """Get auth token, skip if gateway unavailable."""
    if not gateway_available:
        pytest.skip("Gateway not available")
    return get_auth_token()


@pytest.fixture
def modbus_simulator(gateway_available):
    """Start pymodbus ModbusTcpServer as simulation slave."""
    if not gateway_available:
        pytest.skip("Gateway not available")

    sim = ModbusSimulator()
    sim.start()
    yield sim
    sim.stop()


@pytest.fixture
def modbus_node(auth_token, modbus_simulator) -> Dict[str, Any]:
    """
    Create a south Modbus TCP node.
    Returns the node dict with 'id' and 'group_id'.
    Cleans up on teardown.
    """

    node_payload = {
        "name": "test-modbus-tcp-e2e",
        "kind": "south",
        "plugin_name": "modbus-tcp",
        "config": {
            "host": "127.0.0.1",
            "port": MODBUS_PORT,
            "slave_id": 1,
            "connection_timeout_ms": 3000,
            "send_interval_ms": 50,
            "max_retry_times": 2,
            "retry_interval_ms": 100,
            "start_address": 1,
            "endianess": 1,
        },
    }

    # Create node
    resp = api_request("POST", "/nodes", auth_token, json=node_payload)
    resp.raise_for_status()
    node = resp.json()
    node_id = node["id"]

    # Get the default group ID (created by plugin)
    resp = api_request("GET", f"/nodes/{node_id}/groups", auth_token)
    resp.raise_for_status()
    groups = resp.json()
    if not groups:
        raise RuntimeError(f"No groups found for node {node_id}: {groups}")
    group_id = groups[0]["id"]

    node["group_id"] = group_id

    yield node

    # Teardown: stop and delete node
    try:
        api_request("POST", f"/nodes/{node_id}/stop", auth_token)
    except Exception:
        pass
    try:
        api_request("DELETE", f"/nodes/{node_id}", auth_token)
    except Exception:
        pass


@pytest.fixture
def modbus_tags(auth_token, modbus_node) -> List[Dict[str, Any]]:
    """
    Add polled tags (holding registers 0-2 as float32).
    Returns list of created tags.
    """
    tags_payload = [
        {"name": "holding_0", "address": "4x!0", "dataType": "float32", "group_id": modbus_node["group_id"]},
        {"name": "holding_1", "address": "4x!1", "dataType": "float32", "group_id": modbus_node["group_id"]},
        {"name": "holding_2", "address": "4x!2", "dataType": "float32", "group_id": modbus_node["group_id"]},
    ]

    created = []
    for tag_req in tags_payload:
        resp = api_request("POST", f"/nodes/{modbus_node['id']}/tags", auth_token, json=tag_req)
        resp.raise_for_status()
        created.append(resp.json())

    return created


def test_gateway_reachable(gateway_available):
    """Sanity check: gateway must be running."""
    assert gateway_available, "Gateway not reachable at http://127.0.0.1:4001"


def test_create_modbus_node(auth_token, modbus_node, modbus_simulator):
    """Node creation includes modbus-tcp driver."""
    assert modbus_node["plugin_name"] == "modbus-tcp"
    assert modbus_node["kind"] == "south"


def test_start_node_and_poll(auth_token, modbus_node, modbus_tags, modbus_simulator):
    """Start node, wait for polls, verify data matches simulator values."""
    node_id = modbus_node["id"]
    group_id = modbus_node["group_id"]

    # Start the node
    resp = api_request("POST", f"/nodes/{node_id}/start", auth_token)
    resp.raise_for_status()

    # Wait for at least 2 poll cycles (default group interval is 1000ms)
    time.sleep(2.5)

    # Read values
    resp = api_request("GET", f"/nodes/{node_id}/read_tags", auth_token, json={
        "tag_ids": [tag["id"] for tag in modbus_tags]
    })
    resp.raise_for_status()
    values_payload = resp.json()

    # Response is a list of [tag_id, value] tuples
    assert isinstance(values_payload, list), f"Expected list, got: {values_payload}"
    assert len(values_payload) == 3, f"Expected 3 tag values, got: {values_payload}"

    # Verify each value matches expected registers
    for tag_id, value in values_payload:
        # Find corresponding expected value by index
        idx = next((i for i, t in enumerate(modbus_tags) if t["id"] == tag_id), None)
        if idx is None:
            continue
        expected = EXPECTED_REGISTERS[idx]
        # Float32 tolerance
        actual = float(value)
        assert abs(actual - expected) < 0.01, \
            f"Tag {tag_id} value mismatch: expected ~{expected}, got {actual}"

    # Stop node
    resp = api_request("POST", f"/nodes/{node_id}/stop", auth_token)
    resp.raise_for_status()