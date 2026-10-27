"""Transparent, bounded real-daemon IO barriers for ordinary-server acceptance."""
import contextlib
import json
import re
import selectors
import socket
import threading
import traceback
import time
import urllib.parse
import uuid

from custom_ric_sdk import unique_object
from lambda_fault_support import identity


class ProtocolError(ValueError):
    """Static fixture framing failure, without request or daemon bytes."""


def exact(stream, count, deadline):
    output = bytearray()
    while len(output) < count:
        remaining = deadline - time.monotonic()
        if remaining <= 0:
            raise ProtocolError("proxy IO deadline")
        stream.settimeout(min(3, remaining))
        part = stream.recv(min(16384, count - len(output)))
        if not part:
            raise EOFError("proxy stream ended")
        output.extend(part)
    return bytes(output)


def line(stream, deadline, maximum=4096):
    output = bytearray()
    while len(output) <= maximum:
        output.extend(exact(stream, 1, deadline))
        if len(output) > maximum:
            raise ProtocolError("proxy line cap")
        if output.endswith(b"\r\n"):
            return bytes(output)
    raise ProtocolError("proxy line cap")


def head(stream, deadline):
    first = line(stream, deadline)
    raw, fields = bytearray(first), {}
    for _ in range(65):
        value = line(stream, deadline)
        raw.extend(value)
        if len(raw) > 16384:
            raise ProtocolError("proxy header cap")
        if value == b"\r\n":
            return first, fields, bytes(raw)
        name, separator, value = value[:-2].partition(b":")
        if not separator or re.fullmatch(rb"[!#$%&'*+.^_`|~0-9A-Za-z-]+", name) is None:
            raise ProtocolError("proxy header syntax")
        name = name.lower()
        if name in fields:
            raise ProtocolError("proxy duplicate header")
        fields[name] = value.strip()
    raise ProtocolError("proxy header count")


def framing(fields):
    if b"content-length" in fields and b"transfer-encoding" in fields:
        raise ProtocolError("proxy ambiguous body framing")
    if b"transfer-encoding" in fields and fields[b"transfer-encoding"].lower() != b"chunked":
        raise ProtocolError("proxy unsupported transfer coding")
    length = fields.get(b"content-length", b"0")
    if re.fullmatch(rb"[0-9]{1,10}", length) is None:
        raise ProtocolError("proxy invalid body length")
    return int(length)


def body(stream, fields, maximum, deadline):
    length = framing(fields)
    if b"transfer-encoding" not in fields:
        if length > maximum:
            raise ProtocolError("proxy body cap")
        value = exact(stream, length, deadline)
        return value, value
    raw, decoded = bytearray(), bytearray()
    while True:
        chunk = line(stream, deadline)
        size = chunk[:-2]
        if re.fullmatch(rb"[0-9A-Fa-f]{1,8}", size) is None:
            raise ProtocolError("proxy invalid chunk size")
        size = int(size, 16)
        if size > maximum - len(decoded) or len(raw) + len(chunk) > maximum + 16384:
            raise ProtocolError("proxy chunk body cap")
        raw.extend(chunk)
        if size == 0:
            trailer_bytes = 0
            for _ in range(65):
                trailer = line(stream, deadline)
                trailer_bytes += len(trailer)
                if trailer_bytes > 16384:
                    raise ProtocolError("proxy trailer cap")
                if len(raw) + len(trailer) > maximum + 16384:
                    raise ProtocolError("proxy encoded body cap")
                raw.extend(trailer)
                if trailer == b"\r\n":
                    return bytes(raw), bytes(decoded)
            raise ProtocolError("proxy trailer count")
        if len(raw) + size + 2 > maximum + 16384:
            raise ProtocolError("proxy encoded body cap")
        value = exact(stream, size, deadline)
        ending = exact(stream, 2, deadline)
        if ending != b"\r\n":
            raise ProtocolError("proxy truncated chunk")
        raw.extend(value + ending)
        decoded.extend(value)


def duplex(left, right, stopped, deadline):
    """Stream upgrades without read-ahead; propagate each half-close separately."""
    peers = {left: right, right: left}
    with selectors.DefaultSelector() as selected:
        for stream in peers:
            stream.settimeout(1)
            selected.register(stream, selectors.EVENT_READ)
        while selected.get_map() and not stopped.is_set():
            if time.monotonic() >= deadline:
                raise ProtocolError("proxy tunnel deadline")
            for key, _ in selected.select(0.1):
                source, target = key.fileobj, peers[key.fileobj]
                value = source.recv(16384)
                if value:
                    target.sendall(value)
                else:
                    selected.unregister(source)
                    with contextlib.suppress(OSError):
                        target.shutdown(socket.SHUT_WR)


class Proxy:
    def __init__(self, path, daemon, restore=None):
        assert path.startswith("/") and daemon.startswith("/")
        assert len(path.encode()) < 104 and len(daemon.encode()) < 104
        self.path, self.daemon, self.restore_marker = path, daemon, restore
        self.condition = threading.Condition()
        self.stopped = threading.Event()
        self.deadline = time.monotonic() + 480
        self.sockets, self.workers, self.records, self.starts, self.errors = set(), [], {}, set(), []
        self.rule, self.observed, self.released, self.unavailable = None, [], False, False
        self.failed_image = None
        self.listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.listener.bind(path)
        self.listener.listen(64)
        self.listener.settimeout(0.05)
        self.worker = threading.Thread(target=self.serve)
        self.worker.start()

    def arm(self, kind, owner, count=1):
        assert kind in ("create", "delete", "tag") and count in (1, 2)
        identity(owner)
        with self.condition:
            assert self.rule is None and not self.unavailable
            self.rule = (kind, owner, count, time.monotonic() + 30)
            self.observed, self.released = [], False

    def gate(self, kind, record):
        with self.condition:
            rule = self.rule
            if rule is None or rule[0] != kind or len(self.observed) >= rule[2]:
                return
            if kind == "tag":
                if record["owner"] != rule[1]:
                    return
            elif record["owner"] != rule[1] or record["role"] != "function":
                return
            assert time.monotonic() < rule[3]
            self.observed.append(dict(record))
            self.condition.notify_all()
            while not self.released and not self.stopped.is_set():
                remaining = rule[3] - time.monotonic()
                assert remaining > 0, "proxy gate lease expired"
                self.condition.wait(remaining)
            assert not self.stopped.is_set() and time.monotonic() < rule[3]

    def wait(self, count, deadline):
        with self.condition:
            while len(self.observed) < count and not self.errors:
                remaining = deadline - time.monotonic()
                assert remaining > 0, "actual daemon operation gate was not observed"
                self.condition.wait(remaining)
            assert not self.errors and time.monotonic() < deadline
            return [dict(row) for row in self.observed]

    def release(self):
        with self.condition:
            assert self.rule is not None
            self.released = True
            self.condition.notify_all()

    def restore(self):
        with self.condition:
            self.unavailable = False
            self.failed_image = None
            self.released = True
            self.condition.notify_all()

    def fail_image(self, image):
        assert isinstance(image, str) and 0 < len(image) <= 1024
        with self.condition:
            self.failed_image = image

    def loss(self):
        with self.condition:
            self.unavailable = True
            for stream in tuple(self.sockets):
                with contextlib.suppress(OSError):
                    stream.shutdown(socket.SHUT_RDWR)

    def serve(self):
        try:
            while not self.stopped.is_set():
                if self.restore_marker is not None and self.restore_marker.is_file():
                    self.restore()
                if time.monotonic() >= self.deadline:
                    raise ProtocolError("proxy process deadline")
                try:
                    client, _ = self.listener.accept()
                except socket.timeout:
                    continue
                with self.condition:
                    finished = [worker for worker in self.workers if not worker.is_alive()]
                    for worker in finished:
                        worker.join()
                        self.workers.remove(worker)
                    if self.unavailable or len(self.workers) >= 64:
                        client.close()
                        continue
                    self.sockets.add(client)
                    worker = threading.Thread(target=self.connection, args=(client,))
                    self.workers.append(worker)
                    worker.start()
        except OSError as error:
            if not self.stopped.is_set():
                self.failure(error)
        except BaseException as error:
            self.failure(error)

    def failure(self, error):
        with self.condition:
            if not self.errors:
                frame = traceback.extract_tb(error.__traceback__)[-1]
                self.errors.append(type(error).__name__ + " at " + frame.name + ":" + str(frame.lineno))
            self.condition.notify_all()

    def connection(self, client):
        backend = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        try:
            backend.settimeout(3)
            with self.condition:
                self.sockets.add(backend)
            backend.connect(self.daemon)
            for position in range(2):
                first, fields, raw = head(client, time.monotonic() + 5)
                method, target, version = first[:-2].split(b" ")
                assert method in (b"GET", b"POST", b"DELETE") and version == b"HTTP/1.1"
                assert len(target) <= 2048 and re.fullmatch(rb"/[!-~]*", target)
                assert b"transfer-encoding" not in fields
                payload, _ = body(client, fields, 1024 * 1024, time.monotonic() + 5)
                path = urllib.parse.unquote(urllib.parse.urlsplit(target.decode("ascii")).path)
                path = re.sub(r"^/v1\.[0-9]{2}(?=/)", "", path)
                create = method == b"POST" and path == "/containers/create"
                tag = method == b"POST" and path.startswith("/images/") and path.endswith("/tag")
                record = None
                if create:
                    value = json.loads(payload, object_pairs_hook=unique_object)
                    labels = value["Labels"]
                    record = {"owner":identity(labels["rustack.lambda.ownerId"]),
                              "role":labels["rustack.lambda.role"]}
                if method == b"DELETE" and path.startswith("/containers/"):
                    key = path[len("/containers/"):]
                    with self.condition:
                        known = self.records.get(key)
                    if known is not None:
                        self.gate("delete", dict(known, id=key))
                if method == b"POST" and path.startswith("/containers/") and path.endswith("/start"):
                    with self.condition:
                        assert len(self.starts) < 128
                        self.starts.add(path[len("/containers/"):-len("/start")])
                backend.sendall(raw + payload)
                with self.condition:
                    failed_image = self.failed_image
                if failed_image is not None and method == b"GET" and path == "/images/" + failed_image + "/json":
                    return
                if path == "/info" or create or tag:
                    first, headers, raw = head(backend, time.monotonic() + 60)
                    status = int(first.split(b" ", 2)[1])
                    encoded, decoded = body(backend, headers, (1 if create else 2) * 1024 * 1024,
                                            time.monotonic() + 60)
                    if create and status == 201:
                        key = json.loads(decoded, object_pairs_hook=unique_object)["Id"]
                        assert re.fullmatch(r"[0-9a-f]{64}", key)
                        with self.condition:
                            assert len(self.records) < 128
                            self.records[key] = record
                        self.gate("create", dict(record, id=key))
                    if tag and status == 201:
                        query = urllib.parse.parse_qs(urllib.parse.urlsplit(target.decode("ascii")).query, strict_parsing=True)
                        repository = query["repo"][0]
                        match = re.fullmatch(r"rustack-internal/lambda-([0-9a-f]{32})/[0-9a-f]{32}", repository)
                        assert match and re.fullmatch(r"[0-9a-f]{32}", query["tag"][0])
                        self.gate("tag", {"owner":str(uuid.UUID(hex=match[1])),
                                          "alias":repository + ":" + query["tag"][0]})
                    client.sendall(raw + encoded)
                    if path == "/info" and position == 0 and fields.get(b"connection", b"").lower() != b"close":
                        continue
                duplex(client, backend, self.stopped, self.deadline)
                return
            raise ProtocolError("proxy unexpected third request")
        except (OSError, EOFError):
            # Peer loss/half-close is expected in the cancellation and loss cases.
            pass
        except BaseException as error:
            self.failure(error)
        finally:
            with self.condition:
                self.sockets.discard(client)
                self.sockets.discard(backend)
            client.close()
            backend.close()

    def close(self):
        self.stopped.set()
        with self.condition:
            self.released = True
            self.condition.notify_all()
            for stream in tuple(self.sockets):
                with contextlib.suppress(OSError):
                    stream.shutdown(socket.SHUT_RDWR)
        self.listener.close()
        deadline = time.monotonic() + 5
        self.worker.join(max(0, deadline - time.monotonic()))
        assert not self.worker.is_alive(), "proxy accept owner did not join"
        for worker in self.workers:
            worker.join(max(0, deadline - time.monotonic()))
            assert not worker.is_alive(), "proxy connection owner did not join"
        assert not self.errors, "proxy framing/worker failure: " + repr(self.errors)
