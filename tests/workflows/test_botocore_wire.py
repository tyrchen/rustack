"""Offline SDK wire evidence; these tests do not execute an ASL interpreter."""

import json
from datetime import datetime
from pathlib import Path
import unittest

import boto3
import botocore
import botocore.parsers
import botocore.serialize
import botocore.session


class StepFunctionsWireEvidence(unittest.TestCase):
    """Check independently implemented SDK serialization and parsing."""

    @classmethod
    def setUpClass(cls):
        root = Path(__file__).parent
        cls.provenance = json.loads((root / "provenance.json").read_text())
        cls.fixtures = json.loads((root / "semantic-cases.json").read_text())
        cls.service = botocore.session.get_session().get_service_model("stepfunctions")

    def test_should_match_pinned_sdk_versions(self):
        self.assertEqual(boto3.__version__, self.provenance["boto3"])
        self.assertEqual(botocore.__version__, self.provenance["botocore"])

    def test_should_preserve_json_string_input_and_target(self):
        fixture = next(c for c in self.fixtures["cases"] if c["id"] == "wire-input-string")
        serializer = botocore.serialize.create_serializer(self.service.metadata["protocol"])
        request = serializer.serialize_to_request(
            fixture["request"], self.service.operation_model(fixture["operation"])
        )
        self.assertEqual(request["headers"]["X-Amz-Target"], fixture["expectedTarget"])
        self.assertEqual(json.loads(request["body"]), fixture["request"])

    def test_should_parse_fractional_and_negative_epoch_seconds(self):
        parser = botocore.parsers.ResponseParserFactory().create_parser("json")
        output = self.service.operation_model("CreateStateMachine").output_shape
        for fixture in self.fixtures["cases"]:
            if fixture["kind"] != "wireTimestamp":
                continue
            with self.subTest(case=fixture["id"]):
                value = fixture["epochSeconds"]
                body = json.dumps({
                    "creationDate": value,
                    "stateMachineArn": "arn:aws:states:us-east-1:000000000000:stateMachine:P0",
                }).encode()
                parsed = parser.parse({"body": body, "headers": {}, "status_code": 200}, output)
                self.assertIsInstance(parsed["creationDate"], datetime)
                self.assertEqual(parsed["creationDate"].timestamp(), value)

    def test_should_parse_callback_error_codes_without_network(self):
        parser = botocore.parsers.ResponseParserFactory().create_parser("json")
        output = self.service.operation_model("SendTaskSuccess").output_shape
        for code in {"TaskTimedOut", "InvalidToken"}:
            with self.subTest(code=code):
                body = json.dumps({"__type": code, "message": "fixture"}).encode()
                parsed = parser.parse({"body": body, "headers": {}, "status_code": 400}, output)
                self.assertEqual(parsed["Error"]["Code"], code)


if __name__ == "__main__":
    unittest.main()
