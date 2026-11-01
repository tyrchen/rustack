"""No-network controls for replay resistance in the actual SDK consumer."""
import json
import tempfile
import unittest
import uuid

from pulumi_image_sdk import Driver


class ReplayedStates:
    def __init__(self, flavor):
        self.flavor, self.calls, self.request_id = flavor, 0, str(uuid.uuid4())

    def start_execution(self, **_arguments):
        self.calls += 1
        ordinal = 1 if self.flavor == "arn" else self.calls
        return {"executionArn": self.driver.target_arn.replace(":stateMachine:", ":execution:") + ":Replay" + str(ordinal)}

    def describe_execution(self, **_arguments):
        counter = 2 if self.flavor == "counter" else self.calls + 1
        request = self.request_id if self.flavor == "request" else str(uuid.uuid4())
        return {"status": "SUCCEEDED", "output": json.dumps({"computed": 46, "counter": counter,
            "requestId": request, "nodeVersion": "24.22.1", "architecture": "arm64"})}


class ReplayControls(unittest.TestCase):
    def test_should_refuse_old_execution_uuid_or_counter_as_a_later_valid_46(self):
        for flavor in ("arn", "request", "counter"):
            with self.subTest(flavor=flavor), tempfile.TemporaryDirectory() as root:
                states = ReplayedStates(flavor)
                driver = Driver(None, states, None, None, "arm64", "lifecycle", root)
                states.driver = driver
                driver.business_counter = 1
                driver.business_requests.append(str(uuid.uuid4()))
                driver.execute_target(46)
                with self.assertRaises(ValueError):
                    driver.execute_target(46)
