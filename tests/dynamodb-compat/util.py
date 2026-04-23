# Standalone utility module compatible with ScyllaDB Alternator test suite.
# Provides the same API as test/alternator/util.py without ScyllaDB-internal
# dependencies, allowing the Alternator tests to run against any DynamoDB-
# compatible endpoint (e.g., RustStack).

import string
import random
import time
import json
import contextlib
from collections import Counter

import boto3
import botocore
import pytest
import requests


# ---------------------------------------------------------------------------
# Random data generators
# ---------------------------------------------------------------------------

_random = random.Random(42)


def random_string(length=10):
    """Generate a random string of given length."""
    return "".join(_random.choices(string.ascii_letters + string.digits, k=length))


def random_bytes(length=10):
    """Generate random bytes of given length."""
    return bytearray(_random.getrandbits(8) for _ in range(length))


# ---------------------------------------------------------------------------
# Table helpers
# ---------------------------------------------------------------------------


def unique_table_name(prefix="alternator_Test_"):
    """Generate a unique table name with a timestamp-based suffix."""
    return f"{prefix}{int(time.time() * 1000)}_{random_string(8)}"


def create_test_table(
    dynamodb, name=None, BillingMode="PAY_PER_REQUEST", wait_for_active=True, **kwargs
):
    """Create a DynamoDB table for testing.

    If no name is given, a unique name is generated. The table is created
    with PAY_PER_REQUEST billing by default. If ``wait_for_active`` is True,
    waits for the table to become ACTIVE before returning.
    """
    if name is None:
        name = unique_table_name()
    table = dynamodb.create_table(
        TableName=name,
        BillingMode=BillingMode,
        **kwargs,
    )
    if wait_for_active:
        # Wait up to 60 seconds for the table to become ACTIVE.
        waiter = dynamodb.meta.client.get_waiter("table_exists")
        waiter.wait(
            TableName=name,
            WaiterConfig={"Delay": 1, "MaxAttempts": 60},
        )
    return table


@contextlib.contextmanager
def new_test_table(dynamodb, **kwargs):
    """Context manager: create a table, yield it, then delete it."""
    table = create_test_table(dynamodb, **kwargs)
    try:
        yield table
    finally:
        table.delete()


# ---------------------------------------------------------------------------
# Full-scan / full-query helpers (handle pagination)
# ---------------------------------------------------------------------------


def full_scan(table, **kwargs):
    """Perform a Scan that pages through all results."""
    items = []
    response = table.scan(**kwargs)
    items.extend(response.get("Items", []))
    while "LastEvaluatedKey" in response:
        kwargs["ExclusiveStartKey"] = response["LastEvaluatedKey"]
        response = table.scan(**kwargs)
        items.extend(response.get("Items", []))
    return items


def full_scan_and_count(table, **kwargs):
    """Like full_scan but also returns the server-side Count."""
    items = []
    count = 0
    response = table.scan(**kwargs)
    items.extend(response.get("Items", []))
    count += response.get("Count", 0)
    while "LastEvaluatedKey" in response:
        kwargs["ExclusiveStartKey"] = response["LastEvaluatedKey"]
        response = table.scan(**kwargs)
        items.extend(response.get("Items", []))
        count += response.get("Count", 0)
    return count, items


def full_query(table, **kwargs):
    """Perform a Query that pages through all results."""
    items = []
    response = table.query(**kwargs)
    items.extend(response.get("Items", []))
    while "LastEvaluatedKey" in response:
        kwargs["ExclusiveStartKey"] = response["LastEvaluatedKey"]
        response = table.query(**kwargs)
        items.extend(response.get("Items", []))
    return items


def full_query_and_counts(table, **kwargs):
    """Like full_query but also returns ScannedCount, Count, page count, and items."""
    items = []
    prefilter_count = 0
    postfilter_count = 0
    pages = 0
    response = table.query(**kwargs)
    items.extend(response.get("Items", []))
    pages += 1
    postfilter_count += response.get("Count", 0)
    prefilter_count += response.get("ScannedCount", 0)
    while "LastEvaluatedKey" in response:
        kwargs["ExclusiveStartKey"] = response["LastEvaluatedKey"]
        response = table.query(**kwargs)
        items.extend(response.get("Items", []))
        pages += 1
        postfilter_count += response.get("Count", 0)
        prefilter_count += response.get("ScannedCount", 0)
    return (prefilter_count, postfilter_count, pages, items)


# ---------------------------------------------------------------------------
# Comparison helpers
# ---------------------------------------------------------------------------


def _freeze(item):
    """Recursively convert a DynamoDB item (dict/list/set) to a hashable form."""
    if isinstance(item, dict):
        return frozenset((k, _freeze(v)) for k, v in item.items())
    if isinstance(item, (list, tuple)):
        return tuple(_freeze(v) for v in item)
    if isinstance(item, set):
        return frozenset(_freeze(v) for v in item)
    return item


def multiset(items):
    """Convert a list of items to a Counter for order-independent comparison."""
    return Counter(_freeze(item) for item in items)


# ---------------------------------------------------------------------------
# Table listing (paginated)
# ---------------------------------------------------------------------------


def list_tables(dynamodb, limit=100):
    """List all table names, handling pagination."""
    tables = []
    response = dynamodb.meta.client.list_tables(Limit=limit)
    tables.extend(response.get("TableNames", []))
    while "LastEvaluatedTableName" in response:
        response = dynamodb.meta.client.list_tables(
            Limit=limit,
            ExclusiveStartTableName=response["LastEvaluatedTableName"],
        )
        tables.extend(response.get("TableNames", []))
    return tables


# ---------------------------------------------------------------------------
# GSI helpers
# ---------------------------------------------------------------------------


def wait_for_gsi(table, gsi_name, timeout=60):
    """Wait until a GSI becomes ACTIVE on the given table."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        desc = table.meta.client.describe_table(TableName=table.name)
        for gsi in desc["Table"].get("GlobalSecondaryIndexes", []):
            if gsi["IndexName"] == gsi_name and gsi["IndexStatus"] == "ACTIVE":
                return
        time.sleep(1)
    raise TimeoutError(f"GSI {gsi_name} did not become ACTIVE within {timeout}s")


def wait_for_gsi_gone(table, gsi_name, timeout=60):
    """Wait until a GSI is fully removed from the given table."""
    deadline = time.time() + timeout
    while time.time() < deadline:
        desc = table.meta.client.describe_table(TableName=table.name)
        gsis = desc["Table"].get("GlobalSecondaryIndexes", [])
        if not any(g["IndexName"] == gsi_name for g in gsis):
            return
        time.sleep(1)
    raise TimeoutError(f"GSI {gsi_name} was not removed within {timeout}s")


# ---------------------------------------------------------------------------
# AWS detection
# ---------------------------------------------------------------------------


def is_aws(dynamodb):
    """Return True if the endpoint looks like a real AWS DynamoDB service."""
    endpoint = dynamodb.meta.client._endpoint.host
    return endpoint.endswith(".amazonaws.com")


# ---------------------------------------------------------------------------
# Skip helpers (stand-ins for test.pylib.skip_types)
# ---------------------------------------------------------------------------


def skip_env(reason):
    """Runtime skip for a missing environment requirement."""
    pytest.skip(reason)


def skip_bug(reason):
    """Runtime skip for a known bug."""
    pytest.skip(reason)


def skip_not_implemented(reason):
    """Runtime skip for a feature that is not yet implemented."""
    pytest.skip(reason)


def skip_slow(reason):
    """Runtime skip for a test that is too slow for regular CI."""
    pytest.skip(reason)


# ---------------------------------------------------------------------------
# ScyllaDB-specific stubs (no-ops for non-Scylla targets)
# ---------------------------------------------------------------------------


def scylla_log(dynamodb, *args, **kwargs):
    """Stub: ScyllaDB log access. No-op on non-Scylla endpoints."""


def scylla_config_read(dynamodb, *args, **kwargs):
    """Stub: ScyllaDB config read. Returns empty dict on non-Scylla endpoints."""
    return {}


@contextlib.contextmanager
def scylla_config_temporary(dynamodb, *args, **kwargs):
    """Stub: ScyllaDB temporary config change. No-op context manager."""
    yield


def scylla_inject_error(rest_api, *args, **kwargs):
    """Stub: ScyllaDB error injection. No-op on non-Scylla endpoints."""


@contextlib.contextmanager
def client_no_transform(dynamodb):
    """Create a low-level DynamoDB client without number transformation.

    boto3 by default transforms DynamoDB number strings (e.g., "123") into
    Python Decimal objects. This client skips that transformation, returning
    raw DynamoDB JSON responses.

    Accepts either a boto3 DynamoDB resource or a botocore DynamoDB client.
    """
    # Resolve the underlying client: if dynamodb is a resource it has
    # .meta.client; if it is already a client, .meta is ClientMeta which
    # has no .client attribute, so use dynamodb directly.
    if hasattr(dynamodb.meta, "client"):
        underlying = dynamodb.meta.client
    else:
        underlying = dynamodb
    endpoint = underlying._endpoint.host
    config = botocore.client.Config(
        parameter_validation=False,
        retries={"max_attempts": 0},
        read_timeout=300,
    )
    creds = underlying._request_signer._credentials.get_frozen_credentials()
    client = boto3.client(
        "dynamodb",
        endpoint_url=endpoint,
        region_name="us-east-1",
        aws_access_key_id=creds.access_key,
        aws_secret_access_key=creds.secret_key,
        config=config,
    )
    try:
        yield client
    finally:
        client.close()


def manual_request(dynamodb, target, payload, timeout=30):
    """Send a raw DynamoDB JSON request to the endpoint.

    ``target`` is the X-Amz-Target value (e.g., 'DynamoDB_20120810.PutItem').
    ``payload`` is the JSON body as a Python dict.
    """
    endpoint = dynamodb.meta.client._endpoint.host
    creds = dynamodb.meta.client._request_signer._credentials.get_frozen_credentials()
    headers = {
        "Content-Type": "application/x-amz-json-1.0",
        "X-Amz-Target": target,
    }
    response = requests.post(
        endpoint,
        headers=headers,
        data=json.dumps(payload),
        timeout=timeout,
    )
    return response
