"""Causal allocation/cleanup disconnects through the same real daemon proxy."""
import time

from custom_ric_sdk import collecting
from lambda_fault_support import Hold
from provided_zip_sdk import Provided, variables


class ProvidedIo(Provided):
    def __init__(self, endpoint, codes, architecture, metadata, proxy):
        super().__init__(endpoint, codes, architecture, metadata)
        self.proxy, self.server_lost = proxy, False

    def cancel_io(self, host, phase):
        assert phase in ("create", "delete")
        with collecting(lambda: Hold("initializing")) as hold:
            environment = variables() if phase == "delete" else variables(
                PROVIDED_HOLD_STAGE="initializing", IMAGE_FIXTURE_TOKEN=hold.token,
                IMAGE_FIXTURE_COLLECTOR=f"{host}:{hold.server_port}")
            name = self.create(Timeout=900, Environment={"Variables":environment})
            hold.targets([name])
            self.client.put_function_concurrency(FunctionName=name, ReservedConcurrentExecutions=1)
            self.proxy.arm(phase, self.metadata.owner)
            event = {"n":21} if phase == "create" else {"n":21,"providedAction":"exit"}
            connection = self.raw_invoke(name, event)
            try:
                record = self.proxy.wait(1, time.monotonic() + 20)[0]
                rows = self.metadata.rows()
                ids = {row["Id"] for row in rows}
                assert len(ids) == 2 and record["id"] in ids
                function = self.metadata.get("/containers/" + record["id"] + "/json")
                assert function["State"]["Running"] is False
                assert function["State"]["Status"] == ("created" if phase == "create" else "exited")
                if phase == "delete":
                    assert function["State"]["ExitCode"] == 23
                connection.close()
                self.expect_error(lambda: self.invoke(name, {"n":23}), "TooManyRequestsException")
                assert self.metadata.ids() == ids
                assert len(self.proxy.observed) == 1 and not self.proxy.released
                self.proxy.release()
                self.metadata.wait_ids(set(), time.monotonic() + 15)
                if phase == "create":
                    assert record["id"] not in self.proxy.starts
                assert not hold.observed
                self.ready()
                self.cold(name)
                self.delete(name)
            finally:
                connection.close()
                self.proxy.restore()
        return ["zip_" + phase + "-held-real-io", "retained-reserved-slot", "caller-disconnected",
                "post-release-physical-retirement", "cold-computed-recovery"]
