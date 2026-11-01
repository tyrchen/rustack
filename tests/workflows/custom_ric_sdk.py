"""Ordinary boto3 acceptance of independent custom Rust RIC images."""
import concurrent.futures
import contextlib
import hmac
import http.server
import json
import os
import socket
import sys
import threading
import time
import uuid

if __name__ == "__main__":
    print(os.getpid(), flush=True)
import boto3
from botocore.config import Config


def unique_object(pairs):
    result = {}
    for key, value in pairs:
        assert key not in result, "duplicate collector field"
        result[key] = value
    return result


class Collector(http.server.ThreadingHTTPServer):
    """Two bounded causal barriers; valid completions stay held until both reject."""
    daemon_threads = False
    block_on_close = True

    def __init__(self):
        self.condition = threading.Condition()
        self.token = uuid.uuid4().hex
        self.consumed = {}
        self.checked = {}
        self.aborted = False
        self.deadline = time.monotonic() + 90
        self.slots = threading.BoundedSemaphore(4)
        self.connections = set()
        super().__init__(("0.0.0.0", 0), Handler)

    def get_request(self):
        connection, address = super().get_request()
        connection.settimeout(2)
        with self.condition:
            self.connections.add(connection)
        return connection, address

    def shutdown_request(self, request):
        try:
            super().shutdown_request(request)
        finally:
            with self.condition:
                if request.fileno() < 0:
                    self.connections.discard(request)
                self.condition.notify_all()

    def process_request(self, request, address):
        if not self.slots.acquire(blocking=False):
            self.shutdown_request(request)
            return
        try:
            super().process_request(request, address)
        except BaseException:
            self.slots.release()
            raise

    def process_request_thread(self, request, address):
        try:
            super().process_request_thread(request, address)
        finally:
            self.slots.release()

    def abort(self, close_connections=True):
        with self.condition:
            self.aborted = True
            self.condition.notify_all()
            connections = tuple(self.connections) if close_connections else ()
        for connection in connections:
            try:
                connection.shutdown(socket.SHUT_RDWR)
            except OSError:
                pass

    def exchange(self, path, event):
        with self.condition:
            assert time.monotonic() < self.deadline and not self.aborted
            lane, request_id = event["lane"], event["requestId"]
            assert lane in ("A", "B") and str(uuid.UUID(request_id)) == request_id
            if path == "/consume":
                assert lane not in self.consumed and request_id not in self.consumed.values()
                assert set(event) == {"lane", "requestId"}
                self.consumed[lane] = request_id
                state = self.consumed
            else:
                assert path == "/checked" and len(self.consumed) == 2
                assert set(event) == {"lane", "requestId", "statuses"}
                assert self.consumed[lane] == request_id and lane not in self.checked
                assert event["statuses"] == [400, 400, 400]
                self.checked[lane] = request_id
                state = self.checked
            self.condition.notify_all()
            while len(state) != 2 and not self.aborted:
                remaining = self.deadline - time.monotonic()
                assert remaining > 0, "causal barrier deadline"
                self.condition.wait(remaining)
            assert not self.aborted and time.monotonic() < self.deadline
            if path == "/consume":
                return {"foreignRequestId": self.consumed["B" if lane == "A" else "A"]}
            return {"release": True}


class BoundedHeaders:
    def __init__(self, stream):
        self.stream, self.remaining = stream, 4096

    def readline(self, limit=-1):
        bound = self.remaining + 1 if limit < 0 else min(limit, self.remaining + 1)
        line = self.stream.readline(bound)
        self.remaining -= len(line)
        if self.remaining < 0:
            raise ValueError("collector header limit")
        return line

    def __getattr__(self, name):
        return getattr(self.stream, name)


class Handler(http.server.BaseHTTPRequestHandler):
    protocol_version = "HTTP/1.1"

    def handle(self):
        self.rfile = BoundedHeaders(self.rfile)
        try:
            super().handle()
        except (OSError, ValueError):
            self.server.abort(close_connections=False)
            self.close_connection = True

    def log_message(self, *_args):
        pass

    def do_POST(self):
        self.connection.settimeout(15)
        observer = None
        keep_alive, reply_written = False, None
        try:
            assert len(self.headers.get_all("x-fixture-token", [])) == 1
            assert hmac.compare_digest(self.headers["x-fixture-token"], self.server.token)
            assert len(self.headers.get_all("Content-Length", [])) == 1
            assert "Transfer-Encoding" not in self.headers
            length = int(self.headers["Content-Length"])
            assert 0 < length <= 2048
            raw = self.rfile.read(length)
            assert len(raw) == length
            event = json.loads(raw, object_pairs_hook=unique_object)
            policy = getattr(self.server, "transport_policy", None)
            if policy is not None:
                keep_alive, reply_written = policy(self.connection, self.path, event)
                assert type(keep_alive) is bool and callable(reply_written)
            observer = getattr(self.server, "observe_transport", None)
            if observer is not None:
                observer_context = observer(self.path, "accepted")
            result = self.server.exchange(self.path, event)
            data, code = json.dumps(result).encode(), 200
        except BaseException:
            self.server.abort(close_connections=False)
            data, code = b'{}', 400
        self.send_response(code)
        self.send_header("Content-Length", str(len(data)))
        self.send_header("Connection", "keep-alive" if code == 200 and keep_alive else "close")
        self.end_headers()
        self.wfile.write(data)
        self.close_connection = not (code == 200 and keep_alive)
        if code == 200 and reply_written is not None:
            reply_written()
        if code == 200 and observer is not None:
            observer(self.path, "written", observer_context)


@contextlib.contextmanager
def collecting(factory=Collector):
    server = factory()
    worker = threading.Thread(target=server.serve_forever, kwargs={"poll_interval": 0.05})
    worker.start()
    try:
        yield server
    finally:
        server.abort()
        server.shutdown()
        server.server_close()
        worker.join(5)
        assert not worker.is_alive(), "collector thread did not join"


class Matrix:
    def __init__(self, endpoint, image, architecture):
        self.client = boto3.client(
            "lambda", endpoint_url=endpoint, region_name="us-east-1",
            aws_access_key_id="local-key", aws_secret_access_key="local-secret",
            config=Config(retries={"max_attempts": 0}, connect_timeout=2, read_timeout=60),
        )
        self.image, self.architecture = image, architecture
        self.functions = []

    def settled(self, name, updating=False):
        deadline = time.monotonic() + 120
        while True:
            state = self.client.get_function_configuration(FunctionName=name)
            if state["State"] == "Active" and (not updating or state["LastUpdateStatus"] == "Successful"):
                return state
            assert state["State"] != "Failed" and state.get("LastUpdateStatus") != "Failed", state
            assert time.monotonic() < deadline, "image deployment did not settle"
            time.sleep(0.025)

    def create(self, **fields):
        name = "CustomImage" + uuid.uuid4().hex
        self.client.create_function(
            FunctionName=name, Role="arn:aws:iam::000000000000:role/Docker",
            PackageType="Image", Architectures=[self.architecture], Timeout=fields.pop("Timeout", 30),
            MemorySize=fields.pop("MemorySize", 128), Code={"ImageUri": self.image}, **fields,
        )
        self.functions.append(name)
        self.settled(name)
        return name

    def invoke(self, name, event=None, qualifier=None, failed=False, client=None):
        options = {} if qualifier is None else {"Qualifier": qualifier}
        response = (self.client if client is None else client).invoke(FunctionName=name, Payload=json.dumps(event or {}).encode(), **options)
        with response["Payload"] as body:
            raw = body.read(65537)
        assert len(raw) <= 65536
        assert response["StatusCode"] == 200
        assert response.get("FunctionError") == ("Unhandled" if failed else None)
        return json.loads(raw), response.get("ExecutedVersion")

    def cleanup(self):
        errors = []
        for name in self.functions:
            try:
                self.client.delete_function(FunctionName=name)
            except BaseException as error:
                errors.append(error)
        assert not errors, "custom image function cleanup failed"

    def defaults_and_error(self):
        name = self.create()
        ids = set()
        for value, counter in ((35, 1), (0, 2)):
            output, _ = self.invoke(name, {"value": value})
            assert output["computed"] == value + 7 and output["counter"] == counter
            assert output["imageRevision"] == "A" and output["cwd"] == "/var/task"
            assert output["arguments"] == ["/image-bootstrap", "inherited-command"]
            assert str(uuid.UUID(output["requestId"])) == output["requestId"]
            ids.add(output["requestId"])
        assert len(ids) == 2
        error, _ = self.invoke(name, {"mode": "error"}, failed=True)
        assert error["errorType"] == "ImageFixtureError"
        output, _ = self.invoke(name, {"value": 10})
        assert output["computed"] == 17

    def overrides(self):
        empty = self.create(ImageConfig={"Command": []})
        output, _ = self.invoke(empty)
        assert output["arguments"] == ["/image-bootstrap"]
        literal = "$(touch /tmp/rustack-shell-side-effect); printf NOT_A_SHELL"
        replaced = self.create(ImageConfig={"EntryPoint": ["/alternate-bootstrap"],
                                           "Command": [literal], "WorkingDirectory": "/tmp"})
        output, _ = self.invoke(replaced)
        assert output["arguments"] == ["/alternate-bootstrap", literal]
        assert output["cwd"] == "/tmp" and output["shellSideEffect"] is False
        cleared = self.create(ImageConfig={"EntryPoint": [], "Command": ["/image-bootstrap", "empty-entry"]})
        output, _ = self.invoke(cleared)
        assert output["arguments"] == ["/image-bootstrap", "empty-entry"]

    def concurrent_identity(self, host):
        with collecting() as collector:
            environment = {"IMAGE_FIXTURE_COLLECTOR": f"{host}:{collector.server_port}",
                           "IMAGE_FIXTURE_TOKEN": collector.token}
            names = [self.create(Environment={"Variables": dict(environment, IMAGE_FIXTURE_LANE=lane)})
                     for lane in ("A", "B")]
            for name in names:
                self.invoke(name, {"value": 0})  # Seed the stale-ID control in each warm process.
            with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
                jobs = [pool.submit(self.invoke, name, {"value": value, "mode": "barrier-forge"})
                        for name, value in zip(names, (35, 93), strict=True)]
                outputs = [job.result(timeout=120)[0] for job in jobs]
            assert [output["computed"] for output in outputs] == [42, 100]
            assert all(output["counter"] == 2 and output["forgedStatuses"] == [400, 400, 400]
                       for output in outputs)
            assert len({output["requestId"] for output in outputs}) == 2
            assert collector.consumed == collector.checked
            assert set(collector.consumed.values()) == {output["requestId"] for output in outputs}

    def deploy_revision(self, name, tag):
        self.client.create_function(FunctionName=name, Role="arn:aws:iam::000000000000:role/Docker",
                                    PackageType="Image", Architectures=[self.architecture], Timeout=30,
                                    MemorySize=128, Code={"ImageUri": tag})
        self.settled(name)
        version = self.client.publish_version(FunctionName=name)["Version"]
        assert version == "1"
        self.client.create_alias(FunctionName=name, Name="live", FunctionVersion=version)
        result, executed = self.invoke(name, {"value": 35}, qualifier="live")
        assert result["imageRevision"] == "A" and result["computed"] == 42 and executed == "1"

    def inflight_revision(self, tag, host):
        with collecting() as collector:
            environment = {"IMAGE_FIXTURE_COLLECTOR": f"{host}:{collector.server_port}",
                           "IMAGE_FIXTURE_TOKEN": collector.token}
            original = self.create(Environment={"Variables": dict(environment, IMAGE_FIXTURE_LANE="A")})
            version = self.client.publish_version(FunctionName=original)["Version"]
            self.invoke(original, qualifier=version)
            helper = self.create(Environment={"Variables": dict(environment, IMAGE_FIXTURE_LANE="B")})
            self.invoke(helper)
            with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
                old = pool.submit(self.invoke, original, {"value":35, "mode":"barrier-forge"}, version)
                with collector.condition:
                    while "A" not in collector.consumed and not collector.aborted:
                        remaining = collector.deadline - time.monotonic()
                        assert remaining > 0, "published A did not consume its event"
                        collector.condition.wait(remaining)
                    assert not collector.aborted and "B" not in collector.consumed
                self.client.update_function_code(FunctionName=original, ImageUri=tag)
                state = self.settled(original, updating=True)
                assert state["LastUpdateStatus"] == "Successful"
                with collector.condition:
                    assert set(collector.consumed) == {"A"} and not collector.checked
                assert not old.done(), "old revision escaped its causal hold"
                other = pool.submit(self.invoke, helper, {"value":93, "mode":"barrier-forge"})
                output, executed = old.result(timeout=120)
                helper_output, _ = other.result(timeout=120)
            assert output["imageRevision"] == "A" and output["computed"] == 42 and executed == version
            assert output["requestId"] == collector.consumed["A"]
            assert output["forgedStatuses"] == helper_output["forgedStatuses"] == [400, 400, 400]
            assert collector.checked == collector.consumed
            latest, _ = self.invoke(original)
            assert latest["imageRevision"] == "B", "latest did not select B after A completed"

    def update_revision(self, name, tag):
        self.functions.append(name)
        for qualifier in (None, "1", "live"):
            output, _ = self.invoke(name, qualifier=qualifier)
            assert output["imageRevision"] == "A", "tag movement changed captured code"
        self.client.update_function_code(FunctionName=name, ImageUri=tag)
        self.settled(name, updating=True)
        output, _ = self.invoke(name)
        assert output["imageRevision"] == "B"
        version = self.client.publish_version(FunctionName=name)["Version"]
        assert version == "2"
        self.client.update_alias(FunctionName=name, Name="live", FunctionVersion=version)
        for qualifier, expected in (("1", "A"), ("2", "B"), ("live", "B")):
            output, executed = self.invoke(name, qualifier=qualifier)
            assert output["imageRevision"] == expected
            assert executed == ("2" if qualifier == "live" else qualifier)
            assert output["invokedArn"].endswith(":" + qualifier)


def main():
    endpoint, image, architecture, phase, host, name, tag = sys.argv[1:]
    matrix = Matrix(endpoint, image, architecture)
    try:
        if phase == "contracts":
            matrix.defaults_and_error()
            matrix.overrides()
            matrix.concurrent_identity(host)
            cases = ["custom-ric-compute-error", "imageconfig-inherit-clear-replace", "literal-argv", "active-cross-invocation-rejection"]
        elif phase == "deploy":
            matrix.deploy_revision(name, tag)
            cases = ["published-A"]
        else:
            assert phase == "update"
            matrix.update_revision(name, tag)
            matrix.inflight_revision(tag, host)
            # The separate in-flight/helper resources occupy both configured
            # environment slots; the original published/latest launches are cold.
            for qualifier, expected in (("1", "A"), (None, "B")):
                output, executed = matrix.invoke(name, qualifier=qualifier)
                assert output["imageRevision"] == expected and output["counter"] == 1
                assert executed == ("1" if qualifier == "1" else "$LATEST")
            cases = ["tag-move-retains-A", "update-selects-B", "published-alias-isolation", "published-inflight-A-completes-after-B-update"]
    finally:
        primary = sys.exc_info()[1]
        try:
            matrix.cleanup()
        except BaseException:
            if primary is None:
                raise
            primary.add_note("custom image function cleanup also failed")
    report = {"schemaVersion": 1, "cases": cases}
    if phase == "update":
        report["coldRevisionCounters"] = [1, 1]
    print(json.dumps(report), flush=True)


if __name__ == "__main__":
    main()
