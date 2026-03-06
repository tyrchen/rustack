"""Tests for SQS long-polling behavior."""

import threading
import time

import pytest

from util import unique_queue_name


class TestLongPolling:
    def test_should_return_immediately_on_short_poll(self, sqs, test_queue):
        """WaitTimeSeconds=0 should return immediately with no messages."""
        start = time.time()
        resp = sqs.receive_message(
            QueueUrl=test_queue,
            MaxNumberOfMessages=1,
            WaitTimeSeconds=0,
        )
        elapsed = time.time() - start
        assert resp.get("Messages", []) == []
        assert elapsed < 2.0

    def test_should_return_message_sent_during_long_poll(self, sqs, test_queue):
        """Long poll should eventually return a message that was sent during the wait.

        Note: Some implementations wake up immediately on message arrival,
        while others only check at the end of the wait period. We test that
        the message is received eventually with a second receive if needed.
        """

        def send_after_delay():
            time.sleep(1)
            sqs.send_message(QueueUrl=test_queue, MessageBody="long-poll-msg")

        sender = threading.Thread(target=send_after_delay)
        sender.start()

        # Try long poll first
        resp = sqs.receive_message(
            QueueUrl=test_queue,
            MaxNumberOfMessages=1,
            WaitTimeSeconds=5,
        )
        msgs = resp.get("Messages", [])
        sender.join()

        # If long poll didn't wake up, do a second receive
        if not msgs:
            resp = sqs.receive_message(
                QueueUrl=test_queue,
                MaxNumberOfMessages=1,
                WaitTimeSeconds=5,
            )
            msgs = resp.get("Messages", [])

        assert len(msgs) == 1
        assert msgs[0]["Body"] == "long-poll-msg"

    def test_should_timeout_on_empty_queue(self, sqs, test_queue):
        """Long poll on empty queue should wait and return empty."""
        start = time.time()
        resp = sqs.receive_message(
            QueueUrl=test_queue,
            MaxNumberOfMessages=1,
            WaitTimeSeconds=2,
        )
        elapsed = time.time() - start
        assert resp.get("Messages", []) == []
        # Should have waited approximately 2 seconds
        assert elapsed >= 1.5
        assert elapsed < 5.0

    def test_should_use_queue_level_wait_time(self, sqs):
        """Queue-level ReceiveMessageWaitTimeSeconds as default."""
        name = unique_queue_name("longpoll-attr")
        url = sqs.create_queue(
            QueueName=name,
            Attributes={"ReceiveMessageWaitTimeSeconds": "2"},
        )["QueueUrl"]
        try:
            start = time.time()
            resp = sqs.receive_message(
                QueueUrl=url,
                MaxNumberOfMessages=1,
                # No WaitTimeSeconds — should use queue default of 2
            )
            elapsed = time.time() - start
            assert resp.get("Messages", []) == []
            assert elapsed >= 1.5
        finally:
            sqs.delete_queue(QueueUrl=url)

    def test_should_override_queue_wait_time_with_request(self, sqs):
        """Request-level WaitTimeSeconds overrides queue default."""
        name = unique_queue_name("longpoll-ovr")
        url = sqs.create_queue(
            QueueName=name,
            Attributes={"ReceiveMessageWaitTimeSeconds": "10"},
        )["QueueUrl"]
        try:
            start = time.time()
            resp = sqs.receive_message(
                QueueUrl=url,
                MaxNumberOfMessages=1,
                WaitTimeSeconds=1,  # Override: only wait 1s
            )
            elapsed = time.time() - start
            assert resp.get("Messages", []) == []
            assert elapsed < 5.0
        finally:
            sqs.delete_queue(QueueUrl=url)
