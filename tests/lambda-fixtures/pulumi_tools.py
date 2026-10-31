"""Provision checksummed Linux Pulumi executables without executing downloaded code."""

import argparse
import contextlib
import fcntl
import gzip
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import tempfile
import time
import urllib.request


CLI = "3.273.0"
AWS = "7.52.1"
COMPRESSED = 512 * 1024 * 1024
EXPANDED = 2 * 1024 * 1024 * 1024
HASHES = {
    "arm64": ("c2e1b600b37d9d84551259b6f391f5035e336dfe63c49412f22d855a91995ed5",
              "1976366b71dc08d8827a04fbb8eaf50a0d1ec5757726e593b6d7c27e7636daf8"),
    "amd64": ("ec08f86671fd58f0a186540e1cee00066393beaf5c6716b0075f690919800432",
              "2e705824a2103dd6e3dc66744ba2d55cad55d665476b3ffcd8ce024cf4cab081"),
}
SELECTED = {"pulumi/pulumi": "bin/pulumi", "pulumi/pulumi-language-nodejs": "bin/pulumi-language-nodejs"}
PROVIDER = "plugins/resource-aws-v" + AWS + "/pulumi-resource-aws"


class ToolError(ValueError):
    """A fixed, non-content-bearing artifact validation failure."""


def check(value):
    if not value:
        raise ToolError("invalid bounded Pulumi tool artifact")


def fresh(deadline):
    check(time.monotonic() < deadline)


def digest(path, limit, deadline):
    check(path.is_file() and not path.is_symlink() and path.stat().st_size <= limit)
    state, count = hashlib.sha256(), 0
    with path.open("rb") as stream:
        while True:
            fresh(deadline)
            data = stream.read(65536)
            if not data:
                break
            count += len(data)
            check(count <= limit)
            state.update(data)
    fresh(deadline)
    return state.hexdigest(), count


def download(url, expected, cache, deadline):
    path = cache / (expected + ".tar.gz")
    if path.exists():
        actual, _ = digest(path, COMPRESSED, deadline)
        check(actual == expected)
        return path
    temporary = None
    try:
        with tempfile.NamedTemporaryFile(dir=cache, prefix=".download-", delete=False) as output:
            temporary = Path(output.name)
            request = urllib.request.Request(url, headers={"User-Agent": "rustack-iac-fixture"})
            with urllib.request.urlopen(request, timeout=10) as response:
                check(response.status == 200 and response.url.startswith("https://"))
                count, state = 0, hashlib.sha256()
                while True:
                    fresh(deadline)
                    chunk = response.read(65536)
                    if not chunk:
                        break
                    count += len(chunk)
                    check(count <= COMPRESSED)
                    state.update(chunk)
                    output.write(chunk)
            check(state.hexdigest() == expected)
            fresh(deadline)
        os.replace(temporary, path)
        temporary = None
        return path
    finally:
        if temporary is not None:
            temporary.unlink(missing_ok=True)


def number(field):
    value = field.strip(b" \0")
    check(bool(value) and all(48 <= byte <= 55 for byte in value))
    return int(value, 8)


def text(field):
    head, separator, tail = field.partition(b"\0")
    check(not separator or not tail.strip(b"\0"))
    return head.decode("ascii")


def name(value, directory=False):
    check(len(value.encode()) <= 512 and "\\" not in value)
    if directory and value in (".", "./"):
        return ""
    if value.startswith("./"):
        value = value[2:]
    if directory and value.endswith("/"):
        value = value[:-1]
    parts = value.split("/")
    check(all(part not in ("", ".", "..") and re.fullmatch(r"[A-Za-z0-9_.+-]+", part) for part in parts))
    return "/".join(parts)


def pax(data):
    values, at = {}, 0
    while at < len(data):
        end = data.find(b" ", at, at + 8)
        check(end > at and data[at:end].isdigit())
        count = int(data[at:end])
        check(count > end - at + 2 and at + count <= len(data))
        record = data[end + 1:at + count]
        check(record.endswith(b"\n"))
        key, separator, value = record[:-1].partition(b"=")
        check(separator and len(key) <= 64 and re.fullmatch(rb"[A-Za-z0-9_.-]+", key)
              and len(value) <= 512 and key not in values
              and key in (b"path", b"mtime", b"atime", b"ctime", b"comment", b"charset"))
        values[key] = value
        at += count
    return values


def unpack(archive, destination, selected, architecture, deadline):
    check(architecture in HASHES and 1 <= len(selected) <= 3)
    found, seen, total, members, pending, global_fields = {}, set(), 0, 0, {}, {}
    with gzip.open(archive, "rb") as stream:
        def read(size):
            nonlocal total
            fresh(deadline)
            check(0 <= size <= 65536)
            data = stream.read(size)
            total += len(data)
            check(total <= EXPANDED)
            fresh(deadline)
            return data
        while True:
            header = read(512)
            check(len(header) == 512)
            if header == bytes(512):
                check(read(512) == bytes(512))
                trailing = read(10241)
                check(len(trailing) <= 10240 and not trailing.strip(b"\0"))
                check(not read(1))
                break
            members += 1
            check(members <= 4096)
            check(number(header[148:156]) == sum(header[:148]) + 256 + sum(header[156:]))
            check(header[257:263] in (b"ustar\0", b"ustar "))
            kind, size = header[156:157], number(header[124:136])
            check(size <= EXPANDED and total + size <= EXPANDED)
            raw = text(header[:100])
            prefix = text(header[345:500])
            if prefix:
                raw = prefix + "/" + raw
            if kind in (b"x", b"g"):
                check(size <= 4096)
                body = read(size)
                check(len(body) == size)
                fields = pax(body)
                if b"path" in fields:
                    name(fields[b"path"].decode("ascii"))
                if kind == b"g":
                    check(b"path" not in fields)
                    global_fields.update(fields)
                else:
                    check(not pending)
                    pending = fields
            else:
                check(kind in (b"0", b"\0", b"5"))
                fields = {**global_fields, **pending}
                pending = {}
                member = name(fields.get(b"path", raw.encode()).decode("ascii"), kind == b"5")
                check(member not in seen)
                seen.add(member)
                check(kind != b"5" or size == 0)
                target = selected.get(member)
                if target is not None:
                    check(kind != b"5" and member not in found and size >= 64)
                    path = destination / target if destination is not None else None
                    if path is not None:
                        path.parent.mkdir(parents=True, exist_ok=True)
                        check(not path.exists())
                    state, first = hashlib.sha256(), b""
                    with path.open("xb") if path is not None else contextlib.nullcontext() as output:
                        left = size
                        while left:
                            chunk = read(min(left, 65536))
                            check(bool(chunk))
                            if len(first) < 64:
                                first += chunk[:64 - len(first)]
                            if output is not None:
                                output.write(chunk)
                            state.update(chunk)
                            left -= len(chunk)
                    machine = 183 if architecture == "arm64" else 62
                    check(first[:6] == b"\x7fELF\x02\x01" and int.from_bytes(first[18:20], "little") == machine)
                    if path is not None:
                        path.chmod(0o755)
                    found[member] = {"path": target, "sha256": state.hexdigest(), "bytes": size}
                else:
                    left = size
                    while left:
                        chunk = read(min(left, 65536))
                        check(bool(chunk))
                        left -= len(chunk)
            padding = (-size) % 512
            check(read(padding) == bytes(padding))
        check(not pending and found.keys() == selected.keys())
    return list(found.values())


def validate_tree(root, deadline):
    files = set(SELECTED.values()) | {PROVIDER, "manifest.json"}
    directories = {"bin", "plugins", "plugins/resource-aws-v" + AWS}
    seen_files, seen_directories, stack, total = set(), set(), [root], 0
    while stack:
        fresh(deadline)
        directory = stack.pop()
        info = directory.lstat()
        check(stat.S_ISDIR(info.st_mode) and stat.S_IMODE(info.st_mode) == 0o755 and info.st_uid == os.getuid())
        with os.scandir(directory) as entries:
            for entry in entries:
                fresh(deadline)
                relative = str(Path(entry.path).relative_to(root))
                info = entry.stat(follow_symlinks=False)
                check(not stat.S_ISLNK(info.st_mode) and info.st_uid == os.getuid())
                if stat.S_ISDIR(info.st_mode):
                    check(relative in directories and relative not in seen_directories)
                    seen_directories.add(relative)
                    stack.append(Path(entry.path))
                else:
                    check(stat.S_ISREG(info.st_mode) and relative in files and relative not in seen_files and info.st_nlink == 1)
                    expected_mode = 0o644 if relative == "manifest.json" else 0o755
                    check(stat.S_IMODE(info.st_mode) == expected_mode)
                    total += info.st_size
                    check(total <= EXPANDED and (relative != "manifest.json" or info.st_size <= 16384))
                    seen_files.add(relative)
                check(len(seen_files) + len(seen_directories) <= 7)
    check(seen_files == files and seen_directories == directories)


def prepare(destination, architecture):
    check(destination.is_absolute() and len(str(destination).encode()) <= 4096 and not destination.is_symlink())
    destination.parent.mkdir(parents=True, exist_ok=True)
    deadline = time.monotonic() + 600
    lock_path = destination.parent / ".pulumi-tools.lock"
    fd = os.open(lock_path, os.O_CREAT | os.O_RDWR | os.O_NOFOLLOW | os.O_CLOEXEC, 0o600)
    with os.fdopen(fd, "r+b") as lock:
        info = os.fstat(lock.fileno())
        check(stat.S_ISREG(info.st_mode) and info.st_uid == os.getuid() and info.st_nlink == 1 and stat.S_IMODE(info.st_mode) == 0o600)
        while True:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except BlockingIOError:
                fresh(deadline)
                time.sleep(0.025)
        cli_hash, aws_hash = HASHES[architecture]
        cache = destination.parent / "pulumi-archives"
        cache.mkdir(exist_ok=True)
        cli_arch = "arm64" if architecture == "arm64" else "x64"
        cli = download("https://get.pulumi.com/releases/sdk/pulumi-v" + CLI + "-linux-" + cli_arch + ".tar.gz", cli_hash, cache, deadline)
        provider = download("https://github.com/pulumi/pulumi-aws/releases/download/v" + AWS + "/pulumi-resource-aws-v" + AWS + "-linux-" + architecture + ".tar.gz", aws_hash, cache, deadline)
        if destination.exists():
            validate_tree(destination, deadline)
            manifest = destination / "manifest.json"
            check(manifest.is_file() and not manifest.is_symlink() and manifest.stat().st_size <= 16384)
            expected_files = unpack(cli, None, SELECTED, architecture, deadline)
            expected_files += unpack(provider, None, {"pulumi-resource-aws": PROVIDER}, architecture, deadline)
            expected = {"schemaVersion": 1, "architecture": architecture, "cli": CLI, "provider": AWS,
                        "archives": [cli_hash, aws_hash], "files": expected_files}
            check(json.loads(manifest.read_bytes()) == expected)
            for entry in expected_files:
                path = destination / entry["path"]
                for part in path.relative_to(destination).parents:
                    directory = destination / part
                    check(directory.is_dir() and not directory.is_symlink())
                value, size = digest(path, EXPANDED, deadline)
                check(value == entry["sha256"] and size == entry["bytes"])
            return
        with tempfile.TemporaryDirectory(dir=destination.parent, prefix=".pulumi-tools-") as temporary:
            stage = Path(temporary) / "tools"
            stage.mkdir()
            files = unpack(cli, stage, SELECTED, architecture, deadline)
            files += unpack(provider, stage, {"pulumi-resource-aws": PROVIDER}, architecture, deadline)
            data = {"schemaVersion": 1, "architecture": architecture, "cli": CLI, "provider": AWS,
                    "archives": [cli_hash, aws_hash], "files": files}
            (stage / "manifest.json").write_text(json.dumps(data, sort_keys=True) + "\n")
            (stage / "manifest.json").chmod(0o644)
            for directory in (stage, stage / "bin", stage / "plugins", stage / "plugins" / ("resource-aws-v" + AWS)):
                directory.chmod(0o755)
            validate_tree(stage, deadline)
            fresh(deadline)
            os.rename(stage, destination)


if __name__ == "__main__":
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("architecture", choices=tuple(HASHES))
    parser.add_argument("destination", type=Path)
    args = parser.parse_args()
    prepare(args.destination, args.architecture)
