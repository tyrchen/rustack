"""Actual signed clients and managed handler SDKs through a Linux server container."""
import json
import os
import re
import sys
import time
import uuid

from handler_sdk_image import configuration_omits_sdk, inspect_sdk_origin
from lambda_fault_support import Metadata, invoke_json, poll_response
from node_image_sdk import deployed
from server_container_oracle import container_id, failure_origin, internal_origin, result, warm
from stepfunctions_nested import check_history
from stepfunctions_history import assert_omitted, history


def independent_data(s3, dynamodb, bucket, table, key, expected, literal):
    body = s3.get_object(Bucket=bucket, Key=key)["Body"]
    try:
        assert body.read(4097) == literal
    finally:
        body.close()
    item = dynamodb.get_item(TableName=table, Key={"id": {"S": key}}, ConsistentRead=True)
    assert item["Item"]["result"] == {"N": str(expected)}


def environment(metadata, gateway, previous, name, sdk_source, relay_source, network):
    pair = warm(previous, inspect_sdk_origin(metadata, gateway, "local-secret"))
    for role, cid in pair.items():
        view = metadata.get("/containers/" + cid + "/json")
        assert view["Id"] == container_id(cid) and view["State"]["Running"] is True
        assert view["Image"] == {
            "function": sdk_source, "relay": relay_source}[role]
        host = view["HostConfig"]
        assert host["ReadonlyRootfs"] is True and host["Privileged"] is False
        assert not host.get("Binds") and not host.get("Mounts") and not host.get("VolumesFrom")
        assert not host.get("PortBindings") and host["PublishAllPorts"] is False
        assert all(mount.get("Type") == "tmpfs" and mount.get("Destination") == "/tmp"
                   for mount in view.get("Mounts", []))
        if role == "function":
            assert host["NetworkMode"] == "container:" + pair["relay"]
            assert "AWS_LAMBDA_FUNCTION_NAME=" + name in view["Config"]["Env"]
        else:
            assert host["NetworkMode"] == network
    return pair


def exercise(endpoint, image, architecture, socket, owner, engine, gateway, sdk_source, relay_source, network, progress):
    import boto3
    from botocore.config import Config
    assert re.fullmatch(r"http://127\.0\.0\.1:[1-9][0-9]{0,4}", endpoint)
    assert 1 <= int(endpoint.rsplit(":", 1)[1]) <= 65535
    assert isinstance(image, str) and 0 < len(image.encode()) <= 256 and not any(c.isspace() for c in image)
    assert architecture in ("arm64", "x86_64")
    internal_origin(gateway)
    assert all(re.fullmatch(r"sha256:[0-9a-f]{64}", source) for source in (sdk_source, relay_source))
    container_id(network)
    metadata = Metadata(socket, owner, engine)
    settings = dict(endpoint_url=endpoint, region_name="us-east-1", aws_access_key_id="local-key",
                    aws_secret_access_key="local-secret",
                    config=Config(retries={"max_attempts": 0}, connect_timeout=2, read_timeout=60,
                                  s3={"addressing_style": "path"}))
    clients = []
    try:
        for service in ("lambda", "s3", "dynamodb", "stepfunctions"):
            clients.append(boto3.client(service, **settings))
        function, s3, dynamodb, sfn = clients
        suffix = uuid.uuid4().hex
        name, bucket, table = "ServerSdk" + suffix, "server-sdk-" + suffix, "ServerTable" + suffix
        progress["stage"] = "create-data"
        s3.create_bucket(Bucket=bucket)
        dynamodb.create_table(TableName=table, BillingMode="PAY_PER_REQUEST",
            KeySchema=[{"AttributeName": "id", "KeyType": "HASH"}],
            AttributeDefinitions=[{"AttributeName": "id", "AttributeType": "S"}])
        poll_response(lambda: dynamodb.describe_table(TableName=table),
                      lambda value: value["Table"]["TableStatus"] == "ACTIVE", 30)
        progress["stage"] = "create-function"
        function.create_function(FunctionName=name, Role="arn:aws:iam::000000000000:role/ServerSdk",
            PackageType="Image", Architectures=[architecture], Timeout=30, MemorySize=256,
            Code={"ImageUri": image}, ImageConfig={"Command": ["handler-sdk.handler"]},
            Environment={"Variables": {"AWS_PROFILE": "captured-poison",
                "AWS_ENDPOINT_URL": "http://127.0.0.1:9", "AWS_ENDPOINT_URL_DYNAMODB": "http://127.0.0.1:9"}})
        configuration_omits_sdk(deployed(function, name))
        pair = None
        for value, expected, counter, key, literal in (
                (21, 42, 1, "first", b"rustack-sdk:21:42"),
                (23, 46, 2, "second", b"rustack-sdk:23:46")):
            progress["stage"] = "direct-" + key
            response, output = invoke_json(function, name, dict(bucket=bucket, table=table,
                key=key, value=value, mode="roundtrip"))
            assert "FunctionError" not in response
            result(output, expected, counter, architecture)
            independent_data(s3, dynamodb, bucket, table, key, expected, literal)
            pair = environment(metadata, gateway, pair, name, sdk_source, relay_source, network)
        progress["stage"] = "nested"
        child_name, parent_name, child_run = "ServerChild" + suffix, "ServerParent" + suffix, "ChildRun" + suffix
        child_definition = {"StartAt": "Call", "TimeoutSeconds": 90, "States": {
            "Call": {"Type": "Task", "Resource": "arn:aws:states:::lambda:invoke",
                     "Parameters": {"FunctionName": name, "Payload.$": "$"},
                     "OutputPath": "$.Payload", "TimeoutSeconds": 60, "End": True}}}
        role = "arn:aws:iam::000000000000:role/ServerSdk"
        child = sfn.create_state_machine(name=child_name, roleArn=role,
                                        definition=json.dumps(child_definition))["stateMachineArn"]
        parent_definition = {"StartAt": "Child", "TimeoutSeconds": 120, "States": {
            "Child": {"Type": "Task", "Resource": "arn:aws:states:::states:startExecution.sync:2",
                      "Parameters": {"StateMachineArn": child, "Input.$": "$", "Name": child_run},
                      "OutputPath": "$.Output", "TimeoutSeconds": 90, "End": True}}}
        parent = sfn.create_state_machine(name=parent_name, roleArn=role,
                                         definition=json.dumps(parent_definition))["stateMachineArn"]
        event = dict(bucket=bucket, table=table, key="third", value=25, mode="roundtrip")
        parent_execution = sfn.start_execution(stateMachineArn=parent, name="ParentRun" + suffix,
                                              input=json.dumps(event))["executionArn"]
        terminal = poll_response(lambda: sfn.describe_execution(executionArn=parent_execution),
                                 lambda value: value["status"] != "RUNNING", 90)
        assert terminal["status"] == "SUCCEEDED"
        output = json.loads(terminal["output"])
        result(output, 50, 3, architecture)
        independent_data(s3, dynamodb, bucket, table, "third", 50, b"rustack-sdk:25:50")
        events = history(sfn, parent_execution)
        submitted = [event for event in events if event["type"] == "TaskSubmitted"]
        assert len(submitted) == 1
        child_execution = json.loads(submitted[0]["taskSubmittedEventDetails"]["output"])["ExecutionArn"]
        assert child_execution == "arn:aws:states:us-east-1:000000000000:execution:" + child_name + ":" + child_run
        assert child_execution != parent_execution
        check_history(sfn, parent_execution, ".sync:2", child_execution, history, assert_omitted)
        child_terminal = sfn.describe_execution(executionArn=child_execution)
        assert child_terminal["status"] == "SUCCEEDED" and child_terminal["stateMachineArn"] == child
        result(json.loads(child_terminal["output"]), 50, 3, architecture)
        child_events = history(sfn, child_execution)
        assert [event["type"] for event in child_events] == ["ExecutionStarted", "TaskStateEntered",
            "TaskScheduled", "TaskStarted", "TaskSucceeded", "TaskStateExited", "ExecutionSucceeded"]
        for index, child_event in enumerate(child_events):
            assert child_event["id"] == index + 1 and child_event.get("previousEventId", 0) == index
        pair = environment(metadata, gateway, pair, name, sdk_source, relay_source, network)
        progress["stage"] = "delete-data"
        configuration_omits_sdk(function.get_function_configuration(FunctionName=name))
        sfn.delete_state_machine(stateMachineArn=parent)
        sfn.delete_state_machine(stateMachineArn=child)
        for key in ("first", "second", "third"):
            s3.delete_object(Bucket=bucket, Key=key)
        s3.delete_bucket(Bucket=bucket)
        dynamodb.delete_table(TableName=table)
        # Keep this exact warm pair alive: only the server shutdown may retire it.
        metadata.wait_ids(set(pair.values()), time.monotonic() + 10)
        return {"schemaVersion": 1, "cases": ["direct-sdk-cold", "direct-sdk-warm", "nested-sync2-sdk",
                    "internal-dns-origin", "no-server-mounts", "warm-for-shutdown"],
                "versions": {"s3": "3.1105.0", "dynamodb": "3.1105.0", "node": "24.22.1"},
                "computed": [42, 46, 50], "counters": [1, 2, 3], "warm": pair,
                "parentExecution": parent_execution, "childExecution": child_execution}
    finally:
        for client in reversed(clients):
            client.close()


if __name__ == "__main__":
    print(os.getpid(), flush=True)
    progress = {"stage": "input"}
    try:
        assert len(sys.argv) == 11
        observed = exercise(*sys.argv[1:], progress)
        print(json.dumps(observed), flush=True)
    except Exception as error:
        # Fixed stages only; tracebacks could reproduce credential-bearing source.
        print("server-container SDK oracle failed at " + progress["stage"], file=sys.stderr)
        print("origin " + failure_origin(error), file=sys.stderr)
        raise SystemExit(1)
