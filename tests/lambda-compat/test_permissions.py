"""Tests for Lambda permission (resource policy) operations."""

import json

import botocore.exceptions
import pytest


class TestAddPermission:
    def test_should_add_permission(self, test_function, lamb):
        resp = lamb.add_permission(
            FunctionName=test_function,
            StatementId="s3-invoke",
            Action="lambda:InvokeFunction",
            Principal="s3.amazonaws.com",
        )
        assert "Statement" in resp
        stmt = json.loads(resp["Statement"])
        assert stmt["Sid"] == "s3-invoke"

    def test_should_reject_duplicate_statement_id(self, test_function, lamb):
        lamb.add_permission(
            FunctionName=test_function,
            StatementId="dup-sid",
            Action="lambda:InvokeFunction",
            Principal="s3.amazonaws.com",
        )
        with pytest.raises(botocore.exceptions.ClientError) as exc:
            lamb.add_permission(
                FunctionName=test_function,
                StatementId="dup-sid",
                Action="lambda:InvokeFunction",
                Principal="sns.amazonaws.com",
            )
        assert exc.value.response["Error"]["Code"] == "ResourceConflictException"


class TestGetPolicy:
    def test_should_get_policy(self, test_function, lamb):
        lamb.add_permission(
            FunctionName=test_function,
            StatementId="test-sid",
            Action="lambda:InvokeFunction",
            Principal="events.amazonaws.com",
        )
        resp = lamb.get_policy(FunctionName=test_function)
        assert "Policy" in resp
        policy = json.loads(resp["Policy"])
        assert "Statement" in policy
        sids = [s["Sid"] for s in policy["Statement"]]
        assert "test-sid" in sids

    def test_should_error_when_no_policy(self, test_function, lamb):
        with pytest.raises(botocore.exceptions.ClientError) as exc:
            lamb.get_policy(FunctionName=test_function)
        assert exc.value.response["Error"]["Code"] == "ResourceNotFoundException"


class TestRemovePermission:
    def test_should_remove_permission(self, test_function, lamb):
        lamb.add_permission(
            FunctionName=test_function,
            StatementId="to-remove",
            Action="lambda:InvokeFunction",
            Principal="s3.amazonaws.com",
        )
        lamb.remove_permission(FunctionName=test_function, StatementId="to-remove")
        # Policy should be empty now.
        with pytest.raises(botocore.exceptions.ClientError) as exc:
            lamb.get_policy(FunctionName=test_function)
        assert exc.value.response["Error"]["Code"] == "ResourceNotFoundException"

    def test_should_error_removing_nonexistent_statement(self, test_function, lamb):
        with pytest.raises(botocore.exceptions.ClientError) as exc:
            lamb.remove_permission(
                FunctionName=test_function, StatementId="nonexistent"
            )
        assert exc.value.response["Error"]["Code"] == "ResourceNotFoundException"
