"""Tests for SQS queue CRUD operations."""

import time

import botocore.exceptions
import pytest

from util import unique_queue_name


class TestCreateQueue:
    def test_should_create_standard_queue(self, sqs):
        name = unique_queue_name("create-std")
        resp = sqs.create_queue(QueueName=name)
        assert "QueueUrl" in resp
        assert name in resp["QueueUrl"]
        sqs.delete_queue(QueueUrl=resp["QueueUrl"])

    def test_should_create_queue_with_attributes(self, sqs):
        name = unique_queue_name("create-attr")
        resp = sqs.create_queue(
            QueueName=name,
            Attributes={
                "VisibilityTimeout": "60",
                "DelaySeconds": "5",
                "MaximumMessageSize": "1024",
                "MessageRetentionPeriod": "86400",
            },
        )
        url = resp["QueueUrl"]
        attrs = sqs.get_queue_attributes(QueueUrl=url, AttributeNames=["All"])[
            "Attributes"
        ]
        assert attrs["VisibilityTimeout"] == "60"
        assert attrs["DelaySeconds"] == "5"
        assert attrs["MaximumMessageSize"] == "1024"
        assert attrs["MessageRetentionPeriod"] == "86400"
        sqs.delete_queue(QueueUrl=url)

    def test_should_be_idempotent_with_same_attributes(self, sqs):
        name = unique_queue_name("create-idem")
        attrs = {"VisibilityTimeout": "30"}
        url1 = sqs.create_queue(QueueName=name, Attributes=attrs)["QueueUrl"]
        url2 = sqs.create_queue(QueueName=name, Attributes=attrs)["QueueUrl"]
        assert url1 == url2
        sqs.delete_queue(QueueUrl=url1)

    def test_should_fail_idempotent_create_with_different_attributes(self, sqs):
        name = unique_queue_name("create-diff")
        url = sqs.create_queue(QueueName=name, Attributes={"VisibilityTimeout": "30"})[
            "QueueUrl"
        ]
        try:
            with pytest.raises(botocore.exceptions.ClientError) as exc:
                sqs.create_queue(QueueName=name, Attributes={"VisibilityTimeout": "60"})
            assert exc.value.response["Error"]["Code"] == "QueueAlreadyExists"
        finally:
            sqs.delete_queue(QueueUrl=url)

    def test_should_reject_invalid_queue_name(self, sqs):
        with pytest.raises(botocore.exceptions.ClientError) as exc:
            sqs.create_queue(QueueName="invalid name with spaces!")
        err = exc.value.response["Error"]["Code"]
        assert err in ("InvalidParameterValue", "InvalidAddress")

    def test_should_reject_name_longer_than_80_chars(self, sqs):
        name = "a" * 81
        with pytest.raises(botocore.exceptions.ClientError) as exc:
            sqs.create_queue(QueueName=name)
        err = exc.value.response["Error"]["Code"]
        assert err in ("InvalidParameterValue", "InvalidAddress")

    def test_should_create_queue_and_tag_separately(self, sqs):
        name = unique_queue_name("create-tags")
        resp = sqs.create_queue(QueueName=name)
        url = resp["QueueUrl"]
        sqs.tag_queue(
            QueueUrl=url,
            Tags={"env": "test", "service": "sqs"},
        )
        tags = sqs.list_queue_tags(QueueUrl=url).get("Tags", {})
        assert tags["env"] == "test"
        assert tags["service"] == "sqs"
        sqs.delete_queue(QueueUrl=url)


class TestDeleteQueue:
    def test_should_delete_existing_queue(self, sqs):
        name = unique_queue_name("del")
        url = sqs.create_queue(QueueName=name)["QueueUrl"]
        sqs.delete_queue(QueueUrl=url)
        # Queue should no longer be accessible
        with pytest.raises(botocore.exceptions.ClientError) as exc:
            sqs.get_queue_attributes(QueueUrl=url, AttributeNames=["All"])
        assert exc.value.response["Error"]["Code"] in (
            "AWS.SimpleQueueService.NonExistentQueue",
            "QueueDoesNotExist",
        )

    def test_should_not_error_deleting_nonexistent_queue(self, sqs, endpoint_url):
        """AWS returns an error for nonexistent queue; some impls silently succeed."""
        fake_url = f"{endpoint_url}/000000000000/nonexistent-queue-{int(time.time())}"
        # Either succeeds silently or raises NonExistentQueue -- both acceptable
        try:
            sqs.delete_queue(QueueUrl=fake_url)
        except botocore.exceptions.ClientError as e:
            assert e.response["Error"]["Code"] in (
                "AWS.SimpleQueueService.NonExistentQueue",
                "QueueDoesNotExist",
            )


class TestGetQueueUrl:
    def test_should_return_url_for_existing_queue(self, sqs, test_queue):
        # Extract name from the test_queue URL
        name = test_queue.rstrip("/").split("/")[-1]
        resp = sqs.get_queue_url(QueueName=name)
        assert resp["QueueUrl"] == test_queue

    def test_should_fail_for_nonexistent_queue(self, sqs):
        with pytest.raises(botocore.exceptions.ClientError) as exc:
            sqs.get_queue_url(QueueName=f"nonexistent-{int(time.time())}")
        assert exc.value.response["Error"]["Code"] in (
            "AWS.SimpleQueueService.NonExistentQueue",
            "QueueDoesNotExist",
        )


class TestListQueues:
    def test_should_list_created_queues(self, sqs, queue_factory):
        name1 = unique_queue_name("list-a")
        name2 = unique_queue_name("list-b")
        url1 = queue_factory(name1)
        url2 = queue_factory(name2)
        resp = sqs.list_queues()
        urls = resp.get("QueueUrls", [])
        assert url1 in urls
        assert url2 in urls

    def test_should_filter_by_prefix(self, sqs, queue_factory):
        prefix = f"pfx-{int(time.time() * 1000) % 10000000}"
        url1 = queue_factory(f"{prefix}-one")
        queue_factory(unique_queue_name("other"))
        resp = sqs.list_queues(QueueNamePrefix=prefix)
        urls = resp.get("QueueUrls", [])
        assert url1 in urls
        # All returned URLs should contain the prefix
        for u in urls:
            assert prefix in u

    def test_should_return_empty_for_unmatched_prefix(self, sqs):
        resp = sqs.list_queues(QueueNamePrefix=f"zzznoexist{int(time.time())}")
        assert resp.get("QueueUrls", []) == []


class TestPurgeQueue:
    def test_should_purge_all_messages(self, sqs, test_queue):
        # Send some messages
        for i in range(5):
            sqs.send_message(QueueUrl=test_queue, MessageBody=f"msg-{i}")

        sqs.purge_queue(QueueUrl=test_queue)

        # Wait briefly for purge to take effect
        time.sleep(1)

        resp = sqs.receive_message(
            QueueUrl=test_queue, MaxNumberOfMessages=10, WaitTimeSeconds=1
        )
        assert resp.get("Messages", []) == []
