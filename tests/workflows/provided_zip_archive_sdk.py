"""Signed mixed ZIP/Image archive preparation and post-restart computation."""
import contextlib
import json
import os
from pathlib import Path
import sys
import time
import uuid

if __name__ == "__main__":
    print(os.getpid(), flush=True)

import boto3
from botocore.config import Config

from lambda_fault_support import Metadata
from provided_zip_sdk import Provided, digest
from required_suites import read_regular


def canonical(value):
    return json.loads(json.dumps(value, default=lambda value: value.isoformat()))


def execution(sfn, arn):
    deadline = time.monotonic() + 90
    while True:
        result = sfn.describe_execution(executionArn=arn)
        assert time.monotonic() < deadline, "archive workflow observation expired"
        if result["status"] != "RUNNING":
            assert result["status"] == "SUCCEEDED", result
            return {key: canonical(result[key]) for key in
                ("executionArn", "stateMachineArn", "status", "input", "output", "startDate", "stopDate")}
        time.sleep(0.025)


def history(sfn, arn):
    reply = sfn.get_execution_history(executionArn=arn, maxResults=100)
    assert "nextToken" not in reply and 1 <= len(reply["events"]) <= 100
    return canonical(reply["events"])


def configuration(matrix, name, qualifier=None):
    options = {} if qualifier is None else {"Qualifier": qualifier}
    reply = matrix.client.get_function_configuration(FunctionName=name, **options)
    keys = ("FunctionArn", "CodeSha256", "CodeSize", "RevisionId", "Version",
        "PackageType", "Runtime", "Handler", "Timeout", "MemorySize", "Architectures", "Environment")
    return {key: canonical(reply.get(key)) for key in keys}


def image_call(matrix, name, counter):
    reply = matrix.client.invoke(FunctionName=name, Payload=b'{"value":35}')
    with reply["Payload"] as stream:
        raw = stream.read(65537)
    assert len(raw) <= 65536 and reply["StatusCode"] == 200 and "FunctionError" not in reply
    output = json.loads(raw)
    assert output["computed"] == 42 and output["counter"] == counter and output["uid"] == 1000
    return output


def prepare(matrix, sfn, image):
    name = matrix.create()
    matrix.computed(name, 21, counter=1)
    matrix.computed(name, 23, counter=2)
    assert matrix.client.publish_version(FunctionName=name)["Version"] == "1"
    alias = matrix.client.create_alias(FunctionName=name, Name="live", FunctionVersion="1")
    matrix.client.update_function_code(FunctionName=name, ZipFile=matrix.codes[1])
    assert matrix.settled(name, updating=True)["CodeSha256"] == digest(matrix.codes[1])
    matrix.computed(name, 21, marker="B", counter=1)
    _, version = matrix.computed(name, 23, qualifier="live", counter=1)
    assert version == "1"
    definition = {"StartAt":"Compute", "States":{"Compute":{"Type":"Task",
        "Resource":"arn:aws:states:::lambda:invoke", "Parameters":{
        "FunctionName":name, "Qualifier":"live", "Payload.$":"$"}, "End":True}}}
    machine = sfn.create_state_machine(name="ZipArchive" + uuid.uuid4().hex,
        roleArn="arn:aws:iam::000000000000:role/StepFunctions", definition=json.dumps(definition))["stateMachineArn"]
    arn = sfn.start_execution(stateMachineArn=machine, input='{"n":21}')["executionArn"]
    completed = execution(sfn, arn)
    Provided.checked(json.loads(completed["output"])["Payload"], 21, "A")
    image_name = "ZipArchiveImage" + uuid.uuid4().hex
    matrix.client.create_function(FunctionName=image_name, Role="arn:aws:iam::000000000000:role/Docker",
        PackageType="Image", Architectures=[matrix.architecture], Code={"ImageUri":image},
        Timeout=30, MemorySize=128)
    matrix.functions.append(image_name)
    matrix.settled(image_name)
    image_call(matrix, image_name, 1)
    image_call(matrix, image_name, 2)
    return {"name":name, "image":image_name, "latest":configuration(matrix, name),
        "published":configuration(matrix, name, "1"), "imageConfig":configuration(matrix, image_name),
        "aliasVersion":alias["FunctionVersion"], "aliasRevision":alias["RevisionId"],
        "execution":completed, "history":history(sfn, arn), "machine":machine}


def verify(matrix, sfn, saved):
    # Qualification has retired its own probes; no archived work was resumed.
    assert matrix.metadata.ids() == set(), "startup allocated a replay environment"
    name = saved["name"]
    assert configuration(matrix, name) == saved["latest"]
    assert configuration(matrix, name, "1") == saved["published"]
    assert configuration(matrix, saved["image"]) == saved["imageConfig"]
    alias = matrix.client.get_alias(FunctionName=name, Name="live")
    assert (alias["FunctionVersion"], alias["RevisionId"]) == (saved["aliasVersion"], saved["aliasRevision"])
    arn = saved["execution"]["executionArn"]
    assert execution(sfn, arn) == saved["execution"] and history(sfn, arn) == saved["history"]
    matrix.computed(name, 21, marker="B", counter=1)
    matrix.computed(name, 23, marker="B", counter=2)
    _, executed = matrix.computed(name, 21, qualifier="live", counter=1)
    assert executed == "1"
    matrix.computed(name, 23, qualifier="1", counter=2)
    image_call(matrix, saved["image"], 1)
    image_call(matrix, saved["image"], 2)
    fresh = sfn.start_execution(stateMachineArn=saved["machine"], input='{"n":23}')["executionArn"]
    completed = execution(sfn, fresh)
    Provided.checked(json.loads(completed["output"])["Payload"], 23, "A")
    assert fresh != arn and execution(sfn, arn) == saved["execution"] and history(sfn, arn) == saved["history"]
    return saved


def main():
    endpoint, zip_a, zip_b, architecture, image, phase, checkpoint, socket, owner, engine = sys.argv[1:]
    codes = [read_regular(Path(path), 50 * 1024 * 1024) for path in (zip_a, zip_b)]
    matrix = Provided(endpoint, codes, architecture, Metadata(socket, owner, engine))
    try:
        with contextlib.closing(boto3.client("stepfunctions", endpoint_url=endpoint,
                region_name="us-east-1", aws_access_key_id="local-key", aws_secret_access_key="local-secret",
                config=Config(retries={"max_attempts":0}, connect_timeout=2, read_timeout=60))) as sfn:
            if phase == "prepare":
                saved = prepare(matrix, sfn, image)
            else:
                assert phase == "verify"
                original = json.loads(read_regular(Path(checkpoint), 65536))
                assert original["zipSha256"] == [digest(code) for code in codes]
                saved = verify(matrix, sfn, original["checkpoint"])
            matrix.ready()
    finally:
        # Preserve control-plane resources for the archive owner; no live invocations remain.
        matrix.client.close()
    print(json.dumps({"schemaVersion":1, "zipSha256":[digest(code) for code in codes],
        "phase":phase, "cases":(["original-ZIP-revisions", "mixed-Image", "terminal-history-checkpoint"]
            if phase == "prepare" else ["original-ZIP-revisions", "mixed-Image", "terminal-history-no-replay", "cold-warm-after-restart"]),
        "checkpoint":saved}), flush=True)


if __name__ == "__main__":
    main()
