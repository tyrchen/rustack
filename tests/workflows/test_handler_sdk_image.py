"""SDK consumer ownership and private metadata projection controls."""
import json
import unittest

from handler_sdk_image import UNSET, inspect_sdk_environment
from lambda_fault_support import invoke_json


class Body:
    def __init__(self, data):
        self.data, self.closed, self.limit = data, False, None

    def read(self, limit):
        self.limit = limit
        return self.data

    def close(self):
        self.closed = True


class Client:
    def __init__(self, body):
        self.body = body

    def invoke(self, **_):
        return {"StatusCode": 200, "Payload": self.body}


class Metadata:
    def __init__(self, duplicate=False):
        self.duplicate = duplicate

    def rows(self):
        return [{"Id": role, "Labels": {"rustack.lambda.role": role}} for role in ("function", "relay")]

    def get(self, path):
        env = []
        if "/function/" in path:
            env = ["AWS_ACCESS_KEY_ID=local-key", "AWS_SECRET_ACCESS_KEY=private-sdk-secret",
                   "AWS_ENDPOINT_URL=http://192.0.2.1:4566/", "AWS_EC2_METADATA_DISABLED=true",
                   "AWS_CONFIG_FILE=/dev/null", "AWS_SHARED_CREDENTIALS_FILE=/dev/null", *UNSET]
            if self.duplicate:
                env.append("AWS_PROFILE=duplicate")
        return {"State": {"Running": True}, "Config": {"Env": env}}


class HandlerSdkControls(unittest.TestCase):
    def test_should_close_successful_bounded_invoke_body(self):
        body = Body(b'{"computed":42}')
        _, value = invoke_json(Client(body), "fixture", {"value": 21})
        self.assertEqual(value, {"computed": 42})
        self.assertTrue(body.closed)
        self.assertEqual(body.limit, 65537)

    def test_should_close_oversized_invoke_body_before_reporting_error(self):
        body = Body(b"x" * 65537)
        with self.assertRaises(AssertionError):
            invoke_json(Client(body), "fixture", {})
        self.assertTrue(body.closed)

    def test_should_close_malformed_or_duplicate_json_invoke_body(self):
        for data in (b"{", b'{"computed":42,"computed":43}'):
            body = Body(data)
            with self.assertRaises((ValueError, AssertionError)):
                invoke_json(Client(body), "fixture", {})
            self.assertTrue(body.closed)

    def test_should_project_only_nonsecret_environment_identities(self):
        value = inspect_sdk_environment(Metadata(), "http://127.0.0.1:4566", "192.0.2.1", "private-sdk-secret")
        self.assertEqual(value, {"function": "function", "relay": "relay"})
        self.assertNotIn("private-sdk-secret", json.dumps(value))

    def test_should_refuse_duplicate_environment_fields_without_projecting_secrets(self):
        with self.assertRaises(AssertionError):
            inspect_sdk_environment(Metadata(True), "http://127.0.0.1:4566", "192.0.2.1", "private-sdk-secret")


if __name__ == "__main__":
    unittest.main()
