"""Independent, bounded read-only tool-container archive and checkpoint oracle."""

import hashlib
from datetime import datetime
import json
import os
from pathlib import Path
import re
import time

from custom_ric_sdk import unique_object
from lambda_fault_support import UnixHTTP

CAP = 16 * 1024 * 1024
FILE_CAP = 2 * 1024 * 1024
SIGNATURE = "4dabf18193072939515e22adb298388d"
SECRET = "1b47061264138c4ac30d75fd1eb44270"


def require(value):
    if not value:
        raise ValueError("bounded IaC archive validation failed")


def private(data, secrets):
    require(isinstance(data, bytes) and len(data) <= CAP and len(secrets) == 2)
    for secret in secrets:
        require(isinstance(secret, bytes) and 1 <= len(secret) <= 256)
        require(secret not in data)


def document(data, secrets):
    private(data, secrets)
    try:
        value = json.loads(data.decode("utf-8"), object_pairs_hook=unique_object)
        pending, count = [(value, 0)], 0
        while pending:
            current, depth = pending.pop()
            count += 1
            require(depth <= 64 and count <= 65536)
            if isinstance(current, str):
                private(current.encode("utf-8"), secrets)
            elif isinstance(current, dict):
                require(len(current) <= 4096)
                pending.extend((entry, depth + 1) for pair in current.items() for entry in pair)
            elif isinstance(current, list):
                require(len(current) <= 4096)
                pending.extend((entry, depth + 1) for entry in current)
        return value
    except (ValueError, UnicodeError, RecursionError, AssertionError):
        raise ValueError("bounded IaC JSON privacy validation failed") from None


def log_private(data, secrets):
    """Scan raw output and every bounded JSON string token in mixed CLI text."""
    private(data, secrets)
    require(len(data) <= FILE_CAP)
    try:
        text = data.decode("utf-8", "strict")
        offset, tokens = 0, 0
        while offset < len(text):
            if text[offset] != '"':
                offset += 1
                continue
            start = offset
            offset += 1
            escaped = False
            while offset < len(text):
                current = text[offset]
                offset += 1
                if current == '"' and not escaped:
                    break
                escaped = current == "\\" and not escaped
            else:
                raise ValueError("unclosed CLI JSON string")
            tokens += 1
            require(tokens <= 65536)
            value = json.loads(text[start:offset])
            require(isinstance(value, str))
            private(value.encode("utf-8"), secrets)
    except (ValueError, UnicodeError):
        raise ValueError("bounded IaC log privacy validation failed") from None


def evidence_directory(value):
    require(isinstance(value, str) and len(value.encode()) <= 4096 and "\x00" not in value and os.path.isabs(value))
    # macOS /var and /tmp are system aliases. Normalize the already-owned root,
    # then create a new private child rather than assuming tempfile's mode.
    root = Path(value).resolve(strict=True)
    require(root.is_dir() and root.stat().st_uid == os.getuid())
    private_root = root / "iac-private"
    private_root.mkdir(mode=0o700)
    require(private_root.stat().st_mode & 0o777 == 0o700)
    return private_root


def number(field):
    require(not field.startswith(bytes([128])))
    value = field.rstrip(b"\x00 ").lstrip(b" ") or b"0"
    require(all(48 <= c <= 55 for c in value))
    return int(value, 8)


def unpack(data, root, secrets):
    require(isinstance(data, bytes) and len(data) <= CAP and len(data) % 512 == 0)
    private(data, secrets)
    require(root in ("backend", "logs"))
    offset, entries, files = 0, 0, {}
    while offset + 512 <= len(data):
        header = data[offset:offset + 512]
        offset += 512
        if header == bytes(512):
            require(data[offset:offset + 512] == bytes(512) and not any(data[offset:]))
            return files
        entries += 1
        require(entries <= 128 and number(header[148:156]) == sum(header[:148]) + 8 * 32 + sum(header[156:]))
        require(header[257:263] in (b"ustar\x00", b"ustar "))
        name = header[:100].split(b"\x00", 1)[0]
        prefix = header[345:500].split(b"\x00", 1)[0]
        name = ((prefix + b"/") if prefix else b"") + name
        require(len(name) <= 512 and b"\\" not in name and not name.startswith(b"/"))
        name = name.decode("utf-8", "strict").rstrip("/")
        parts = name.split("/")
        require(1 <= len(parts) <= 8 and all(part not in ("", ".", "..") for part in parts) and parts[0] == root)
        size, kind = number(header[124:136]), header[156:157]
        require(size <= FILE_CAP and kind in (b"0", b"\x00", b"5") and not any(header[157:257]))
        require(offset + ((size + 511) // 512) * 512 <= len(data))
        body = data[offset:offset + size]
        require(not any(data[offset + size:offset + ((size + 511) // 512) * 512]))
        offset += ((size + 511) // 512) * 512
        if kind == b"5":
            require(size == 0)
            continue
        require(name not in files)
        log_private(body, secrets)
        if name.endswith((".json", ".json.bak")):
            document(body, secrets)
        elif root == "logs":
            require(name.endswith((".stdout", ".stderr")))
        files[name] = body
    raise ValueError("truncated IaC archive")


def checkpoint(files, observation, stage, target_arn, secrets):
    require(set(observation) == {"path", "present", "bytes", "sha256"})
    require(observation["path"] == "backend/.pulumi/stacks/rustack-iac/fixture.json")
    source = files.get(observation["path"])
    require(observation["present"] is (stage != "post-removal"))
    if stage == "post-removal":
        require(source is None and observation["bytes"] == 0 and observation["sha256"] == "")
        return
    require(source is not None and len(source) == observation["bytes"] and hashlib.sha256(source).hexdigest() == observation["sha256"])
    value = document(source, secrets)
    require(value.get("version") == 3)
    current = value["checkpoint"]
    require(isinstance(current, dict) and current.get("stack") == "organization/rustack-iac/fixture")
    latest = current.get("latest")
    require(isinstance(latest, dict) and isinstance(latest.get("manifest"), dict))
    manifest = latest["manifest"]
    version = manifest.get("version")
    require(version in ("3.273.0", "v3.273.0") and
            manifest.get("magic") == hashlib.sha256(version.encode()).hexdigest())
    stamp = manifest.get("time")
    require(isinstance(stamp, str) and len(stamp) <= 64 and
            re.fullmatch(r"[0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9]{2}:[0-9]{2}:[0-9]{2}(?:\.[0-9]{1,9})?(?:Z|[+-][0-9]{2}:[0-9]{2})", stamp) is not None)
    require(datetime.fromisoformat(stamp.replace("Z", "+00:00")).tzinfo is not None)
    resources = latest.get("resources", [])
    require(isinstance(resources, list) and 0 <= len(resources) <= 3)
    require(all(isinstance(entry, dict) for entry in resources))
    targets = [entry for entry in resources if entry.get("type") == "aws:sfn/stateMachine:StateMachine"]
    if stage == "pre-destroy":
        require(len(targets) == 1 and targets[0]["id"] == target_arn)
        providers = [entry for entry in resources if entry.get("type") == "pulumi:providers:aws"]
        require(len(providers) == 1)
        for name in ("accessKey", "secretKey"):
            encrypted = providers[0]["inputs"][name]
            require(isinstance(encrypted, dict) and encrypted.get(SIGNATURE) == SECRET and
                    isinstance(encrypted.get("ciphertext"), str) and 1 <= len(encrypted["ciphertext"]) <= 4096 and
                    "plaintext" not in encrypted and "value" not in encrypted)
    else:
        require(stage == "post-destroy" and not targets)


def exec_stream(response, cutoff):
    data, frames = bytearray(), 0
    def exact(count, allow_eof=False):
        result = bytearray()
        while len(result) < count:
            require(time.monotonic() < cutoff)
            value = response.read(min(65536, count - len(result)))
            require(time.monotonic() < cutoff)
            if not value and allow_eof and not result:
                return None
            require(value)
            result.extend(value)
        return bytes(result)
    while True:
        header = exact(8, allow_eof=True)
        if header is None:
            break
        frames += 1
        require(frames <= 4096 and header[0] in (1, 2) and header[1:4] == bytes(3))
        size = int.from_bytes(header[4:], "big")
        require(0 < size <= 2 * 1024 * 1024 and len(data) + size <= CAP)
        content = exact(size)
        require(header[0] == 1)  # Reader errors never cross the host diagnostic boundary.
        data.extend(content)
    return bytes(data)


def exec_complete(observed, key, container, command):
    config = observed["ProcessConfig"]
    require(observed["ID"] == key and observed["ContainerID"] == container and
            config["entrypoint"] == command[0] and config["arguments"] == command[1:] and
            config["privileged"] is False and config["tty"] is False and config["user"] == "1000:1000" and
            type(observed["Running"]) is bool)
    if observed["Running"]:
        return False
    require(type(observed["ExitCode"]) is int and observed["ExitCode"] == 0)
    return True


class Archive:
    def __init__(self, metadata, row, source, secrets):
        self.metadata, self.row, self.source, self.secrets = metadata, row, source, secrets
        require(re.fullmatch(r"sha256:[0-9a-f]{64}", source) is not None)
        require(row["Labels"]["rustack.lambda.role"] == "function")

    def request(self, method, target, payload, cutoff):
        require(time.monotonic() < cutoff)
        connection = UnixHTTP(self.metadata.path)
        try:
            identity = self.metadata.read(connection, "/v" + self.metadata.api + "/info")["ID"]
            require(identity == self.metadata.engine and connection.sock is not None)
            encoded = json.dumps(payload, separators=(",", ":")).encode() if payload is not None else None
            require(encoded is None or len(encoded) <= 2048)
            connection.request(method, "/v" + self.metadata.api + target, body=encoded,
                               headers={"Content-Type": "application/json"} if encoded is not None else {})
            response = connection.getresponse()
            require(response.status in (200, 201))
            raw = response.read(65537)
            require(len(raw) <= 65536 and time.monotonic() < cutoff)
            return document(raw, self.secrets)
        finally:
            connection.close()

    def exec_read(self, request_id, root, cutoff):
        command = ["/var/lang/bin/node", "/var/task/pulumi/evidence_reader.js", request_id, root]
        self.progress = "exec-create"
        created = self.request("POST", "/containers/" + self.row["Id"] + "/exec", {
            "AttachStdin": False, "AttachStdout": True, "AttachStderr": True, "Tty": False,
            "Privileged": False, "User": "1000:1000", "Cmd": command,
        }, cutoff)
        require(set(created) == {"Id"} and re.fullmatch(r"[0-9a-f]{64}", created["Id"]) is not None)
        key = created["Id"]
        connection = UnixHTTP(self.metadata.path)
        try:
            identity = self.metadata.read(connection, "/v" + self.metadata.api + "/info")["ID"]
            require(identity == self.metadata.engine and connection.sock is not None)
            self.progress = "exec-start"
            connection.request("POST", "/v" + self.metadata.api + "/exec/" + key + "/start",
                               body=b'{"Detach":false,"Tty":false}', headers={"Content-Type": "application/json"})
            response = connection.getresponse()
            require(response.status == 200)
            self.progress = "exec-stream"
            data = exec_stream(response, cutoff)
        finally:
            connection.close()
        self.progress = "exec-join"
        while True:
            observed = self.request("GET", "/exec/" + key + "/json", None, cutoff)
            if exec_complete(observed, key, self.row["Id"], command):
                require(time.monotonic() < cutoff)
                return data
            require(observed["Running"] is True and time.monotonic() < cutoff)
            time.sleep(min(0.025, max(0, cutoff - time.monotonic())))

    def read(self, request_id, root, retain=None):
        require(re.fullmatch(r"[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}", request_id) is not None)
        require(root in ("backend", "logs"))
        cutoff = time.monotonic() + 20
        self.progress = "inspect"
        current = self.metadata.get("/containers/" + self.row["Id"] + "/json")
        require(current["Id"] == self.row["Id"] and current["Config"]["Labels"] == self.row["Labels"] and
                current["State"]["Running"] is True and current["Image"] == self.source)
        data = self.exec_read(request_id, root, cutoff)
        self.progress = "retain"
        if retain is not None:
            # Retain only the bounded raw TAR; no archive path becomes a host path.
            require(retain.name == "iac-failure-" + request_id + "-" + root + ".tar")
            descriptor = os.open(retain, os.O_WRONLY | os.O_CREAT | os.O_EXCL | os.O_NOFOLLOW, 0o600)
            with os.fdopen(descriptor, "wb") as output:
                require(output.write(data) == len(data))
            require(time.monotonic() < cutoff)
        self.progress = "unpack"
        result = unpack(data, root, self.secrets)
        require(time.monotonic() < cutoff)
        self.progress = "complete"
        return result
