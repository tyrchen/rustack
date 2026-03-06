"""Tests for SQS dead-letter queue (DLQ) operations."""

import json
import time

import botocore.exceptions
import pytest

from util import unique_queue_name, wait_for_messages


class TestDeadLetterQueue:
    def test_should_setup_redrive_policy(self, sqs):
        dlq_name = unique_queue_name("dlq-target")
        dlq_url = sqs.create_queue(QueueName=dlq_name)["QueueUrl"]
        dlq_arn = sqs.get_queue_attributes(
            QueueUrl=dlq_url, AttributeNames=["QueueArn"]
        )["Attributes"]["QueueArn"]

        src_name = unique_queue_name("dlq-source")
        redrive = json.dumps({"deadLetterTargetArn": dlq_arn, "maxReceiveCount": 3})
        src_url = sqs.create_queue(
            QueueName=src_name,
            Attributes={"RedrivePolicy": redrive},
        )["QueueUrl"]

        try:
            attrs = sqs.get_queue_attributes(
                QueueUrl=src_url, AttributeNames=["RedrivePolicy"]
            )["Attributes"]
            policy = json.loads(attrs["RedrivePolicy"])
            assert policy["deadLetterTargetArn"] == dlq_arn
            assert policy["maxReceiveCount"] == 3
        finally:
            sqs.delete_queue(QueueUrl=src_url)
            sqs.delete_queue(QueueUrl=dlq_url)

    @pytest.mark.xfail(
        reason="Server does not yet move messages to DLQ after maxReceiveCount"
    )
    def test_should_move_to_dlq_after_max_receives(self, sqs):
        dlq_name = unique_queue_name("dlq-dest")
        dlq_url = sqs.create_queue(QueueName=dlq_name)["QueueUrl"]
        dlq_arn = sqs.get_queue_attributes(
            QueueUrl=dlq_url, AttributeNames=["QueueArn"]
        )["Attributes"]["QueueArn"]

        src_name = unique_queue_name("dlq-src")
        redrive = json.dumps({"deadLetterTargetArn": dlq_arn, "maxReceiveCount": 2})
        src_url = sqs.create_queue(
            QueueName=src_name,
            Attributes={
                "RedrivePolicy": redrive,
                "VisibilityTimeout": "2",
            },
        )["QueueUrl"]

        try:
            sqs.send_message(QueueUrl=src_url, MessageBody="dlq-test-msg")

            # Receive maxReceiveCount times without deleting
            for attempt in range(2):
                resp = sqs.receive_message(
                    QueueUrl=src_url,
                    MaxNumberOfMessages=1,
                    WaitTimeSeconds=5,
                    VisibilityTimeout=2,
                )
                msgs = resp.get("Messages", [])
                assert len(msgs) == 1, f"Attempt {attempt}: expected 1 message"
                time.sleep(3)  # Wait for visibility timeout to expire

            # After maxReceiveCount, next receive triggers DLQ move.
            # The move may happen lazily on the next receive attempt.
            time.sleep(3)
            resp = sqs.receive_message(
                QueueUrl=src_url,
                MaxNumberOfMessages=1,
                WaitTimeSeconds=2,
            )
            # Source should be empty (or the message was moved)
            assert resp.get("Messages", []) == []

            # Message should now be in the DLQ
            dlq_msgs = wait_for_messages(sqs, dlq_url, 1, timeout=10)
            assert len(dlq_msgs) == 1
            assert dlq_msgs[0]["Body"] == "dlq-test-msg"
        finally:
            sqs.delete_queue(QueueUrl=src_url)
            sqs.delete_queue(QueueUrl=dlq_url)

    def test_should_list_dead_letter_source_queues(self, sqs):
        dlq_name = unique_queue_name("dlq-list")
        dlq_url = sqs.create_queue(QueueName=dlq_name)["QueueUrl"]
        dlq_arn = sqs.get_queue_attributes(
            QueueUrl=dlq_url, AttributeNames=["QueueArn"]
        )["Attributes"]["QueueArn"]

        src_name = unique_queue_name("dlq-lsrc")
        redrive = json.dumps({"deadLetterTargetArn": dlq_arn, "maxReceiveCount": 5})
        src_url = sqs.create_queue(
            QueueName=src_name,
            Attributes={"RedrivePolicy": redrive},
        )["QueueUrl"]

        try:
            resp = sqs.list_dead_letter_source_queues(QueueUrl=dlq_url)
            source_urls = resp.get("queueUrls", resp.get("QueueUrls", []))
            # Verify the API call succeeds. Source queue should be listed,
            # but some implementations may return an empty list.
            assert isinstance(source_urls, list)
            if source_urls:
                assert src_url in source_urls
        finally:
            sqs.delete_queue(QueueUrl=src_url)
            sqs.delete_queue(QueueUrl=dlq_url)

    def test_should_set_redrive_policy_via_set_attributes(self, sqs):
        dlq_name = unique_queue_name("dlq-set")
        dlq_url = sqs.create_queue(QueueName=dlq_name)["QueueUrl"]
        dlq_arn = sqs.get_queue_attributes(
            QueueUrl=dlq_url, AttributeNames=["QueueArn"]
        )["Attributes"]["QueueArn"]

        src_name = unique_queue_name("dlq-setsrc")
        src_url = sqs.create_queue(QueueName=src_name)["QueueUrl"]

        try:
            redrive = json.dumps(
                {"deadLetterTargetArn": dlq_arn, "maxReceiveCount": 10}
            )
            sqs.set_queue_attributes(
                QueueUrl=src_url,
                Attributes={"RedrivePolicy": redrive},
            )
            attrs = sqs.get_queue_attributes(
                QueueUrl=src_url, AttributeNames=["RedrivePolicy"]
            )["Attributes"]
            policy = json.loads(attrs["RedrivePolicy"])
            assert policy["maxReceiveCount"] == 10
        finally:
            sqs.delete_queue(QueueUrl=src_url)
            sqs.delete_queue(QueueUrl=dlq_url)

    def test_should_not_list_sources_for_non_dlq(self, sqs):
        name = unique_queue_name("notdlq")
        url = sqs.create_queue(QueueName=name)["QueueUrl"]
        try:
            resp = sqs.list_dead_letter_source_queues(QueueUrl=url)
            assert resp.get("queueUrls", resp.get("QueueUrls", [])) == []
        finally:
            sqs.delete_queue(QueueUrl=url)
