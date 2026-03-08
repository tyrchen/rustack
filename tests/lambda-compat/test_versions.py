"""Tests for Lambda version operations."""

from util import unique_name, make_zip


class TestPublishVersion:
    def test_should_publish_version(self, test_function, lamb):
        resp = lamb.publish_version(
            FunctionName=test_function,
            Description="Version 1",
        )
        assert resp["Version"] == "1"
        assert resp["Description"] == "Version 1"

    def test_should_publish_multiple_versions(self, test_function, lamb):
        v1 = lamb.publish_version(FunctionName=test_function)
        v2 = lamb.publish_version(FunctionName=test_function)
        assert v1["Version"] == "1"
        assert v2["Version"] == "2"

    def test_should_have_correct_arn(self, test_function, lamb):
        resp = lamb.publish_version(FunctionName=test_function)
        assert resp["FunctionArn"].endswith(":1")


class TestListVersionsByFunction:
    def test_should_list_versions(self, test_function, lamb):
        lamb.publish_version(FunctionName=test_function)
        lamb.publish_version(FunctionName=test_function)

        resp = lamb.list_versions_by_function(FunctionName=test_function)
        versions = [v["Version"] for v in resp["Versions"]]
        assert "$LATEST" in versions
        assert "1" in versions
        assert "2" in versions

    def test_should_list_only_latest_before_publish(self, test_function, lamb):
        resp = lamb.list_versions_by_function(FunctionName=test_function)
        versions = [v["Version"] for v in resp["Versions"]]
        assert versions == ["$LATEST"]

    def test_should_support_max_items(self, test_function, lamb):
        for _ in range(3):
            lamb.publish_version(FunctionName=test_function)
        resp = lamb.list_versions_by_function(FunctionName=test_function, MaxItems=2)
        assert len(resp["Versions"]) <= 2
