#!/usr/bin/env python3
import json
import socket
import sys
import unittest
from pathlib import Path

sys.path.insert(0, str(Path(__file__).resolve().parent))

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
