"""Protocol and lifecycle controls for the transparent actual-daemon fault proxy."""
import concurrent.futures
import contextlib
import http.client
import importlib
import io
import json
from pathlib import Path
import socket
import tempfile
import threading
import time
import unittest
import urllib.parse
import uuid

from docker_io_proxy import ProtocolError, Proxy, body, exact, head, line
from lambda_fault_support import UnixHTTP


@contextlib.contextmanager
def daemon(handler):
    with tempfile.TemporaryDirectory(prefix="rio-", dir="/tmp") as root:
        path = str(Path(root) / "daemon.sock")
        listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        listener.bind(path)
        listener.listen(2)
        listener.settimeout(0.05)
        stop, streams, errors = threading.Event(), [], []
        def serve():
            try:
                while not stop.is_set():
                    try:
                        stream, _ = listener.accept()
                    except socket.timeout:
                        continue
                    streams.append(stream)
                    try:
                        handler(stream)
                    finally:
                        stream.close()
                    return
            except (OSError, EOFError):
                pass
            except BaseException as error:
                errors.append(error)
        worker = threading.Thread(target=serve)
        worker.start()
        proxy = Proxy(str(Path(root) / "proxy.sock"), path)
        try:
            yield proxy
        finally:
            proxy.close()
            stop.set()
            for stream in streams:
                with contextlib.suppress(OSError):
                    stream.shutdown(socket.SHUT_RDWR)
            listener.close()
            worker.join(3)
            assert not worker.is_alive(), "controlled daemon did not join"
            assert not errors, "controlled daemon had an unexpected failure"


def request(stream):
    first, fields, _ = head(stream, time.monotonic() + 2)
    _, decoded = body(stream, fields, 1024 * 1024, time.monotonic() + 2)
    return first, decoded


class ProxyControls(unittest.TestCase):
    def test_should_not_emit_process_handshakes_when_importing_fault_helpers(self):
        output = io.StringIO()
        with contextlib.redirect_stdout(output):
            importlib.import_module("lambda_fault_sdk")
        self.assertEqual(output.getvalue(), "")

    def test_should_preserve_chunked_identity_and_operation_on_one_backend_connection(self):
        for operation in ("/v1.53/images/source/json", "/info"):
            requests = []
            engine = str(uuid.uuid4())
            def backend(stream):
                requests.append(request(stream)[0])
                payload = json.dumps({"ID":engine}).encode()
                stream.sendall(b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n" +
                               format(len(payload), "x").encode() + b"\r\n" + payload + b"\r\n0\r\n\r\n")
                requests.append(request(stream)[0])
                stream.sendall(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{}")
            with daemon(backend) as proxy:
                connection = UnixHTTP(proxy.path)
                try:
                    connection.request("GET", "/info")
                    self.assertEqual(json.loads(connection.getresponse().read()), {"ID":engine})
                    connection.request("GET", operation)
                    self.assertEqual(connection.getresponse().read(), b"{}")
                finally:
                    connection.close()
            self.assertEqual(requests, [b"GET /info HTTP/1.1\r\n", ("GET " + operation + " HTTP/1.1\r\n").encode()])

    def test_should_forward_upgrade_bytes_and_preserve_independent_half_close(self):
        observed = []
        def backend(stream):
            request(stream)
            stream.sendall(b"HTTP/1.1 101 UPGRADED\r\nConnection: Upgrade\r\nUpgrade: tcp\r\n\r\nfirst")
            data = bytearray()
            while True:
                part = stream.recv(1024)
                if not part:
                    break
                data.extend(part)
            observed.append(bytes(data))
            stream.sendall(b"last")
        with daemon(backend) as proxy, socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
            client.settimeout(2)
            client.connect(proxy.path)
            client.sendall(b"POST /v1.53/containers/id/attach HTTP/1.1\r\nContent-Length: 0\r\n\r\n")
            first, _, _ = head(client, time.monotonic() + 2)
            self.assertTrue(first.startswith(b"HTTP/1.1 101 "))
            self.assertEqual(exact(client, 5, time.monotonic() + 2), b"first")
            client.sendall(b"control")
            client.shutdown(socket.SHUT_WR)
            self.assertEqual(exact(client, 4, time.monotonic() + 2), b"last")
            self.assertEqual(client.recv(1), b"")
        self.assertEqual(observed, [b"control"])

    def test_should_hold_create_ack_only_after_the_backend_effect_and_exact_owner(self):
        owner, key, effects = str(uuid.uuid4()), "a" * 64, []
        def backend(stream):
            _, payload = request(stream)
            effects.append(json.loads(payload))
            reply = json.dumps({"Id":key}).encode()
            stream.sendall(b"HTTP/1.1 201 Created\r\nContent-Length: " + str(len(reply)).encode() + b"\r\n\r\n" + reply)
        with daemon(backend) as proxy:
            proxy.arm("create", owner)
            proxy.gate("create", {"owner":str(uuid.uuid4()), "role":"function", "id":"b"*64})
            self.assertFalse(proxy.observed)
            def client():
                connection = UnixHTTP(proxy.path)
                try:
                    connection.request("POST", "/v1.53/containers/create", json.dumps({
                        "Labels":{"rustack.lambda.ownerId":owner, "rustack.lambda.role":"function"}}))
                    return json.loads(connection.getresponse().read())
                finally:
                    connection.close()
            with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
                result = pool.submit(client)
                self.assertEqual(proxy.wait(1, time.monotonic() + 2)[0]["id"], key)
                self.assertEqual(len(effects), 1)
                self.assertFalse(result.done())
                proxy.release()
                self.assertEqual(result.result(2), {"Id":key})

    def test_should_hold_private_tag_ack_after_effect_and_scope_it_to_the_owner(self):
        owner, run, pin = (uuid.uuid4() for _ in range(3))
        repository = "rustack-internal/lambda-" + owner.hex + "/" + run.hex
        target = "/v1.53/images/sha256:" + "a"*64 + "/tag?" + urllib.parse.urlencode({"repo":repository, "tag":pin.hex})
        effected = threading.Event()
        def backend(stream):
            first, _ = request(stream)
            self.assertEqual(first, ("POST " + target + " HTTP/1.1\r\n").encode())
            effected.set()
            stream.sendall(b"HTTP/1.1 201 Created\r\nContent-Length: 0\r\n\r\n")
        with daemon(backend) as proxy:
            proxy.arm("tag", str(owner))
            proxy.gate("tag", {"owner":str(uuid.uuid4()), "alias":"foreign"})
            self.assertFalse(proxy.observed)
            def client():
                connection = UnixHTTP(proxy.path)
                try:
                    connection.request("POST", target)
                    reply = connection.getresponse()
                    reply.read()
                    return reply.status
                finally:
                    connection.close()
            with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
                result = pool.submit(client)
                observed = proxy.wait(1, time.monotonic() + 2)
                self.assertTrue(effected.is_set())
                self.assertEqual(observed[0]["alias"], repository + ":" + pin.hex)
                self.assertFalse(result.done())
                proxy.release()
                self.assertEqual(result.result(2), 201)

    def test_should_drop_only_the_selected_image_ack_after_forwarding_real_io(self):
        effected = threading.Event()
        def backend(stream):
            first, _ = request(stream)
            self.assertEqual(first, b"GET /v1.53/images/source:b/json HTTP/1.1\r\n")
            effected.set()
            stream.sendall(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\n\r\n{}")
        with daemon(backend) as proxy:
            proxy.fail_image("source:b")
            connection = UnixHTTP(proxy.path)
            try:
                connection.request("GET", "/v1.53/images/source:b/json")
                with self.assertRaises((http.client.RemoteDisconnected, OSError)):
                    connection.getresponse()
                self.assertTrue(effected.wait(1))
            finally:
                connection.close()
            proxy.restore()
            self.assertIsNone(proxy.failed_image)

    def test_should_hold_delete_before_forwarding_the_recorded_effect(self):
        owner, key, effected = str(uuid.uuid4()), "b"*64, threading.Event()
        def backend(stream):
            first, _ = request(stream)
            self.assertEqual(first, ("DELETE /v1.53/containers/" + key + " HTTP/1.1\r\n").encode())
            effected.set()
            stream.sendall(b"HTTP/1.1 204 No Content\r\nContent-Length: 0\r\n\r\n")
        with daemon(backend) as proxy:
            proxy.records[key] = {"owner":owner, "role":"function"}
            proxy.arm("delete", owner)
            def client():
                connection = UnixHTTP(proxy.path)
                try:
                    connection.request("DELETE", "/v1.53/containers/" + key)
                    reply = connection.getresponse()
                    reply.read()
                    return reply.status
                finally:
                    connection.close()
            with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
                result = pool.submit(client)
                proxy.wait(1, time.monotonic() + 2)
                self.assertFalse(effected.is_set())
                proxy.release()
                self.assertEqual(result.result(2), 204)
                self.assertTrue(effected.is_set())

    def test_should_reject_ambiguous_oversized_duplicate_or_truncated_framing(self):
        values = [
            b"GET / HTTP/1.1\r\nContent-Length: 0\r\nTransfer-Encoding: chunked\r\n\r\n",
            b"GET / HTTP/1.1\r\nContent-Length: -1\r\n\r\n",
            b"GET / HTTP/1.1\r\nContent-Length: 2000000\r\n\r\n",
            b"GET / HTTP/1.1\r\nContent-Length: 0\r\nContent-Length: 0\r\n\r\n",
            b"GET / HTTP/1.1\r\nTransfer-Encoding: gzip\r\n\r\n",
            b"GET / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n200000\r\n",
            b"GET / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n2\r\na",
            b"GET / HTTP/1.1\r\n" + b"a"*4097 + b"\r\n\r\n",
        ]
        for value in values:
            left, right = socket.socketpair()
            try:
                left.sendall(value)
                left.shutdown(socket.SHUT_WR)
                with self.subTest(value=value[:80]), self.assertRaises((ProtocolError, EOFError)):
                    _, fields, _ = head(right, time.monotonic() + 2)
                    body(right, fields, 1024 * 1024, time.monotonic() + 2)
            finally:
                left.close()
                right.close()

    def test_should_enforce_exact_encoded_chunk_and_terminated_line_caps(self):
        wire = b"1\r\na\r\n" * 3412 + b"0\r\n\r\n"
        self.assertEqual(len(wire), 4093 + 16384)
        for value, valid in ((wire, True), (wire[:-5] + b"00\r\n\r\n", False)):
            left, right = socket.socketpair()
            left.settimeout(2)
            try:
                with concurrent.futures.ThreadPoolExecutor(max_workers=1) as pool:
                    sent = pool.submit(left.sendall, value)
                    if valid:
                        raw, decoded = body(right, {b"transfer-encoding":b"chunked"}, 4093, time.monotonic() + 2)
                        self.assertEqual(raw, value)
                        self.assertEqual(decoded, b"a"*3412)
                    else:
                        with self.assertRaises(ProtocolError):
                            body(right, {b"transfer-encoding":b"chunked"}, 4093, time.monotonic() + 2)
                    sent.result(2)
            finally:
                left.close()
                right.close()
        for count, valid in ((4096, True), (4097, False)):
            left, right = socket.socketpair()
            try:
                left.sendall(b"x"*(count-2) + b"\r\n")
                if valid:
                    self.assertEqual(len(line(right, time.monotonic() + 2)), 4096)
                else:
                    with self.assertRaises(ProtocolError):
                        line(right, time.monotonic() + 2)
            finally:
                left.close()
                right.close()

    def test_should_refuse_expired_gate_without_recording_ready_evidence(self):
        with daemon(lambda stream: None) as proxy:
            owner = str(uuid.uuid4())
            proxy.arm("create", owner)
            proxy.rule = proxy.rule[:-1] + (time.monotonic() - 1,)
            with self.assertRaises(AssertionError):
                proxy.gate("create", {"owner":owner, "role":"function", "id":"a"*64})
            self.assertFalse(proxy.observed)

    def test_should_join_quiet_and_partial_header_connections_on_abort(self):
        def backend(stream):
            while stream.recv(1024):
                pass
        for initial in (b"", b"GET /info HTTP/1.1\r\nX-Partial:"):
            with daemon(backend) as proxy, socket.socket(socket.AF_UNIX, socket.SOCK_STREAM) as client:
                client.connect(proxy.path)
                client.sendall(initial)
                deadline = time.monotonic() + 2
                while len(proxy.sockets) < 2:
                    self.assertLess(time.monotonic(), deadline)
                    time.sleep(0.005)
                started = time.monotonic()
                proxy.close()
                self.assertLess(time.monotonic() - started, 2)


if __name__ == "__main__":
    unittest.main()
