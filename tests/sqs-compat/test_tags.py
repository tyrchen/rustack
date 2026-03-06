"""Tests for SQS queue tagging operations."""

import pytest

from util import unique_queue_name


class TestQueueTags:
    def test_should_tag_queue(self, sqs, test_queue):
        sqs.tag_queue(
            QueueUrl=test_queue,
            Tags={"env": "test", "team": "platform"},
        )
        tags = sqs.list_queue_tags(QueueUrl=test_queue).get("Tags", {})
        assert tags["env"] == "test"
        assert tags["team"] == "platform"

    def test_should_overwrite_existing_tag(self, sqs, test_queue):
        sqs.tag_queue(QueueUrl=test_queue, Tags={"env": "dev"})
        sqs.tag_queue(QueueUrl=test_queue, Tags={"env": "prod"})
        tags = sqs.list_queue_tags(QueueUrl=test_queue).get("Tags", {})
        assert tags["env"] == "prod"

    def test_should_merge_tags(self, sqs, test_queue):
        sqs.tag_queue(QueueUrl=test_queue, Tags={"a": "1"})
        sqs.tag_queue(QueueUrl=test_queue, Tags={"b": "2"})
        tags = sqs.list_queue_tags(QueueUrl=test_queue).get("Tags", {})
        assert tags["a"] == "1"
        assert tags["b"] == "2"

    def test_should_untag_queue(self, sqs, test_queue):
        sqs.tag_queue(
            QueueUrl=test_queue,
            Tags={"keep": "yes", "remove": "bye"},
        )
        sqs.untag_queue(QueueUrl=test_queue, TagKeys=["remove"])
        tags = sqs.list_queue_tags(QueueUrl=test_queue).get("Tags", {})
        assert "keep" in tags
        assert "remove" not in tags

    def test_should_list_empty_tags(self, sqs, test_queue):
        tags = sqs.list_queue_tags(QueueUrl=test_queue).get("Tags", {})
        assert tags == {}

    def test_should_tag_queue_immediately_after_create(self, sqs):
        name = unique_queue_name("tag-create")
        url = sqs.create_queue(QueueName=name)["QueueUrl"]
        try:
            sqs.tag_queue(QueueUrl=url, Tags={"created": "with-tags"})
            tags = sqs.list_queue_tags(QueueUrl=url).get("Tags", {})
            assert tags["created"] == "with-tags"
        finally:
            sqs.delete_queue(QueueUrl=url)
