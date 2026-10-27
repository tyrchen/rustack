"""Controls that prevent cancellation/metadata observations from passing vacuously."""
import concurrent.futures
import contextlib
import json
import socketserver
import tempfile
import threading
import time
import unittest
import uuid
from unittest.mock import Mock

from botocore.exceptions import ClientError
from custom_ric_sdk import collecting
from lambda_fault_support import Hold, Metadata, recover_before


@contextlib.contextmanager
def engine_fixture(engine, rows, change=False):
    with tempfile.TemporaryDirectory(prefix="rustack-fault-", dir="/tmp") as root:
        path = root + "/engine.sock"
        calls = []
        infos = []
        class Handler(socketserver.StreamRequestHandler):
            def handle(self):
                self.connection.settimeout(2)
                try:
                    for _ in range(4):
                        line = self.rfile.readline(4097)
                        if not line:
                            return
                        assert len(line) <= 4096
                        method, target, _version = line.decode().split()
                        assert method == "GET"
                        used = len(line)
                        while True:
                            header = self.rfile.readline(4097)
                            used += len(header)
                            assert header and used <= 4096
                            if header == b"\r\n":
                                break
                        calls.append(target)
                        if target == "/version":
                            body = {"ApiVersion":"1.53", "MinAPIVersion":"1.44"}
                        elif target.endswith("/info"):
                            infos.append(target)
                            body = {"ID":str(uuid.uuid4()) if change and len(infos) > 1 else engine}
                        else:
                            assert target.startswith("/v1.53/containers/json?")
                            body = rows
                        raw = json.dumps(body).encode()
                        self.wfile.write(b"HTTP/1.1 200 OK\r\nContent-Length: " + str(len(raw)).encode() + b"\r\n\r\n" + raw)
                        self.wfile.flush()
                except (OSError, AssertionError):
                    return
        with socketserver.UnixStreamServer(path, Handler) as server:
            worker = threading.Thread(target=server.serve_forever, kwargs={"poll_interval":0.01})
            worker.start()
            try:
                yield path, calls
            finally:
                server.shutdown()
                worker.join(3)
                assert not worker.is_alive()


class FaultControls(unittest.TestCase):
    def test_should_refuse_recovery_after_deadline_before_any_invocation(self):
        invoke = Mock()
        with self.assertRaisesRegex(AssertionError, "before invocation"):
            recover_before(5, invoke, Mock(), clock=lambda: 5)
        invoke.assert_not_called()

    def test_should_refuse_first_attempt_success_after_deadline(self):
        invoke = Mock(return_value="late")
        with self.assertRaisesRegex(AssertionError, "succeeded after"):
            recover_before(5, invoke, Mock(), clock=Mock(side_effect=[1, 5]))
        invoke.assert_called_once_with(4)

    def test_should_accept_timely_recovery_only_after_allocation_free_throttle(self):
        throttle = ClientError({"Error":{"Code":"TooManyRequestsException"}}, "Invoke")
        invoke, empty, pause = Mock(side_effect=[throttle, "ready"]), Mock(return_value=True), Mock()
        self.assertEqual(recover_before(5, invoke, empty,
                         clock=Mock(side_effect=[1, 2, 2, 3, 4]), pause=pause), "ready")
        self.assertEqual(invoke.call_args_list[0].args, (4,))
        self.assertEqual(invoke.call_args_list[1].args, (2,))
        empty.assert_called_once()
        pause.assert_called_once_with(0.025)

    def test_should_propagate_non_throttle_recovery_error(self):
        failure = ClientError({"Error":{"Code":"ServiceException"}}, "Invoke")
        empty = Mock()
        with self.assertRaises(ClientError) as observed:
            recover_before(5, Mock(side_effect=failure), empty, clock=lambda: 1)
        self.assertIs(observed.exception, failure)
        empty.assert_not_called()

    def test_should_require_causal_stage_and_explicit_release(self):
        with collecting(lambda: Hold("busy")) as hold:
            hold.targets(["HeldFunction"])
            event = {"stage":"busy", "functionName":"HeldFunction", "pid":1, "requestId":str(uuid.uuid4())}
            with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
                result = pool.submit(hold.exchange, "/hold", event)
                observed = hold.wait_seen(1, time.monotonic() + 2)
                self.assertEqual(observed, {"HeldFunction":event})
                self.assertFalse(result.done())
                with hold.condition:
                    hold.released = True
                    hold.condition.notify_all()
                self.assertEqual(result.result(2), {"release":True})

    def test_should_refuse_expired_or_wrong_initialization_evidence_without_mutation(self):
        hold = Hold("initializing")
        try:
            hold.targets(["HeldFunction"])
            event = {"stage":"initializing", "functionName":"HeldFunction", "pid":1, "requestId":str(uuid.uuid4())}
            with self.assertRaises(AssertionError):
                hold.exchange("/hold", event)
            self.assertEqual(hold.observed, {})
            event["requestId"] = None
            hold.deadline = time.monotonic() - 1
            with self.assertRaises(AssertionError):
                hold.exchange("/hold", event)
            with self.assertRaises(AssertionError):
                hold.wait_seen(0, time.monotonic() + 2)
            self.assertEqual(hold.observed, {})
        finally:
            hold.server_close()

    def test_should_refuse_wrong_daemon_before_interpreting_an_empty_inventory(self):
        expected, wrong, owner = (str(uuid.uuid4()) for _ in range(3))
        with engine_fixture(wrong, []) as (path, calls):
            metadata = Metadata(path, owner, expected)
            with self.assertRaises(AssertionError):
                metadata.ids()
            self.assertFalse(any("/containers/" in target for target in calls))

    def test_should_refuse_daemon_change_on_subsequent_observation(self):
        engine, owner = str(uuid.uuid4()), str(uuid.uuid4())
        with engine_fixture(engine, [], change=True) as (path, calls):
            metadata = Metadata(path, owner, engine)
            self.assertEqual(metadata.ids(), set())
            with self.assertRaises(AssertionError):
                metadata.ids()
            self.assertEqual(sum("/containers/" in target for target in calls), 1)

    def test_should_refuse_foreign_container_in_a_scoped_inventory(self):
        engine, owner = str(uuid.uuid4()), str(uuid.uuid4())
        row = {"Id":"a"*64, "Labels":{"rustack.lambda.ownerId":str(uuid.uuid4())}}
        with engine_fixture(engine, [row]) as (path, _calls):
            metadata = Metadata(path, owner, engine)
            with self.assertRaises(AssertionError):
                metadata.ids()

    def test_should_refuse_foreign_private_image_before_forwarding_its_inspection(self):
        engine, owner, foreign = (str(uuid.uuid4()) for _ in range(3))
        with engine_fixture(engine, []) as (path, calls):
            metadata = Metadata(path, owner, engine)
            alias = "rustack-internal/lambda-" + uuid.UUID(foreign).hex + "/" + uuid.uuid4().hex + ":" + uuid.uuid4().hex
            with self.assertRaises(AssertionError):
                metadata.image_id(alias)
            self.assertFalse(any("/images/" in target for target in calls))

    def test_should_bound_metadata_body_before_materializing_json(self):
        engine, owner = str(uuid.uuid4()), str(uuid.uuid4())
        with engine_fixture(engine, "x"*(1024*1024+1)) as (path, _calls):
            metadata = Metadata(path, owner, engine)
            with self.assertRaises(AssertionError):
                metadata.ids()

    def test_should_reject_nil_identity_before_opening_a_socket(self):
        with self.assertRaises(AssertionError):
            Metadata("/not-an-existing-socket", str(uuid.UUID(int=0)), str(uuid.uuid4()))


if __name__ == "__main__":
    unittest.main()
