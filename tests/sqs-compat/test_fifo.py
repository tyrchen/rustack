"""Tests for SQS FIFO queue operations."""

import time

import botocore.exceptions
import pytest

from util import unique_queue_name, wait_for_messages


class TestFifoQueueCreation:
    def test_should_create_fifo_queue(self, sqs):
        name = unique_queue_name("fifo") + ".fifo"
        resp = sqs.create_queue(
            QueueName=name,
            Attributes={"FifoQueue": "true"},
        )
        url = resp["QueueUrl"]
        try:
            attrs = sqs.get_queue_attributes(
                QueueUrl=url, AttributeNames=["FifoQueue"]
            )["Attributes"]
            assert attrs["FifoQueue"] == "true"
        finally:
            sqs.delete_queue(QueueUrl=url)

    def test_should_require_fifo_suffix(self, sqs):
        name = unique_queue_name("nofifo")
        with pytest.raises(botocore.exceptions.ClientError) as exc:
            sqs.create_queue(
                QueueName=name,
                Attributes={"FifoQueue": "true"},
            )
        err = exc.value.response["Error"]["Code"]
        assert err in ("InvalidParameterValue", "InvalidAddress")

    def test_should_reject_or_auto_enable_fifo_suffix_without_attribute(self, sqs):
        """Queue name ends with .fifo but FifoQueue attribute is not set.

        AWS rejects this. Some implementations auto-enable FifoQueue when the
        name ends with .fifo. Both behaviors are acceptable.
        """
        name = unique_queue_name("bad") + ".fifo"
        try:
            resp = sqs.create_queue(QueueName=name)
            url = resp["QueueUrl"]
            # If it succeeds, verify it's actually a FIFO queue
            attrs = sqs.get_queue_attributes(
                QueueUrl=url, AttributeNames=["FifoQueue"]
            )["Attributes"]
            assert attrs.get("FifoQueue") == "true"
            sqs.delete_queue(QueueUrl=url)
        except botocore.exceptions.ClientError as e:
            err = e.response["Error"]["Code"]
            assert err in ("InvalidParameterValue", "InvalidAddress")


class TestFifoOrdering:
    def test_should_preserve_order_within_group(self, sqs, test_fifo_queue):
        for i in range(5):
            sqs.send_message(
                QueueUrl=test_fifo_queue,
                MessageBody=f"ordered-{i}",
                MessageGroupId="group-a",
            )

        # In FIFO queues, a message group is blocked until the in-flight
        # message is deleted or its visibility timeout expires.
        # We receive and delete one at a time to verify ordering.
        bodies = []
        for _ in range(5):
            resp = sqs.receive_message(
                QueueUrl=test_fifo_queue,
                MaxNumberOfMessages=1,
                WaitTimeSeconds=5,
            )
            msgs = resp.get("Messages", [])
            assert len(msgs) == 1
            bodies.append(msgs[0]["Body"])
            sqs.delete_message(
                QueueUrl=test_fifo_queue,
                ReceiptHandle=msgs[0]["ReceiptHandle"],
            )
        assert bodies == [f"ordered-{i}" for i in range(5)]

    def test_should_support_multiple_groups(self, sqs, test_fifo_queue):
        # Send interleaved messages to two groups
        for i in range(3):
            sqs.send_message(
                QueueUrl=test_fifo_queue,
                MessageBody=f"a-{i}",
                MessageGroupId="group-a",
            )
            sqs.send_message(
                QueueUrl=test_fifo_queue,
                MessageBody=f"b-{i}",
                MessageGroupId="group-b",
            )

        msgs = wait_for_messages(sqs, test_fifo_queue, 6, timeout=10)
        # Within each group, order must be preserved
        a_msgs = [m["Body"] for m in msgs if m["Body"].startswith("a-")]
        b_msgs = [m["Body"] for m in msgs if m["Body"].startswith("b-")]
        assert a_msgs == [f"a-{i}" for i in range(len(a_msgs))]
        assert b_msgs == [f"b-{i}" for i in range(len(b_msgs))]


class TestFifoDeduplication:
    def test_should_dedup_with_content_based(self, sqs, test_fifo_queue):
        """Content-based dedup: identical bodies within window are deduped."""
        body = "duplicate-content"
        sqs.send_message(
            QueueUrl=test_fifo_queue,
            MessageBody=body,
            MessageGroupId="grp",
        )
        sqs.send_message(
            QueueUrl=test_fifo_queue,
            MessageBody=body,
            MessageGroupId="grp",
        )

        msgs = wait_for_messages(sqs, test_fifo_queue, 2, timeout=5)
        # Only one message should be received
        assert len(msgs) == 1
        assert msgs[0]["Body"] == body

    def test_should_dedup_with_explicit_dedup_id(self, sqs):
        name = unique_queue_name("dedup-explicit") + ".fifo"
        url = sqs.create_queue(
            QueueName=name,
            Attributes={"FifoQueue": "true"},
        )["QueueUrl"]
        try:
            sqs.send_message(
                QueueUrl=url,
                MessageBody="first",
                MessageGroupId="grp",
                MessageDeduplicationId="dedup-1",
            )
            sqs.send_message(
                QueueUrl=url,
                MessageBody="second-but-same-dedup",
                MessageGroupId="grp",
                MessageDeduplicationId="dedup-1",
            )

            msgs = wait_for_messages(sqs, url, 2, timeout=5)
            assert len(msgs) == 1
            assert msgs[0]["Body"] == "first"
        finally:
            sqs.delete_queue(QueueUrl=url)

    def test_should_return_same_message_id_for_dedup(self, sqs, test_fifo_queue):
        body = "dedup-msgid-test"
        r1 = sqs.send_message(
            QueueUrl=test_fifo_queue,
            MessageBody=body,
            MessageGroupId="grp",
        )
        r2 = sqs.send_message(
            QueueUrl=test_fifo_queue,
            MessageBody=body,
            MessageGroupId="grp",
        )
        # Deduped sends should return the same MessageId
        assert r1["MessageId"] == r2["MessageId"]

    def test_should_not_dedup_different_bodies(self, sqs, test_fifo_queue):
        sqs.send_message(
            QueueUrl=test_fifo_queue,
            MessageBody="body-a",
            MessageGroupId="grp",
        )
        sqs.send_message(
            QueueUrl=test_fifo_queue,
            MessageBody="body-b",
            MessageGroupId="grp",
        )

        # FIFO group blocks after first receive; receive-delete each message.
        bodies = []
        for _ in range(2):
            resp = sqs.receive_message(
                QueueUrl=test_fifo_queue,
                MaxNumberOfMessages=1,
                WaitTimeSeconds=5,
            )
            msgs = resp.get("Messages", [])
            assert len(msgs) == 1
            bodies.append(msgs[0]["Body"])
            sqs.delete_message(
                QueueUrl=test_fifo_queue,
                ReceiptHandle=msgs[0]["ReceiptHandle"],
            )
        assert set(bodies) == {"body-a", "body-b"}

    def test_should_require_group_id_for_fifo(self, sqs, test_fifo_queue):
        with pytest.raises(botocore.exceptions.ClientError) as exc:
            sqs.send_message(
                QueueUrl=test_fifo_queue,
                MessageBody="no group",
            )
        err = exc.value.response["Error"]["Code"]
        assert err in ("MissingParameter", "InvalidParameterValue")

    def test_should_require_dedup_id_when_content_based_disabled(self, sqs):
        name = unique_queue_name("fifo-nodedup") + ".fifo"
        url = sqs.create_queue(
            QueueName=name,
            Attributes={
                "FifoQueue": "true",
                "ContentBasedDeduplication": "false",
            },
        )["QueueUrl"]
        try:
            with pytest.raises(botocore.exceptions.ClientError) as exc:
                sqs.send_message(
                    QueueUrl=url,
                    MessageBody="no dedup id",
                    MessageGroupId="grp",
                )
            err = exc.value.response["Error"]["Code"]
            assert err in ("MissingParameter", "InvalidParameterValue")
        finally:
            sqs.delete_queue(QueueUrl=url)
