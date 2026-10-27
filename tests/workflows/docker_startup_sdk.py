"""Pinned ordinary boto3 acceptance of the actual production Docker binary."""
import json
import os
import sys
import time
import uuid

print(os.getpid(), flush=True)
import boto3
from botocore.config import Config

endpoint, image, architecture = sys.argv[1:]
client = boto3.client("lambda", endpoint_url=endpoint, region_name="us-east-1",
                      aws_access_key_id="local-key", aws_secret_access_key="local-secret",
                      config=Config(retries={"max_attempts": 0}, connect_timeout=2, read_timeout=60))
name = "BotoDocker" + uuid.uuid4().hex
client.create_function(FunctionName=name, Role="arn:aws:iam::000000000000:role/Docker",
                       PackageType="Image", Architectures=[architecture], Timeout=30,
                       Code={"ImageUri": image})
deadline = time.monotonic() + 120
while True:
    state = client.get_function_configuration(FunctionName=name)["State"]
    if state == "Active":
        break
    assert state != "Failed" and time.monotonic() < deadline
    time.sleep(0.025)
for value, counter in ((35, 1), (0, 2)):
    reply = client.invoke(FunctionName=name, Payload=json.dumps({"value": value}).encode())
    output = json.load(reply["Payload"])
    assert reply["StatusCode"] == 200 and "FunctionError" not in reply
    assert output["computed"] == value + 7 and output["counter"] == counter and output["uid"] == 1000
reply = client.invoke(FunctionName=name, Payload=b'{"value":1,"fail":true}')
assert reply["StatusCode"] == 200 and reply["FunctionError"] == "Unhandled"
reply["Payload"].read()
client.delete_function(FunctionName=name)
print(json.dumps({"schemaVersion": 1, "computed": 42, "warmCounter": 2, "functionError": True}), flush=True)
