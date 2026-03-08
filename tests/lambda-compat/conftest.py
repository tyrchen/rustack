"""Pytest configuration for Lambda compatibility tests."""

import boto3
import botocore
import pytest

from util import unique_name, make_zip


def pytest_addoption(parser):
    parser.addoption(
        "--url",
        action="store",
        default="http://localhost:4566",
        help="Lambda endpoint URL (default: http://localhost:4566)",
    )
    parser.addoption(
        "--aws",
        action="store_true",
        default=False,
        help="Run against real AWS Lambda (requires credentials)",
    )


@pytest.fixture(scope="session")
def endpoint_url(request):
    """Return the endpoint URL for the Lambda service."""
    if request.config.getoption("--aws"):
        return None
    return request.config.getoption("--url")


@pytest.fixture(scope="session")
def lamb(endpoint_url):
    """Session-scoped boto3 Lambda client."""
    config = botocore.config.Config(
        retries={"max_attempts": 0},
        read_timeout=60,
    )
    kwargs = {
        "service_name": "lambda",
        "region_name": "us-east-1",
        "config": config,
    }
    if endpoint_url is not None:
        kwargs["endpoint_url"] = endpoint_url
        kwargs["aws_access_key_id"] = "test"
        kwargs["aws_secret_access_key"] = "test"
    return boto3.client(**kwargs)


@pytest.fixture
def test_function(lamb):
    """Create a test function and delete it afterwards."""
    name = unique_name("compat")
    lamb.create_function(
        FunctionName=name,
        Runtime="python3.12",
        Role="arn:aws:iam::000000000000:role/test-role",
        Handler="index.handler",
        Code={"ZipFile": make_zip()},
    )
    yield name
    try:
        lamb.delete_function(FunctionName=name)
    except Exception:
        pass


@pytest.fixture
def function_factory(lamb):
    """Factory fixture that creates functions and cleans them up after the test."""
    created = []

    def _create(prefix="fac", **overrides):
        name = unique_name(prefix)
        params = {
            "FunctionName": name,
            "Runtime": "python3.12",
            "Role": "arn:aws:iam::000000000000:role/test-role",
            "Handler": "index.handler",
            "Code": {"ZipFile": make_zip()},
        }
        params.update(overrides)
        lamb.create_function(**params)
        created.append(name)
        return name

    yield _create

    for name in created:
        try:
            lamb.delete_function(FunctionName=name)
        except Exception:
            pass
