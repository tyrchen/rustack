"""Tests for SQS queue attribute operations."""

import botocore.exceptions
import pytest

from util import unique_queue_name


class TestGetQueueAttributes:
    def test_should_return_all_attributes(self, sqs, test_queue):
        attrs = sqs.get_queue_attributes(QueueUrl=test_queue, AttributeNames=["All"])[
            "Attributes"
        ]
        # Standard attributes that must be present
        assert "QueueArn" in attrs
        assert "CreatedTimestamp" in attrs
        assert "LastModifiedTimestamp" in attrs
        assert "VisibilityTimeout" in attrs
        assert "DelaySeconds" in attrs
        assert "MaximumMessageSize" in attrs
        assert "MessageRetentionPeriod" in attrs
        assert "ReceiveMessageWaitTimeSeconds" in attrs
        assert "ApproximateNumberOfMessages" in attrs
        assert "ApproximateNumberOfMessagesNotVisible" in attrs
        assert "ApproximateNumberOfMessagesDelayed" in attrs

    def test_should_return_specific_attributes(self, sqs, test_queue):
        attrs = sqs.get_queue_attributes(
            QueueUrl=test_queue,
            AttributeNames=["VisibilityTimeout", "DelaySeconds"],
        )["Attributes"]
        assert "VisibilityTimeout" in attrs
        assert "DelaySeconds" in attrs
        # Should not return unrequested attrs
        assert "QueueArn" not in attrs

    def test_should_return_queue_arn(self, sqs, test_queue):
        attrs = sqs.get_queue_attributes(
            QueueUrl=test_queue, AttributeNames=["QueueArn"]
        )["Attributes"]
        arn = attrs["QueueArn"]
        assert arn.startswith("arn:aws:sqs:")
        # ARN should contain the queue name
        name = test_queue.rstrip("/").split("/")[-1]
        assert name in arn

    def test_should_return_default_attribute_values(self, sqs):
        name = unique_queue_name("defaults")
        url = sqs.create_queue(QueueName=name)["QueueUrl"]
        try:
            attrs = sqs.get_queue_attributes(QueueUrl=url, AttributeNames=["All"])[
                "Attributes"
            ]
            assert attrs["VisibilityTimeout"] == "30"
            assert attrs["DelaySeconds"] == "0"
            assert attrs["MaximumMessageSize"] == "262144"
            assert attrs["MessageRetentionPeriod"] == "345600"
            assert attrs["ReceiveMessageWaitTimeSeconds"] == "0"
        finally:
            sqs.delete_queue(QueueUrl=url)

    def test_should_return_approximate_message_counts(self, sqs, test_queue):
        attrs = sqs.get_queue_attributes(
            QueueUrl=test_queue,
            AttributeNames=[
                "ApproximateNumberOfMessages",
                "ApproximateNumberOfMessagesNotVisible",
                "ApproximateNumberOfMessagesDelayed",
            ],
        )["Attributes"]
        # Fresh queue should have 0 messages
        assert int(attrs["ApproximateNumberOfMessages"]) == 0
        assert int(attrs["ApproximateNumberOfMessagesNotVisible"]) == 0
        assert int(attrs["ApproximateNumberOfMessagesDelayed"]) == 0


class TestSetQueueAttributes:
    def test_should_update_visibility_timeout(self, sqs, test_queue):
        sqs.set_queue_attributes(
            QueueUrl=test_queue,
            Attributes={"VisibilityTimeout": "120"},
        )
        attrs = sqs.get_queue_attributes(
            QueueUrl=test_queue, AttributeNames=["VisibilityTimeout"]
        )["Attributes"]
        assert attrs["VisibilityTimeout"] == "120"

    def test_should_update_delay_seconds(self, sqs, test_queue):
        sqs.set_queue_attributes(
            QueueUrl=test_queue,
            Attributes={"DelaySeconds": "10"},
        )
        attrs = sqs.get_queue_attributes(
            QueueUrl=test_queue, AttributeNames=["DelaySeconds"]
        )["Attributes"]
        assert attrs["DelaySeconds"] == "10"

    def test_should_update_maximum_message_size(self, sqs, test_queue):
        sqs.set_queue_attributes(
            QueueUrl=test_queue,
            Attributes={"MaximumMessageSize": "4096"},
        )
        attrs = sqs.get_queue_attributes(
            QueueUrl=test_queue, AttributeNames=["MaximumMessageSize"]
        )["Attributes"]
        assert attrs["MaximumMessageSize"] == "4096"

    def test_should_update_message_retention_period(self, sqs, test_queue):
        sqs.set_queue_attributes(
            QueueUrl=test_queue,
            Attributes={"MessageRetentionPeriod": "172800"},
        )
        attrs = sqs.get_queue_attributes(
            QueueUrl=test_queue, AttributeNames=["MessageRetentionPeriod"]
        )["Attributes"]
        assert attrs["MessageRetentionPeriod"] == "172800"

    def test_should_update_receive_wait_time(self, sqs, test_queue):
        sqs.set_queue_attributes(
            QueueUrl=test_queue,
            Attributes={"ReceiveMessageWaitTimeSeconds": "10"},
        )
        attrs = sqs.get_queue_attributes(
            QueueUrl=test_queue,
            AttributeNames=["ReceiveMessageWaitTimeSeconds"],
        )["Attributes"]
        assert attrs["ReceiveMessageWaitTimeSeconds"] == "10"
