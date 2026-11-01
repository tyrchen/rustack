"""Build current production fixtures behind retained source caches and receipts."""
import hashlib
import json
import os
from pathlib import Path
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
from linux_performance import ActionError
import rootless_platform as storage
from image_performance_images import catalog, fenced
from linux_performance import MIB, QualificationError, Runner, bounded_read, deadline_guard, require, safe_path
from rootless_functional import source_seal
from rootless_resources import REFRESH_SOURCES, captured

TARGETS = ("lambda-runtime-image", "lambda-custom-ric-images", "lambda-workflows-server-image", "lambda-workflows-release-image")
NAMES = REFRESH_SOURCES


def run(directory, socket, builder, owner, deadline):
    require(not os.path.lexists(directory), "refresh-build-evidence-exists")
    directory.mkdir(mode=0o700)
    source = source_seal(owner, deadline)
    hashed = hashlib.sha256(json.dumps(source, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    endpoint = "unix://" + str(socket)
    engine = owner.run(["docker", "--host", endpoint, "info", "--format", "{{.ID}}"], deadline)
    require(engine.code == 0 and engine.stderr == b"", "refresh-build-engine")
    engine = engine.stdout.decode().strip()
    ids = catalog(owner, endpoint, deadline, engine)
    before = {name: captured(owner, endpoint, name, deadline, engine) for name in NAMES}
    storage.publish(directory / "intent.json", {"sourceSealSha256": hashed, "engine": engine,
        "sourceSocket": str(socket), "originalIds": sorted(ids), "before": before, "targets": list(TARGETS)})
    # The source cache guard must settle before any build can move a fixture tag.
    arguments = ["make", "--no-print-directory", "-j1", "workflows-retain-fixture-sources"]
    require(builder and len(builder) <= 256 and all(character.isalnum() or character in "_./ -" for character in builder), "refresh-builder")
    arguments += list(TARGETS) + ["WORKFLOWS_ARCH=arm64", "WORKFLOWS_DOCKER_SOCKET=" + str(socket), "WORKFLOWS_IMAGE_BUILDER=" + builder]
    log = directory / "build.log"
    wrapper = [sys.executable, "-I", "-B", str(Path(__file__).with_name("gate_command.py")), str(log)] + arguments
    reply = owner.run(wrapper, deadline)
    require(reply.code == 0 and reply.stdout == reply.stderr == b"", "refresh-build-failed")
    require(source_seal(owner, deadline) == source, "refresh-build-source-changed")
    after = {name: captured(owner, endpoint, name, deadline, engine) for name in NAMES}
    final_ids = catalog(owner, endpoint, deadline, engine)
    require(ids <= final_ids, "refresh-build-lost-original-image")
    report = {"schemaVersion": 1, "sourceSealSha256": hashed, "engine": engine, "sourceSocket": str(socket),
              "originalIds": sorted(ids), "resultIds": sorted(final_ids), "before": before, "images": after,
              "targets": list(TARGETS), "logSha256": hashlib.sha256(bounded_read(log, 16 * MIB)).hexdigest(),
              "allChildGroupsJoined": not owner.unsettled}
    storage.publish(directory / "report.json", report)


def main():
    try:
        require(len(sys.argv) == 4, "refresh-build-arguments")
        owner = Runner(os.environ)
        with deadline_guard(3600, owner) as deadline:
            run(safe_path(sys.argv[1]), safe_path(sys.argv[2]), sys.argv[3], owner, deadline)
        return 0
    except (ActionError, OSError, ValueError, TypeError, KeyError):
        print("refresh-build-rejected", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
