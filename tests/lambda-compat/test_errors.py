"""Tests for Lambda error handling and validation.

Note: Some boundary values (e.g. Timeout=0, MemorySize=64, FunctionName="")
are rejected by boto3 client-side validation (ParamValidationError) before
the request reaches the server. We test those with `Exception` to cover both
client-side and server-side rejection. Tests for values just beyond the valid
range that pass client validation test server-side enforcement.
"""

import botocore.exceptions
import pytest

from util import unique_name, make_zip


class TestValidation:
    def test_should_reject_empty_function_name(self, lamb):
        """boto3 rejects empty FunctionName client-side."""
        with pytest.raises(Exception):
            lamb.create_function(
                FunctionName="",
                Runtime="python3.12",
                Role="arn:aws:iam::000000000000:role/test-role",
                Handler="index.handler",
                Code={"ZipFile": make_zip()},
            )

    def test_should_reject_too_long_function_name(self, lamb):
        with pytest.raises(botocore.exceptions.ClientError):
            lamb.create_function(
                FunctionName="a" * 141,
                Runtime="python3.12",
                Role="arn:aws:iam::000000000000:role/test-role",
                Handler="index.handler",
                Code={"ZipFile": make_zip()},
            )

    def test_should_reject_too_long_handler(self, lamb):
        with pytest.raises(botocore.exceptions.ClientError):
            lamb.create_function(
                FunctionName=unique_name("bad-handler"),
                Runtime="python3.12",
                Role="arn:aws:iam::000000000000:role/test-role",
                Handler="a" * 129,
                Code={"ZipFile": make_zip()},
            )

    def test_should_reject_too_long_description(self, lamb):
        with pytest.raises(botocore.exceptions.ClientError):
            lamb.create_function(
                FunctionName=unique_name("bad-desc"),
                Runtime="python3.12",
                Role="arn:aws:iam::000000000000:role/test-role",
                Handler="index.handler",
                Code={"ZipFile": make_zip()},
                Description="x" * 257,
            )

    def test_should_reject_timeout_too_large(self, lamb):
        """Server-side: timeout > 900 passes boto3 validation."""
        with pytest.raises(botocore.exceptions.ClientError):
            lamb.create_function(
                FunctionName=unique_name("bad-timeout-big"),
                Runtime="python3.12",
                Role="arn:aws:iam::000000000000:role/test-role",
                Handler="index.handler",
                Code={"ZipFile": make_zip()},
                Timeout=901,
            )

    def test_should_reject_memory_too_large(self, lamb):
        """Server-side: memory > 10240 passes boto3 validation."""
        with pytest.raises(botocore.exceptions.ClientError):
            lamb.create_function(
                FunctionName=unique_name("bad-mem-big"),
                Runtime="python3.12",
                Role="arn:aws:iam::000000000000:role/test-role",
                Handler="index.handler",
                Code={"ZipFile": make_zip()},
                MemorySize=10241,
            )

    def test_should_accept_boundary_timeout(self, lamb):
        """Timeout=1 and Timeout=900 should both be accepted."""
        name = unique_name("timeout-min")
        lamb.create_function(
            FunctionName=name,
            Runtime="python3.12",
            Role="arn:aws:iam::000000000000:role/test-role",
            Handler="index.handler",
            Code={"ZipFile": make_zip()},
            Timeout=1,
        )
        resp = lamb.get_function_configuration(FunctionName=name)
        assert resp["Timeout"] == 1
        lamb.delete_function(FunctionName=name)

    def test_should_accept_max_timeout(self, lamb):
        name = unique_name("timeout-max")
        lamb.create_function(
            FunctionName=name,
            Runtime="python3.12",
            Role="arn:aws:iam::000000000000:role/test-role",
            Handler="index.handler",
            Code={"ZipFile": make_zip()},
            Timeout=900,
        )
        resp = lamb.get_function_configuration(FunctionName=name)
        assert resp["Timeout"] == 900
        lamb.delete_function(FunctionName=name)

    def test_should_accept_max_memory(self, lamb):
        name = unique_name("mem-max")
        lamb.create_function(
            FunctionName=name,
            Runtime="python3.12",
            Role="arn:aws:iam::000000000000:role/test-role",
            Handler="index.handler",
            Code={"ZipFile": make_zip()},
            MemorySize=10240,
        )
        resp = lamb.get_function_configuration(FunctionName=name)
        assert resp["MemorySize"] == 10240
        lamb.delete_function(FunctionName=name)


class TestUpdateValidation:
    def test_should_reject_timeout_too_large_on_update(self, test_function, lamb):
        with pytest.raises(botocore.exceptions.ClientError):
            lamb.update_function_configuration(FunctionName=test_function, Timeout=901)

    def test_should_reject_memory_too_large_on_update(self, test_function, lamb):
        with pytest.raises(botocore.exceptions.ClientError):
            lamb.update_function_configuration(
                FunctionName=test_function, MemorySize=10241
            )

    def test_should_reject_too_long_handler_on_update(self, test_function, lamb):
        with pytest.raises(botocore.exceptions.ClientError):
            lamb.update_function_configuration(
                FunctionName=test_function, Handler="a" * 129
            )

    def test_should_reject_too_long_description_on_update(self, test_function, lamb):
        with pytest.raises(botocore.exceptions.ClientError):
            lamb.update_function_configuration(
                FunctionName=test_function, Description="x" * 257
            )


class TestInvokeErrors:
    def test_should_error_invoking_nonexistent_function(self, lamb):
        with pytest.raises(botocore.exceptions.ClientError) as exc:
            lamb.invoke(FunctionName="nonexistent-func-12345")
        assert exc.value.response["Error"]["Code"] == "ResourceNotFoundException"
