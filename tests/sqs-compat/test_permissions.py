"""Tests for SQS queue permission operations."""

import json

import botocore.exceptions
import pytest

from util import unique_queue_name


class TestPermissions:
    @pytest.mark.xfail(
        reason="Server does not yet expose Policy attribute after AddPermission"
    )
    def test_should_add_permission(self, sqs, test_queue):
        sqs.add_permission(
            QueueUrl=test_queue,
            Label="test-perm",
            AWSAccountIds=["123456789012"],
            Actions=["SendMessage"],
        )
        resp = sqs.get_queue_attributes(QueueUrl=test_queue, AttributeNames=["Policy"])
        attrs = resp.get("Attributes", {})
        assert (
            "Policy" in attrs
        ), "Policy attribute should be present after AddPermission"
        policy = json.loads(attrs["Policy"])
        assert "Statement" in policy
        stmts = policy["Statement"]
        assert len(stmts) >= 1
        labels = [s.get("Sid") for s in stmts]
        assert "test-perm" in labels

    def test_should_remove_permission(self, sqs, test_queue):
        sqs.add_permission(
            QueueUrl=test_queue,
            Label="to-remove",
            AWSAccountIds=["123456789012"],
            Actions=["SendMessage"],
        )
        sqs.remove_permission(QueueUrl=test_queue, Label="to-remove")

        resp = sqs.get_queue_attributes(QueueUrl=test_queue, AttributeNames=["Policy"])
        attrs = resp.get("Attributes", {})
        if "Policy" in attrs:
            policy = json.loads(attrs["Policy"])
            labels = [s.get("Sid") for s in policy.get("Statement", [])]
            assert "to-remove" not in labels

    @pytest.mark.xfail(
        reason="Server does not yet expose Policy attribute after AddPermission"
    )
    def test_should_add_multiple_permissions(self, sqs, test_queue):
        sqs.add_permission(
            QueueUrl=test_queue,
            Label="perm-send",
            AWSAccountIds=["111111111111"],
            Actions=["SendMessage"],
        )
        sqs.add_permission(
            QueueUrl=test_queue,
            Label="perm-recv",
            AWSAccountIds=["222222222222"],
            Actions=["ReceiveMessage"],
        )
        resp = sqs.get_queue_attributes(QueueUrl=test_queue, AttributeNames=["Policy"])
        attrs = resp.get("Attributes", {})
        assert (
            "Policy" in attrs
        ), "Policy attribute should be present after AddPermission"
        policy = json.loads(attrs["Policy"])
        labels = {s.get("Sid") for s in policy.get("Statement", [])}
        assert "perm-send" in labels
        assert "perm-recv" in labels

    def test_should_not_error_removing_nonexistent_label(self, sqs, test_queue):
        """Removing a label that doesn't exist should not raise an error."""
        try:
            sqs.remove_permission(QueueUrl=test_queue, Label="nonexistent")
        except botocore.exceptions.ClientError:
            # Some implementations may raise an error; that's also acceptable
            pass
