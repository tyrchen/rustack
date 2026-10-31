"""Real Pulumi CLI lifecycle in an owned image Lambda; this child spawns no process."""

import hashlib
import json
import os
import re
import sys
import time
import uuid

from botocore.exceptions import ClientError
from custom_ric_sdk import unique_object
from lambda_fault_support import Metadata, invoke_json, poll_response
from node_image_sdk import deployed
from pulumi_archive import Archive, checkpoint, document, evidence_directory, private, require

VERSIONS = {"cli": "3.273.0", "provider": "7.52.1", "sdk": "3.273.0", "aws": "7.52.1",
            "node": "24.22.1", "dynamodb": "3.1105.0"}
COMMANDS = ("version", "login", "init", "create", "refresh-created", "update", "refresh-updated",
            "reject-publish", "recover-publish", "settle-publish", "reject-logging", "recover-logging", "settle-logging",
            "reject-encryption", "recover-encryption", "settle-encryption", "destroy", "remove")
STAGES = (("created", 4), ("refreshed-created", 5), ("updated", 6), ("refreshed-updated", 7),
          ("rejected-publish", 8), ("recovered-publish", 10), ("rejected-logging", 11), ("recovered-logging", 13),
          ("rejected-encryption", 14), ("recovered-encryption", 16),
          ("pre-destroy", 16), ("post-destroy", 17), ("post-removal", 18))
SECRETS = (b"local-key", b"local-secret")


def canonical_uuid(value):
    require(isinstance(value, str) and len(value) == 36 and str(uuid.UUID(value, version=4)) == value)


def expected_definition(business, value):
    require(value in (21, 23))
    return {"StartAt": "Call", "States": {"Call": {"Type": "Task", "Resource": "arn:aws:states:::lambda:invoke",
        "Parameters": {"FunctionName": business, "Payload": {"value": value}},
        "ResultSelector": {"computed.$": "$.Payload.computed", "requestId.$": "$.Payload.requestId",
            "nodeVersion.$": "$.Payload.nodeVersion", "architecture.$": "$.Payload.architecture",
            "counter.$": "$.Payload.counter"}, "End": True}}}


class Driver:
    def __init__(self, function, states, ddb, metadata, architecture, mode, evidence_root):
        require(mode in ("lifecycle", "stop"))
        self.function, self.states, self.ddb, self.metadata = function, states, ddb, metadata
        self.architecture, self.mode, self.nonce = architecture, mode, uuid.uuid4().hex
        self.tool, self.business, self.table = "IacTool" + self.nonce, "IacBusiness" + self.nonce, "IacControl" + self.nonce
        self.target, self.controller = "IacTarget" + self.nonce, "IacController" + self.nonce
        self.target_arn = "arn:aws:states:us-east-1:000000000000:stateMachine:" + self.target
        self.role = "arn:aws:iam::000000000000:role/ImageIac"
        self.executions, self.observations, self.audit_stages = [], [], []
        self.business_requests, self.business_counter = [], 0
        self.deadline = time.monotonic() + 960
        self.progress = "start"
        self.evidence_root = evidence_directory(evidence_root)

    def fresh(self):
        require(time.monotonic() < self.deadline)

    def prepare(self, tool_image, business_image):
        self.progress = "create-table"
        self.ddb.create_table(TableName=self.table, BillingMode="PAY_PER_REQUEST",
            KeySchema=[{"AttributeName": "id", "KeyType": "HASH"}], AttributeDefinitions=[{"AttributeName": "id", "AttributeType": "S"}])
        poll_response(lambda: self.ddb.describe_table(TableName=self.table), lambda row: row["Table"]["TableStatus"] == "ACTIVE", 30)
        for name, image, memory, timeout in ((self.business, business_image, 256, 30), (self.tool, tool_image, 1024, 720)):
            self.progress = "create-business" if name == self.business else "create-tool"
            result = self.function.create_function(FunctionName=name, Role=self.role, PackageType="Image",
                Architectures=[self.architecture], Timeout=timeout, MemorySize=memory, Code={"ImageUri": image})
            if name == self.business:
                self.business_arn = result["FunctionArn"]
            self.progress = "deploy-business" if name == self.business else "deploy-tool"
            deployed(self.function, name)
            self.function.put_function_concurrency(FunctionName=name, ReservedConcurrentExecutions=1)
            require(self.function.get_function_concurrency(FunctionName=name)["ReservedConcurrentExecutions"] == 1)
        self.progress = "probe-business"
        self.business_probe()
        self.business_ids = self.metadata.ids()
        require(len(self.business_ids) == 2)
        self.progress = "probe-tool"
        response, probe = invoke_json(self.function, self.tool, {"mode": "probe"})
        if "FunctionError" in response:
            reason = probe.get("errorMessage")
            allowed = {"bounded managed IaC fixture failed at " + stage for stage in
                       ("validation", "provenance", "versions", "setup", "lifecycle")}
            if reason in allowed:
                print(reason, file=sys.stderr, flush=True)
        require("FunctionError" not in response and probe["probe"] is True and probe["versions"] == VERSIONS and probe["counter"] == 1)
        require(probe["architecture"] == {"arm64": "arm64", "x86_64": "x64"}[self.architecture])
        canonical_uuid(probe["requestId"])
        self.progress = "inspect-pairs"
        rows = self.metadata.rows()
        self.tool_ids = {row["Id"] for row in rows} - self.business_ids
        require(len(self.tool_ids) == 2)
        function_rows = [row for row in rows if row["Id"] in self.tool_ids and row["Labels"]["rustack.lambda.role"] == "function"]
        require(len(function_rows) == 1)
        inspected = self.metadata.get("/containers/" + function_rows[0]["Id"] + "/json")
        resolved = self.function.get_function(FunctionName=self.tool)["Code"]["ResolvedImageUri"]
        require(resolved == "local://" + inspected["Image"])
        self.archive = Archive(self.metadata, function_rows[0], resolved[len("local://"):], SECRETS)
        total = 0
        for row in rows:
            inspected = self.metadata.get("/containers/" + row["Id"] + "/json")
            memory = inspected["HostConfig"]["Memory"]
            require(memory == (64 if row["Labels"]["rustack.lambda.role"] == "relay" else
                              1024 if row["Id"] in self.tool_ids else 256) * 1024 * 1024)
            require(inspected["HostConfig"]["MemorySwap"] == memory and inspected["State"]["Running"] is True)
            total += memory
        require(total == 1408 * 1024 * 1024)

    def remember_business(self, value):
        canonical_uuid(value["requestId"])
        require(value["requestId"] not in self.business_requests and len(self.business_requests) < 16 and
                type(value["counter"]) is int and value["counter"] == self.business_counter + 1)
        self.business_requests.append(value["requestId"])
        self.business_counter = value["counter"]

    def business_probe(self):
        response, value = invoke_json(self.function, self.business, {"value": 7})
        require("FunctionError" not in response and value["computed"] == 14 and value["uid"] == 1000 and
                value["rootReadOnly"] is True and value["temporaryRoundtrip"] is True)
        self.remember_business(value)

    def start(self):
        event = {"mode": self.mode, "nonce": self.nonce, "table": self.table, "businessArn": self.business_arn, "target": self.target}
        definition = {"StartAt": "Tool", "TimeoutSeconds": 840, "States": {"Tool": {
            "Type": "Task", "Resource": "arn:aws:states:::lambda:invoke", "TimeoutSeconds": 780,
            "Parameters": {"FunctionName": self.tool, "Payload": event}, "End": True}}}
        self.controller_arn = self.states.create_state_machine(name=self.controller, roleArn=self.role,
            definition=json.dumps(definition), type="STANDARD")["stateMachineArn"]
        self.controller_execution = self.states.start_execution(stateMachineArn=self.controller_arn)["executionArn"]

    def observe(self, stage, ordinal):
        self.progress = "observe-" + stage
        def read():
            self.fresh()
            controller = self.states.describe_execution(executionArn=self.controller_execution)
            if controller["status"] != "RUNNING":
                cause = controller.get("cause", "")
                for label in ("validation", "provenance", "versions", "setup", "lifecycle"):
                    if "bounded managed IaC fixture failed at " + label in cause:
                        print("managed tool failed at " + label, file=sys.stderr, flush=True)
                for kind in ("command-start", "command-end", "command-failed"):
                    result = self.ddb.get_item(TableName=self.table, ConsistentRead=True,
                        Key={"id": {"S": self.nonce + ":" + self.mode + ":" + kind}})
                    if "Item" in result:
                        progress = document(result["Item"]["payload"]["S"].encode(), SECRETS)
                        if progress.get("label") in COMMANDS and type(progress.get("ordinal")) is int and 0 <= progress["ordinal"] <= 20:
                            code = progress.get("code")
                            require(code is None or type(code) is int and 0 <= code <= 255)
                            category = progress.get("category", "none")
                            require(category in ("none", "module-missing", "plugin-missing", "home-missing", "user-missing", "permission",
                                                 "readonly", "syntax", "invalid-flag", "provider-validation", "connection", "gate-not-reached", "gate-validation", "other"))
                            print("managed tool " + kind + ":" + progress["label"] + ":" + str(progress["ordinal"]) + ":" + str(code) + ":" + category,
                                  file=sys.stderr, flush=True)
            require(controller["status"] == "RUNNING")
            response = self.ddb.get_item(TableName=self.table, ConsistentRead=True,
                Key={"id": {"S": self.nonce + ":" + self.mode + ":" + stage}})
            self.fresh()
            if "Item" not in response:
                failure = self.ddb.get_item(TableName=self.table, ConsistentRead=True,
                    Key={"id": {"S": self.nonce + ":" + self.mode + ":command-failed"}})
                if "Item" in failure:
                    failed = document(failure["Item"]["payload"]["S"].encode(), SECRETS)
                    require(failed["nonce"] == self.nonce and failed["mode"] == self.mode and
                            failed["label"] in COMMANDS and type(failed["ordinal"]) is int and 1 <= failed["ordinal"] <= 20)
                    canonical_uuid(failed["requestId"])
                    self.archive.read(failed["requestId"], "logs", self.evidence_root /
                        ("iac-failure-" + failed["requestId"] + "-logs.tar"))
                    raise ValueError("actual CLI failed before required live gate")
            return response
        result = poll_response(read, lambda row: "Item" in row, min(120, self.deadline - time.monotonic()))
        item = result["Item"]
        require(set(item) == {"id", "payload"} and len(json.dumps(item).encode()) <= 2048 and set(item["payload"]) == {"S"})
        row = document(item["payload"]["S"].encode(), SECRETS)
        require(row["nonce"] == self.nonce and row["mode"] == self.mode and row["stage"] == stage and
                row["ordinal"] == ordinal and row["targetArn"] == self.target_arn)
        canonical_uuid(row["requestId"])
        if not hasattr(self, "request_id"):
            self.request_id = row["requestId"]
        require(row["requestId"] == self.request_id)
        if stage in ("created", "updated"):
            require(set(row) == {"nonce", "mode", "stage", "ordinal", "requestId", "targetArn", "processes", "versions"} and row["versions"] == VERSIONS)
            processes = row["processes"]
            require(isinstance(processes, list) and 3 <= len(processes) <= 32)
            for process in processes:
                require(set(process) == {"pid", "group", "provider", "language", "cli"} and
                        all(type(process[key]) is int and 0 < process[key] < 2**31 for key in ("pid", "group")) and
                        all(type(process[key]) is bool for key in ("provider", "language", "cli")))
            require(all(any(process[kind] for process in processes) for kind in ("provider", "language", "cli")))
            require(len({process["pid"] for process in processes}) == len(processes))
        require(self.metadata.ids() == self.business_ids | self.tool_ids)
        self.observations.append({"stage": stage, "ordinal": ordinal})
        return row

    def grant(self, stage, ordinal):
        self.fresh()
        self.ddb.put_item(TableName=self.table, Item={"id": {"S": "gate:" + self.nonce + ":" + self.mode + ":" + str(ordinal) + ":" + stage}, "open": {"BOOL": True}})
        self.fresh()

    def target_state(self, value):
        response = self.states.describe_state_machine(stateMachineArn=self.target_arn)
        require(response["stateMachineArn"] == self.target_arn and response["name"] == self.target and
                response["roleArn"] == self.role and response["type"] == "STANDARD" and response["status"] == "ACTIVE")
        require(json.loads(response["definition"], object_pairs_hook=unique_object) == expected_definition(self.business_arn, value))
        tags = self.states.list_tags_for_resource(resourceArn=self.target_arn)["tags"]
        require(len(tags) == 2 and {tag["key"]: tag["value"] for tag in tags} == {"Fixture": self.nonce, "Value": str(value)})
        versions = self.states.list_state_machine_versions(stateMachineArn=self.target_arn, maxResults=1000)
        require(versions["stateMachineVersions"] == [] and not versions.get("nextToken"))
        revision = response["revisionId"]
        require(isinstance(revision, str) and 1 <= len(revision) <= 256)
        return revision, hashlib.sha256(response["definition"].encode()).hexdigest()

    def execute_target(self, expected):
        arn = self.states.start_execution(stateMachineArn=self.target_arn)["executionArn"]
        require(isinstance(arn, str) and len(arn) <= 256 and
                arn.startswith("arn:aws:states:us-east-1:000000000000:execution:" + self.target + ":") and
                arn not in self.executions)
        self.executions.append(arn)
        result = poll_response(lambda: self.states.describe_execution(executionArn=arn), lambda row: row["status"] != "RUNNING", 40)
        require(result["status"] == "SUCCEEDED")
        value = document(result["output"].encode(), SECRETS)
        require(set(value) == {"computed", "requestId", "nodeVersion", "architecture", "counter"} and value["computed"] == expected and
                value["nodeVersion"] == "24.22.1" and value["architecture"] == {"arm64": "arm64", "x86_64": "x64"}[self.architecture] and
                type(value["counter"]) is int and value["counter"] >= 2)
        self.remember_business(value)

    def absent_target(self):
        try:
            self.states.describe_state_machine(stateMachineArn=self.target_arn)
        except ClientError as error:
            require(error.response["Error"]["Code"] == "StateMachineDoesNotExist")
        else:
            raise ValueError("IaC target still exists")
        listed = self.states.list_state_machines(maxResults=1000)
        require(not listed.get("nextToken") and all(row["stateMachineArn"] != self.target_arn for row in listed["stateMachines"]))

    def audit(self, stage, row):
        require(set(row) == {"nonce", "mode", "stage", "ordinal", "requestId", "targetArn", "checkpoint", "commands"})
        count = row["ordinal"]
        require(isinstance(row["commands"], list) and len(row["commands"]) == count)
        for command, (ordinal, label) in zip(row["commands"], enumerate(COMMANDS[:count], 1), strict=True):
            require(set(command) == {"ordinal", "label", "code"} and command["ordinal"] == ordinal and command["label"] == label)
            require(type(command["code"]) is int and (1 <= command["code"] <= 255 if label.startswith("reject-") else command["code"] == 0))
        backend = self.archive.read(self.request_id, "backend")
        checkpoint(backend, row["checkpoint"], stage, self.target_arn, SECRETS)
        logs = self.archive.read(self.request_id, "logs")
        require(len(logs) == count * 2)
        for ordinal, label in enumerate(COMMANDS[:count], 1):
            stdout = logs["logs/" + str(ordinal) + "-" + label + ".stdout"]
            stderr = logs["logs/" + str(ordinal) + "-" + label + ".stderr"]
            private(stdout + stderr, SECRETS)
            if label.startswith("reject-"):
                require(b"ValidationException" in stdout + stderr)
            elif label == "version":
                require(stdout.strip() == b"v3.273.0")
        self.audit_stages.append(stage)

    def exercise(self):
        self.start()
        row = self.observe("created", 4)
        initial = self.target_state(21)
        self.execute_target(42)
        if self.mode == "stop":
            self.states.stop_execution(executionArn=self.controller_execution, error="Stopped")
            self.metadata.wait_ids(self.business_ids, time.monotonic() + 40)
            result = self.states.describe_execution(executionArn=self.controller_execution)
            require(result["status"] == "ABORTED" and result["error"] == "Stopped")
            self.business_probe()
            require(self.metadata.ids() == self.business_ids)
            self.states.delete_state_machine(stateMachineArn=self.target_arn)
            self.absent_target()
            self.states.delete_state_machine(stateMachineArn=self.controller_arn)
            return
        self.grant("created", 4)
        self.observe("refreshed-created", 5)
        require(self.target_state(21) == initial)
        self.grant("refreshed-created", 5)
        self.observe("updated", 6)
        updated = self.target_state(23)
        require(updated[0] != initial[0] and updated[1] != initial[1])
        self.execute_target(46)
        self.grant("updated", 6)
        self.observe("refreshed-updated", 7)
        require(self.target_state(23) == updated)
        self.grant("refreshed-updated", 7)
        for negative, ordinal in (("publish", 8), ("logging", 11), ("encryption", 14)):
            stage = "rejected-" + negative
            self.observe(stage, ordinal)
            require(self.target_state(23) == updated)
            self.execute_target(46)
            self.grant(stage, ordinal)
            recovered = "recovered-" + negative
            self.observe(recovered, ordinal + 2)
            actual = self.target_state(23)
            require(actual[1] == updated[1])
            updated = actual
            self.execute_target(46)
            self.grant(recovered, ordinal + 2)
        for stage, ordinal in STAGES[-3:]:
            row = self.observe(stage, ordinal)
            if stage == "pre-destroy":
                require(self.target_state(23) == updated)
            else:
                self.absent_target()
            self.audit(stage, row)
            self.grant(stage, ordinal)
        result = poll_response(lambda: self.states.describe_execution(executionArn=self.controller_execution), lambda value: value["status"] != "RUNNING", 30)
        require(result["status"] == "SUCCEEDED")
        output = document(result["output"].encode(), SECRETS)["Payload"]
        require(output == {"requestId": self.request_id, "versions": VERSIONS, "commandCount": 18})
        self.absent_target()
        self.states.delete_state_machine(stateMachineArn=self.controller_arn)
        self.business_probe()


def exercise(endpoint, tool_image, business_image, architecture, socket, owner, engine, mode, evidence_root):
    import boto3
    from botocore.config import Config
    clients = []
    try:
        settings = dict(endpoint_url=endpoint, region_name="us-east-1", aws_access_key_id="local-key", aws_secret_access_key="local-secret",
                        config=Config(retries={"max_attempts": 0}, connect_timeout=2, read_timeout=100))
        for service in ("lambda", "stepfunctions", "dynamodb"):
            clients.append(boto3.client(service, **settings))
        driver = Driver(*clients, Metadata(socket, owner, engine), architecture, mode, evidence_root)
        try:
            driver.prepare(tool_image, business_image)
            driver.exercise()
        except Exception as error:
            kind = "sdk" if isinstance(error, ClientError) else "timeout" if isinstance(error, TimeoutError) else "validation"
            print("managed IaC observation failed: " + driver.progress + ":" + kind, file=sys.stderr, flush=True)
            point = getattr(getattr(driver, "archive", None), "progress", None)
            if point in ("inspect", "connect", "request", "status-200", "status-400", "status-403", "status-404", "status-500", "status-other", "read", "retain", "unpack", "exec-create", "exec-start", "exec-stream", "exec-join"):
                print("managed IaC archive failed: " + point, file=sys.stderr, flush=True)
            raise
        return {"schemaVersion": 1, "mode": mode, "versions": VERSIONS, "stages": driver.observations,
            "audits": driver.audit_stages, "commands": list(COMMANDS) if mode == "lifecycle" else list(COMMANDS[:4]),
            "executions": driver.executions, "businessRequests": driver.business_requests, "requestId": driver.request_id,
            "physicalStop": mode == "stop", "actualDestroy": mode == "lifecycle"}
    finally:
        for client in reversed(clients):
            client.close()


if __name__ == "__main__":
    print(os.getpid(), flush=True)
    try:
        print(json.dumps(exercise(*sys.argv[1:])), flush=True)
    except Exception:
        print("managed image IaC driver failed", file=sys.stderr, flush=True)
        sys.exit(1)
