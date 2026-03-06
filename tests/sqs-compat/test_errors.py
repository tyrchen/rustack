"""Tests for SQS error handling and edge cases."""

import time

import botocore.exceptions
import pytest

from util import unique_queue_name


class TestNonExistentQueue:
    def test_should_error_on_send_to_nonexistent_queue(self, sqs, endpoint_url):
        fake_url = f"{endpoint_url}/000000000000/nonexistent-{int(time.time())}"
        with pytest.raises(botocore.exceptions.ClientError) as exc:
            sqs.send_message(QueueUrl=fake_url, MessageBody="test")
        assert exc.value.response["Error"]["Code"] in (
            "AWS.SimpleQueueService.NonExistentQueue",
            "QueueDoesNotExist",
        )

    def test_should_error_on_receive_from_nonexistent_queue(self, sqs, endpoint_url):
        fake_url = f"{endpoint_url}/000000000000/nonexistent-{int(time.time())}"
        with pytest.raises(botocore.exceptions.ClientError) as exc:
            sqs.receive_message(QueueUrl=fake_url)
        assert exc.value.response["Error"]["Code"] in (
            "AWS.SimpleQueueService.NonExistentQueue",
            "QueueDoesNotExist",
        )

    def test_should_error_on_get_attributes_nonexistent(self, sqs, endpoint_url):
        fake_url = f"{endpoint_url}/000000000000/nonexistent-{int(time.time())}"
        with pytest.raises(botocore.exceptions.ClientError) as exc:
            sqs.get_queue_attributes(QueueUrl=fake_url, AttributeNames=["All"])
        assert exc.value.response["Error"]["Code"] in (
            "AWS.SimpleQueueService.NonExistentQueue",
            "QueueDoesNotExist",
        )

    def test_should_error_on_purge_nonexistent(self, sqs, endpoint_url):
        fake_url = f"{endpoint_url}/000000000000/nonexistent-{int(time.time())}"
        with pytest.raises(botocore.exceptions.ClientError) as exc:
            sqs.purge_queue(QueueUrl=fake_url)
        assert exc.value.response["Error"]["Code"] in (
            "AWS.SimpleQueueService.NonExistentQueue",
            "QueueDoesNotExist",
        )


class TestInvalidParameters:
    def test_should_reject_empty_queue_name(self, sqs):
        with pytest.raises(
            (botocore.exceptions.ClientError, botocore.exceptions.ParamValidationError)
        ):
            sqs.create_queue(QueueName="")

    def test_should_reject_invalid_visibility_timeout(self, sqs, test_queue):
        with pytest.raises(botocore.exceptions.ClientError) as exc:
            sqs.set_queue_attributes(
                QueueUrl=test_queue,
                Attributes={"VisibilityTimeout": "999999"},
            )
        assert exc.value.response["Error"]["Code"] == "InvalidParameterValue"

    def test_should_reject_invalid_delay_seconds(self, sqs, test_queue):
        with pytest.raises(botocore.exceptions.ClientError) as exc:
            sqs.set_queue_attributes(
                QueueUrl=test_queue,
                Attributes={"DelaySeconds": "1000"},
            )
        assert exc.value.response["Error"]["Code"] == "InvalidParameterValue"

    def test_should_reject_message_too_large(self, sqs, test_queue):
        # Default max is 256KB
        big_body = "x" * (256 * 1024 + 1)
        with pytest.raises(botocore.exceptions.ClientError) as exc:
            sqs.send_message(QueueUrl=test_queue, MessageBody=big_body)
        assert exc.value.response["Error"]["Code"] == "InvalidParameterValue"


class TestFifoErrors:
    def test_should_reject_group_id_on_standard_queue(self, sqs, test_queue):
        """MessageGroupId should not be accepted on standard queues."""
        with pytest.raises(botocore.exceptions.ClientError) as exc:
            sqs.send_message(
                QueueUrl=test_queue,
                MessageBody="test",
                MessageGroupId="group",
            )
        err = exc.value.response["Error"]["Code"]
        assert err in ("InvalidParameterValue", "InvalidAddress")

    def test_should_reject_dedup_id_on_standard_queue(self, sqs, test_queue):
        """MessageDeduplicationId should not be accepted on standard queues."""
        with pytest.raises(botocore.exceptions.ClientError) as exc:
            sqs.send_message(
                QueueUrl=test_queue,
                MessageBody="test",
                MessageDeduplicationId="dedup",
            )
        err = exc.value.response["Error"]["Code"]
        assert err in ("InvalidParameterValue", "InvalidAddress")

    def test_should_reject_queue_already_exists_with_different_attributes(self, sqs):
        """Creating a queue with same name but different attributes should fail."""
        name = unique_queue_name("mismatch")
        url = sqs.create_queue(
            QueueName=name,
            Attributes={"VisibilityTimeout": "30"},
        )["QueueUrl"]
        try:
            with pytest.raises(botocore.exceptions.ClientError) as exc:
                sqs.create_queue(
                    QueueName=name,
                    Attributes={"VisibilityTimeout": "60"},
                )
            assert "QueueAlreadyExists" in exc.value.response["Error"]["Code"]
        finally:
            sqs.delete_queue(QueueUrl=url)

    def test_should_reject_invalid_max_message_size(self, sqs, test_queue):
        with pytest.raises(botocore.exceptions.ClientError) as exc:
            sqs.set_queue_attributes(
                QueueUrl=test_queue,
                Attributes={"MaximumMessageSize": "0"},
            )
        assert exc.value.response["Error"]["Code"] == "InvalidParameterValue"
