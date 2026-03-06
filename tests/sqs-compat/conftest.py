"""Pytest configuration for SQS compatibility tests."""

import boto3
import botocore
import pytest

from util import unique_queue_name


def pytest_addoption(parser):
    parser.addoption(
        "--url",
        action="store",
        default="http://localhost:4566",
        help="SQS endpoint URL (default: http://localhost:4566)",
    )
    parser.addoption(
        "--aws",
        action="store_true",
        default=False,
        help="Run against real AWS SQS (requires credentials)",
    )


@pytest.fixture(scope="session")
def endpoint_url(request):
    """Return the endpoint URL for the SQS service."""
    if request.config.getoption("--aws"):
        return None
    return request.config.getoption("--url")


@pytest.fixture(scope="session")
def sqs(endpoint_url):
    """Session-scoped boto3 SQS client."""
    config = botocore.config.Config(
        retries={"max_attempts": 0},
        read_timeout=60,
    )
    kwargs = {
        "service_name": "sqs",
        "region_name": "us-east-1",
        "config": config,
    }
    if endpoint_url is not None:
        kwargs["endpoint_url"] = endpoint_url
        kwargs["aws_access_key_id"] = "test"
        kwargs["aws_secret_access_key"] = "test"
    return boto3.client(**kwargs)


@pytest.fixture
def test_queue(sqs):
    """Create a standard queue for a single test, delete it afterwards."""
    name = unique_queue_name("compat-std")
    resp = sqs.create_queue(QueueName=name)
    url = resp["QueueUrl"]
    yield url
    try:
        sqs.delete_queue(QueueUrl=url)
    except Exception:
        pass


@pytest.fixture
def test_fifo_queue(sqs):
    """Create a FIFO queue with content-based dedup for a single test."""
    name = unique_queue_name("compat-fifo") + ".fifo"
    resp = sqs.create_queue(
        QueueName=name,
        Attributes={
            "FifoQueue": "true",
            "ContentBasedDeduplication": "true",
        },
    )
    url = resp["QueueUrl"]
    yield url
    try:
        sqs.delete_queue(QueueUrl=url)
    except Exception:
        pass


@pytest.fixture
def queue_factory(sqs):
    """Factory fixture that creates queues and cleans them up after the test."""
    created = []

    def _create(name=None, **kwargs):
        if name is None:
            name = unique_queue_name("compat-fac")
        resp = sqs.create_queue(QueueName=name, **kwargs)
        url = resp["QueueUrl"]
        created.append(url)
        return url

    yield _create

    for url in created:
        try:
            sqs.delete_queue(QueueUrl=url)
        except Exception:
            pass
