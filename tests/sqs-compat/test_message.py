"""Tests for SQS message send/receive/delete operations."""

import hashlib
import time

import botocore.exceptions
import pytest

from util import receive_all, wait_for_messages


class TestSendMessage:
    def test_should_send_and_receive_simple_message(self, sqs, test_queue):
        body = "hello world"
        send = sqs.send_message(QueueUrl=test_queue, MessageBody=body)
        assert "MessageId" in send
        assert "MD5OfMessageBody" in send

        msgs = wait_for_messages(sqs, test_queue, 1)
        assert len(msgs) >= 1
        assert msgs[0]["Body"] == body

    def test_should_return_correct_md5(self, sqs, test_queue):
        body = "test body for md5"
        send = sqs.send_message(QueueUrl=test_queue, MessageBody=body)
        expected_md5 = hashlib.md5(body.encode("utf-8")).hexdigest()
        assert send["MD5OfMessageBody"] == expected_md5

    def test_should_handle_unicode_message(self, sqs, test_queue):
        body = "Hello 世界 🌍 café résumé"
        sqs.send_message(QueueUrl=test_queue, MessageBody=body)
        msgs = wait_for_messages(sqs, test_queue, 1)
        assert len(msgs) >= 1
        assert msgs[0]["Body"] == body

    def test_should_send_with_delay_seconds(self, sqs, test_queue):
        sqs.send_message(QueueUrl=test_queue, MessageBody="delayed", DelaySeconds=2)
        # Immediately should not be visible
        resp = sqs.receive_message(
            QueueUrl=test_queue, MaxNumberOfMessages=1, WaitTimeSeconds=0
        )
        assert resp.get("Messages", []) == []

        # After delay, should be visible
        msgs = wait_for_messages(sqs, test_queue, 1, timeout=10)
        assert len(msgs) >= 1
        assert msgs[0]["Body"] == "delayed"

    def test_should_send_with_message_attributes(self, sqs, test_queue):
        msg_attrs = {
            "Author": {"DataType": "String", "StringValue": "Alice"},
            "Count": {"DataType": "Number", "StringValue": "42"},
        }
        send = sqs.send_message(
            QueueUrl=test_queue,
            MessageBody="with attrs",
            MessageAttributes=msg_attrs,
        )
        assert "MD5OfMessageAttributes" in send

        msgs = wait_for_messages(sqs, test_queue, 1)
        assert len(msgs) >= 1

    def test_should_receive_message_attributes(self, sqs, test_queue):
        msg_attrs = {
            "Color": {"DataType": "String", "StringValue": "blue"},
        }
        sqs.send_message(
            QueueUrl=test_queue,
            MessageBody="with attrs",
            MessageAttributes=msg_attrs,
        )
        resp = sqs.receive_message(
            QueueUrl=test_queue,
            MaxNumberOfMessages=1,
            WaitTimeSeconds=5,
            MessageAttributeNames=["All"],
        )
        msgs = resp.get("Messages", [])
        assert len(msgs) == 1
        received_attrs = msgs[0].get("MessageAttributes", {})
        assert "Color" in received_attrs
        assert received_attrs["Color"]["StringValue"] == "blue"

    def test_should_receive_system_attributes(self, sqs, test_queue):
        sqs.send_message(QueueUrl=test_queue, MessageBody="sys attrs test")
        resp = sqs.receive_message(
            QueueUrl=test_queue,
            MaxNumberOfMessages=1,
            WaitTimeSeconds=5,
            AttributeNames=["All"],
        )
        msgs = resp.get("Messages", [])
        assert len(msgs) == 1
        sys_attrs = msgs[0].get("Attributes", {})
        assert "SentTimestamp" in sys_attrs
        assert "ApproximateReceiveCount" in sys_attrs
        assert sys_attrs["ApproximateReceiveCount"] == "1"
        assert "ApproximateFirstReceiveTimestamp" in sys_attrs


class TestReceiveMessage:
    def test_should_return_empty_when_no_messages(self, sqs, test_queue):
        resp = sqs.receive_message(
            QueueUrl=test_queue, MaxNumberOfMessages=1, WaitTimeSeconds=0
        )
        assert resp.get("Messages", []) == []

    def test_should_respect_max_number_of_messages(self, sqs, test_queue):
        for i in range(5):
            sqs.send_message(QueueUrl=test_queue, MessageBody=f"msg-{i}")

        resp = sqs.receive_message(
            QueueUrl=test_queue, MaxNumberOfMessages=3, WaitTimeSeconds=2
        )
        msgs = resp.get("Messages", [])
        assert len(msgs) <= 3

    def test_should_include_receipt_handle(self, sqs, test_queue):
        sqs.send_message(QueueUrl=test_queue, MessageBody="handle test")
        msgs = wait_for_messages(sqs, test_queue, 1)
        assert len(msgs) >= 1
        assert "ReceiptHandle" in msgs[0]
        assert len(msgs[0]["ReceiptHandle"]) > 0

    def test_should_increment_receive_count(self, sqs, test_queue):
        sqs.send_message(QueueUrl=test_queue, MessageBody="count test")

        # First receive
        resp = sqs.receive_message(
            QueueUrl=test_queue,
            MaxNumberOfMessages=1,
            WaitTimeSeconds=5,
            VisibilityTimeout=1,
            AttributeNames=["ApproximateReceiveCount"],
        )
        msgs = resp.get("Messages", [])
        assert len(msgs) == 1
        assert msgs[0]["Attributes"]["ApproximateReceiveCount"] == "1"

        # Wait for visibility timeout to expire
        time.sleep(2)

        # Second receive
        resp = sqs.receive_message(
            QueueUrl=test_queue,
            MaxNumberOfMessages=1,
            WaitTimeSeconds=5,
            VisibilityTimeout=30,
            AttributeNames=["ApproximateReceiveCount"],
        )
        msgs = resp.get("Messages", [])
        assert len(msgs) == 1
        assert msgs[0]["Attributes"]["ApproximateReceiveCount"] == "2"


class TestDeleteMessage:
    def test_should_delete_received_message(self, sqs, test_queue):
        sqs.send_message(QueueUrl=test_queue, MessageBody="to delete")
        msgs = wait_for_messages(sqs, test_queue, 1)
        assert len(msgs) >= 1

        sqs.delete_message(QueueUrl=test_queue, ReceiptHandle=msgs[0]["ReceiptHandle"])

        # Message should not reappear
        time.sleep(1)
        resp = sqs.receive_message(
            QueueUrl=test_queue, MaxNumberOfMessages=1, WaitTimeSeconds=1
        )
        assert resp.get("Messages", []) == []

    def test_should_not_error_on_already_deleted_message(self, sqs, test_queue):
        sqs.send_message(QueueUrl=test_queue, MessageBody="double delete")
        msgs = wait_for_messages(sqs, test_queue, 1)
        handle = msgs[0]["ReceiptHandle"]

        sqs.delete_message(QueueUrl=test_queue, ReceiptHandle=handle)
        # Second delete should not raise (or raise ReceiptHandleIsInvalid)
        try:
            sqs.delete_message(QueueUrl=test_queue, ReceiptHandle=handle)
        except botocore.exceptions.ClientError as e:
            assert e.response["Error"]["Code"] in (
                "ReceiptHandleIsInvalid",
                "InvalidParameterValue",
            )


class TestChangeMessageVisibility:
    def test_should_extend_visibility_timeout(self, sqs, test_queue):
        sqs.send_message(QueueUrl=test_queue, MessageBody="visibility test")
        resp = sqs.receive_message(
            QueueUrl=test_queue,
            MaxNumberOfMessages=1,
            WaitTimeSeconds=5,
            VisibilityTimeout=2,
        )
        msgs = resp.get("Messages", [])
        assert len(msgs) == 1

        # Extend visibility to 30 seconds
        sqs.change_message_visibility(
            QueueUrl=test_queue,
            ReceiptHandle=msgs[0]["ReceiptHandle"],
            VisibilityTimeout=30,
        )

        # After 3s the original 2s timeout would have expired,
        # but extended timeout means message is still invisible
        time.sleep(3)
        resp = sqs.receive_message(
            QueueUrl=test_queue, MaxNumberOfMessages=1, WaitTimeSeconds=0
        )
        assert resp.get("Messages", []) == []

    def test_should_make_message_immediately_visible(self, sqs, test_queue):
        sqs.send_message(QueueUrl=test_queue, MessageBody="vis zero test")
        resp = sqs.receive_message(
            QueueUrl=test_queue,
            MaxNumberOfMessages=1,
            WaitTimeSeconds=5,
            VisibilityTimeout=30,
        )
        msgs = resp.get("Messages", [])
        assert len(msgs) == 1

        # Set visibility to 0 → message should become available again
        sqs.change_message_visibility(
            QueueUrl=test_queue,
            ReceiptHandle=msgs[0]["ReceiptHandle"],
            VisibilityTimeout=0,
        )

        resp = sqs.receive_message(
            QueueUrl=test_queue, MaxNumberOfMessages=1, WaitTimeSeconds=2
        )
        assert len(resp.get("Messages", [])) == 1
