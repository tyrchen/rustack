"""Actual managed-image callbacks with independent Event work and safe control rows."""

import json
import os
from pathlib import Path
import re
import sys
import time
import uuid

from callback_image_support import ScanBudget
from lambda_fault_support import Metadata, invoke_json, poll_response
from node_image_sdk import deployed


VERSION = "3.1105.0"
VERSIONS = {"lambda": VERSION, "sfn": VERSION, "dynamodb": VERSION}
GROUPS = {"causal": ("early", "late", "externalFailure", "invocationFailure"),
          "timing": ("retry", "heartbeat", "absoluteTimeout", "heartbeatTimeout", "stop")}
FIELDS = {"id", "nonce", "role", "requestId", "tokenHash", "stage", "retry", "heartbeats",
          "historySafe", "closedToken", "oldTokenRejected", "sdkVersion"}


def canonical_uuid(value):
    assert isinstance(value, str) and len(value) == 36 and str(uuid.UUID(value, version=4)) == value


def catalog(group):
    assert group in GROUPS
    raw = Path(__file__).with_name("callback-cases.json").read_bytes()
    assert len(raw) <= 8192
    source = json.loads(raw)
    assert source["source"] == "https://docs.aws.amazon.com/step-functions/latest/dg/connect-lambda.html"
    assert len(source["cases"]) == 9
    selected = [case for case in source["cases"] if case["mode"] in GROUPS[group]]
    assert tuple(case["mode"] for case in selected) == GROUPS[group]
    return selected


class Driver:
    def __init__(self, function, states, dynamodb, metadata, architecture, group):
        self.function, self.states, self.ddb = function, states, dynamodb
        self.metadata, self.architecture, self.group = metadata, architecture, group
        suffix = uuid.uuid4().hex
        self.a, self.b, self.table = "CbDispatch-" + suffix, "CbWorker-" + suffix, "ImageCallback" + suffix
        self.scanner = ScanBudget()
        self.executions, self.workers = [], []
        self.b_ids = set()
        self.a_ids = set()

    def ids(self):
        return {row["Id"] for row in self.metadata.rows()}

    def probe(self, name):
        response, value = invoke_json(self.function, name, {"mode": "probe"})
        assert "FunctionError" not in response and value["probe"] is True and value["versions"] == VERSIONS
        assert value["architecture"] == {"arm64": "arm64", "x86_64": "x64"}[self.architecture]
        assert type(value["counter"]) is int and 1 <= value["counter"] <= 128
        canonical_uuid(value["requestId"])
        return value

    def prepare(self, image):
        self.ddb.create_table(TableName=self.table, BillingMode="PAY_PER_REQUEST",
            KeySchema=[{"AttributeName": "id", "KeyType": "HASH"}],
            AttributeDefinitions=[{"AttributeName": "id", "AttributeType": "S"}])
        poll_response(lambda: self.ddb.describe_table(TableName=self.table),
                      lambda value: value["Table"]["TableStatus"] == "ACTIVE", 30)
        for name, handler in ((self.a, "callback.dispatcher"), (self.b, "callback.worker")):
            self.function.create_function(FunctionName=name, Role="arn:aws:iam::000000000000:role/ImageCallback",
                PackageType="Image", Architectures=[self.architecture], Timeout=45, MemorySize=256,
                Code={"ImageUri": image}, ImageConfig={"Command": [handler]},
                Environment={"Variables": {"AWS_PROFILE": "captured-callback-poison",
                    "AWS_ENDPOINT_URL": "http://127.0.0.1:9"}})
            configuration = deployed(self.function, name)
            variables = configuration["Environment"]["Variables"]
            assert "AWS_ACCESS_KEY_ID" not in variables and "AWS_SECRET_ACCESS_KEY" not in variables
            self.function.put_function_concurrency(FunctionName=name, ReservedConcurrentExecutions=1)
            assert self.function.get_function_concurrency(FunctionName=name)["ReservedConcurrentExecutions"] == 1
        assert self.probe(self.b)["counter"] == 1
        self.b_ids = self.ids()
        assert len(self.b_ids) == 2
        assert self.probe(self.a)["counter"] == 1
        self.a_ids = self.ids() - self.b_ids
        assert len(self.a_ids) == 2 and self.ids() == self.a_ids | self.b_ids

    def rows(self):
        response = self.ddb.scan(TableName=self.table, ConsistentRead=True, Limit=65)
        items = response["Items"]
        assert not response.get("LastEvaluatedKey") and len(items) <= 64
        records = []
        for item in items:
            assert len(json.dumps(item).encode()) <= 2048
            if set(item) == {"id", "open"}:
                assert set(item["id"]) == {"S"} and set(item["open"]) == {"BOOL"}
                assert re.fullmatch(r"gate:[0-9a-f]{32}:[01]:(return|finish|heartbeats)", item["id"]["S"])
                assert type(item["open"]["BOOL"]) is bool
                continue
            assert set(item) == FIELDS
            row = {}
            for key, value in item.items():
                kind = "N" if key in ("retry", "heartbeats") else "BOOL" if key in (
                    "historySafe", "closedToken", "oldTokenRejected") else "S"
                assert set(value) == {kind}
                row[key] = int(value[kind]) if kind == "N" else value[kind]
            canonical_uuid(row["id"])
            assert row["id"] == row["requestId"] and row["role"] in ("A", "B")
            assert re.fullmatch(r"[0-9a-f]{32}", row["nonce"])
            assert row["stage"] in ("consumed", "callback", "accepted", "waiting", "heartbeat", "finished")
            assert 0 <= row["retry"] <= 1 and 0 <= row["heartbeats"] <= 100 and row["sdkVersion"] == VERSION
            assert all(type(row[key]) is bool for key in ("historySafe", "closedToken", "oldTokenRejected"))
            self.scanner.add_hashes([row["tokenHash"]])
            records.append(row)
        return records

    def observe(self, nonce, role, retry, stages):
        def read():
            found = [row for row in self.rows() if row["nonce"] == nonce and row["role"] == role
                     and row["retry"] == retry]
            assert len(found) <= 1, "duplicate image callback invocation"
            return found[0] if found else None
        return poll_response(read, lambda row: row is not None and row["stage"] in stages, 30)

    def gate(self, nonce, kind, retry=0):
        assert isinstance(nonce, str) and re.fullmatch(r"[0-9a-f]{32}", nonce)
        assert kind in ("return", "finish", "heartbeats") and type(retry) is int and retry in (0, 1)
        self.ddb.put_item(TableName=self.table,
            Item={"id": {"S": "gate:" + nonce + ":" + str(retry) + ":" + kind}, "open": {"BOOL": True}})

    def history(self, arn):
        value = self.states.get_execution_history(executionArn=arn, includeExecutionData=True, maxResults=1000)
        assert not value.get("nextToken") and len(value["events"]) <= 256
        encoded = json.dumps(value, default=str).encode()
        self.scanner.scan(encoded)
        return value["events"]

    def running(self, arn):
        assert self.states.describe_execution(executionArn=arn)["status"] == "RUNNING"

    def submitted(self, arn):
        return poll_response(lambda: self.history(arn),
            lambda events: any(event["type"] == "TaskSubmitted" for event in events), 30)

    def terminal(self, arn):
        return poll_response(lambda: self.states.describe_execution(executionArn=arn),
                             lambda result: result["status"] != "RUNNING", 30)

    def create(self, definition):
        return self.states.create_state_machine(name="Callback" + uuid.uuid4().hex,
            roleArn="arn:aws:iam::000000000000:role/ImageCallback", type="STANDARD",
            definition=json.dumps(definition))["stateMachineArn"]

    def unrelated(self):
        definition = {"StartAt": "Call", "States": {"Call": {"Type": "Task",
            "Resource": "arn:aws:lambda:us-east-1:000000000000:function:" + self.a,
            "Parameters": {"mode": "compute", "n": 21}, "End": True}}}
        machine = self.create(definition)
        arn = self.states.start_execution(stateMachineArn=machine)["executionArn"]
        self.executions.append(arn)
        value = self.terminal(arn)
        assert value["status"] == "SUCCEEDED" and json.loads(value["output"])["computed"] == 42

    def definition(self, case, nonce):
        task = {"Type": "Task", "Resource": "arn:aws:states:::lambda:invoke.waitForTaskToken",
            "Parameters": {"FunctionName": self.a, "InvocationType": "RequestResponse", "Payload": {
                "token.$": "$$.Task.Token", "executionArn.$": "$$.Execution.Id", "nonce": nonce,
                "table": self.table, "worker": self.b, "n": 21,
                "retry.$": "$$.State.RetryCount", "mode": case["mode"]}}, "TimeoutSeconds": 60, "End": True}
        if case["mode"] in ("heartbeat", "absoluteTimeout", "heartbeatTimeout"):
            task["HeartbeatSeconds"] = 3
        if case["mode"] == "absoluteTimeout":
            task["TimeoutSeconds"] = 8
        if case["mode"] == "retry":
            task["Retry"] = [{"ErrorEquals": ["CallbackRetry"], "IntervalSeconds": 1, "MaxAttempts": 1}]
        return {"StartAt": "Call", "TimeoutSeconds": 120, "States": {"Call": task}}

    def drain_worker(self, nonce):
        complete = self.observe(nonce, "B", 0, ("finished",))
        assert complete["historySafe"] and complete["closedToken"]
        self.workers.append(complete["requestId"])
        from botocore.exceptions import ClientError
        def probe():
            try:
                return self.probe(self.b)
            except ClientError as error:
                assert error.response["Error"]["Code"] == "TooManyRequestsException"
                return None
        poll_response(probe, lambda value: value is not None, 30)
        return complete

    def verify(self, case, arn, attempts, completed):
        result = self.terminal(arn)
        assert result["status"] == case["status"] and result.get("error") == case.get("error")
        if case["status"] == "SUCCEEDED":
            output = json.loads(result["output"])
            assert output == {"value": 42, "sdk": "node", "requestId": completed["requestId"],
                              "retry": completed["retry"]}
        events = self.history(arn)
        submitted = []
        for index, event in enumerate(events, start=1):
            assert event["id"] == index and event.get("previousEventId", 0) == (0 if index == 2 else index - 1)
            if event["type"] == "TaskScheduled":
                parameters = json.loads(event["taskScheduledEventDetails"]["parameters"])
                assert parameters["Payload"]["token"] is None
            if event["type"] == "TaskSubmitted":
                detail = event["taskSubmittedEventDetails"]
                metadata = json.loads(detail["output"])
                request = metadata["SdkResponseMetadata"]["RequestId"]
                assert request in {row["requestId"] for row in attempts}
                assert metadata["ExecutedVersion"] == "$LATEST" and metadata["SdkHttpMetadata"]["HttpStatusCode"] == 200
                assert detail["outputDetails"]["truncated"] is True
                submitted.append(request)
        count = 0 if case["mode"] in ("stop", "invocationFailure") else 2 if case["mode"] == "retry" else 1
        assert len(submitted) == len(set(submitted)) == count

    def exercise(self, case):
        nonce = uuid.uuid4().hex
        machine = self.create(self.definition(case, nonce))
        arn = self.states.start_execution(stateMachineArn=machine, name="run-" + nonce)["executionArn"]
        self.executions.append(arn)
        mode = case["mode"]
        first = self.observe(nonce, "A", 0, ("callback",) if mode in (
            "early", "externalFailure", "invocationFailure", "retry") else ("accepted",))
        attempts, complete = [first], first
        self.running(arn)
        if mode in ("early", "externalFailure", "invocationFailure", "retry"):
            assert first["historySafe"] and first["closedToken"]
            assert not any(event["type"] == "TaskSubmitted" for event in self.history(arn))
            self.gate(nonce, "return")
            if mode == "retry":
                second = self.observe(nonce, "A", 1, ("callback",))
                assert second["oldTokenRejected"] and second["historySafe"] and second["closedToken"]
                assert second["requestId"] != first["requestId"] and second["tokenHash"] != first["tokenHash"]
                assert self.ids() == self.a_ids | self.b_ids
                attempts.append(second)
                complete = second
                self.running(arn)
                self.gate(nonce, "return", retry=1)
        else:
            waiting = self.observe(nonce, "B", 0, ("waiting",))
            assert waiting["tokenHash"] == first["tokenHash"] and self.ids() == self.a_ids | self.b_ids
            if mode == "stop":
                self.states.stop_execution(executionArn=arn, error="Stopped")
                self.metadata.wait_ids(self.b_ids, time.monotonic() + 30)
            else:
                if mode == "heartbeat":
                    time.sleep(3.6)
                    self.running(arn)
                    assert not any(event["type"] == "TaskSubmitted" for event in self.history(arn))
                    self.gate(nonce, "return")
                self.submitted(arn)
                self.running(arn)
                if mode == "late":
                    self.unrelated()
                    self.running(arn)
                if mode in ("heartbeat", "absoluteTimeout"):
                    self.gate(nonce, "heartbeats")
                if mode == "heartbeat":
                    time.sleep(3.6)
                    self.running(arn)
                    beat = self.observe(nonce, "B", 0, ("heartbeat",))
                    assert beat["heartbeats"] >= 4
                if mode in ("absoluteTimeout", "heartbeatTimeout"):
                    self.terminal(arn)
            self.gate(nonce, "finish")
            complete = self.drain_worker(nonce)
            if mode == "absoluteTimeout":
                assert complete["heartbeats"] >= 4
        self.verify(case, arn, attempts, complete)
        if mode == "invocationFailure":
            self.metadata.wait_ids(self.b_ids, time.monotonic() + 30)


def exercise(endpoint, image, architecture, socket, owner, engine, group):
    import boto3
    from botocore.config import Config
    settings = dict(endpoint_url=endpoint, region_name="us-east-1", aws_access_key_id="local-key",
                    aws_secret_access_key="local-secret", config=Config(retries={"max_attempts": 0},
                    connect_timeout=2, read_timeout=60))
    clients = []
    try:
        for service in ("lambda", "stepfunctions", "dynamodb"):
            clients.append(boto3.client(service, **settings))
        driver = Driver(*clients, Metadata(socket, owner, engine), architecture, group)
        driver.prepare(image)
        cases = catalog(group)
        for case in cases:
            driver.exercise(case)
        rows = driver.rows()
        assert {row["requestId"] for row in rows if row["role"] == "B"} == set(driver.workers)
        assert all(row["stage"] == "finished" for row in rows if row["role"] == "B")
        report = {"schemaVersion": 1, "group": group, "cases": [case["mode"] for case in cases],
            "versions": VERSIONS, "functions": [driver.a, driver.b], "executions": driver.executions,
            "workerRequests": driver.workers, "hashes": sorted(driver.scanner.hashes),
            "historyCandidates": driver.scanner.candidates, "historyBytes": driver.scanner.history_bytes}
        assert len(json.dumps(report).encode()) <= 16384
        return report
    finally:
        for client in reversed(clients):
            client.close()


if __name__ == "__main__":
    print(os.getpid(), flush=True)
    try:
        print(json.dumps(exercise(*sys.argv[1:])), flush=True)
    except Exception:
        # SDK failures can contain request credentials; only a fixed diagnostic crosses stdout/stderr.
        print("image callback driver failed", file=sys.stderr, flush=True)
        sys.exit(1)
