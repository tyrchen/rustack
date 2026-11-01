"""Real Image faults, causal client disconnects and accepted-work shutdown."""
import contextlib
import http.client
import json
import os
import sys
import time
import urllib.parse

if __name__ == "__main__":
    print(os.getpid(), flush=True)
import boto3
from botocore.config import Config
from botocore.auth import SigV4Auth
from botocore.awsrequest import AWSRequest
from botocore.credentials import Credentials
from botocore.exceptions import ClientError

from custom_ric_sdk import Matrix, collecting
from lambda_fault_support import Hold, Metadata, recover_before


class Faults(Matrix):
    def __init__(self, endpoint, image, architecture, metadata):
        super().__init__(endpoint, image, architecture)
        self.endpoint, self.metadata = endpoint, metadata

    def ready(self):
        endpoint = urllib.parse.urlsplit(self.endpoint)
        connection = http.client.HTTPConnection(endpoint.hostname, endpoint.port, timeout=3)
        try:
            connection.request("GET", "/_rustack/capabilities")
            response = connection.getresponse()
            raw = response.read(65537)
            assert response.status == 200 and len(raw) <= 65536
            status = json.loads(raw)
            assert status["ready"] is True and status["workers"]["docker"]["ready"] is True
        finally:
            connection.close()

    def expect_error(self, operation, code, message=None):
        try:
            operation()
        except ClientError as error:
            assert error.response["Error"]["Code"] == code, error.response["Error"]["Code"]
            assert error.response["ResponseMetadata"]["HTTPStatusCode"] == {"TooManyRequestsException":429, "ServiceException":500, "ResourceNotFoundException":404, "ResourceConflictException":409, "ResourceNotReadyException":502}.get(code, 400)
            text = error.response["Error"].get("Message", "")
            assert "fixture-init-payload-sentinel" not in text
            if message is not None:
                assert message in text, "infrastructure failure was misclassified: " + text
        else:
            raise AssertionError("negative Image operation unexpectedly succeeded")

    def delete(self, name):
        self.client.delete_function(FunctionName=name)
        self.functions.remove(name)

    def fault_matrix(self):
        cases = [
            ("init-error", "error", {}, "function initialization failed", 30),
            ("prepoll-exit", "exit", {}, "function exited with status 42; oom=false", 30),
            ("bootstrap-timeout", "stall", {}, "function bootstrap initialization deadline exceeded", 30),
            ("handler-exit", None, {"mode":"exit"}, "function exited with status 42; oom=false", 30),
            ("cgroup-oom", None, {"mode":"oom"}, "function exited with status 137; oom=true", 30),
            ("handler-timeout", None, {"delayMillis":5000}, "function timed out after 1s", 1),
        ]
        completed = []
        for case, mode, event, message, timeout in cases:
            environment = {} if mode is None else {"IMAGE_FIXTURE_INIT_MODE":mode}
            name = self.create(Timeout=timeout, Environment={"Variables":environment})
            before = self.metadata.ids()
            self.expect_error(lambda: self.invoke(name, event), "ServiceException", message)
            assert self.metadata.ids() <= before, "failed environment survived the returned error"
            self.ready()
            self.client.update_function_configuration(FunctionName=name, Environment={"Variables":{}}, Timeout=30)
            self.settled(name, updating=True)
            output, _ = self.invoke(name, {"value":8})
            assert output["computed"] == 15 and output["counter"] == 1 and output["imageRevision"] == "A"
            self.delete(name)
            completed.append(case)
        return completed

    def raw_invoke(self, name, event):
        endpoint = urllib.parse.urlsplit(self.endpoint)
        path = "/2015-03-31/functions/" + name + "/invocations"
        body = json.dumps(event).encode()
        request = AWSRequest(method="POST", url=self.endpoint + path, data=body,
                             headers={"Content-Type":"application/json", "X-Amz-Invocation-Type":"RequestResponse"})
        SigV4Auth(Credentials("local-key", "local-secret"), "lambda", "us-east-1").add_auth(request)
        connection = http.client.HTTPConnection(endpoint.hostname, endpoint.port, timeout=3)
        try:
            connection.request("POST", path, body, dict(request.headers))
        except BaseException:
            connection.close()
            raise
        return connection

    def disconnected(self, host, stage):
        with collecting(lambda: Hold(stage)) as collector:
            variables = {"IMAGE_FIXTURE_COLLECTOR":f"{host}:{collector.server_port}",
                         "IMAGE_FIXTURE_TOKEN":collector.token}
            if stage == "initializing":
                variables["IMAGE_FIXTURE_INIT_MODE"] = "hold"
            name = self.create(Timeout=900, Environment={"Variables":variables})
            collector.targets([name])
            self.client.put_function_concurrency(FunctionName=name, ReservedConcurrentExecutions=0)
            baseline = self.metadata.ids()
            self.expect_error(lambda: self.invoke(name), "TooManyRequestsException")
            assert self.metadata.ids() == baseline
            # Only these malformed wire controls disable botocore's local range
            # check; signing and the actual service boundary remain enabled.
            with contextlib.closing(boto3.client("lambda", endpoint_url=self.endpoint, region_name="us-east-1",
                              aws_access_key_id="local-key", aws_secret_access_key="local-secret",
                              config=self.client.meta.config.merge(Config(parameter_validation=False)))) as malformed:
                for invalid in (-1, 1001):
                    self.expect_error(lambda: malformed.put_function_concurrency(FunctionName=name, ReservedConcurrentExecutions=invalid), "InvalidParameterValueException")
                    assert self.client.get_function_concurrency(FunctionName=name)["ReservedConcurrentExecutions"] == 0
            self.client.put_function_concurrency(FunctionName=name, ReservedConcurrentExecutions=1)
            started = time.monotonic()
            cutoff = started + 15
            assert collector.deadline > cutoff + 5
            connection = self.raw_invoke(name, {"value":35, "mode":"hold" if stage == "busy" else "compute"})
            try:
                observed = collector.wait_seen(1, started + 10)[name]
                active = self.metadata.ids()
                assert len(active) == 2 and not baseline
                assert observed["stage"] == stage
                self.expect_error(lambda: self.invoke(name), "TooManyRequestsException")
                assert self.metadata.ids() == active
            finally:
                connection.close()
            self.metadata.wait_ids(set(), cutoff)
            assert time.monotonic() < cutoff < collector.deadline
            self.ready()
            self.client.update_function_configuration(FunctionName=name, Environment={"Variables":{}})
            self.settled(name, updating=True)
            output, _ = self.cold_recovery(name, min(cutoff, time.monotonic() + 5))
            assert output["computed"] == 15 and output["counter"] == 1
            self.delete(name)
        return ["reserved-zero-no-allocation", "reserved-range-atomic", "held-reserved-one", stage + "-disconnect-removal-before-expiry", "cold-recovery"]

    def cold_recovery(self, name, deadline):
        def invoke(remaining):
            connect = min(0.1, remaining / 10)
            with contextlib.closing(boto3.client(
                    "lambda", endpoint_url=self.endpoint, region_name="us-east-1",
                    aws_access_key_id="local-key", aws_secret_access_key="local-secret",
                    config=self.client.meta.config.merge(Config(
                        connect_timeout=connect, read_timeout=remaining - connect)))) as client:
                return self.invoke(name, {"value":8}, client=client)
        return recover_before(deadline, invoke, lambda: not self.metadata.ids())

    def event(self, name, event):
        reply = self.client.invoke(FunctionName=name, InvocationType="Event", Payload=json.dumps(event).encode())
        payload = reply.get("Payload")
        if payload is not None:
            payload.close()
        assert reply["StatusCode"] == 202

    def saturation(self, host):
        too_large = self.create(MemorySize=512)
        before = self.metadata.ids()
        started = time.monotonic()
        self.expect_error(lambda: self.invoke(too_large), "TooManyRequestsException")
        assert time.monotonic() - started < 5 and self.metadata.ids() == before
        self.delete(too_large)
        self.ready()
        def shutdown_hold():
            hold = Hold("busy")
            hold.deadline = time.monotonic() + 180
            return hold
        with collecting(shutdown_hold) as collector:
            variables = {"IMAGE_FIXTURE_COLLECTOR":f"{host}:{collector.server_port}",
                         "IMAGE_FIXTURE_TOKEN":collector.token}
            names = [self.create(Timeout=900, Environment={"Variables":variables}) for _ in range(2)]
            collector.targets(names)
            for name in names:
                self.client.put_function_concurrency(FunctionName=name, ReservedConcurrentExecutions=1)
                self.event(name, {"mode":"hold"})
            collector.wait_seen(2, min(time.monotonic() + 20, collector.deadline - 20))
            physical = self.metadata.pair_limits()
            for index in range(126):
                self.event(names[index % 2], {"mode":"hold"})
            self.expect_error(lambda: self.event(names[0], {}), "TooManyRequestsException")
            assert self.metadata.ids() == physical
            assert time.monotonic() + 80 < collector.deadline
            self.ready()
            # A second bounded PID barrier hands shutdown to the exact Rust child owner.
            print(os.getpid(), flush=True)
            self.metadata.wait_ids(set(), min(time.monotonic() + 75, collector.deadline - 5))
            assert time.monotonic() < collector.deadline - 5
        return ["impossible-memory-429", "event-capacity-128", "event-129-throttled", "two-pairs-384MiB", "held-backlog-shutdown-cutoff"]


def main():
    endpoint, image, architecture, phase, host, path, owner, engine = sys.argv[1:]
    matrix = Faults(endpoint, image, architecture, Metadata(path, owner, engine))
    shutdown = phase == "saturation"
    try:
        if phase == "faults":
            cases = matrix.fault_matrix()
        elif phase in ("initializing", "busy"):
            cases = matrix.disconnected(host, phase)
        else:
            assert shutdown
            cases = matrix.saturation(host)
    finally:
        if not shutdown:
            primary = sys.exc_info()[1]
            try:
                matrix.cleanup()
            except BaseException:
                if primary is None:
                    raise
                primary.add_note("fault fixture function cleanup also failed")
    print(json.dumps({"schemaVersion":1, "cases":cases}), flush=True)


if __name__ == "__main__":
    main()
