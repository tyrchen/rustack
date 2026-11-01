"""Fixture-only adapter; upstream test bodies and snapshot assertions are unchanged."""
import hashlib
import json
import os
from pathlib import Path
import signal

import boto3
from botocore.config import Config
import pytest
from localstack_snapshot.snapshots import SnapshotAssertionError, SnapshotSession

from localstack_policy import VENDOR, digest, exact, outcomes, publish, read, require

ROLE = "arn:aws:iam::000000000000:role/Workflows"


class RecordedSession(SnapshotSession):
    """Record verification outcome without changing upstream comparison semantics."""
    checked = False
    comparisons = 0
    passed = False

    def _assert_all(self, *args, **kwargs):
        self.checked = True
        try:
            results = super()._assert_all(*args, **kwargs)
        except SnapshotAssertionError as error:
            self.comparisons = len(error.result)
            raise
        self.comparisons = len(results)
        self.passed = bool(results) and all(bool(result) for result in results)
        return results


class Clients:
    """Explicit signed local clients; no credential or endpoint discovery."""
    def __init__(self, endpoint, credentials):
        self.endpoint = endpoint
        self.credentials = credentials
        self.clients = {}

    def __getattr__(self, name):
        require(name in ("stepfunctions", "lambda_"), "compat-service-outside-profile")
        if name not in self.clients:
            self.clients[name] = boto3.client(name.rstrip("_"), endpoint_url=self.endpoint,
                region_name="us-east-1", aws_access_key_id=self.credentials["accessKey"],
                aws_secret_access_key=self.credentials["secretKey"],
                config=Config(signature_version="v4", connect_timeout=2, read_timeout=10,
                              retries={"max_attempts": 0}))
        return self.clients[name]

    def close(self):
        for client in self.clients.values():
            client.close()


class Fixtures:
    """Private snapshot sessions, exact outcomes and public-API cleanup ownership."""
    def __init__(self, evidence, endpoint, mode, selected, images, credentials, collecting=False):
        self.root = evidence
        self.endpoint = endpoint
        self.mode = mode
        self.selected = selected
        self.images = images
        self.credentials = credentials
        self.rows = {}
        self.snapshots = {}
        self.collected = []
        self.collecting = collecting

    def pytest_configure(self, config):
        for marker in ("aws_validated", "aws_needs_fixing", "requires_docker", "skip_snapshot_verify"):
            config.addinivalue_line("markers", marker)

    def pytest_collection_finish(self, session):
        self.collected = [item.nodeid for item in session.items]
        publish(self.root / ("preflight-collection.json" if self.collecting else "collection.json"), self.collected)
        exact(self.selected, self.collected)

    @pytest.hookimpl(tryfirst=True)
    def pytest_runtest_setup(self, item):
        def expired(_number, _frame):
            raise TimeoutError("compat-case-deadline")
        self.previous_signal = signal.signal(signal.SIGALRM, expired)
        signal.setitimer(signal.ITIMER_REAL, 180)

    @pytest.hookimpl(trylast=True)
    def pytest_runtest_teardown(self, item, nextitem):
        signal.setitimer(signal.ITIMER_REAL, 0)
        signal.signal(signal.SIGALRM, self.previous_signal)

    def pytest_runtest_logreport(self, report):
        row = self.rows.setdefault(report.nodeid, {})
        row[report.when] = report.outcome
        row["xfail"] = row.get("xfail", False) or bool(getattr(report, "wasxfail", False))

    @pytest.fixture
    def aws_client(self):
        clients = Clients(self.endpoint, self.credentials)
        try:
            yield clients
        finally:
            clients.close()

    @pytest.fixture
    def create_state_machine_iam_role(self):
        return lambda target_aws_client: ROLE

    @pytest.fixture
    def create_state_machine(self, aws_client, create_lambda_function):
        owned = []
        def create(clients, **kwargs):
            reply = clients.stepfunctions.create_state_machine(**kwargs)
            owned.append((clients.stepfunctions, reply["stateMachineArn"]))
            return reply
        yield create
        errors = []
        for client, arn in reversed(owned):
            try:
                token = None
                for _ in range(32):
                    result = client.list_executions(stateMachineArn=arn, statusFilter="RUNNING",
                        **({"nextToken": token} if token else {}))
                    for execution in result["executions"]:
                        client.stop_execution(executionArn=execution["executionArn"])
                    token = result.get("nextToken")
                    if not token:
                        break
                require(not token, "compat-cleanup-pagination")
                client.delete_state_machine(stateMachineArn=arn)
            except Exception as error:
                errors.append(type(error).__name__)
        require(not errors, "compat-machine-cleanup-failed")

    @pytest.fixture
    def create_lambda_function(self, aws_client):
        owned = []
        def create(*, func_name, handler_file, runtime, **kwargs):
            require(self.mode == "docker" and not kwargs and runtime == "python3.12",
                    "compat-unadapted-lambda-fixture")
            source = Path(handler_file).resolve(strict=True)
            name = str(source.relative_to(VENDOR))
            require(name in self.images and digest(read(source)) == self.images[name]["handlerSha256"],
                    "compat-handler-drift")
            reply = aws_client.lambda_.create_function(FunctionName=func_name, Role=ROLE,
                PackageType="Image", Code={"ImageUri": self.images[name]["imageId"]},
                Architectures=["arm64"], Timeout=30, MemorySize=128,
                Environment={"Variables":{}}, Tags={})
            owned.append(func_name)
            aws_client.lambda_.get_waiter("function_active_v2").wait(FunctionName=func_name,
                WaiterConfig={"Delay": 1, "MaxAttempts": 30})
            return {"CreateFunctionResponse": reply, "CreateEventSourceMappingResponse":None}
        yield create
        errors = []
        for name in reversed(owned):
            try:
                aws_client.lambda_.delete_function(FunctionName=name)
            except Exception as error:
                errors.append(type(error).__name__)
        require(not errors, "compat-function-cleanup-failed")

    @pytest.fixture
    def _snapshot_session(self, request):
        source = Path(str(request.fspath)).with_suffix(".snapshot.json")
        raw = read(source)
        key = hashlib.sha256(request.node.nodeid.encode()).hexdigest()
        directory = self.root / ("snapshot-" + key)
        directory.mkdir(mode=0o700)
        base = directory / "upstream"
        baseline = base.with_suffix(".snapshot.json")
        baseline.write_bytes(raw)
        os.chmod(baseline, 0o600)
        session = RecordedSession(base_file_path=str(base), scope_key=request.node.nodeid,
                                  update=False, raw=False, verify=True)
        require(isinstance(session.recorded_state, dict) and session.recorded_state, "compat-snapshot-key")
        expected_keys = set(session.recorded_state)
        self.snapshots[request.node.nodeid] = {"baselineSha256": digest(raw),
            "source": str(source.relative_to(VENDOR)), "directory": directory.name,
            "recordedDate": json.loads(raw)[request.node.nodeid]["recorded-date"]}
        try:
            yield session
        finally:
            require(digest(read(source)) == digest(raw) == digest(read(baseline)),
                    "compat-baseline-mutated")
            require(session.update is False and session.raw is False and session.verify is True,
                    "compat-verification-disabled")
            publish(directory / "observations.json", json.loads(json.dumps(session.observed_state,
                default=str)))
            assertions = {"keys": sorted(session.called_keys), "expectedKeys": sorted(expected_keys),
                "checked": session.checked, "comparisons": session.comparisons, "passed": session.passed,
                "upstreamSkipPaths": getattr(session, "skip_verification_paths", [])}
            publish(directory / "assertions.json", assertions)
            self.snapshots[request.node.nodeid]["assertions"] = assertions

    @pytest.fixture
    def snapshot(self, request, _snapshot_session):
        # Import this fixture as an ordinary function, never register the AWS
        # conftest's LocalStack lifecycle hooks. This preserves its exact base
        # normalizers and AWS-specific TransformerUtility without copying them.
        from tests.aws.conftest import snapshot as upstream_snapshot
        return upstream_snapshot.__wrapped__(request, _snapshot_session,
                                               "000000000000", "us-east-1")

    @pytest.fixture
    def sfn_snapshot(self, snapshot):
        snapshot.add_transformers_list(snapshot.transform.stepfunctions_api())
        return snapshot

    def result(self):
        exact(self.selected, self.collected)
        outcomes(self.selected, self.rows)
        require(set(self.snapshots) == set(self.selected), "compat-missing-snapshot-session")
        require(all(row["assertions"]["checked"] and row["assertions"]["passed"]
                and row["assertions"]["keys"] == row["assertions"]["expectedKeys"]
                for row in self.snapshots.values()), "compat-incomplete-snapshot-assertions")
        return {"nodes": self.rows, "snapshots": self.snapshots}
