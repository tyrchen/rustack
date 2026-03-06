"""Tests for SQS batch operations."""

import time

import botocore.exceptions
import pytest

from util import receive_all, wait_for_messages


class TestSendMessageBatch:
    def test_should_send_batch_successfully(self, sqs, test_queue):
        entries = [{"Id": f"msg-{i}", "MessageBody": f"body-{i}"} for i in range(5)]
        resp = sqs.send_message_batch(QueueUrl=test_queue, Entries=entries)
        assert len(resp.get("Successful", [])) == 5
        assert resp.get("Failed", []) == []

        for item in resp["Successful"]:
            assert "MessageId" in item
            assert "MD5OfMessageBody" in item

    def test_should_send_max_10_messages(self, sqs, test_queue):
        entries = [{"Id": f"msg-{i}", "MessageBody": f"body-{i}"} for i in range(10)]
        resp = sqs.send_message_batch(QueueUrl=test_queue, Entries=entries)
        assert len(resp.get("Successful", [])) == 10

    def test_should_reject_more_than_10_entries(self, sqs, test_queue):
        entries = [{"Id": f"msg-{i}", "MessageBody": f"body-{i}"} for i in range(11)]
        with pytest.raises(botocore.exceptions.ClientError) as exc:
            sqs.send_message_batch(QueueUrl=test_queue, Entries=entries)
        assert "TooManyEntriesInBatchRequest" in exc.value.response["Error"]["Code"]

    def test_should_reject_empty_batch(self, sqs, test_queue):
        with pytest.raises(botocore.exceptions.ClientError) as exc:
            sqs.send_message_batch(QueueUrl=test_queue, Entries=[])
        assert "EmptyBatchRequest" in exc.value.response["Error"]["Code"]

    def test_should_reject_duplicate_ids(self, sqs, test_queue):
        entries = [
            {"Id": "dup", "MessageBody": "body-1"},
            {"Id": "dup", "MessageBody": "body-2"},
        ]
        with pytest.raises(botocore.exceptions.ClientError) as exc:
            sqs.send_message_batch(QueueUrl=test_queue, Entries=entries)
        assert "BatchEntryIdsNotDistinct" in exc.value.response["Error"]["Code"]

    def test_should_send_batch_with_delay(self, sqs, test_queue):
        entries = [
            {"Id": "nodelay", "MessageBody": "immediate"},
            {"Id": "delay", "MessageBody": "delayed", "DelaySeconds": 3},
        ]
        resp = sqs.send_message_batch(QueueUrl=test_queue, Entries=entries)
        assert len(resp.get("Successful", [])) == 2

        # Only the non-delayed message should be immediately visible
        resp = sqs.receive_message(
            QueueUrl=test_queue, MaxNumberOfMessages=10, WaitTimeSeconds=1
        )
        msgs = resp.get("Messages", [])
        bodies = [m["Body"] for m in msgs]
        assert "immediate" in bodies


class TestDeleteMessageBatch:
    def test_should_delete_batch_successfully(self, sqs, test_queue):
        # Send messages
        for i in range(3):
            sqs.send_message(QueueUrl=test_queue, MessageBody=f"del-{i}")

        msgs = wait_for_messages(sqs, test_queue, 3)
        assert len(msgs) >= 3

        entries = [
            {"Id": f"del-{i}", "ReceiptHandle": m["ReceiptHandle"]}
            for i, m in enumerate(msgs[:3])
        ]
        resp = sqs.delete_message_batch(QueueUrl=test_queue, Entries=entries)
        assert len(resp.get("Successful", [])) == 3

        # Queue should be empty
        time.sleep(1)
        resp = sqs.receive_message(
            QueueUrl=test_queue, MaxNumberOfMessages=10, WaitTimeSeconds=1
        )
        assert resp.get("Messages", []) == []

    def test_should_reject_empty_batch(self, sqs, test_queue):
        with pytest.raises(botocore.exceptions.ClientError) as exc:
            sqs.delete_message_batch(QueueUrl=test_queue, Entries=[])
        assert "EmptyBatchRequest" in exc.value.response["Error"]["Code"]


class TestChangeMessageVisibilityBatch:
    def test_should_change_visibility_batch(self, sqs, test_queue):
        for i in range(3):
            sqs.send_message(QueueUrl=test_queue, MessageBody=f"vis-{i}")

        resp = sqs.receive_message(
            QueueUrl=test_queue,
            MaxNumberOfMessages=10,
            WaitTimeSeconds=5,
            VisibilityTimeout=30,
        )
        msgs = resp.get("Messages", [])
        assert len(msgs) >= 1

        entries = [
            {
                "Id": f"vis-{i}",
                "ReceiptHandle": m["ReceiptHandle"],
                "VisibilityTimeout": 0,
            }
            for i, m in enumerate(msgs)
        ]
        resp = sqs.change_message_visibility_batch(QueueUrl=test_queue, Entries=entries)
        assert len(resp.get("Successful", [])) == len(msgs)

        # Messages should now be visible again
        resp = sqs.receive_message(
            QueueUrl=test_queue, MaxNumberOfMessages=10, WaitTimeSeconds=2
        )
        assert len(resp.get("Messages", [])) >= 1

    def test_should_reject_empty_batch(self, sqs, test_queue):
        with pytest.raises(botocore.exceptions.ClientError) as exc:
            sqs.change_message_visibility_batch(QueueUrl=test_queue, Entries=[])
        assert "EmptyBatchRequest" in exc.value.response["Error"]["Code"]
