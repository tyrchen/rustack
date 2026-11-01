"""Observe an explicit daemon after resource qualification; no context mutation."""
import hashlib
import json
import os
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from linux_performance import ActionError
import rootless_platform as storage
from linux_performance import Runner, deadline_guard, document, require, safe_path
from image_performance_images import fenced
from rootless_functional import source_seal
from rootless_resources import private_aliases

FORMAT = ('{"engineId":{{json .ID}},"os":{{json .OSType}},"architecture":{{json .Architecture}},'
          '"operatingSystem":{{json .OperatingSystem}},"securityOptions":{{json .SecurityOptions}}}')


def run(socket, output, owner, deadline):
    endpoint = "unix://" + str(socket)
    before = source_seal(owner, deadline)
    reply = owner.run(["docker", "--host", endpoint, "info", "--format", FORMAT], deadline)
    require(reply.code == 0 and reply.stderr == b"", "platform-info")
    value = document(reply.stdout)
    require(set(value) == {"engineId", "os", "architecture", "operatingSystem", "securityOptions"}
            and value["os"] == "linux" and value["architecture"] in ("arm64", "aarch64")
            and isinstance(value["securityOptions"], list) and len(value["securityOptions"]) <= 32
            and all(isinstance(row, str) and len(row) <= 256 for row in value["securityOptions"])
            and isinstance(value["operatingSystem"], str) and len(value["operatingSystem"]) <= 256, "platform-info-shape")
    for tail in (["ps", "--all", "--quiet"], ["volume", "ls", "--quiet"]):
        require(fenced(owner, endpoint, value["engineId"], ["docker", "--host", endpoint] + tail, deadline) == b"", "platform-resource-remnant")
    raw = fenced(owner, endpoint, value["engineId"], ["docker", "--host", endpoint, "image", "ls", "--all", "--format", "{{.Repository}}:{{.Tag}}"], deadline)
    private_aliases(raw)
    require(source_seal(owner, deadline) == before, "platform-source-changed")
    storage.publish(output, {"schemaVersion":1, "sourceSealSha256":hashlib.sha256(json.dumps(before,
        sort_keys=True, separators=(",", ":")).encode()).hexdigest(), "daemon":value, "endpoint":endpoint,
        "containersAbsent":True, "volumesAbsent":True, "privateAliasesAbsent":True,
        "allChildGroupsJoined":not owner.unsettled})


def main():
    try:
        require(len(sys.argv) == 3, "platform-arguments")
        owner = Runner(os.environ)
        with deadline_guard(90, owner) as deadline:
            run(safe_path(sys.argv[1]), safe_path(sys.argv[2]), owner, deadline)
        return 0
    except (ActionError, OSError, ValueError, TypeError, KeyError):
        print("platform-receipt-rejected", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
