"""Controls that prevent cancellation/metadata observations from passing vacuously."""
import concurrent.futures
import contextlib
import json
from http.client import HTTPException
import socketserver
import tempfile
import threading
import time
import unittest
import uuid
from unittest.mock import Mock, patch

from botocore.exceptions import ClientError
from custom_ric_sdk import collecting
from lambda_fault_support import Hold, Metadata, recover_before


@contextlib.contextmanager
def engine_fixture(engine, rows, change=False, close_inventory=False):
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
                        if close_inventory and "/containers/" in target:
                            return
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

    def test_should_pin_one_verified_peer_and_resume_per_operation_fencing(self):
        engine, owner = str(uuid.uuid4()), str(uuid.uuid4())
        with engine_fixture(engine, []) as (path, calls):
            metadata = Metadata(path, owner, engine)
            with metadata.pinned():
                self.assertEqual(metadata.ids(), set())
                self.assertEqual(metadata.ids(), set())
                with self.assertRaisesRegex(AssertionError, "nested"):
                    with metadata.pinned():
                        self.fail("nested pin admitted")
                with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
                    with self.assertRaisesRegex(AssertionError, "concurrent"):
                        pool.submit(metadata.ids).result(2)
                self.assertEqual(sum(target.endswith("/info") for target in calls), 1)
            self.assertEqual(metadata.ids(), set())
            self.assertEqual(sum(target.endswith("/info") for target in calls), 2)

    def test_should_reject_closed_peer_without_reconnecting(self):
        engine, owner = str(uuid.uuid4()), str(uuid.uuid4())
        with engine_fixture(engine, [], close_inventory=True) as (path, calls):
            metadata = Metadata(path, owner, engine)
            with metadata.pinned():
                self.assertEqual(metadata.ids(), set())
                with self.assertRaises((OSError, AssertionError, HTTPException)):
                    metadata.ids()
            self.assertEqual(sum(target.endswith("/info") for target in calls), 1)
            self.assertEqual(sum("/containers/" in target for target in calls), 1)
            self.assertIsNone(metadata._pinned)

    def test_should_reject_socket_replacement_and_close_the_retained_peer(self):
        engine, owner = str(uuid.uuid4()), str(uuid.uuid4())
        with engine_fixture(engine, []) as (path, _calls):
            metadata = Metadata(path, owner, engine)
            with metadata.pinned():
                connection, peer = metadata._pinned
                replacement = Mock()
                def replace(_connection, _path):
                    connection.sock = replacement
                    return []
                with patch.object(Metadata, "read", side_effect=replace):
                    with self.assertRaisesRegex(AssertionError, "replaced"):
                        metadata.ids()
                self.assertEqual(peer.fileno(), -1)
            replacement.close.assert_called_once()

    def test_should_close_a_pin_when_workload_fails_and_refuse_wrong_engine(self):
        engine, owner = str(uuid.uuid4()), str(uuid.uuid4())
        with engine_fixture(engine, []) as (path, calls):
            metadata = Metadata(path, owner, engine)
            with self.assertRaisesRegex(ValueError, "workload"):
                with metadata.pinned():
                    peer = metadata._pinned[1]
                    raise ValueError("workload")
            self.assertEqual(peer.fileno(), -1)
            self.assertIsNone(metadata._pinned)
            wrong = Metadata(path, owner, str(uuid.uuid4()))
            with self.assertRaisesRegex(AssertionError, "daemon changed"):
                with wrong.pinned():
                    self.fail("wrong engine admitted")
            self.assertIsNone(wrong._pinned)
            self.assertFalse(any("/containers/" in target for target in calls))

    def test_should_observe_authenticated_collector_acceptance_and_response_write_with_bound_context(self):
        import http.client
        from custom_ric_sdk import Collector
        marker = object()
        class Observed(Collector):
            def __init__(self):
                super().__init__()
                self.observed = []
            def observe_transport(self, path, phase, context=None):
                with self.condition:
                    if phase == "written":
                        assert context is marker
                    self.observed.append((path, phase))
                    self.condition.notify_all()
                    return marker
            def exchange(self, path, event):
                assert path == "/flood-begin" and event == {}
                return {"release":True}
        with collecting(Observed) as server:
            connection = http.client.HTTPConnection("127.0.0.1", server.server_port, timeout=2)
            try:
                connection.request("POST", "/flood-begin", body=b"{}", headers={"x-fixture-token":server.token})
                response = connection.getresponse()
                self.assertEqual(response.status, 200)
                self.assertEqual(json.loads(response.read()), {"release":True})
                deadline = time.monotonic() + 2
                with server.condition:
                    while len(server.observed) < 2:
                        remaining = deadline - time.monotonic()
                        self.assertGreater(remaining, 0)
                        server.condition.wait(remaining)
                self.assertEqual(server.observed, [("/flood-begin", "accepted"), ("/flood-begin", "written")])
            finally:
                connection.close()
        with collecting(Observed) as server:
            connection = http.client.HTTPConnection("127.0.0.1", server.server_port, timeout=2)
            try:
                connection.request("POST", "/flood-begin", body=b"{}", headers={"x-fixture-token":"wrong"})
                response = connection.getresponse()
                self.assertEqual(response.status, 400)
                self.assertEqual(response.read(), b"{}")
                self.assertEqual(server.observed, [])
            finally:
                connection.close()

    def test_should_use_exactly_one_authenticated_http_channel_and_reject_reconnection(self):
        import http.client
        from custom_ric_sdk import Collector
        from image_performance_channel import FloodChannels
        from image_performance_sdk import wait_flood_channel
        class ChannelCollector(Collector):
            def __init__(self, held=False):
                super().__init__()
                self.channels = FloodChannels()
                self.ready_entered, self.close_entered = threading.Event(), threading.Event()
                self.release_ready, self.release_close = threading.Event(), threading.Event()
                if not held:
                    self.release_ready.set()
                    self.release_close.set()
            def transport_policy(self, connection, path, event):
                with self.condition:
                    keep, complete = self.channels.accept(connection, path, event)
                def written():
                    if path == "/flood-ready":
                        self.ready_entered.set()
                        assert self.release_ready.wait(2)
                    with self.condition:
                        complete()
                        self.condition.notify_all()
                return keep, written
            def shutdown_request(self, request):
                self.close_entered.set()
                assert self.release_close.wait(2)
                super().shutdown_request(request)
            def exchange(self, path, event):
                return {"ready":True} if path == "/flood-ready" else {"recorded":True}
        def post(connection, server, phase, body):
            connection.request("POST", "/flood-" + phase, body=json.dumps(body).encode(),
                               headers={"x-fixture-token":server.token})
            response = connection.getresponse()
            response.read()
            return response
        def wait(server, accepted):
            deadline = time.monotonic() + 2
            with server.condition:
                while not accepted():
                    remaining = deadline - time.monotonic()
                    self.assertGreater(remaining, 0)
                    server.condition.wait(min(0.01, remaining))
        with collecting(lambda:ChannelCollector(held=True)) as server:
            connection = http.client.HTTPConnection("127.0.0.1", server.server_port, timeout=2)
            try:
                with concurrent.futures.ThreadPoolExecutor(max_workers=1) as observations:
                    initialization_cutoff = time.monotonic_ns() + 2000000000
                    response = post(connection, server, "ready", {"functionName":"Flood"})
                    self.assertEqual((response.status, response.getheader("Connection")), (200, "keep-alive"))
                    peer = connection.sock
                    self.assertTrue(server.ready_entered.wait(2))
                    self.assertEqual(server.channels.current.phase, "readyReply")
                    entered = threading.Event()
                    def prepared():
                        entered.set()
                        return server.channels.ready(1)
                    waiter = observations.submit(wait_flood_channel, server, prepared, initialization_cutoff)
                    self.assertTrue(entered.wait(2))
                    self.assertFalse(waiter.done())
                    server.release_ready.set()
                    waiter.result(2)
                    with server.condition:
                        channel = server.channels.submit(1, time.monotonic_ns())
                    request = str(uuid.uuid4())
                    response = post(connection, server, "begin", {"functionName":"Flood", "requestId":request})
                    self.assertEqual(response.status, 200)
                    self.assertIs(connection.sock, peer)
                    closure_cutoff = time.monotonic_ns() + 2000000000
                    response = post(connection, server, "end", {"functionName":"Flood", "requestId":request})
                    self.assertEqual((response.status, response.getheader("Connection")), (200, "close"))
                    self.assertIsNone(connection.sock)
                    self.assertTrue(server.close_entered.wait(2))
                    self.assertGreaterEqual(channel.connection.fileno(), 0)
                    entered = threading.Event()
                    def closed():
                        entered.set()
                        return channel.phase == "complete" and channel.connection.fileno() < 0
                    waiter = observations.submit(wait_flood_channel, server, closed, closure_cutoff)
                    self.assertTrue(entered.wait(2))
                    self.assertFalse(waiter.done())
                    server.release_close.set()
                    waiter.result(2)
                    with server.condition:
                        server.channels.finish(channel, request)
                    self.assertEqual(server.channels.same_connection, 1)
            finally:
                server.release_ready.set()
                server.release_close.set()
                connection.close()
        with collecting(ChannelCollector) as server:
            first = http.client.HTTPConnection("127.0.0.1", server.server_port, timeout=2)
            replacement = http.client.HTTPConnection("127.0.0.1", server.server_port, timeout=2)
            try:
                self.assertEqual(post(first, server, "ready", {"functionName":"Flood"}).status, 200)
                wait(server, lambda:server.channels.prepared == 1)
                with server.condition:
                    server.channels.submit(1, time.monotonic_ns())
                response = post(replacement, server, "begin", {"functionName":"Flood", "requestId":str(uuid.uuid4())})
                self.assertEqual((response.status, response.getheader("Connection")), (400, "close"))
                self.assertEqual(server.channels.current.phase, "ready")
                self.assertEqual(server.channels.same_connection, 0)
            finally:
                first.close()
                replacement.close()

    def test_should_reject_nil_identity_before_opening_a_socket(self):
        with self.assertRaises(AssertionError):
            Metadata("/not-an-existing-socket", str(uuid.UUID(int=0)), str(uuid.uuid4()))


if __name__ == "__main__":
    unittest.main()
