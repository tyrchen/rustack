"""Bounded causal holds and independent read-only actual-daemon observations."""
import http.client
import json
import socket
import time
import urllib.parse
import uuid

from botocore.exceptions import ClientError

from custom_ric_sdk import Collector, unique_object


def recover_before(deadline, invoke, empty, clock=time.monotonic, pause=time.sleep):
    """Accept only timely success; retain only explicit, allocation-free throttling."""
    while True:
        remaining = deadline - clock()
        assert remaining > 0, "recovery observation deadline expired before invocation"
        try:
            result = invoke(remaining)
        except ClientError as error:
            if error.response["Error"]["Code"] != "TooManyRequestsException":
                raise
            assert clock() < deadline, "joined cleanup never returned the reserved slot"
            assert empty(), "a throttled recovery allocated a new environment"
            remaining = deadline - clock()
            assert remaining > 0, "recovery metadata exceeded its observation deadline"
            pause(min(0.025, remaining))
        else:
            assert clock() < deadline, "recovery succeeded after its observation deadline"
            return result


class Hold(Collector):
    def __init__(self, stage):
        self.stage, self.expected, self.observed = stage, set(), {}
        self.released = False
        super().__init__()

    def targets(self, names):
        assert 1 <= len(names) <= 2 and len(set(names)) == len(names)
        assert all(0 < len(name) <= 64 and name.isascii()
                   and all(c.isalnum() or c in "_-" for c in name) for name in names)
        with self.condition:
            assert not self.observed
            self.expected = set(names)

    def wait_seen(self, count, deadline):
        with self.condition:
            while len(self.observed) < count and not self.aborted:
                remaining = min(deadline, self.deadline) - time.monotonic()
                assert remaining > 0, "fixture stage was not observed before its cutoff"
                self.condition.wait(remaining)
            assert not self.aborted and time.monotonic() < min(deadline, self.deadline)
            return dict(self.observed)

    def exchange(self, path, event):
        with self.condition:
            assert path == "/hold" and not self.aborted and time.monotonic() < self.deadline
            assert set(event) == {"stage", "functionName", "pid", "requestId"}
            name = event["functionName"]
            assert event["stage"] == self.stage and name in self.expected and name not in self.observed
            assert type(event["pid"]) is int and 0 < event["pid"] < 2**31
            if self.stage == "busy":
                assert str(uuid.UUID(event["requestId"])) == event["requestId"]
            else:
                assert self.stage == "initializing" and event["requestId"] is None
            self.observed[name] = event
            self.condition.notify_all()
            while not self.released and not self.aborted:
                remaining = self.deadline - time.monotonic()
                assert remaining > 0, "fixture hold lease expired"
                self.condition.wait(remaining)
            assert not self.aborted and time.monotonic() < self.deadline
            return {"release": True}


class UnixHTTP(http.client.HTTPConnection):
    def __init__(self, path):
        super().__init__("docker", timeout=3)
        self.path = path

    def connect(self):
        self.sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        self.sock.settimeout(self.timeout)
        self.sock.connect(self.path)


def identity(text):
    assert isinstance(text, str) and len(text) == 36
    parsed = uuid.UUID(text)
    assert parsed.int != 0 and str(parsed) == text
    return text


class Metadata:
    def __init__(self, path, owner, engine):
        assert isinstance(path, str) and path.startswith("/") and len(path.encode()) <= 4096
        assert "\x00" not in path
        self.path, self.owner, self.engine, self.api = path, identity(owner), identity(engine), None

    @staticmethod
    def read(connection, path):
        connection.request("GET", path)
        response = connection.getresponse()
        assert response.status == 200, "independent metadata GET failed"
        raw = response.read(1024 * 1024 + 1)
        assert len(raw) <= 1024 * 1024, "independent metadata body cap"
        return json.loads(raw, object_pairs_hook=unique_object)

    def get(self, path):
        connection = UnixHTTP(self.path)
        try:
            if self.api is None:
                version = self.read(connection, "/version")
                def minor(value):
                    assert isinstance(value, str) and 4 <= len(value) <= 6 and value.startswith("1.")
                    number = value[2:]
                    assert number.isascii() and number.isdigit() and str(int(number)) == number
                    return int(number)
                maximum = min(53, minor(version["ApiVersion"]))
                assert 44 <= maximum and minor(version["MinAPIVersion"]) <= maximum
                self.api = "1." + str(maximum)
                assert connection.sock is not None, "metadata version negotiation lost its connection"
            current = identity(self.read(connection, "/v" + self.api + "/info")["ID"])
            assert self.engine == current, "independent metadata daemon changed"
            assert connection.sock is not None, "metadata identity fence lost its connection"
            assert path.startswith("/containers/") and len(path) <= 2048
            return self.read(connection, "/v" + self.api + path)
        finally:
            connection.close()

    def rows(self):
        query = urllib.parse.urlencode({"all": "true", "filters": json.dumps({
            "label": ["rustack.lambda.ownerId=" + self.owner]})})
        rows = self.get("/containers/json?" + query)
        assert isinstance(rows, list) and len(rows) <= 128
        seen = set()
        for row in rows:
            key = row["Id"]
            assert isinstance(key, str) and len(key) == 64 and all(c in "0123456789abcdef" for c in key)
            assert key not in seen
            seen.add(key)
            labels = row["Labels"]
            assert isinstance(labels, dict) and len(labels) <= 32
            assert labels["rustack.lambda.ownerId"] == self.owner
            assert labels["rustack.lambda.role"] in ("function", "relay")
            for label in ("runId", "environmentId", "functionResourceId"):
                identity(labels["rustack.lambda." + label])
        return rows

    def ids(self):
        return {row["Id"] for row in self.rows()}

    def wait_ids(self, expected, deadline):
        while True:
            current = self.ids()
            assert time.monotonic() < deadline, "physical retirement missed its causal cutoff"
            if current == expected:
                return
            time.sleep(0.025)

    def pair_limits(self):
        rows = self.rows()
        assert len(rows) == 4
        environments, total = {}, 0
        for row in rows:
            labels = row["Labels"]
            role, environment = labels["rustack.lambda.role"], labels["rustack.lambda.environmentId"]
            environments.setdefault(environment, set()).add(role)
            inspected = self.get("/containers/" + row["Id"] + "/json")
            assert inspected["Id"] == row["Id"] and inspected["Config"]["Labels"] == labels
            assert inspected["State"]["Running"] is True
            memory = inspected["HostConfig"]["Memory"]
            assert memory == (128 if role == "function" else 64) * 1024 * 1024
            assert inspected["HostConfig"]["MemorySwap"] == memory
            total += memory
        assert len(environments) == 2 and all(roles == {"function", "relay"} for roles in environments.values())
        assert total == 384 * 1024 * 1024
        return {row["Id"] for row in rows}
