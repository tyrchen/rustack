"""Real default-provider image SDK calls to the strict local S3/DynamoDB gateway."""
import ipaddress
import json
import os
import sys
import time
import urllib.parse
import uuid

from lambda_fault_support import Metadata, invoke_json, poll_response
from node_image_sdk import deployed

VERSION = "3.1105.0"
UNSET = ("AWS_PROFILE", "AWS_DEFAULT_PROFILE", "AWS_SDK_LOAD_CONFIG", "AWS_SESSION_TOKEN",
         "AWS_SECURITY_TOKEN", "AWS_ACCESS_KEY", "AWS_SECRET_KEY", "AWS_WEB_IDENTITY_TOKEN_FILE",
         "AWS_ROLE_ARN", "AWS_ROLE_SESSION_NAME", "AWS_CONTAINER_CREDENTIALS_FULL_URI",
         "AWS_CONTAINER_CREDENTIALS_RELATIVE_URI", "AWS_CONTAINER_AUTHORIZATION_TOKEN",
         "AWS_CONTAINER_AUTHORIZATION_TOKEN_FILE", "AWS_EC2_METADATA_SERVICE_ENDPOINT",
         "AWS_EC2_METADATA_SERVICE_ENDPOINT_MODE", "AWS_ENDPOINT_URL_S3", "AWS_ENDPOINT_URL_DYNAMODB")


def inspect_sdk_environment(metadata, endpoint, gateway, secret):
    rows = metadata.rows()
    assert len(rows) == 2 and {r["Labels"]["rustack.lambda.role"] for r in rows} == {"function", "relay"}
    projected = {}
    host = ipaddress.ip_address(gateway)
    visible = "http://" + ("[" + str(host) + "]" if host.version == 6 else str(host))
    visible += ":" + str(urllib.parse.urlsplit(endpoint).port) + "/"
    for row in rows:
        inspected = metadata.get("/containers/" + row["Id"] + "/json")
        assert inspected["State"]["Running"] is True
        entries = inspected["Config"]["Env"]
        assert isinstance(entries, list) and len(entries) <= 320
        assert all(isinstance(entry, str) and len(entry.encode()) <= 4097 and "\x00" not in entry for entry in entries)
        assert sum(len(entry.encode()) for entry in entries) <= 80 * 1024
        env = {}
        for entry in entries:
            key, separator, value = entry.partition("=")
            assert key not in env
            env[key] = value if separator else None
        role = row["Labels"]["rustack.lambda.role"]
        if role == "function":
            assert env["AWS_ACCESS_KEY_ID"] == "local-key"
            assert env["AWS_SECRET_ACCESS_KEY"] == secret
            assert env["AWS_ENDPOINT_URL"] == visible
            assert all(env.get(key) is None for key in UNSET)
            assert env["AWS_EC2_METADATA_DISABLED"] == "true"
            assert env["AWS_CONFIG_FILE"] == env["AWS_SHARED_CREDENTIALS_FILE"] == "/dev/null"
        else:
            assert "AWS_ACCESS_KEY_ID" not in env and "AWS_SECRET_ACCESS_KEY" not in env
        projected[role] = row["Id"]
    return projected


def configuration_omits_sdk(configuration):
    variables = configuration["Environment"]["Variables"]
    assert "AWS_ACCESS_KEY_ID" not in variables and "AWS_SECRET_ACCESS_KEY" not in variables
    assert variables["AWS_PROFILE"] == "captured-poison"
    assert variables["AWS_ENDPOINT_URL"] == "http://127.0.0.1:9"
    encoded = json.dumps(configuration)
    assert "local-secret" not in encoded and "sdk-wrong-secret" not in encoded


def exercise(endpoint, image, architecture, socket, owner, engine, mode, gateway):
    import boto3
    from botocore.config import Config
    assert mode in ("positive", "wrong")
    settings = dict(endpoint_url=endpoint, region_name="us-east-1", aws_access_key_id="local-key",
                    aws_secret_access_key="local-secret",
                    config=Config(retries={"max_attempts": 0}, connect_timeout=2, read_timeout=60,
                                  s3={"addressing_style": "path"}))
    clients = []
    try:
        for service in ("lambda", "s3", "dynamodb"):
            clients.append(boto3.client(service, **settings))
        function, s3, dynamodb = clients
        suffix = uuid.uuid4().hex
        name, bucket, table = "HandlerSdk" + suffix, "sdk-bucket-" + suffix, "SdkTable" + suffix
        s3.create_bucket(Bucket=bucket)
        dynamodb.create_table(TableName=table, BillingMode="PAY_PER_REQUEST",
            KeySchema=[{"AttributeName": "id", "KeyType": "HASH"}],
            AttributeDefinitions=[{"AttributeName": "id", "AttributeType": "S"}])
        poll_response(lambda: dynamodb.describe_table(TableName=table),
                      lambda value: value["Table"]["TableStatus"] == "ACTIVE", 30)
        function.create_function(FunctionName=name, Role="arn:aws:iam::000000000000:role/HandlerSdk",
            PackageType="Image", Architectures=[architecture], Timeout=30, MemorySize=256,
            Code={"ImageUri": image}, ImageConfig={"Command": ["handler-sdk.bodyControl" if mode == "positive" else "handler-sdk.handler"]}, Environment={"Variables": {
                "AWS_PROFILE": "captured-poison", "AWS_ENDPOINT_URL": "http://127.0.0.1:9",
                "AWS_ENDPOINT_URL_DYNAMODB": "http://127.0.0.1:9"}})
        configuration_omits_sdk(deployed(function, name))
        metadata = Metadata(socket, owner, engine)
        if mode == "positive":
            response, controlled = invoke_json(function, name, {})
            assert "FunctionError" not in response and controlled == {
                "quietBodyRejected": True, "headersObserved": True, "destroyed": True}
            function.update_function_configuration(FunctionName=name, ImageConfig={"Command": ["handler-sdk.handler"]})
            configuration_omits_sdk(deployed(function, name))
            previous = None
            for value, expected, counter, key, literal in (
                    (21, 42, 1, "first", b"rustack-sdk:21:42"),
                    (23, 46, 2, "second", b"rustack-sdk:23:46")):
                response, output = invoke_json(function, name, dict(bucket=bucket, table=table,
                    key=key, value=value, mode="roundtrip"))
                assert "FunctionError" not in response
                assert output["computed"] == expected and output["counter"] == counter
                assert output["selectorsCleared"] is True
                assert output["versions"] == {"s3": VERSION, "dynamodb": VERSION}
                assert output["architecture"] == {"arm64": "arm64", "x86_64": "x64"}[architecture]
                assert output["nodeVersion"].startswith("24.")
                object_body = s3.get_object(Bucket=bucket, Key=key)["Body"]
                try:
                    assert object_body.read(4097) == literal
                finally:
                    object_body.close()
                item = dynamodb.get_item(TableName=table, Key={"id": {"S": key}}, ConsistentRead=True)
                assert item["Item"]["result"] == {"N": str(expected)}
                current = inspect_sdk_environment(metadata, endpoint, gateway, "local-secret")
                if previous is not None:
                    assert current == previous, "warm SDK environment changed"
                previous = current
                s3.delete_object(Bucket=bucket, Key=key)
            cases = ["quiet-body-abort", "local-s3-dynamodb-cold-warm", "authoritative-SDK-env", "metadata-secret-omission"]
        else:
            for selected, error, message in (
                    ("deny-s3", "SdkS3AuthorizationError", "actual SDK AccessDenied/403 " + VERSION),
                    ("deny-dynamodb", "SdkDynamoAuthorizationError", "actual SDK AccessDeniedException/400 " + VERSION)):
                response, output = invoke_json(function, name, dict(bucket=bucket, table=table,
                    key="denied", value=21, mode=selected))
                assert response.get("FunctionError") == "Unhandled"
                assert output["errorType"] == error and output["errorMessage"] == message
                metadata.wait_ids(set(), time.monotonic() + 30)
            assert "Item" not in dynamodb.get_item(TableName=table, Key={"id": {"S": "denied"}})
            assert s3.list_objects_v2(Bucket=bucket).get("KeyCount", 0) == 0
            cases = ["explicit-invalid-s3-authorization", "explicit-invalid-dynamodb-authorization", "no-ambient-fallback"]
        configuration_omits_sdk(function.get_function_configuration(FunctionName=name))
        function.delete_function(FunctionName=name)
        s3.delete_bucket(Bucket=bucket)
        dynamodb.delete_table(TableName=table)
        return {"schemaVersion": 1, "mode": mode, "versions": {"s3": VERSION, "dynamodb": VERSION}, "cases": cases}
    finally:
        for client in reversed(clients):
            client.close()


if __name__ == "__main__":
    print(os.getpid(), flush=True)
    print(json.dumps(exercise(*sys.argv[1:])), flush=True)
