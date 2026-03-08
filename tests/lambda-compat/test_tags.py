"""Tests for Lambda tag operations."""


class TestTagResource:
    def test_should_tag_function(self, test_function, lamb):
        arn = lamb.get_function(FunctionName=test_function)["Configuration"][
            "FunctionArn"
        ]
        lamb.tag_resource(Resource=arn, Tags={"env": "test", "team": "platform"})
        resp = lamb.list_tags(Resource=arn)
        assert resp["Tags"]["env"] == "test"
        assert resp["Tags"]["team"] == "platform"

    def test_should_overwrite_existing_tag(self, test_function, lamb):
        arn = lamb.get_function(FunctionName=test_function)["Configuration"][
            "FunctionArn"
        ]
        lamb.tag_resource(Resource=arn, Tags={"env": "dev"})
        lamb.tag_resource(Resource=arn, Tags={"env": "prod"})
        resp = lamb.list_tags(Resource=arn)
        assert resp["Tags"]["env"] == "prod"

    def test_should_add_tags_incrementally(self, test_function, lamb):
        arn = lamb.get_function(FunctionName=test_function)["Configuration"][
            "FunctionArn"
        ]
        lamb.tag_resource(Resource=arn, Tags={"key1": "val1"})
        lamb.tag_resource(Resource=arn, Tags={"key2": "val2"})
        resp = lamb.list_tags(Resource=arn)
        assert resp["Tags"]["key1"] == "val1"
        assert resp["Tags"]["key2"] == "val2"


class TestUntagResource:
    def test_should_untag_function(self, test_function, lamb):
        arn = lamb.get_function(FunctionName=test_function)["Configuration"][
            "FunctionArn"
        ]
        lamb.tag_resource(Resource=arn, Tags={"a": "1", "b": "2", "c": "3"})
        lamb.untag_resource(Resource=arn, TagKeys=["b"])
        resp = lamb.list_tags(Resource=arn)
        assert "b" not in resp["Tags"]
        assert resp["Tags"]["a"] == "1"
        assert resp["Tags"]["c"] == "3"

    def test_should_untag_multiple_keys(self, test_function, lamb):
        arn = lamb.get_function(FunctionName=test_function)["Configuration"][
            "FunctionArn"
        ]
        lamb.tag_resource(Resource=arn, Tags={"x": "1", "y": "2", "z": "3"})
        lamb.untag_resource(Resource=arn, TagKeys=["x", "z"])
        resp = lamb.list_tags(Resource=arn)
        assert set(resp["Tags"].keys()) == {"y"}


class TestListTags:
    def test_should_list_empty_tags(self, test_function, lamb):
        arn = lamb.get_function(FunctionName=test_function)["Configuration"][
            "FunctionArn"
        ]
        resp = lamb.list_tags(Resource=arn)
        # May be empty dict or absent.
        tags = resp.get("Tags", {})
        assert isinstance(tags, dict)
