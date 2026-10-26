"""Ordinary boto3 against a controlled typed HTTP handler, not an ASL engine."""

import json
import sys
import uuid

import boto3
from botocore.config import Config
from botocore.exceptions import ClientError

client = boto3.client(
    "stepfunctions",
    endpoint_url=sys.argv[1],
    region_name="us-east-1",
    aws_access_key_id="local-key",
    aws_secret_access_key="local-secret",
    config=Config(signature_version="v4", retries={"max_attempts": 0}, connect_timeout=3, read_timeout=5),
)
try:
    started = client.start_execution(
        stateMachineArn="arn:aws:states:us-east-1:000000000000:stateMachine:wire",
        name="wire",
        input=' {"n":7}',
        traceHeader="",
    )
    assert started["executionArn"].endswith(":execution:wire:execution")
    assert started["startDate"].timestamp() == -1.25
    uuid.UUID(started["ResponseMetadata"]["RequestId"])
    listed = client.list_state_machines()
    assert listed["stateMachines"] == []
    try:
        client.describe_execution(executionArn=started["executionArn"])
        raise AssertionError("expected typed fixture rejection")
    except ClientError as error:
        assert error.response["Error"]["Code"] == "ExecutionDoesNotExist"
        assert error.response["Error"]["Message"] == "fixture missing"
    try:
        client.create_activity(name="outside-profile")
        raise AssertionError("expected unsupported-operation rejection")
    except ClientError as error:
        assert error.response["Error"]["Code"] == "UnsupportedOperation"
        assert error.response["ResponseMetadata"]["HTTPStatusCode"] == 400
    print(json.dumps({"transportOnly": True, "signedCalls": 4, "epochSeconds": -1.25}))
finally:
    client.close()
