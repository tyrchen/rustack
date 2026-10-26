"""Pinned ordinary boto3 acceptance of the actual Rustack ASL/Native runtime."""

import copy
import io
import json
import platform
import sys
import time
import uuid
import zipfile
from concurrent.futures import ThreadPoolExecutor
from datetime import datetime
from pathlib import Path
from urllib.parse import urlsplit

import boto3
from stepfunctions_structured import exercise as structured_exercise
import botocore
from botocore.config import Config
from botocore.exceptions import ClientError

assert boto3.__version__ == "1.43.118" and botocore.__version__ == "1.43.118"
ENDPOINT, BOOTSTRAP, MARKERS = sys.argv[1:]
assert urlsplit(ENDPOINT).scheme == "http" and urlsplit(ENDPOINT).hostname == "127.0.0.1"
ROLE = "arn:aws:iam::000000000000:role/Workflows"
PREFIX = "arn:aws:states:us-east-1:000000000000:"
ACTIONS = set()


def client(service, secret="local-secret"):
    return boto3.client(
        service, endpoint_url=ENDPOINT, region_name="us-east-1",
        aws_access_key_id="local-key", aws_secret_access_key=secret,
        config=Config(signature_version="v4", retries={"total_max_attempts": 1}, connect_timeout=2, read_timeout=15),
    )


def call(sfn, action, **args):
    result = getattr(sfn, action)(**args)
    ACTIONS.add(action)
    assert 200 <= result["ResponseMetadata"]["HTTPStatusCode"] < 300
    uuid.UUID(result["ResponseMetadata"]["RequestId"])
    return result


def rejected(sfn, action, code, **args):
    try:
        call(sfn, action, **args)
    except ClientError as error:
        assert error.response["Error"]["Code"] == code, error.response
        return
    raise AssertionError(f"{action} unexpectedly accepted")


def deploy(lam):
    bootstrap = Path(BOOTSTRAP).read_bytes()
    assert 0 < len(bootstrap) <= 32 * 1024 * 1024
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w") as archive:
        info = zipfile.ZipInfo("bootstrap")
        info.external_attr = 0o755 << 16
        archive.writestr(info, bootstrap)
    for name, env, timeout in [
        ("SdkCompute", {"WORKFLOW_MODE": "1"}, 5),
        ("SdkRetry", {"WORKFLOW_MODE": "1", "WORKFLOW_FAIL_ONCE": "1"}, 5),
        ("SdkFail", {"FAIL_MODE": "panic"}, 5),
        ("SdkSlow", {"SLEEP_SECS": "5"}, 5),
        ("SdkFunctionTimeout", {"SLEEP_SECS": "5"}, 1),
    ]:
        lam.create_function(FunctionName=name, Runtime="provided.al2023", Role=ROLE, Handler="bootstrap",
                            Code={"ZipFile": buffer.getvalue()}, Timeout=timeout,
                            Architectures=["arm64" if platform.machine().lower() in ("arm64", "aarch64") else "x86_64"],
                            Environment={"Variables": {**env, "PROCESS_MARKER_DIR": MARKERS}})
    gates = Path(MARKERS).parent / "gates"
    lam.create_function(FunctionName="SdkGate", Runtime="provided.al2023", Role=ROLE, Handler="bootstrap",
                        Code={"ZipFile": buffer.getvalue()}, Timeout=900,
                        Architectures=["arm64" if platform.machine().lower() in ("arm64", "aarch64") else "x86_64"],
                        Environment={"Variables": {"WORKFLOW_MODE": "1", "PROCESS_MARKER_DIR": MARKERS,
                                                  "INVOCATION_FILE": str(gates / "entered"),
                                                  "WORKFLOW_GATE_FILE": str(gates / "release")}})
    response = lam.invoke(FunctionName="SdkCompute", Payload=b'{"n":21}')
    try:
        output = json.loads(response["Payload"].read(262145))
        assert response["StatusCode"] == 200 and "FunctionError" not in response
        assert output["computed"] == 42 and output["pid"] > 1
    finally:
        response["Payload"].close()


def create(sfn, name, definition):
    result = call(sfn, "create_state_machine", name=name, roleArn=ROLE, definition=json.dumps(definition))
    assert result["stateMachineArn"].startswith(PREFIX + "stateMachine:")
    assert isinstance(result["creationDate"], datetime)
    assert abs(result["creationDate"].timestamp() - time.time()) < 60
    return result["stateMachineArn"]


def terminal(sfn, arn):
    deadline = time.monotonic() + 20
    while time.monotonic() < deadline:
        result = call(sfn, "describe_execution", executionArn=arn)
        if result["status"] != "RUNNING":
            return result
        time.sleep(0.015)
    raise AssertionError("execution did not terminate")


def history(sfn, arn, reverse=False, data=True):
    events, tokens = [], set()
    token = None
    while True:
        params = dict(executionArn=arn, maxResults=7, reverseOrder=reverse, includeExecutionData=data)
        if token is not None:
            params["nextToken"] = token
        page = call(sfn, "get_execution_history", **params)
        events.extend(page["events"])
        assert len(events) <= 1000
        token = page.get("nextToken")
        if token is None:
            return events
        assert token not in tokens and len(tokens) < 200
        tokens.add(token)


def assert_omitted(events):
    for event in events:
        for details in ["executionStartedEventDetails", "executionSucceededEventDetails", "stateEnteredEventDetails",
                        "stateExitedEventDetails", "taskSucceededEventDetails", "lambdaFunctionScheduledEventDetails",
                        "lambdaFunctionSucceededEventDetails"]:
            if details in event:
                assert not {"input", "output", "inputDetails", "outputDetails"} & set(event[details])
        assert "taskScheduledEventDetails" not in event


def pagination(sfn, arn, events):
    reverse = history(sfn, arn, reverse=True, data=False)
    assert [(event["id"], event.get("previousEventId", 0)) for event in reverse] == [(event["id"], event.get("previousEventId", 0)) for event in reversed(events)]
    assert_omitted(reverse)
    first = call(sfn, "get_execution_history", executionArn=arn, maxResults=1)
    token = first["nextToken"]
    next_page = call(sfn, "get_execution_history", executionArn=arn, maxResults=1, nextToken=token)
    replay = call(sfn, "get_execution_history", executionArn=arn, maxResults=1, nextToken=token)
    assert next_page["events"] == replay["events"]
    rejected(sfn, "get_execution_history", "InvalidToken", executionArn=arn, maxResults=2, nextToken=token)
    rejected(sfn, "get_execution_history", "InvalidToken", executionArn=arn, nextToken="0" * 64)


def catalog(sfn):
    fixture = json.loads(Path(__file__).with_name("sdk-cases.json").read_text())
    assert fixture["schemaVersion"] == 1 and fixture["profile"] == "rustack-standard-jsonpath-v1"
    assert len(fixture["cases"]) == 17
    identifiers = set()
    for case in fixture["cases"]:
        allowed = {"id", "definition", "input", "expectedOutput", "uuidField", "optimizedComputed", "taskAttempts", "expectedStatus", "expectedError", "expectedCause", "largeInputBytes", "evidence"}
        assert set(case) <= allowed and {"id", "definition", "input", "evidence"} <= set(case)
        assert case["id"] not in identifiers
        identifiers.add(case["id"])
        assert sum(key in case for key in ("expectedOutput", "optimizedComputed", "expectedError")) == 1
        assert case["evidence"]["sources"] and set(case["evidence"]["sources"]) <= set(fixture["sources"])
        value = copy.deepcopy(case["input"])
        if "largeInputBytes" in case:
            value["large"] = "x" * case["largeInputBytes"]
        machine = create(sfn, "SdkCase-" + case["id"], case["definition"])
        raw = json.dumps(value, separators=(",", ":"), ensure_ascii=False)
        started = call(sfn, "start_execution", stateMachineArn=machine, name="Run", input=raw)
        result = terminal(sfn, started["executionArn"])
        assert result["status"] == case.get("expectedStatus", "SUCCEEDED"), (case["id"], result)
        assert result["input"] == raw
        assert isinstance(started["startDate"], datetime) and result["startDate"] == started["startDate"]
        assert result["stopDate"] >= result["startDate"]
        if "expectedOutput" in case:
            output = json.loads(result["output"])
            if "uuidField" in case:
                assert uuid.UUID(output.pop(case["uuidField"])).version == 4
            assert output == case["expectedOutput"], (case["id"], output)
        if "optimizedComputed" in case:
            output = json.loads(result["output"])
            assert output["Payload"]["computed"] == case["optimizedComputed"]
            assert output["Payload"]["pid"] > 1
            assert output["StatusCode"] == 200 and output["SdkHttpMetadata"]["HttpStatusCode"] == 200
            assert output["ExecutedVersion"] == "$LATEST"
            assert output["SdkResponseMetadata"]["RequestId"] == output["Payload"]["request_id"]
            uuid.UUID(output["Payload"]["request_id"])
        if "expectedError" in case:
            assert result["error"] == case["expectedError"], (case["id"], result)
        if "expectedCause" in case:
            assert result["cause"] == case["expectedCause"]
        events = history(sfn, started["executionArn"])
        assert events
        for index, event in enumerate(events, 1):
            assert event["id"] == index and event.get("previousEventId", 0) == index - 1
            assert isinstance(event["timestamp"], datetime) and event["timestamp"] >= result["startDate"]
        expected_terminal = {"SUCCEEDED": "ExecutionSucceeded", "FAILED": "ExecutionFailed", "TIMED_OUT": "ExecutionTimedOut"}
        assert events[-1]["type"] == expected_terminal[result["status"]]
        if "taskAttempts" in case:
            assert sum(event["type"].endswith("Scheduled") for event in events) == case["taskAttempts"]
        if case["id"] == "optimized":
            pagination(sfn, started["executionArn"], events)
    return len(fixture["cases"])


def pure(value):
    return {"StartAt": "Done", "States": {"Done": {"Type": "Pass", "Result": {"revision": value}, "End": True}}}


def controls(sfn):
    old = {"StartAt": "Gate", "States": {"Gate": {"Type": "Task", "Resource": "arn:aws:lambda:us-east-1:000000000000:function:SdkGate",
                                                     "Parameters": {"n": 21}, "ResultSelector": {"revision": "old"}, "End": True}}}
    machine = create(sfn, "SdkControl", old)
    described = call(sfn, "describe_state_machine", stateMachineArn=machine)
    assert described["definition"] == json.dumps(old)
    call(sfn, "tag_resource", resourceArn=machine, tags=[{"key": "owner", "value": "sdk"}])
    assert call(sfn, "list_tags_for_resource", resourceArn=machine)["tags"] == [{"key": "owner", "value": "sdk"}]
    call(sfn, "untag_resource", resourceArn=machine, tagKeys=["owner"])
    assert call(sfn, "list_tags_for_resource", resourceArn=machine)["tags"] == []
    raw = ' {"n":1}'
    def start():
        return call(sfn, "start_execution", stateMachineArn=machine, name="Same", input=raw)
    with ThreadPoolExecutor(max_workers=2) as workers:
        left, right = list(workers.map(lambda _: start(), range(2)))
    assert left["executionArn"] == right["executionArn"] and left["startDate"] == right["startDate"]
    rejected(sfn, "start_execution", "ExecutionAlreadyExists", stateMachineArn=machine, name="Same", input="{}")
    rejected(sfn, "start_execution", "InvalidExecutionInput", stateMachineArn=machine, name="Same", input="{")
    entered = Path(MARKERS).parent / "gates" / "entered"
    cutoff = time.monotonic() + 15
    while True:
        try:
            raw_marker = entered.read_bytes()
            assert len(raw_marker) <= 512
            marker = json.loads(raw_marker)
            assert marker["pid"] > 1
            uuid.UUID(marker["request_id"])
            break
        except (FileNotFoundError, json.JSONDecodeError):
            assert time.monotonic() < cutoff, "handler did not consume the event"
            time.sleep(0.005)
    uuid.UUID(described["revisionId"])
    updated = call(sfn, "update_state_machine", stateMachineArn=machine, definition=json.dumps(pure("new")))
    uuid.UUID(updated["revisionId"])
    assert updated["revisionId"] != described["revisionId"]
    captured = call(sfn, "describe_state_machine_for_execution", executionArn=left["executionArn"])
    uuid.UUID(captured["revisionId"])
    assert captured["definition"] == json.dumps(old) and captured["revisionId"] == described["revisionId"]
    assert captured["updateDate"] == described["creationDate"]
    new = call(sfn, "start_execution", stateMachineArn=machine, name="New")
    assert json.loads(terminal(sfn, new["executionArn"])["output"]) == {"revision": "new"}
    (Path(MARKERS).parent / "gates" / "release").write_bytes(b"release")
    assert json.loads(terminal(sfn, left["executionArn"])["output"]) == {"revision": "old"}
    rejected(sfn, "start_execution", "ExecutionAlreadyExists", stateMachineArn=machine, name="Same", input=raw)
    wait = {"StartAt": "Wait", "States": {"Wait": {"Type": "Wait", "Seconds": 3600, "End": True}}}
    call(sfn, "update_state_machine", stateMachineArn=machine, definition=json.dumps(wait))
    stopped = call(sfn, "start_execution", stateMachineArn=machine, name="Stop")
    call(sfn, "stop_execution", executionArn=stopped["executionArn"], error="", cause='quoted "stop" 🙂')
    result = terminal(sfn, stopped["executionArn"])
    assert result["status"] == "ABORTED" and result["error"] == "" and result["cause"] == 'quoted "stop" 🙂'
    page = call(sfn, "list_executions", stateMachineArn=machine, maxResults=1)
    captures = {item["executionArn"] for item in page["executions"]}
    token = page.get("nextToken")
    call(sfn, "update_state_machine", stateMachineArn=machine, definition=json.dumps(pure("late")))
    late = call(sfn, "start_execution", stateMachineArn=machine, name="Late")
    terminal(sfn, late["executionArn"])
    first_history = call(sfn, "get_execution_history", executionArn=left["executionArn"], maxResults=1)
    call(sfn, "delete_state_machine", stateMachineArn=machine)
    seen = set()
    while token is not None:
        assert token not in seen and len(seen) < 10
        seen.add(token)
        page = call(sfn, "list_executions", stateMachineArn=machine, maxResults=1, nextToken=token)
        captures.update(item["executionArn"] for item in page["executions"])
        token = page.get("nextToken")
    assert len(captures) == 3 and late["executionArn"] not in captures
    call(sfn, "get_execution_history", executionArn=left["executionArn"], maxResults=1, nextToken=first_history["nextToken"])
    assert call(sfn, "describe_execution", executionArn=left["executionArn"])["input"] == raw
    machines = []
    for count, page in enumerate(sfn.get_paginator("list_state_machines").paginate(maxResults=2), 1):
        assert count <= 64
        ACTIONS.add("list_state_machines")
        machines.extend(item["name"] for item in page["stateMachines"])
        assert len(machines) < 64
    assert "SdkCase-optimized" in machines and "SdkControl" not in machines
    assert call(sfn, "validate_state_machine_definition", definition=json.dumps(pure("valid")), severity="WARNING")["result"] == "OK"
    for definition in [
        {"StartAt": "Missing", "States": {}},
        {"StartAt": "Run", "States": {"Run": {"Type": "Pass", "InputPath": "$[", "End": True}}},
        {"StartAt": "Run", "States": {"Run": {"Type": "Pass", "Parameters": {"duplicate": 1, "duplicate.$": "$.n"}, "End": True}}},
    ]:
        invalid = call(sfn, "validate_state_machine_definition", definition=json.dumps(definition))
        assert invalid["result"] == "FAIL" and invalid["diagnostics"]
    rejected(sfn, "create_activity", "UnsupportedOperation", name="unsupported")


sfn, lam, bad = client("stepfunctions"), client("lambda"), client("stepfunctions", "wrong-local-secret")
try:
    rejected(bad, "list_state_machines", "AccessDeniedException")
    deploy(lam)
    cases = catalog(sfn)
    controls(sfn)
    structured_cases = structured_exercise(sfn, lam, BOOTSTRAP, MARKERS, create, call, terminal, history, assert_omitted)
    required = {"create_state_machine", "describe_state_machine", "update_state_machine", "delete_state_machine",
                "list_state_machines", "start_execution", "stop_execution", "describe_execution",
                "describe_state_machine_for_execution", "list_executions", "get_execution_history",
                "validate_state_machine_definition", "tag_resource", "untag_resource", "list_tags_for_resource"}
    assert required <= ACTIONS, required - ACTIONS
    print(json.dumps({"realBinary": True, "actualNative": True, "strictAuth": True, "catalogCases": cases,
                      "m1Actions": len(required), "structuredCases": structured_cases, "structuredLifecycle": 2, "cloudDifferential": False, "dockerQualified": False}))
finally:
    sfn.close()
    lam.close()
    bad.close()
