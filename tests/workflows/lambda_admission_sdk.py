"""Actual Runtime HTTP bounds and independently binding execution/resource ceilings."""
import base64
import contextlib
import json
import http.client
import time
import urllib.parse
import uuid

import boto3
from botocore.config import Config
from botocore.exceptions import ClientError

from custom_ric_sdk import collecting, unique_object
from lambda_fault_support import Hold


def wait_work(matrix, accepted, active, deadline):
    address = urllib.parse.urlsplit(matrix.endpoint)
    while True:
        remaining = deadline - time.monotonic()
        assert remaining > 0, "actual invocation ownership never reached the required counts"
        connection = http.client.HTTPConnection(address.hostname, address.port, timeout=min(2, remaining))
        try:
            connection.request("GET", "/_rustack/capabilities")
            response = connection.getresponse()
            raw = response.read(65537)
            assert response.status == 200 and len(raw) <= 65536
            work = json.loads(raw, object_pairs_hook=unique_object)["workers"]["lambdaAdmission"]
            assert work["limits"] == {"acceptedInvocations":128,"activeExecutions":32,"defaultFunctionExecutions":8}
            assert work["accepting"] is True and time.monotonic() < deadline
            assert type(work["acceptedInvocations"]) is type(work["activeExecutions"]) is int
            assert 0 <= work["acceptedInvocations"] <= 128 and 0 <= work["activeExecutions"] <= 32
            if work["acceptedInvocations"] == accepted and work["activeExecutions"] == active:
                return work
        finally:
            connection.close()
        time.sleep(0.025)


def capacity(matrix, allocated, memory):
    address = urllib.parse.urlsplit(matrix.endpoint)
    connection = http.client.HTTPConnection(address.hostname, address.port, timeout=2)
    try:
        connection.request("GET", "/_rustack/capabilities")
        response = connection.getresponse()
        raw = response.read(65537)
        assert response.status == 200 and len(raw) <= 65536
        actual = json.loads(raw, object_pairs_hook=unique_object)["workers"]["docker"]["capacity"]
        assert actual == {"allocated":allocated,"memoryBytes":memory * 1024 * 1024,
                          "quarantined":0,"accepting":True,"workerFailed":False}
        return actual
    finally:
        connection.close()


class QuotaHold(Hold):
    """Only first handlers block; bounded computing events publish unique tickets."""
    def __init__(self):
        super().__init__("busy")
        self.deadline = time.monotonic() + 150
        self.tickets, self.completed = set(), {}

    def expect_tickets(self, count):
        with self.condition:
            assert type(count) is int and 1 <= count <= 30 and not self.tickets and not self.completed
            self.tickets = set(range(count))

    def exchange(self, path, event):
        if path == "/hold":
            return super().exchange(path, event)
        with self.condition:
            assert path == "/completed" and not self.aborted and time.monotonic() < self.deadline
            assert set(event) == {"ticket", "functionName", "requestId", "computed", "counter"}
            ticket = event["ticket"]
            assert type(ticket) is int and ticket in self.tickets and ticket not in self.completed
            assert event["functionName"] in self.expected and event["computed"] == 15
            assert type(event["counter"]) is int and 1 <= event["counter"] <= 30
            assert str(uuid.UUID(event["requestId"])) == event["requestId"]
            assert all(row["requestId"] != event["requestId"] for row in self.completed.values())
            self.completed[ticket] = dict(event)
            self.condition.notify_all()
            return {"recorded":True}

    def wait_completed(self, deadline):
        with self.condition:
            while set(self.completed) != self.tickets and not self.aborted:
                remaining = min(deadline, self.deadline) - time.monotonic()
                assert remaining > 0, "acknowledged event computation never completed"
                self.condition.wait(remaining)
            assert self.tickets and not self.aborted and time.monotonic() < min(deadline, self.deadline)
            return dict(self.completed)


def wait_absent(matrix, old, deadline):
    assert len(old) == 2
    while True:
        current = matrix.metadata.ids()
        assert time.monotonic() < deadline, "old pair did not physically retire"
        if current.isdisjoint(old):
            return current
        time.sleep(0.025)


def raw_result(connection, expected, message=None):
    assert connection.sock is not None
    connection.sock.settimeout(30)
    response = connection.getresponse()
    raw = response.read(65537)
    assert response.status == expected and len(raw) <= 65536
    value = json.loads(raw, object_pairs_hook=unique_object)
    if expected == 500:
        assert response.getheader("x-amzn-errortype") == "ServiceException"
        assert message in value.get("message", value.get("Message", ""))
    else:
        assert response.getheader("x-amz-function-error") is None
    return value


def retire(matrix, name):
    matrix.expect_error(lambda: matrix.invoke(name, {"mode":"exit"}), "ServiceException",
                        "function exited with status 42; oom=false")


def held_exit(matrix, hold, name):
    before = matrix.metadata.rows()
    ids = {row["Id"] for row in before}
    matrix.proxy.arm("pair_delete", matrix.metadata.owner, 2)
    hold.release_target(name)
    record = matrix.proxy.wait(1, time.monotonic() + 20)[0]
    assert record["id"] in ids and matrix.metadata.ids() == ids
    function = matrix.metadata.get("/containers/" + record["id"] + "/json")
    assert function["State"]["Running"] is False and function["State"]["ExitCode"] == 42
    assert "AWS_LAMBDA_FUNCTION_NAME=" + name in function["Config"]["Env"]
    environment = function["Config"]["Labels"]["rustack.lambda.environmentId"]
    old = {row["Id"] for row in before if row["Labels"]["rustack.lambda.environmentId"] == environment}
    assert len(old) == 2 and not matrix.proxy.released
    return ids, old


def held_relay(matrix, ids, old):
    function = matrix.proxy.observed[0]["id"]
    matrix.proxy.release_function()
    record = matrix.proxy.wait(2, time.monotonic() + 15)[1]
    assert record["role"] == "relay" and record["id"] in old and record["id"] != function
    expected = ids - {function}
    assert matrix.metadata.ids() == expected and not matrix.proxy.released
    return expected


def pair(matrix):
    rows = matrix.metadata.rows()
    assert len(rows) == 2
    roles, environments = set(), set()
    for row in rows:
        role = row["Labels"]["rustack.lambda.role"]
        roles.add(role)
        environments.add(row["Labels"]["rustack.lambda.environmentId"])
        inspected = matrix.metadata.get("/containers/" + row["Id"] + "/json")
        assert inspected["State"]["Running"] is True
        memory = (128 if role == "function" else 64) * 1024 * 1024
        assert inspected["HostConfig"]["Memory"] == inspected["HostConfig"]["MemorySwap"] == memory
    assert roles == {"function", "relay"} and len(environments) == 1
    return {row["Id"] for row in rows}


def runtime_bounds(matrix):
    name = matrix.create()
    response = matrix.client.invoke(FunctionName=name, LogType="Tail", Payload=b'{"mode":"framing","value":8}')
    with response["Payload"] as body:
        raw = body.read(65537)
    assert response["StatusCode"] == 200 and "FunctionError" not in response and len(raw) <= 65536
    output = json.loads(raw, object_pairs_hook=unique_object)
    assert output["computed"] == 15 and output["counter"] == 1
    assert output["forgedStatuses"] == [405,431,413,413,400]
    assert str(uuid.UUID(output["requestId"])) == output["requestId"]
    logs = base64.b64decode(response.get("LogResult", ""), validate=True)
    assert len(logs) <= 4096 and b"runtime-boundary-secret" not in logs
    before = pair(matrix)
    warm, _ = matrix.invoke(name, {"value":8})
    assert warm["computed"] == 15 and warm["counter"] == 2 and warm["requestId"] != output["requestId"]
    assert matrix.metadata.ids() == before
    retire(matrix, name)
    matrix.metadata.wait_ids(set(), time.monotonic() + 15)
    matrix.ready()
    matrix.delete(name)
    return ["runtime-http-405-431-413-413-400", "same-attempt-computed-result", "warm-counter-two", "retired-healthy"]


def resource_pressure(matrix, host, phase):
    wait_work(matrix, 0, 0, time.monotonic() + 5)
    with collecting(QuotaHold) as hold:
        variables = {"IMAGE_FIXTURE_COLLECTOR":f"{host}:{hold.server_port}", "IMAGE_FIXTURE_TOKEN":hold.token}
        names = [matrix.create(Timeout=900, Environment={"Variables":variables}) for _ in range(2)]
        hold.targets(names)
        for name in names:
            matrix.client.put_function_concurrency(FunctionName=name, ReservedConcurrentExecutions=1)
        a = matrix.raw_invoke(names[0], {"mode":"hold-exit"})
        b = None
        try:
            hold.wait_seen(1, time.monotonic() + 20)
            first = pair(matrix)
            b = matrix.raw_invoke(names[1], {"mode":"hold", "value":8})
            matrix.expect_error(lambda: matrix.invoke(names[1]), "TooManyRequestsException")
            wait_work(matrix, 2, 2, time.monotonic() + 5)
            assert matrix.metadata.ids() == first and names[1] not in hold.observed
            held, old = held_exit(matrix, hold, names[0])
            assert held == first == old
            capacity(matrix, 1, 192)
            wait_work(matrix, 2, 2, time.monotonic() + 5)
            matrix.expect_error(lambda: matrix.invoke(names[1]), "TooManyRequestsException")
            assert matrix.metadata.ids() == first and names[1] not in hold.observed
            relay_only = held_relay(matrix, held, old)
            wait_work(matrix, 2, 2, time.monotonic() + 5)
            capacity(matrix, 1, 192)
            matrix.expect_error(lambda: matrix.invoke(names[1]), "TooManyRequestsException")
            assert matrix.metadata.ids() == relay_only and names[1] not in hold.observed
            matrix.proxy.release()
            raw_result(a, 500, "function exited with status 42; oom=false")
            seen = hold.wait_seen(2, time.monotonic() + 30)
            replacement = pair(matrix)
            assert replacement.isdisjoint(old)
            wait_work(matrix, 1, 1, time.monotonic() + 5)
            hold.release_target(names[1])
            output = raw_result(b, 200)
            assert output["computed"] == 15 and output["counter"] == 1
            assert output["requestId"] == seen[names[1]]["requestId"]
            retire(matrix, names[1])
            matrix.metadata.wait_ids(set(), time.monotonic() + 15)
            wait_work(matrix, 0, 0, time.monotonic() + 5)
            matrix.ready()
            for name in names:
                matrix.delete(name)
        finally:
            a.close()
            if b is not None:
                b.close()
            matrix.proxy.restore()
    return [phase + "-independent-ceiling", "admitted-B-waits-without-allocation",
            "retained-until-old-pair-retired", "B-cold-computed-recovery"]


def execution_pressure(matrix, host, global_limit):
    wait_work(matrix, 0, 0, time.monotonic() + 5)
    with collecting(QuotaHold) as hold:
        variables = {"IMAGE_FIXTURE_COLLECTOR":f"{host}:{hold.server_port}", "IMAGE_FIXTURE_TOKEN":hold.token}
        names = [matrix.create(Timeout=900, Environment={"Variables":variables}) for _ in range(2 if global_limit else 1)]
        probe = matrix.create() if global_limit else names[0]
        hold.targets(names)
        if global_limit:
            for name in names:
                matrix.client.put_function_concurrency(FunctionName=name, ReservedConcurrentExecutions=32)
                assert matrix.client.get_function_concurrency(FunctionName=name)["ReservedConcurrentExecutions"] == 32
        else:
            assert "ReservedConcurrentExecutions" not in matrix.client.get_function_concurrency(FunctionName=probe)
        total = 30 if global_limit else 7
        hold.expect_tickets(total)
        try:
            for name in names:
                matrix.event(name, {"mode":"hold-exit"})
            hold.wait_seen(len(names), time.monotonic() + 20)
            physical = matrix.metadata.pair_limits() if global_limit else pair(matrix)
            for ticket in range(total):
                name = names[ticket // 15] if global_limit else names[0]
                matrix.event(name, {"mode":"counted", "value":8, "ticket":ticket})
            count = 32 if global_limit else 8
            wait_work(matrix, count, count, time.monotonic() + 10)
            matrix.expect_error(lambda: matrix.invoke(probe), "TooManyRequestsException")
            assert matrix.metadata.ids() == physical and not hold.completed
            held, old = held_exit(matrix, hold, names[0])
            assert held == physical
            capacity(matrix, len(names), len(names) * 192)
            wait_work(matrix, count, count, time.monotonic() + 5)
            matrix.expect_error(lambda: matrix.invoke(probe), "TooManyRequestsException")
            assert matrix.metadata.ids() == physical and not hold.completed
            relay_only = held_relay(matrix, held, old)
            wait_work(matrix, count, count, time.monotonic() + 5)
            capacity(matrix, len(names), len(names) * 192)
            matrix.expect_error(lambda: matrix.invoke(probe), "TooManyRequestsException")
            assert matrix.metadata.ids() == relay_only and not hold.completed
            matrix.proxy.release()
            wait_absent(matrix, old, time.monotonic() + 15)
            completed = hold.wait_completed(time.monotonic() + 70)
            assert len(completed) == total
            with contextlib.closing(boto3.client("lambda", endpoint_url=matrix.endpoint, region_name="us-east-1",
                    aws_access_key_id="local-key", aws_secret_access_key="local-secret",
                    config=matrix.client.meta.config.merge(Config(read_timeout=30)))) as client:
                # Only explicit429 may lag the already observed event computations.
                deadline = time.monotonic() + 40
                while True:
                    try:
                        output, _ = matrix.invoke(probe, {"value":8}, client=client)
                        assert time.monotonic() < deadline and output["computed"] == 15
                        break
                    except ClientError as error:
                        assert error.response["Error"]["Code"] == "TooManyRequestsException"
                        assert time.monotonic() < deadline
                        time.sleep(0.025)
            if not global_limit:
                assert output["counter"] == 8
                assert sorted(row["counter"] for row in completed.values()) == list(range(1, 8))
            else:
                hold.release_target(names[1])
            retire(matrix, probe)
            matrix.metadata.wait_ids(set(), time.monotonic() + 15)
            wait_work(matrix, 0, 0, time.monotonic() + 5)
            matrix.ready()
            for name in dict.fromkeys(names + [probe]):
                matrix.delete(name)
        finally:
            matrix.proxy.restore()
    phase = "global_limit" if global_limit else "default_limit"
    return [phase + "-positive-capacity-proof", "held-delete-retains-charge",
            "physical-retirement-before-recovery", "computed-recovery"]
