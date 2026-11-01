"""Exact external-test inventory and immutable upstream baseline policy."""
import hashlib
import json
import os
from pathlib import Path
import re
import stat

ROOT = Path(__file__).resolve().parents[2]
VENDOR = ROOT / "vendors/localstack"
MANIFEST = Path(__file__).with_name("localstack-required.json")
PIN = "45dfee8ce27df00fcac3f91b8071c8156eb5c5c7"
FILE_LIMIT = 16 * 1024 * 1024


def require(condition, code):
    if not condition:
        raise ValueError(code)


def read(path, limit=FILE_LIMIT):
    require(not path.is_symlink(), "compat-symlink")
    metadata = path.stat()
    require(stat.S_ISREG(metadata.st_mode) and metadata.st_size <= limit, "compat-file-bound")
    with path.open("rb") as source:
        raw = source.read(limit + 1)
    require(len(raw) <= limit, "compat-file-growth")
    return raw


def digest(raw):
    return hashlib.sha256(raw).hexdigest()


def relative(value):
    require(isinstance(value, str) and len(value) <= 1024 and
            re.fullmatch(r"[A-Za-z0-9_./-]+", value) and
            all(part not in ("", ".", "..") for part in value.split("/")), "compat-relative-path")
    return VENDOR / value


def manifest():
    raw = read(MANIFEST, 1024 * 1024)
    value = json.loads(raw)
    require(set(value) == {"schemaVersion", "vendorCommit", "groups", "files", "exclusions"}
            and value["schemaVersion"] == 1 and value["vendorCommit"] == PIN
            and set(value["groups"]) == {"pure", "docker"}, "compat-manifest-shape")
    names = []
    for nodes in value["groups"].values():
        require(isinstance(nodes, list) and 1 <= len(nodes) <= 512, "compat-inventory-bound")
        for node in nodes:
            require(isinstance(node, str) and len(node.encode()) <= 2048 and "::" in node
                    and not any(ord(c) < 32 for c in node), "compat-node-id")
            relative(node.split("::")[0])
        names.extend(nodes)
    require(len(names) == len(set(names)), "compat-repeated-node")
    require(isinstance(value["files"], dict) and 1 <= len(value["files"]) <= 256,
            "compat-file-inventory")
    for path, expected in value["files"].items():
        require(re.fullmatch(r"[0-9a-f]{64}", expected) and digest(read(relative(path))) == expected,
                "compat-upstream-file-drift")
    for node in names:
        path = node.split("::")[0]
        snapshot = path[:-3] + ".snapshot.json"
        require(path in value["files"] and snapshot in value["files"], "compat-unbound-node")
        recorded = json.loads(read(relative(snapshot)))
        require(node in recorded and isinstance(recorded[node].get("recorded-content"), dict)
                and bool(recorded[node]["recorded-content"]), "compat-missing-baseline")
    return value, digest(raw)


def upstream():
    import time
    from linux_performance import Runner
    owner = Runner(os.environ)
    def git(*args):
        reply = owner.run(["git", "-C", str(VENDOR), *args], time.monotonic() + 20,
                          stdout_limit=65536)
        require(reply.code == 0 and not reply.stderr, "compat-git-diagnostics")
        return reply.stdout.decode().strip()
    require(git("rev-parse", "HEAD") == PIN and not git("status", "--porcelain", "--untracked-files=normal"),
            "compat-vendor-drift")


def guards(arguments, environment):
    forbidden = ("--snapshot-update", "--snapshot-raw", "--snapshot-skip-all", "--collect-only",
                 "--lf", "--last-failed", "--maxfail", "-x")
    require(not any(arg == flag or arg.startswith(flag + "=") for arg in arguments for flag in forbidden),
            "compat-forbidden-option")
    require(not any(environment.get(key) for key in ("SNAPSHOT_UPDATE", "SNAPSHOT_RAW", "PYTEST_ADDOPTS",
            "PYTEST_PLUGINS", "TEST_TARGET", "AWS_PROFILE", "AWS_DEFAULT_PROFILE")), "compat-ambient-option")


def exact(expected, collected):
    require(len(collected) == len(set(collected)) and sorted(collected) == sorted(expected),
            "compat-collection-mismatch")


def outcomes(expected, rows):
    exact(expected, list(rows))
    require(all(row.get("setup") == row.get("call") == row.get("teardown") == "passed"
                and not row.get("xfail") for row in rows.values()), "compat-unsuccessful-node")


def private(path):
    require(path.is_absolute() and not path.is_symlink() and path.resolve(strict=True) == path,
            "compat-evidence-path")
    info = path.stat()
    require(stat.S_ISDIR(info.st_mode) and info.st_uid == os.getuid()
            and stat.S_IMODE(info.st_mode) == 0o700, "compat-evidence-ownership")


def publish(path, value):
    raw = json.dumps(value, separators=(",", ":"), allow_nan=False).encode()
    require(len(raw) <= FILE_LIMIT, "compat-report-bound")
    with path.open("xb") as output:
        os.chmod(path, 0o600)
        output.write(raw)
        output.flush()
        os.fsync(output.fileno())
