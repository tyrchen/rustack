"""Tests for Lambda alias operations."""

import botocore.exceptions
import pytest

from util import unique_name


class TestCreateAlias:
    def test_should_create_alias(self, test_function, lamb):
        lamb.publish_version(FunctionName=test_function)
        resp = lamb.create_alias(
            FunctionName=test_function,
            Name="prod",
            FunctionVersion="1",
            Description="Production alias",
        )
        assert resp["Name"] == "prod"
        assert resp["FunctionVersion"] == "1"
        assert resp["Description"] == "Production alias"
        assert "AliasArn" in resp

    def test_should_create_alias_pointing_to_latest(self, test_function, lamb):
        resp = lamb.create_alias(
            FunctionName=test_function,
            Name="dev",
            FunctionVersion="$LATEST",
        )
        assert resp["FunctionVersion"] == "$LATEST"

    def test_should_reject_duplicate_alias(self, test_function, lamb):
        lamb.publish_version(FunctionName=test_function)
        lamb.create_alias(FunctionName=test_function, Name="dup", FunctionVersion="1")
        with pytest.raises(botocore.exceptions.ClientError) as exc:
            lamb.create_alias(
                FunctionName=test_function, Name="dup", FunctionVersion="1"
            )
        assert exc.value.response["Error"]["Code"] == "ResourceConflictException"

    def test_should_reject_alias_to_nonexistent_version(self, test_function, lamb):
        with pytest.raises(botocore.exceptions.ClientError) as exc:
            lamb.create_alias(
                FunctionName=test_function, Name="bad", FunctionVersion="99"
            )
        err = exc.value.response["Error"]["Code"]
        assert err in ("ResourceNotFoundException", "InvalidParameterValueException")


class TestGetAlias:
    def test_should_get_alias(self, test_function, lamb):
        lamb.publish_version(FunctionName=test_function)
        lamb.create_alias(
            FunctionName=test_function, Name="staging", FunctionVersion="1"
        )
        resp = lamb.get_alias(FunctionName=test_function, Name="staging")
        assert resp["Name"] == "staging"
        assert resp["FunctionVersion"] == "1"

    def test_should_error_on_nonexistent_alias(self, test_function, lamb):
        with pytest.raises(botocore.exceptions.ClientError) as exc:
            lamb.get_alias(FunctionName=test_function, Name="nonexistent")
        assert exc.value.response["Error"]["Code"] == "ResourceNotFoundException"


class TestUpdateAlias:
    def test_should_update_alias_version(self, test_function, lamb):
        lamb.publish_version(FunctionName=test_function)
        lamb.publish_version(FunctionName=test_function)
        lamb.create_alias(FunctionName=test_function, Name="prod", FunctionVersion="1")
        resp = lamb.update_alias(
            FunctionName=test_function, Name="prod", FunctionVersion="2"
        )
        assert resp["FunctionVersion"] == "2"

    def test_should_update_alias_description(self, test_function, lamb):
        lamb.publish_version(FunctionName=test_function)
        lamb.create_alias(FunctionName=test_function, Name="prod", FunctionVersion="1")
        resp = lamb.update_alias(
            FunctionName=test_function,
            Name="prod",
            Description="Updated desc",
        )
        assert resp["Description"] == "Updated desc"


class TestDeleteAlias:
    def test_should_delete_alias(self, test_function, lamb):
        lamb.publish_version(FunctionName=test_function)
        lamb.create_alias(
            FunctionName=test_function, Name="todelete", FunctionVersion="1"
        )
        lamb.delete_alias(FunctionName=test_function, Name="todelete")
        with pytest.raises(botocore.exceptions.ClientError) as exc:
            lamb.get_alias(FunctionName=test_function, Name="todelete")
        assert exc.value.response["Error"]["Code"] == "ResourceNotFoundException"


class TestListAliases:
    def test_should_list_aliases(self, test_function, lamb):
        lamb.publish_version(FunctionName=test_function)
        lamb.create_alias(FunctionName=test_function, Name="prod", FunctionVersion="1")
        lamb.create_alias(
            FunctionName=test_function, Name="staging", FunctionVersion="1"
        )
        resp = lamb.list_aliases(FunctionName=test_function)
        names = [a["Name"] for a in resp["Aliases"]]
        assert "prod" in names
        assert "staging" in names

    def test_should_list_empty_when_no_aliases(self, test_function, lamb):
        resp = lamb.list_aliases(FunctionName=test_function)
        assert resp["Aliases"] == []
