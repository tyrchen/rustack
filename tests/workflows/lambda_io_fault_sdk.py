"""Owned real-daemon proxy plus signed public lifecycle/deployment acceptance."""
import json
import os
import http.client
import urllib.parse
from pathlib import Path
import sys
import time
import traceback
import uuid

print(os.getpid(), flush=True)
import boto3
from botocore.config import Config

from custom_ric_sdk import collecting
from docker_io_proxy import Proxy
from lambda_fault_sdk import Faults
from lambda_fault_support import Hold, Metadata
from required_suites import read_regular


class IoCases(Faults):
    def __init__(self, endpoint, image, other, architecture, metadata, proxy):
        super().__init__(endpoint, image, architecture, metadata)
        config = self.client.meta.config.merge(Config(read_timeout=10))
        self.client.close()
        self.client = boto3.client("lambda", endpoint_url=endpoint, region_name="us-east-1",
                                   aws_access_key_id="local-key", aws_secret_access_key="local-secret",
                                   config=config)
        self.other, self.proxy = other, proxy
        self.server_lost = False

    def state(self, name, predicate):
        deadline = time.monotonic() + 20
        while True:
            assert time.monotonic() < deadline, "deployment observation expired"
            result = self.client.get_function_configuration(FunctionName=name)
            assert time.monotonic() < deadline, "late deployment state observation"
            if predicate(result):
                return result
            time.sleep(0.025)

    def cold(self, name):
        self.client.update_function_configuration(FunctionName=name, Environment={"Variables":{}})
        self.settled(name, updating=True)
        output, _ = self.cold_recovery(name, time.monotonic() + 5)
        assert output["computed"] == 15 and output["counter"] == 1

    def cancel_io(self, host, phase):
        with collecting(lambda: Hold("initializing")) as hold:
            variables = {} if phase == "delete" else {
                "IMAGE_FIXTURE_INIT_MODE":"hold", "IMAGE_FIXTURE_TOKEN":hold.token,
                "IMAGE_FIXTURE_COLLECTOR":f"{host}:{hold.server_port}"}
            name = self.create(Timeout=900, Environment={"Variables":variables})
            hold.targets([name])
            self.client.put_function_concurrency(FunctionName=name, ReservedConcurrentExecutions=1)
            self.proxy.arm(phase, self.metadata.owner)
            connection = self.raw_invoke(name, {"mode":"exit"} if phase == "delete" else {})
            try:
                record = self.proxy.wait(1, time.monotonic() + 15)[0]
                rows = self.metadata.rows()
                ids = {row["Id"] for row in rows}
                assert len(ids) == 2 and record["id"] in ids
                function = self.metadata.get("/containers/" + record["id"] + "/json")
                assert function["State"]["Running"] is False
                if phase == "create":
                    assert function["State"]["Status"] == "created"
                else:
                    assert function["State"]["ExitCode"] == 42
                connection.close()
                self.expect_error(lambda: self.invoke(name), "TooManyRequestsException")
                assert self.metadata.ids() == ids
                assert len(self.proxy.observed) == 1 and not self.proxy.released
                self.proxy.release()
                self.metadata.wait_ids(set(), time.monotonic() + 10)
                if phase == "create":
                    assert record["id"] not in self.proxy.starts, "canceled allocation started user code"
                assert not hold.observed
                self.ready()
                self.cold(name)
                self.delete(name)
            finally:
                connection.close()
                self.proxy.restore()
        return [phase + "-held-real-io", "retained-reserved-slot", "caller-disconnected",
                "post-release-physical-retirement", "cold-computed-recovery"]

    def loss_update(self):
        name = self.create()
        output, _ = self.invoke(name, {"value":8})
        assert output["computed"] == 15 and output["imageRevision"] == "A"
        published = self.client.publish_version(FunctionName=name)["Version"]
        before = self.client.get_function_configuration(FunctionName=name)
        self.proxy.fail_image(self.other)
        try:
            self.client.update_function_code(FunctionName=name, ImageUri=self.other)
            failed = self.state(name, lambda row: row.get("LastUpdateStatus") == "Failed")
            assert failed["State"] == "Active"
            assert failed["CodeSha256"] == before["CodeSha256"]
            assert failed["RevisionId"] == before["RevisionId"]
        finally:
            self.proxy.restore()
        self.client.update_function_configuration(FunctionName=name, Environment={"Variables":{"RECOVERED":"1"}})
        self.settled(name, updating=True)
        output, _ = self.invoke(name, {"value":8}, qualifier=published)
        assert output["computed"] == 15 and output["imageRevision"] == "A"
        self.client.update_function_code(FunctionName=name, ImageUri=self.other)
        self.settled(name, updating=True)
        output, _ = self.invoke(name, {"value":8})
        assert output["computed"] == 15 and output["imageRevision"] == "B"
        missing = "MissingImage" + uuid.uuid4().hex
        self.client.create_function(FunctionName=missing, Role="arn:aws:iam::000000000000:role/Docker",
                                    PackageType="Image", Architectures=[self.architecture],
                                    Code={"ImageUri":"rustack-custom-ric:absent-" + uuid.uuid4().hex})
        self.functions.append(missing)
        failed = self.state(missing, lambda row: row["State"] == "Failed")
        assert failed["State"] != "Active"
        self.expect_error(lambda: self.invoke(missing), "ResourceNotReadyException")
        assert len(self.metadata.ids()) >= 2, "full-loss stage lacked live environment IO"
        self.server_lost = True
        self.proxy.loss()
        held = self.raw_invoke(name, {})
        address = urllib.parse.urlsplit(self.endpoint)
        deadline = time.monotonic() + 10
        try:
            while True:
                assert time.monotonic() < deadline, "full transport loss did not revoke readiness"
                connection = http.client.HTTPConnection(address.hostname, address.port, timeout=1)
                try:
                    connection.request("GET", "/_rustack/capabilities")
                    response = connection.getresponse()
                    raw = response.read(65537)
                    assert response.status in (200, 503) and len(raw) <= 65536
                    value = json.loads(raw)
                    if value["ready"] is False and value["workers"]["docker"]["ready"] is False:
                        assert time.monotonic() < deadline
                        break
                finally:
                    connection.close()
                time.sleep(0.025)
            self.expect_error(lambda: self.invoke(name), "ServiceException", "Docker execution is not available")
        finally:
            held.close()
            self.proxy.restore()
        return ["image-management-ack-lost", "failed-update-preserves-active-A",
                "published-A-preserved", "recovered-B-computation", "missing-source-never-active",
                "full-loss-revokes-readiness"]

    def pending(self, name, image):
        self.client.create_function(FunctionName=name, Role="arn:aws:iam::000000000000:role/Docker",
                                    PackageType="Image", Architectures=[self.architecture],
                                    MemorySize=128, Code={"ImageUri":image})
        self.functions.append(name)

    def saturation(self):
        names = ["Deploy" + uuid.uuid4().hex for _ in range(3)]
        self.proxy.arm("tag", self.metadata.owner, 2)
        self.pending(names[0], self.image)
        self.pending(names[1], self.other)
        tagged = self.proxy.wait(2, time.monotonic() + 15)
        assert len({self.metadata.image_id(row["alias"]) for row in tagged}) == 2
        for name in names[:2]:
            assert self.client.get_function_configuration(FunctionName=name)["State"] == "Pending"
        self.expect_error(lambda: self.pending(names[2], self.image), "TooManyRequestsException")
        self.expect_error(lambda: self.client.get_function(FunctionName=names[2]), "ResourceNotFoundException")
        self.delete(names[0])
        self.expect_error(lambda: self.pending(names[2], self.image), "TooManyRequestsException")
        self.proxy.release()
        self.settled(names[1])
        self.expect_error(lambda: self.client.get_function(FunctionName=names[0]), "ResourceNotFoundException")
        output, _ = self.invoke(names[1], {"value":8})
        assert output["computed"] == 15 and output["imageRevision"] == "B"
        self.pending(names[2], self.image)
        self.settled(names[2])
        output, _ = self.invoke(names[2], {"value":8})
        assert output["computed"] == 15 and output["imageRevision"] == "A"
        return ["two-held-pending-deployments", "third-rejected-before-insertion",
                "deleted-preparation-retains-capacity", "deleted-job-never-resurrects", "capacity-recovery-computation"]


def wait_file(path, deadline, maximum=2048, abort=None):
    while not path.exists():
        assert abort is None or not abort.exists(), "parent canceled fixture handoff"
        assert time.monotonic() < deadline, "parent fixture handoff deadline"
        time.sleep(0.025)
    assert time.monotonic() < deadline
    return read_regular(path, maximum)


def main():
    root, daemon, image, other, architecture, phase, host = sys.argv[1:]
    root = Path(root)
    proxy = Proxy(str(root / "engine.sock"), daemon, root / "restore")
    print(os.getpid(), flush=True)
    failed, cases, matrix = False, [], None
    try:
        config = json.loads(wait_file(root / "config.json", time.monotonic() + 200, abort=root / "stop"))
        assert set(config) == {"endpoint", "owner", "engine"}
        matrix = IoCases(config["endpoint"], image, other, architecture,
                         Metadata(daemon, config["owner"], config["engine"]), proxy)
        if phase in ("create", "delete"):
            cases = matrix.cancel_io(host, phase)
        elif phase == "loss":
            cases = matrix.loss_update()
        else:
            assert phase == "saturation"
            cases = matrix.saturation()
    except BaseException:
        failed = True
        traceback.print_exc(file=sys.stderr)
    finally:
        proxy.restore()
        if matrix is not None:
            try:
                if not matrix.server_lost:
                    matrix.cleanup()
                matrix.client.close()
            except BaseException:
                failed = True
                traceback.print_exc(file=sys.stderr)
        print(os.getpid(), flush=True)
        try:
            wait_file(root / "stop", time.monotonic() + 90)
        finally:
            proxy.close()
    if failed:
        return 1
    print(json.dumps({"schemaVersion":1, "cases":cases}))
    return 0


if __name__ == "__main__":
    sys.exit(main())
