"""Signed ordinary provided.al2023 deployment, computation and causal cancellation."""
import base64
import contextlib
import hashlib
import io
import json
import os
from pathlib import Path
import sys
import time
import uuid
import zipfile

if __name__ == "__main__":
    print(os.getpid(), flush=True)
import boto3
from botocore.config import Config

from custom_ric_sdk import collecting
from lambda_fault_sdk import Faults
from lambda_fault_support import Hold, Metadata, recover_before
from required_suites import read_regular


def digest(raw):
    return base64.b64encode(hashlib.sha256(raw).digest()).decode("ascii")


def variables(**extra):
    return dict(WORKFLOW_MODE="1", PROVIDED_QUALIFICATION="1", **extra)


class Provided(Faults):
    def __init__(self, endpoint, codes, architecture, metadata):
        super().__init__(endpoint, "unused", architecture, metadata)
        self.codes = codes

    def create(self, **fields):
        name = "ProvidedZip" + uuid.uuid4().hex
        self.client.create_function(
            FunctionName=name, Role="arn:aws:iam::000000000000:role/Docker",
            Runtime="provided.al2023", Handler="handler.entry", PackageType="Zip",
            Architectures=[self.architecture], Timeout=fields.pop("Timeout", 30),
            MemorySize=128, Code={"ZipFile": self.codes[0]},
            Environment=fields.pop("Environment", {"Variables":variables()}), **fields)
        self.functions.append(name)
        assert self.settled(name)["CodeSha256"] == digest(self.codes[0])
        return name

    def invoke(self, name, event=None, qualifier=None, failed=False, client=None):
        options = {} if qualifier is None else {"Qualifier":qualifier}
        reply = (self.client if client is None else client).invoke(FunctionName=name,
            Payload=json.dumps(event or {}).encode(), **options)
        with reply["Payload"] as body:
            raw = body.read(65537)
        assert len(raw) <= 65536 and reply["StatusCode"] == 200
        output = json.loads(raw)
        if output.get("errorType") == "ProvidedQualificationError":
            raise AssertionError("provided fixture observation failed: " + output["errorMessage"])
        assert reply.get("FunctionError") == ("Unhandled" if failed else None)
        return output, reply.get("ExecutedVersion")

    @staticmethod
    def checked(output, n, marker, counter=None):
        assert output["computed"] == n * 2 and output["codeMarker"] == marker
        assert output["pid"] == 1
        assert output["providedSandbox"] == {"codeReadOnly":True, "codeUid":0,
                "codeGid":0, "codeMode":0o555, "tmpWritable":True}
        assert str(uuid.UUID(output["request_id"])) == output["request_id"]
        if counter is not None:
            assert output["providedCounter"] == counter
        return output

    def computed(self, name, n, marker="A", counter=None, qualifier=None):
        output, executed = self.invoke(name, {"n":n}, qualifier=qualifier)
        return self.checked(output, n, marker, counter), executed

    def cold(self, name):
        self.client.update_function_configuration(FunctionName=name, Environment={"Variables":variables()}, Timeout=30)
        self.settled(name, updating=True)
        def invoke(remaining):
            connect = min(0.1, remaining / 10)
            with contextlib.closing(boto3.client("lambda", endpoint_url=self.endpoint,
                    region_name="us-east-1", aws_access_key_id="local-key", aws_secret_access_key="local-secret",
                    config=self.client.meta.config.merge(Config(connect_timeout=connect, read_timeout=remaining-connect)))) as client:
                output, _ = self.invoke(name, {"n":21}, client=client)
                return self.checked(output, 21, "A", 1)
        recover_before(time.monotonic() + 10, invoke, lambda: not self.metadata.ids())

    def deployments(self):
        name = self.create()
        self.computed(name, 21, counter=1)
        self.computed(name, 23, counter=2)
        version = self.client.publish_version(FunctionName=name)["Version"]
        assert version == "1"
        self.client.create_alias(FunctionName=name, Name="live", FunctionVersion=version)
        self.computed(name, 21, qualifier="live")
        before = self.client.get_function_configuration(FunctionName=name)
        invalid = io.BytesIO()
        with zipfile.ZipFile(invalid, "w") as archive:
            archive.writestr("bootstrap", b"not a Linux executable")
        self.expect_error(lambda: self.client.update_function_code(FunctionName=name, ZipFile=invalid.getvalue()), "InvalidParameterValueException")
        after = self.client.get_function_configuration(FunctionName=name)
        assert (after["RevisionId"], after["CodeSha256"]) == (before["RevisionId"], before["CodeSha256"])
        self.expect_error(lambda: self.client.update_function_configuration(FunctionName=name, Runtime="python3.13"), "InvalidParameterValueException")
        after = self.client.get_function_configuration(FunctionName=name)
        assert (after["RevisionId"], after["CodeSha256"]) == (before["RevisionId"], before["CodeSha256"])
        self.client.update_function_code(FunctionName=name, ZipFile=self.codes[1])
        assert self.settled(name, updating=True)["CodeSha256"] == digest(self.codes[1])
        self.computed(name, 21, marker="B", counter=1)
        old, executed = self.computed(name, 23, qualifier="live")
        assert old["codeMarker"] == "A" and executed == "1"
        # Repeated immutable revision changes exercise eviction and snapshot IO.
        for marker, index in (("A",0), ("B",1), ("A",0), ("B",1)):
            self.client.update_function_code(FunctionName=name, ZipFile=self.codes[index])
            assert self.settled(name, updating=True)["CodeSha256"] == digest(self.codes[index])
            self.computed(name, 21, marker=marker, counter=1)
            preserved, executed = self.computed(name, 23, qualifier="live")
            assert preserved["codeMarker"] == "A" and executed == "1"
        self.client.update_alias(FunctionName=name, Name="live", FunctionVersion=self.client.publish_version(FunctionName=name)["Version"])
        self.computed(name, 21, marker="B", qualifier="live")
        self.workflow(name)
        self.delete(name)
        return ["cold-warm-readonly", "transactional-rejection", "immutable-A-B-alias", "ASL-Lambda-computation"]

    def workflow(self, name):
        with contextlib.closing(boto3.client("stepfunctions", endpoint_url=self.endpoint,
                region_name="us-east-1", aws_access_key_id="local-key", aws_secret_access_key="local-secret",
                config=Config(retries={"max_attempts":0}, connect_timeout=2, read_timeout=60))) as sfn:
            definition = {"StartAt":"Compute", "States":{"Compute":{"Type":"Task",
                "Resource":"arn:aws:states:::lambda:invoke", "Parameters":{"FunctionName":name, "Qualifier":"live", "Payload.$":"$"}, "End":True}}}
            machine = sfn.create_state_machine(name="ProvidedZip" + uuid.uuid4().hex,
                roleArn="arn:aws:iam::000000000000:role/StepFunctions", definition=json.dumps(definition))["stateMachineArn"]
            try:
                execution = sfn.start_execution(stateMachineArn=machine, input='{"n":21}')["executionArn"]
                deadline = time.monotonic() + 90
                while True:
                    result = sfn.describe_execution(executionArn=execution)
                    assert time.monotonic() < deadline
                    if result["status"] != "RUNNING":
                        break
                    time.sleep(0.025)
                assert result["status"] == "SUCCEEDED", result
                self.checked(json.loads(result["output"])["Payload"], 21, "B")
            finally:
                sfn.delete_state_machine(stateMachineArn=machine)

    def faults(self):
        completed = []
        cases = (("init-error", {"INIT_FAIL":"1"}, {}, "function initialization failed", 30),
                 ("handler-exit", {}, {"providedAction":"exit", "n":21}, "function exited with status 23; oom=false", 30),
                 ("handler-timeout", {"SLEEP_SECS":"5"}, {"n":21}, "function timed out after 1s", 1))
        for label, extra, event, message, seconds in cases:
            name = self.create(Timeout=seconds, Environment={"Variables":variables(**extra)})
            before = self.metadata.ids()
            self.expect_error(lambda: self.invoke(name, event), "ServiceException", message)
            assert self.metadata.ids() <= before, "failed environment survived returned error"
            self.ready()
            self.cold(name)
            self.delete(name)
            completed.append(label)
        name = self.create()
        output, _ = self.invoke(name, {}, failed=True)
        assert output["errorType"] == "WorkflowInputError"
        self.computed(name, 21, counter=1)
        self.delete(name)
        return completed + ["FunctionError-recovery"]

    def disconnect(self, host, stage):
        with collecting(lambda: Hold(stage)) as hold:
            name = self.create(Timeout=900, Environment={"Variables":variables(
                PROVIDED_HOLD_STAGE=stage, IMAGE_FIXTURE_TOKEN=hold.token,
                IMAGE_FIXTURE_COLLECTOR=f"{host}:{hold.server_port}")})
            hold.targets([name])
            self.client.put_function_concurrency(FunctionName=name, ReservedConcurrentExecutions=1)
            started = time.monotonic()
            cutoff = started + 20
            assert hold.deadline > cutoff + 5
            connection = self.raw_invoke(name, {"n":21})
            try:
                observed = hold.wait_seen(1, started + 15)[name]
                assert observed["stage"] == stage and len(self.metadata.ids()) == 2
                self.expect_error(lambda: self.invoke(name, {"n":23}), "TooManyRequestsException")
            finally:
                connection.close()
            self.metadata.wait_ids(set(), cutoff)
            assert time.monotonic() < cutoff < hold.deadline and not hold.released
            self.ready()
            self.cold(name)
            self.delete(name)
        return [stage + "-causal-disconnect", "physical-retirement", "cold-recovery"]


def main():
    endpoint, zip_a, zip_b, architecture, phase, host, socket, owner, engine = sys.argv[1:]
    codes = [read_regular(Path(path), 50 * 1024 * 1024) for path in (zip_a, zip_b)]
    assert codes[0] != codes[1]
    matrix = Provided(endpoint, codes, architecture, Metadata(socket, owner, engine))
    try:
        if phase == "deployments":
            cases = matrix.deployments()
        elif phase == "faults":
            cases = matrix.faults()
        else:
            assert phase in ("initializing", "busy")
            cases = matrix.disconnect(host, phase)
        matrix.ready()
    finally:
        primary = sys.exc_info()[1]
        try:
            matrix.cleanup()
            matrix.client.close()
        except BaseException:
            if primary is None:
                raise
            primary.add_note("ZIP fixture cleanup also failed")
    print(json.dumps({"schemaVersion":1, "zipSha256":[digest(code) for code in codes], "cases":cases}), flush=True)


if __name__ == "__main__":
    main()
