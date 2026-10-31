"""Single composite-Action producer: validate, own launch, qualify, hand off."""
import os
from pathlib import Path
import re
import shutil
import signal
import stat
import sys
import tempfile
import time
import uuid
import json

from docker_client import Client
import metadata
from process_owner import Runner
from settings import ActionError, Settings


def safe_path(value):
    if not isinstance(value, str) or not value or not os.path.isabs(value):
        raise ActionError("runner-path")
    if len(os.fsencode(value)) > 4096 or any(ord(char) < 32 or ord(char) == 127 for char in value):
        raise ActionError("runner-path")
    return Path(value)


def write_exclusive(path, body):
    if len(body) > 8192:
        raise ActionError("private-file-budget")
    descriptor = os.open(path, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
    with os.fdopen(descriptor, "wb") as stream:
        stream.write(body)
        stream.flush()
        os.fsync(stream.fileno())


def append_runner_file(path, body):
    if len(body) > 8192:
        raise ActionError("runner-output-budget")
    descriptor = os.open(path, os.O_WRONLY | os.O_APPEND | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "ab") as stream:
        observed = os.fstat(stream.fileno())
        if not stat.S_ISREG(observed.st_mode) or observed.st_nlink != 1 or observed.st_size > 1048576:
            raise ActionError("runner-output-file")
        stream.write(body)
        stream.flush()
        os.fsync(stream.fileno())


def lines(values):
    return "".join(key + "=" + value + "\n" for key, value in values.items()).encode("ascii")


class Owner:
    def __init__(self, settings, environment):
        self.launch_cutoff = time.monotonic() + 420
        self.settings = settings
        self.environment = environment
        self.runner = Runner(environment)
        self.program = shutil.which("docker", path=environment.get("PATH"))
        self.curl = shutil.which("curl", path=environment.get("PATH"))
        if not self.program or not self.curl:
            raise ActionError("required-command-unavailable")
        parent = safe_path(environment.get("RUNNER_TEMP"))
        self.output = safe_path(environment.get("GITHUB_OUTPUT"))
        self.export = safe_path(environment.get("GITHUB_ENV"))
        self.path = Path(tempfile.mkdtemp(prefix="rustack-action-", dir=parent))
        os.chmod(self.path, 0o700)
        self.env_file = self.path / "server.env"
        self.nonce = str(uuid.uuid4())
        self.stage = "launch"
        self.client, self.source, self.cid = None, None, None
        self.uncertain, self.handed_off = False, False

    def record(self, name, value, deadline=None):
        cutoff = self.launch_cutoff if deadline is None else deadline
        if time.monotonic() >= cutoff:
            raise ActionError("private-file-deadline")
        write_exclusive(self.path / name, (json.dumps(value, separators=(",", ":")) + "\n").encode("ascii"))
        if time.monotonic() >= cutoff:
            raise ActionError("private-file-late-observation")

    def launch(self):
        self.stage = "select-daemon"
        self.client = Client(self.runner, self.program, self.environment, self.launch_cutoff)
        self.record("intent.json", {"schemaVersion": 1, "engine": self.client.engine, "nonce": self.nonce,
                                    "reference": self.settings.reference, "name": "rustack", "port": self.settings.port})
        self.stage = "image"
        self.source = self.client.image(self.settings.reference, self.launch_cutoff)
        self.record("source.json", {"schemaVersion": 1, "engine": self.client.engine, "image": self.source,
                                    "nonce": self.nonce})
        write_exclusive(self.env_file, lines(self.settings.docker_environment()))
        self.stage = "start"
        arguments = ["run", "--detach", "--name", "rustack", "--restart=no",
                     "--publish", "127.0.0.1:" + str(self.settings.port) + ":4566",
                     "--label", metadata.NONCE_LABEL + "=" + self.nonce,
                     "--label", metadata.ENGINE_LABEL + "=" + self.client.engine,
                     "--env-file", str(self.env_file), self.source]
        self.uncertain = True
        try:
            reply = self.client.call(arguments, self.launch_cutoff, 60)
        finally:
            self.remove_input()
        if not reply.endswith(b"\n"):
            raise ActionError("container-start-reply")
        try:
            self.cid = metadata.identifier(reply[:-1].decode("ascii"))
        except UnicodeDecodeError:
            raise ActionError("container-start-reply") from None
        self.ready_cutoff = time.monotonic() + self.settings.wait_seconds
        self.record("server.json", {"schemaVersion": 1, "engine": self.client.engine, "image": self.source,
                                    "nonce": self.nonce, "id": self.cid}, self.ready_cutoff)
        self.client.inspect(self.cid, self.source, self.nonce, self.ready_cutoff)
        self.uncertain = False

    def ready(self):
        self.stage = "readiness"
        while True:
            if time.monotonic() >= self.ready_cutoff:
                raise ActionError("readiness-deadline")
            view = self.client.inspect(self.cid, self.source, self.nonce, self.ready_cutoff)
            started, healthy = metadata.publication(view, self.settings.port)
            if healthy:
                remaining = self.ready_cutoff - time.monotonic()
                if remaining <= 0:
                    raise ActionError("readiness-deadline")
                seconds = min(3, remaining)
                args = [self.curl, "--disable", "--silent", "--show-error", "--fail", "--noproxy", "*",
                        "--write-out", "%{stderr}%{http_code}", "--proto", "=http",
                        "--connect-timeout", str(min(2, seconds)), "--max-time", str(seconds),
                        "--url", "http://127.0.0.1:" + str(self.settings.port) + "/_health/ready"]
                reply = self.runner.run(args, min(self.ready_cutoff, time.monotonic() + seconds), 65536)
                if reply.code == 0:
                    if reply.stderr != b"200":
                        raise ActionError("http-status")
                    metadata.readiness(reply.stdout)
                    final = self.client.inspect(self.cid, self.source, self.nonce, self.ready_cutoff)
                    after, still_healthy = metadata.publication(final, self.settings.port)
                    if started != after or not still_healthy:
                        raise ActionError("container-changed-during-readiness")
                    if time.monotonic() >= self.ready_cutoff:
                        raise ActionError("readiness-late-result")
                    return
            remaining = self.ready_cutoff - time.monotonic()
            if remaining <= 0:
                raise ActionError("readiness-deadline")
            time.sleep(min(0.5, remaining))

    def handoff(self):
        self.stage = "handoff"
        if time.monotonic() >= self.ready_cutoff:
            raise ActionError("handoff-deadline")
        append_runner_file(self.export, lines(self.settings.caller_environment()))
        append_runner_file(self.output, lines({"container_id": self.cid, "endpoint": self.settings.endpoint}))
        if time.monotonic() >= self.ready_cutoff:
            raise ActionError("handoff-late-result")
        shutil.rmtree(self.path)
        self.handed_off = True
        print("Rustack is ready at " + self.settings.endpoint, flush=True)

    def remove_input(self):
        try:
            self.env_file.unlink()
        except FileNotFoundError:
            pass

    def cleanup(self):
        self.remove_input()
        if self.runner.unsettled:
            return "retained-unsettled-child"
        if self.uncertain:
            return "retained-uncertain"
        if self.handed_off:
            return "caller-owned"
        if not self.cid:
            return "no-known-container"
        cutoff = time.monotonic() + 90
        view = self.client.inspect(self.cid, self.source, self.nonce, cutoff)
        if view["running"]:
            self.uncertain = True
            reply = self.client.call(["kill", "--signal", "TERM", self.cid], cutoff)
            if reply != (self.cid + "\n").encode("ascii"):
                raise ActionError("cleanup-signal-reply")
            self.uncertain = False
        reply = self.client.call(["wait", self.cid], cutoff, 70)
        if reply != b"0\n":
            raise ActionError("cleanup-unexpected-exit")
        final = self.client.inspect(self.cid, self.source, self.nonce, cutoff)
        if final["running"] is not False or final["exit"] != 0:
            raise ActionError("cleanup-unsettled-server")
        self.uncertain = True
        reply = self.client.call(["container", "rm", self.cid], cutoff)
        if reply != (self.cid + "\n").encode("ascii"):
            raise ActionError("cleanup-remove-reply")
        remaining = self.client.call(["container", "ls", "--all", "--no-trunc", "--filter", "id=" + self.cid,
                                      "--format", "{{json .ID}}"], cutoff)
        if remaining.strip():
            raise ActionError("cleanup-container-still-present")
        self.uncertain = False
        if self.path.is_dir():
            self.record("cleanup.json", {"schemaVersion": 1, "engine": self.client.engine, "id": self.cid,
                                         "serverAbsent": True, "uncertain": False}, cutoff)
        return "removed"


def main(environment):
    owner = None
    stage = "input"
    interrupted = False
    def interrupt(_number, _frame):
        nonlocal interrupted
        if not interrupted:
            interrupted = True
            if owner is None or not owner.runner.retiring:
                raise ActionError("interrupted")
    try:
        settings = Settings.load(environment)
        for value in (settings.authentication.access, settings.authentication.secret):
            if value:
                escaped = value.replace("%", "%25").replace("\r", "%0D").replace("\n", "%0A")
                print("::add-mask::" + escaped, flush=True)
        signal.signal(signal.SIGTERM, interrupt)
        signal.signal(signal.SIGINT, interrupt)
        owner = Owner(settings, environment)
        owner.launch()
        owner.ready()
        owner.handoff()
        return 0
    except BaseException as error:
        cleanup = "no-owner"
        if owner is not None:
            stage = owner.stage
            try:
                cleanup = owner.cleanup()
            except BaseException:
                cleanup = "retained-cleanup-failure"
        code = error.args[0] if isinstance(error, ActionError) and len(error.args) == 1 else "internal-error"
        if not isinstance(code, str) or re.fullmatch(r"[a-z][a-z0-9-]{0,63}", code) is None:
            code = "internal-error"
        print("::error::Rustack Action failed at " + stage + " (" + code + ")", file=sys.stderr)
        print("Cleanup status: " + cleanup, file=sys.stderr)
        if owner is not None:
            if owner.cid:
                print("Owned container ID: " + owner.cid, file=sys.stderr)
            print("Retained private evidence: " + str(owner.path), file=sys.stderr)
        return 1


if __name__ == "__main__":
    if len(sys.argv) != 1:
        print("::error::Rustack Action accepts inputs through environment only", file=sys.stderr)
        raise SystemExit(2)
    raise SystemExit(main(os.environ))
