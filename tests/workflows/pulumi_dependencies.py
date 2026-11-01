"""Serialize the task-owned, locked Pulumi fixture dependency installation."""

import fcntl
import hashlib
import json
import os
from pathlib import Path
import shutil
import stat
import subprocess
import sys
import time


SOURCE = Path(__file__).resolve().parent.parent / "lambda-fixtures" / "pulumi"
VERSIONS = {"@pulumi/aws": "7.52.1", "@pulumi/pulumi": "3.273.0", "glob": "14.0.3", "minimatch": "11.0.3"}


def manifest():
    raw = (SOURCE / "package-lock.json").read_bytes()
    assert len(raw) <= 2 * 1024 * 1024
    lock = json.loads(raw)
    assert lock["lockfileVersion"] == 3 and len(lock["packages"]) <= 1024
    for key, value in lock["packages"].items():
        if key:
            assert value["resolved"].startswith("https://registry.npmjs.org/")
            assert value["integrity"].startswith("sha512-")
    return hashlib.sha256(raw + (SOURCE / "package.json").read_bytes()).hexdigest()


def installed(fingerprint):
    base = SOURCE / "node_modules"
    try:
        if (base / ".rustack-iac-lock").read_text() != fingerprint:
            return False
        for name, expected in VERSIONS.items():
            path = base / name / "package.json"
            if path.is_symlink() or json.loads(path.read_bytes())["version"] != expected:
                return False
        return True
    except (OSError, ValueError, KeyError):
        return False


def prepare(cache):
    assert cache.is_absolute() and not cache.is_symlink()
    cache.mkdir(parents=True, exist_ok=True)
    fingerprint = manifest()
    flags = os.O_RDWR | os.O_CREAT | os.O_NOFOLLOW | os.O_CLOEXEC
    fd = os.open(SOURCE / ".rustack-pulumi-install.lock", flags, 0o600)
    with os.fdopen(fd, "r+b") as lock:
        info = os.fstat(lock.fileno())
        assert stat.S_ISREG(info.st_mode) and info.st_nlink == 1 and info.st_uid == os.getuid()
        assert stat.S_IMODE(info.st_mode) == 0o600
        deadline = time.monotonic() + 300
        while True:
            try:
                fcntl.flock(lock, fcntl.LOCK_EX | fcntl.LOCK_NB)
                break
            except BlockingIOError:
                assert time.monotonic() < deadline, "fixture dependency lock expired"
                time.sleep(0.025)
        configs = cache / "config"
        configs.mkdir(exist_ok=True)
        for kind in ("user", "global"):
            path = configs / kind
            if not path.exists():
                path.write_bytes(b"")
            assert path.is_file() and not path.is_symlink() and path.stat().st_size == 0
        if installed(fingerprint):
            return
        executable = shutil.which("npm")
        assert executable is not None
        environment = {"PATH": os.environ["PATH"], "LC_ALL": "C", "NPM_CONFIG_UPDATE_NOTIFIER": "false"}
        remaining = deadline - time.monotonic()
        assert remaining > 0
        subprocess.run([executable, "ci", "--ignore-scripts", "--no-fund", "--registry=https://registry.npmjs.org",
            "--userconfig=" + str(configs / "user"), "--globalconfig=" + str(configs / "global"),
            "--cache=" + str(cache)], cwd=SOURCE, env=environment, stdin=subprocess.DEVNULL,
            check=True, timeout=remaining)
        assert time.monotonic() < deadline
        (SOURCE / "node_modules" / ".rustack-iac-lock").write_text(fingerprint)
        assert installed(fingerprint), "fixture dependency installation differs"


if __name__ == "__main__":
    assert len(sys.argv) == 2
    prepare(Path(sys.argv[1]))
