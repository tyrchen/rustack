"""Tests for Lambda function CRUD operations."""

import botocore.exceptions
import pytest

from util import unique_name, make_zip


class TestCreateFunction:
    def test_should_create_function_with_zip(self, lamb):
        name = unique_name("create-zip")
        resp = lamb.create_function(
            FunctionName=name,
            Runtime="python3.12",
            Role="arn:aws:iam::000000000000:role/test-role",
            Handler="index.handler",
            Code={"ZipFile": make_zip()},
        )
        assert resp["FunctionName"] == name
        assert resp["Runtime"] == "python3.12"
        assert resp["Handler"] == "index.handler"
        assert resp["State"] == "Active"
        assert "FunctionArn" in resp
        assert resp["CodeSize"] > 0
        assert resp["CodeSha256"]
        lamb.delete_function(FunctionName=name)

    def test_should_create_function_with_defaults(self, lamb):
        name = unique_name("create-defaults")
        resp = lamb.create_function(
            FunctionName=name,
            Runtime="python3.12",
            Role="arn:aws:iam::000000000000:role/test-role",
            Handler="index.handler",
            Code={"ZipFile": make_zip()},
        )
        assert resp["Timeout"] == 3
        assert resp["MemorySize"] == 128
        assert resp["Version"] == "$LATEST"
        lamb.delete_function(FunctionName=name)

    def test_should_create_function_with_custom_settings(self, lamb):
        name = unique_name("create-custom")
        resp = lamb.create_function(
            FunctionName=name,
            Runtime="python3.12",
            Role="arn:aws:iam::000000000000:role/test-role",
            Handler="index.handler",
            Code={"ZipFile": make_zip()},
            Timeout=30,
            MemorySize=512,
            Description="My test function",
            Environment={"Variables": {"KEY": "value"}},
        )
        assert resp["Timeout"] == 30
        assert resp["MemorySize"] == 512
        assert resp["Description"] == "My test function"
        lamb.delete_function(FunctionName=name)

    def test_should_create_function_with_tags(self, lamb):
        name = unique_name("create-tags")
        resp = lamb.create_function(
            FunctionName=name,
            Runtime="python3.12",
            Role="arn:aws:iam::000000000000:role/test-role",
            Handler="index.handler",
            Code={"ZipFile": make_zip()},
            Tags={"env": "test", "team": "platform"},
        )
        assert resp["FunctionName"] == name
        # Verify tags via list_tags.
        tags = lamb.list_tags(Resource=resp["FunctionArn"])
        assert tags["Tags"]["env"] == "test"
        assert tags["Tags"]["team"] == "platform"
        lamb.delete_function(FunctionName=name)

    def test_should_create_function_with_publish(self, lamb):
        name = unique_name("create-pub")
        resp = lamb.create_function(
            FunctionName=name,
            Runtime="python3.12",
            Role="arn:aws:iam::000000000000:role/test-role",
            Handler="index.handler",
            Code={"ZipFile": make_zip()},
            Publish=True,
        )
        assert resp["Version"] == "1"
        # Should have $LATEST and version 1.
        versions = lamb.list_versions_by_function(FunctionName=name)
        assert len(versions["Versions"]) == 2
        lamb.delete_function(FunctionName=name)

    def test_should_create_function_with_arm64(self, lamb):
        name = unique_name("create-arm")
        resp = lamb.create_function(
            FunctionName=name,
            Runtime="python3.12",
            Role="arn:aws:iam::000000000000:role/test-role",
            Handler="index.handler",
            Code={"ZipFile": make_zip()},
            Architectures=["arm64"],
        )
        assert resp["Architectures"] == ["arm64"]
        lamb.delete_function(FunctionName=name)

    def test_should_reject_duplicate_function_name(self, test_function, lamb):
        with pytest.raises(botocore.exceptions.ClientError) as exc:
            lamb.create_function(
                FunctionName=test_function,
                Runtime="python3.12",
                Role="arn:aws:iam::000000000000:role/test-role",
                Handler="index.handler",
                Code={"ZipFile": make_zip()},
            )
        assert exc.value.response["Error"]["Code"] == "ResourceConflictException"


class TestGetFunction:
    def test_should_get_function(self, test_function, lamb):
        resp = lamb.get_function(FunctionName=test_function)
        assert "Configuration" in resp
        assert "Code" in resp
        config = resp["Configuration"]
        assert config["FunctionName"] == test_function
        assert config["Runtime"] == "python3.12"
        assert config["State"] == "Active"

    def test_should_error_on_nonexistent_function(self, lamb):
        with pytest.raises(botocore.exceptions.ClientError) as exc:
            lamb.get_function(FunctionName="nonexistent-function-12345")
        assert exc.value.response["Error"]["Code"] == "ResourceNotFoundException"


class TestGetFunctionConfiguration:
    def test_should_get_configuration(self, test_function, lamb):
        resp = lamb.get_function_configuration(FunctionName=test_function)
        assert resp["FunctionName"] == test_function
        assert resp["Timeout"] == 3
        assert resp["MemorySize"] == 128
        assert resp["Runtime"] == "python3.12"


class TestDeleteFunction:
    def test_should_delete_function(self, lamb):
        name = unique_name("delete")
        lamb.create_function(
            FunctionName=name,
            Runtime="python3.12",
            Role="arn:aws:iam::000000000000:role/test-role",
            Handler="index.handler",
            Code={"ZipFile": make_zip()},
        )
        lamb.delete_function(FunctionName=name)
        with pytest.raises(botocore.exceptions.ClientError) as exc:
            lamb.get_function(FunctionName=name)
        assert exc.value.response["Error"]["Code"] == "ResourceNotFoundException"

    def test_should_error_deleting_nonexistent(self, lamb):
        with pytest.raises(botocore.exceptions.ClientError) as exc:
            lamb.delete_function(FunctionName="nonexistent-12345")
        assert exc.value.response["Error"]["Code"] == "ResourceNotFoundException"


class TestListFunctions:
    def test_should_list_functions(self, function_factory, lamb):
        name1 = function_factory("list-a")
        name2 = function_factory("list-b")
        resp = lamb.list_functions()
        names = [f["FunctionName"] for f in resp["Functions"]]
        assert name1 in names
        assert name2 in names

    def test_should_list_with_max_items(self, function_factory, lamb):
        for i in range(3):
            function_factory(f"list-max-{i}")
        resp = lamb.list_functions(MaxItems=2)
        assert len(resp["Functions"]) <= 2


class TestUpdateFunctionConfiguration:
    def test_should_update_timeout_and_memory(self, test_function, lamb):
        resp = lamb.update_function_configuration(
            FunctionName=test_function,
            Timeout=60,
            MemorySize=1024,
        )
        assert resp["Timeout"] == 60
        assert resp["MemorySize"] == 1024

    def test_should_update_description(self, test_function, lamb):
        resp = lamb.update_function_configuration(
            FunctionName=test_function,
            Description="Updated description",
        )
        assert resp["Description"] == "Updated description"

    def test_should_update_environment(self, test_function, lamb):
        resp = lamb.update_function_configuration(
            FunctionName=test_function,
            Environment={"Variables": {"FOO": "bar", "BAZ": "qux"}},
        )
        assert resp["Environment"]["Variables"]["FOO"] == "bar"
        assert resp["Environment"]["Variables"]["BAZ"] == "qux"

    def test_should_update_handler(self, test_function, lamb):
        resp = lamb.update_function_configuration(
            FunctionName=test_function,
            Handler="app.main",
        )
        assert resp["Handler"] == "app.main"


class TestUpdateFunctionCode:
    def test_should_update_code(self, test_function, lamb):
        new_code = make_zip("def handler(e, c): return 'v2'\n")
        resp = lamb.update_function_code(
            FunctionName=test_function,
            ZipFile=new_code,
        )
        assert resp["FunctionName"] == test_function
        assert resp["CodeSize"] > 0

    def test_should_update_code_with_publish(self, test_function, lamb):
        new_code = make_zip("def handler(e, c): return 'v2'\n")
        resp = lamb.update_function_code(
            FunctionName=test_function,
            ZipFile=new_code,
            Publish=True,
        )
        assert resp["Version"] == "1"
