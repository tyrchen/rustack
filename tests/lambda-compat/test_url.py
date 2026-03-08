"""Tests for Lambda function URL configuration operations."""

import botocore.exceptions
import pytest


class TestCreateFunctionUrlConfig:
    def test_should_create_url_config(self, test_function, lamb):
        resp = lamb.create_function_url_config(
            FunctionName=test_function, AuthType="NONE"
        )
        assert "FunctionUrl" in resp
        assert resp["AuthType"] == "NONE"
        assert "CreationTime" in resp

    def test_should_reject_duplicate_url_config(self, test_function, lamb):
        lamb.create_function_url_config(FunctionName=test_function, AuthType="NONE")
        with pytest.raises(botocore.exceptions.ClientError) as exc:
            lamb.create_function_url_config(FunctionName=test_function, AuthType="NONE")
        assert exc.value.response["Error"]["Code"] == "ResourceConflictException"


class TestGetFunctionUrlConfig:
    def test_should_get_url_config(self, test_function, lamb):
        created = lamb.create_function_url_config(
            FunctionName=test_function, AuthType="NONE"
        )
        resp = lamb.get_function_url_config(FunctionName=test_function)
        assert resp["FunctionUrl"] == created["FunctionUrl"]
        assert resp["AuthType"] == "NONE"

    def test_should_error_when_no_url_config(self, test_function, lamb):
        with pytest.raises(botocore.exceptions.ClientError) as exc:
            lamb.get_function_url_config(FunctionName=test_function)
        assert exc.value.response["Error"]["Code"] == "ResourceNotFoundException"


class TestUpdateFunctionUrlConfig:
    def test_should_update_auth_type(self, test_function, lamb):
        lamb.create_function_url_config(FunctionName=test_function, AuthType="NONE")
        resp = lamb.update_function_url_config(
            FunctionName=test_function, AuthType="AWS_IAM"
        )
        assert resp["AuthType"] == "AWS_IAM"


class TestDeleteFunctionUrlConfig:
    def test_should_delete_url_config(self, test_function, lamb):
        lamb.create_function_url_config(FunctionName=test_function, AuthType="NONE")
        lamb.delete_function_url_config(FunctionName=test_function)
        with pytest.raises(botocore.exceptions.ClientError) as exc:
            lamb.get_function_url_config(FunctionName=test_function)
        assert exc.value.response["Error"]["Code"] == "ResourceNotFoundException"
