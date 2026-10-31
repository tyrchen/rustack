"""Ordinary signed Node24 image consumption; importing performs no SDK IO."""
import base64
import json
import os
import re
import sys
import uuid

from lambda_fault_support import Metadata, invoke_json, poll_response


def invoke(client, name, value, *, qualifier=None, fail=False, logs=False):
    arguments = {}
    if qualifier is not None:
        arguments["Qualifier"] = qualifier
    if logs:
        arguments["LogType"] = "Tail"
    return invoke_json(client, name, {"value": value, "fail": fail, "logs": logs}, **arguments)


def check(output, name, architecture, expected, counter, version, handler):
    assert output["computed"] == expected and output["counter"] == counter
    assert output["runtime"] == "node24" and re.fullmatch(r"24\.\d+\.\d+", output["nodeVersion"])
    assert output["architecture"] == {"arm64": "arm64", "x86_64": "x64"}[architecture]
    assert output["uid"] == 1000 and output["rootReadOnly"] is True
    assert output["temporaryRoundtrip"] is True and output["temporaryMode"] == 0o600
    assert output["functionName"] == name and output["functionVersion"] == version
    assert output["memoryMb"] == "256" and output["configuredValue"] == "node-configured"
    assert output["handler"] == handler
    uuid.UUID(output["requestId"])


def deployed(client, name):
    def accepted(value):
        assert value["State"] != "Failed" and value.get("LastUpdateStatus") != "Failed"
        return value["State"] == "Active" and value.get("LastUpdateStatus", "Successful") == "Successful"
    return poll_response(lambda: client.get_function_configuration(FunctionName=name), accepted, 120)


def observe_environments(metadata, stage):
    rows = []
    for row in metadata.rows():
        if row["Labels"]["rustack.lambda.role"] != "function":
            continue
        inspected = metadata.get("/containers/" + row["Id"] + "/json")
        version = [entry.split("=", 1)[1] for entry in inspected["Config"]["Env"]
                   if entry.startswith("AWS_LAMBDA_FUNCTION_VERSION=")]
        state = inspected["State"]
        observed = {"id": row["Id"], "version": version,
                    "command": inspected["Config"]["Cmd"], "running": state["Running"],
                    "exitCode": state["ExitCode"], "oom": state["OOMKilled"]}
        rows.append(observed)
    print(json.dumps({"nodeEnvironmentObservation": stage, "functions": rows}), file=sys.stderr, flush=True)
    assert rows and all(row["running"] for row in rows), "Node function unexpectedly exited"


def exercise(endpoint, image, architecture, socket, owner, engine):
    import boto3
    from botocore.config import Config
    settings = dict(endpoint_url=endpoint, region_name="us-east-1",
                    aws_access_key_id="local-key", aws_secret_access_key="local-secret",
                    config=Config(retries={"max_attempts": 0}, connect_timeout=2, read_timeout=60))
    client = boto3.client("lambda", **settings)
    states = boto3.client("stepfunctions", **settings)
    name = "NodeOfficial" + uuid.uuid4().hex
    metadata = Metadata(socket, owner, engine)
    try:
        created = client.create_function(FunctionName=name,
            Role="arn:aws:iam::000000000000:role/NodeFixture", PackageType="Image",
            Architectures=[architecture], Timeout=30, MemorySize=256, Code={"ImageUri": image},
            Environment={"Variables": {"TEST_VALUE": "node-configured"}})
        arn = created["FunctionArn"]
        deployed(client, name)
        for value, expected, counter in ((21, 42, 1), (23, 46, 2)):
            response, output = invoke(client, name, value, logs=counter == 1)
            assert "FunctionError" not in response
            check(output, name, architecture, expected, counter, "$LATEST", "handler")
            assert output["invokedArn"] == arn
            if counter == 1:
                tail = base64.b64decode(response["LogResult"], validate=True)
                assert len(tail) <= 4096 and b"NODE_STDOUT_CAPTURE" in tail and b"NODE_STDERR_CAPTURE" in tail
        version = client.publish_version(FunctionName=name)["Version"]
        assert version == "1"
        client.create_alias(FunctionName=name, Name="live", FunctionVersion=version)
        response, output = invoke(client, name, 7, qualifier="live")
        assert "FunctionError" not in response and response["ExecutedVersion"] == version
        check(output, name, architecture, 14, 1, version, "handler")
        assert output["invokedArn"] == arn + ":live"
        client.update_function_configuration(FunctionName=name, ImageConfig={"Command": ["node.alternate"]})
        configuration = deployed(client, name)
        assert configuration["ImageConfigResponse"]["ImageConfig"]["Command"] == ["node.alternate"]
        response, output = invoke(client, name, 7)
        assert "FunctionError" not in response
        check(output, name, architecture, 21, 1, "$LATEST", "alternate")
        response, output = invoke(client, name, 7, qualifier=version)
        assert "FunctionError" not in response and response["ExecutedVersion"] == version
        check(output, name, architecture, 14, 2, version, "handler")
        assert output["invokedArn"] == arn + ":1"
        response, output = invoke(client, name, 7, qualifier="live")
        assert "FunctionError" not in response and response["ExecutedVersion"] == version
        check(output, name, architecture, 14, 3, version, "handler")
        assert output["invokedArn"] == arn + ":live"
        observe_environments(metadata, "before-latest-error")
        response, output = invoke(client, name, 7, fail=True)
        assert response["FunctionError"] == "Unhandled" and output["errorType"] == "NodeFixtureError"
        observe_environments(metadata, "after-latest-error")
        for value, expected, counter in ((21, 63, 1), (23, 69, 2)):
            response, output = invoke(client, name, value)
            assert "FunctionError" not in response
            check(output, name, architecture, expected, counter, "$LATEST", "alternate")
        observe_environments(metadata, "after-latest-recovery")
        response, output = invoke(client, name, 7, qualifier="live")
        assert "FunctionError" not in response and response["ExecutedVersion"] == version
        check(output, name, architecture, 14, 4, version, "handler")
        assert output["invokedArn"] == arn + ":live"
        definition = {"StartAt": "Node", "States": {"Node": {"Type": "Task",
            "Resource": "arn:aws:states:::lambda:invoke", "Parameters": {
                "FunctionName": arn + ":live", "Payload.$": "$"},
            "OutputPath": "$.Payload", "End": True}}}
        machine = states.create_state_machine(name="NodeWorkflow" + uuid.uuid4().hex,
            roleArn="arn:aws:iam::000000000000:role/NodeFixture", definition=json.dumps(definition))["stateMachineArn"]
        execution = states.start_execution(stateMachineArn=machine, input='{"value":21}')["executionArn"]
        terminal = poll_response(lambda: states.describe_execution(executionArn=execution),
                                 lambda value: value["status"] != "RUNNING", 120)
        assert terminal["status"] == "SUCCEEDED", {
            "error": terminal.get("error"), "cause": terminal.get("cause")}
        output = json.loads(terminal["output"])
        assert output["computed"] == 42 and output["handler"] == "handler" and output["counter"] == 5
        assert output["functionVersion"] == version and output["invokedArn"] == arn + ":live"
        states.delete_state_machine(stateMachineArn=machine)
        client.delete_function(FunctionName=name)
        return {"schemaVersion": 1, "nodeVersion": output["nodeVersion"], "cases": [
            "cold-warm-context-filesystem-logs", "published-alias-command-isolation",
            "FunctionError-replacement-warm", "ASL-node-computation"]}
    finally:
        states.close()
        client.close()


if __name__ == "__main__":
    print(os.getpid(), flush=True)
    print(json.dumps(exercise(*sys.argv[1:])), flush=True)
