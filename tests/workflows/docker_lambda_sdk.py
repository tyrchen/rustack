"""Real boto3 -> Lambda HTTP/provider -> Docker Executor composition evidence.

The Rust harness supplies controlled readiness for a fresh explicit-daemon scope.
This does not qualify the unfinished production startup coordinator.
"""

import base64
import json
import sys

import boto3
from botocore.config import Config
from botocore.exceptions import ClientError


def main(endpoint, image, architecture):
    client = boto3.client(
        "lambda", endpoint_url=endpoint, region_name="us-east-1",
        aws_access_key_id="test", aws_secret_access_key="test",
        config=Config(connect_timeout=10, read_timeout=120, retries={"max_attempts": 0}),
    )
    name = "executor-image-fixture"
    created = False
    try:
        client.create_function(
            FunctionName=name, PackageType="Image", Code={"ImageUri": image},
            Architectures=[architecture], MemorySize=192, Timeout=30,
            Role="arn:aws:iam::000000000000:role/lambda-test",
            Environment={"Variables": {"TEST_VALUE": "version-one"}},
        )
        created = True
        client.get_waiter("function_active_v2").wait(
            FunctionName=name, WaiterConfig={"Delay": 1, "MaxAttempts": 120},
        )
        version = client.publish_version(FunctionName=name)["Version"]
        assert version == "1"
        client.create_alias(FunctionName=name, Name="prod", FunctionVersion=version)

        def invoke(event, qualifier="prod", logs="Tail", failed=False):
            response = client.invoke(
                FunctionName=name, Qualifier=qualifier,
                Payload=json.dumps(event).encode(), LogType=logs,
            )
            assert response["StatusCode"] == 200, response
            assert response["ExecutedVersion"] == (version if qualifier == "prod" else qualifier)
            with response["Payload"] as body:
                raw = body.read(6 * 1024 * 1024 + 1)
            assert len(raw) <= 6 * 1024 * 1024
            assert response.get("FunctionError") == ("Unhandled" if failed else None)
            tail = response.get("LogResult")
            if logs == "Tail":
                assert tail is not None
                tail = base64.b64decode(tail, validate=True)
                assert len(tail) <= 4096
            else:
                assert tail is None
            return json.loads(raw), tail

        first, tail = invoke({"value": 35, "logs": True, "policy": True, "response_bytes": 1024 * 1024})
        assert first["computed"] == 42 and first["counter"] == 1
        assert first["functionVersion"] == version
        assert first["functionName"] == name
        assert first["invokedArn"] == f"arn:aws:lambda:us-east-1:000000000000:function:{name}:prod"
        assert first["configuredValue"] == "version-one"
        assert b"WIRE_STDOUT_CAPTURE" in tail and b"WIRE_STDERR_CAPTURE" in tail
        second, tail = invoke({"value": 1})
        assert second["counter"] == 2 and tail == b""
        third, tail = invoke({"value": 2, "logs": True, "response_bytes": 1024 * 1024}, logs="None")
        assert third["counter"] == 3 and tail is None
        failure, _ = invoke({"fail": True}, failed=True)
        assert failure["errorType"] == "ValueError"
        assert "intentional P0 failure" in failure["errorMessage"]
        replacement, _ = invoke({"value": 3})
        assert replacement["counter"] == 1

        client.update_function_configuration(
            FunctionName=name, Environment={"Variables": {"TEST_VALUE": "version-two"}},
        )
        client.get_waiter("function_updated_v2").wait(
            FunctionName=name, WaiterConfig={"Delay": 1, "MaxAttempts": 120},
        )
        latest, _ = invoke({"value": 4, "policy": True}, qualifier="$LATEST")
        assert latest["configuredValue"] == "version-two" and latest["counter"] == 1
        assert latest["functionVersion"] == "$LATEST"
        assert latest["invokedArn"] == f"arn:aws:lambda:us-east-1:000000000000:function:{name}:$LATEST"
        dry = client.invoke(FunctionName=name, Qualifier="$LATEST", InvocationType="DryRun")
        assert dry["StatusCode"] == 204
        after_dry, _ = invoke({"value": 5}, qualifier="$LATEST")
        assert after_dry["counter"] == 2
        old, _ = invoke({"value": 6, "policy": True})
        assert old["configuredValue"] == "version-one" and old["functionVersion"] == version
        assert old["invokedArn"] == f"arn:aws:lambda:us-east-1:000000000000:function:{name}:prod"
        client.update_function_configuration(FunctionName=name, Timeout=1)
        client.get_waiter("function_updated_v2").wait(
            FunctionName=name, WaiterConfig={"Delay": 1, "MaxAttempts": 120},
        )
        try:
            invoke({"value": 7, "sleep": True}, qualifier="$LATEST")
        except ClientError as error:
            assert error.response["Error"]["Code"] == "ServiceException"
            assert "timed out after 1s" in error.response["Error"]["Message"]
        else:
            raise AssertionError("handler timeout returned successful output")
        after_timeout, _ = invoke({"value": 8}, qualifier="$LATEST")
        assert after_timeout["counter"] == 1
        print(json.dumps({"computed": first["computed"], "version": version,
                          "warmCounters": [first["counter"], second["counter"], third["counter"]],
                          "functionError": failure["errorType"], "replacementCounter": replacement["counter"], "timeoutReplacementCounter": after_timeout["counter"]}))
    finally:
        if created:
            client.delete_function(FunctionName=name)
        client.close()


if __name__ == "__main__":
    main(*sys.argv[1:])
