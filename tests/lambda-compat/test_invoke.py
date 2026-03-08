"""Tests for Lambda invoke operations."""

import json

from util import unique_name, make_zip


class TestInvokeDryRun:
    def test_should_dry_run_invoke(self, test_function, lamb):
        resp = lamb.invoke(
            FunctionName=test_function,
            InvocationType="DryRun",
            Payload=b"{}",
        )
        assert resp["StatusCode"] == 204
