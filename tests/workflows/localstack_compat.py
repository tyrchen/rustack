"""Run original upstream tests in-process under an externally owned Rustack server."""
from contextlib import redirect_stderr, redirect_stdout
import importlib.metadata
import json
import os
import re
from pathlib import Path
import sys
import urllib.parse

from localstack_policy import (MANIFEST, PIN, ROOT, VENDOR, digest, guards, manifest, private,
                               publish, read, require, upstream)


class Log:
    """Bound pytest diagnostics without imposing filesystem limits on workloads."""
    def __init__(self, path):
        self.output = path.open("xb")
        os.chmod(path, 0o600)
        self.count = 0

    def write(self, value):
        raw = value.encode("utf-8")
        self.count += len(raw)
        require(self.count <= 16 * 1024 * 1024, "compat-diagnostics-bound")
        self.output.write(raw)
        return len(value)

    def flush(self):
        self.output.flush()

    def close(self):
        self.flush()
        os.fsync(self.output.fileno())
        self.output.close()

    def isatty(self):
        return False


def environment():
    guards([], os.environ)
    os.environ.update(PYTEST_DISABLE_PLUGIN_AUTOLOAD="1", PYTHONDONTWRITEBYTECODE="1",
                      AWS_EC2_METADATA_DISABLED="true", AWS_CONFIG_FILE="/dev/null",
                      AWS_SHARED_CREDENTIALS_FILE="/dev/null", AWS_DEFAULT_REGION="us-east-1")
    packages = {dist.metadata["Name"].lower().replace("_", "-"):dist.version
                for dist in importlib.metadata.distributions()}
    expected = {}
    for line in read(Path(__file__).with_name("localstack-requirements.txt"), 65536).decode().splitlines():
        if line and not line.startswith("#"):
            name, version = line.split("==")
            expected[name.lower().replace("_", "-")] = version
    require(all(packages.get(name) == version for name, version in expected.items())
            and set(packages) == set(expected) | {"localstack-core"}, "compat-dependency-drift")
    return packages


def source_seal():
    from linux_performance import Runner
    from rootless_functional import source_seal as seal
    import time
    return digest(json.dumps(seal(Runner(os.environ), time.monotonic() + 30),
                             sort_keys=True, separators=(",", ":")).encode())


def image_receipt(path, endpoint):
    import time
    from linux_performance import Runner
    from localstack_images import validate_receipt
    raw = read(Path(path))
    value = json.loads(raw)
    executable = Path(value["dockerExecutable"])
    require(executable.is_absolute() and executable.is_file(), "compat-docker-executable")
    # Runner intentionally inherits only explicit process settings. Rust's owner
    # clears PATH; give these read-only Docker commands the recorded executable.
    owner = Runner({"PATH":str(executable.parent) + ":/usr/bin:/bin"})
    validate_receipt(Path(path), endpoint, owner, time.monotonic() + 60)
    return value, digest(raw)


def pytest_arguments(root, nodes):
    return ["--rootdir=" + str(VENDOR), "--noconftest", "--override-ini=addopts=",
            "--override-ini=log_level=WARNING", "--tb=short", "-q", "--color=no", "-o", "cache_dir=" + str(root / "pytest-cache")] + [
            str(VENDOR / node.split("::")[0]) + "::" + node.split("::", 1)[1] for node in nodes]


def preflight(root, mode, images_path, docker_endpoint):
    require(mode in ("pure", "docker"), "compat-mode")
    private(root)
    environment()
    upstream()
    value, hashed = manifest()
    image_hash = None
    if mode == "docker":
        _, image_hash = image_receipt(images_path, docker_endpoint)
    # Import the real modules before starting Rustack, so fixture dependency errors
    # cannot leave an execution environment behind.
    sys.path.insert(0, str(VENDOR))
    for module in sorted({node.split("::")[0][:-3].replace("/", ".") for node in value["groups"][mode]}):
        __import__(module)
    from localstack.testing.snapshots.transformer_utility import TransformerUtility
    require(callable(TransformerUtility.stepfunctions_api), "compat-upstream-snapshot-transformers")
    import pytest
    from localstack_fixtures import Fixtures
    collecting = Fixtures(root, "http://127.0.0.1:1", mode, value["groups"][mode], {}, {}, collecting=True)
    log = Log(root / "preflight-pytest.log")
    try:
        with redirect_stdout(log), redirect_stderr(log):
            code = pytest.main(pytest_arguments(root, value["groups"][mode]) + ["--collect-only"],
                               plugins=[collecting])
    finally:
        log.close()
    require(code == 0 and sorted(collecting.collected) == sorted(value["groups"][mode]),
            "compat-preflight-collection")
    publish(root / "intent.json", {"schemaVersion":1, "vendorCommit":PIN,
        "manifestSha256":hashed, "mode":mode, "nodes":value["groups"][mode],
        "packages":environment(), "sourceSealSha256":source_seal(),
        "imageReceiptSha256":image_hash, "dockerEndpoint":docker_endpoint,
        "serverExecutable":json.loads(read(root / "server-intent.json", 65536))})


def run(root, mode, endpoint, images_path):
    private(root)
    packages = environment()
    upstream()
    value, hashed = manifest()
    intent = json.loads(read(root / "intent.json", 1024 * 1024))
    require(intent["mode"] == mode and intent["manifestSha256"] == hashed
            and intent["nodes"] == value["groups"][mode] and intent["packages"] == packages
            and intent["sourceSealSha256"] == source_seal(),
            "compat-intent-changed")
    parsed = urllib.parse.urlsplit(endpoint)
    require(parsed.scheme == "http" and parsed.hostname == "127.0.0.1" and parsed.port
            and not parsed.username and not parsed.password and parsed.path == ""
            and not parsed.query and not parsed.fragment, "compat-local-endpoint")
    images = {}
    if mode == "docker":
        image_value, image_hash = image_receipt(images_path, intent["dockerEndpoint"])
        require(image_hash == intent["imageReceiptSha256"], "compat-image-receipt-changed")
        images = image_value["handlers"]
    sys.path.insert(0, str(VENDOR))
    import pytest
    from localstack_snapshot.pytest import snapshot as snapshot_plugin
    from localstack_fixtures import Fixtures
    credentials = json.loads(read(root / "auth.json", 1024))
    require(set(credentials) == {"accessKey", "secretKey"}
            and all(isinstance(v, str) and 1 <= len(v) <= 128 for v in credentials.values()),
            "compat-owned-authentication")
    plugin = Fixtures(root, endpoint, mode, intent["nodes"], images, credentials)
    arguments = pytest_arguments(root, intent["nodes"]) + ["--junitxml=" + str(root / "junit.xml")]
    guards(arguments, os.environ)
    log = Log(root / "pytest.log")
    try:
        with redirect_stdout(log), redirect_stderr(log):
            code = pytest.main(arguments, plugins=[snapshot_plugin, plugin])
    finally:
        log.close()
    successful = False
    rejection = None
    try:
        require(code == 0, "compat-pytest-failed")
        plugin.result()
        successful = True
    except ValueError as error:
        rejection = str(error)
    upstream()
    require(manifest()[1] == hashed, "compat-final-manifest-drift")
    require(source_seal() == intent["sourceSealSha256"], "compat-final-source-drift")
    if mode == "docker":
        require(image_receipt(images_path, intent["dockerEndpoint"])[1] == intent["imageReceiptSha256"],
                "compat-final-image-drift")
    report = {"schemaVersion":1, "vendorCommit":PIN, "manifestSha256":hashed, "mode":mode,
        "endpoint":endpoint, "sourceSealSha256":intent["sourceSealSha256"],
        "serverExecutable":intent["serverExecutable"],
        "expected":len(intent["nodes"]), "collected":len(plugin.collected),
        "passed":sum(all(row.get(phase) == "passed" for phase in ("setup", "call", "teardown"))
                     and not row.get("xfail") for row in plugin.rows.values()),
        "successful":successful, "pytestExitCode":int(code), "rejection":rejection,
        "nodes":plugin.rows, "snapshots":plugin.snapshots, "packages":packages,
        "logSha256":digest(read(root / "pytest.log")), "upstreamBaselinesUnchanged":True,
        "packaging":"original Python3.12 source/runtime in RIC Image; upstream requested managed ZIP" if mode == "docker" else "no Lambda",
        "exclusions":value["exclusions"]}
    publish(root / "report.json", report)
    print(json.dumps({"expected":report["expected"], "passed":report["passed"],
                      "successful":successful, "evidence":str(root)}), flush=True)
    return 0 if successful else 1


def main():
    require(len(sys.argv) == 6 and sys.argv[1] in ("preflight", "run"), "compat-arguments")
    root, mode = Path(sys.argv[2]), sys.argv[3]
    if sys.argv[1] == "preflight":
        preflight(root, mode, sys.argv[4], sys.argv[5])
        return 0
    require(len(sys.argv) == 6, "compat-run-arguments")
    return run(root, mode, sys.argv[4], sys.argv[5])


if __name__ == "__main__":
    try:
        print(os.getpid(), flush=True)
        sys.exit(main())
    except (OSError, ValueError, KeyError, TypeError) as error:
        reason = str(error) if re.fullmatch(r"compat-[a-z0-9-]{1,80}", str(error)) else type(error).__name__
        print("localstack-compatibility-rejected:" + reason, file=sys.stderr)
        sys.exit(2)
