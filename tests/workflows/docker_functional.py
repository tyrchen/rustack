"""Explicit Linux/Desktop resource controller with engine-bound command evidence."""
import hashlib
import ipaddress
import json
import os
from pathlib import Path
import re
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from linux_performance import ActionError
import rootless_platform as storage
import rootless_resources as resources
from docker_required_suites import parse_manifest
from functional_inventory import execute
from linux_performance import Runner, bounded_read, deadline_guard, document, require, safe_path
from rootless_functional import SUITES, artifact_seal, environment, observe_clean, retain_source_seal, run_case, source_seal
from image_performance_images import catalog, fenced


def run(config_path, root, owner, deadline):
    require(not os.path.lexists(root), "docker-functional-evidence-exists")
    raw = bounded_read(config_path, 65536)
    config = document(raw)
    require(isinstance(config, dict) and set(config) == {"socket", "otherSocket", "daemonSocket", "socketGid",
        "network", "gateway", "fixtures", "sdk", "target", "buildReceipt"}, "docker-functional-config")
    socket, other, daemon, fixtures, sdk, target = (safe_path(config[key]) for key in
        ("socket", "otherSocket", "daemonSocket", "fixtures", "sdk", "target"))
    require(isinstance(config["socketGid"], str) and re.fullmatch(r"0|[1-9][0-9]{0,9}", config["socketGid"])
            and int(config["socketGid"]) <= 4294967295 and isinstance(config["network"], str)
            and re.fullmatch(r"[A-Za-z0-9_.-]{1,64}", config["network"]), "docker-functional-topology")
    gateway = ipaddress.IPv4Address(config["gateway"])
    require(gateway.is_private and not gateway.is_unspecified and not gateway.is_multicast, "docker-functional-gateway")
    endpoint = "unix://" + str(socket)
    result = owner.run(["docker", "--host", endpoint, "info", "--format", "{{.ID}}"], deadline)
    require(result.code == 0 and result.stderr == b"", "docker-functional-engine")
    engine = result.stdout.decode().strip()
    require(str(socket) != str(other), "docker-functional-distinct-daemons")
    peer = owner.run(["docker", "--host", "unix://" + str(other), "info", "--format", "{{.ID}}"], deadline)
    require(peer.code == 0 and peer.stderr == b"" and peer.stdout.decode().strip() != engine, "docker-functional-peer")
    peer_engine = peer.stdout.decode().strip()
    peer_ids = catalog(owner, "unix://" + str(other), deadline, peer_engine)
    network = document(fenced(owner, endpoint, engine, ["docker", "--host", endpoint, "network", "inspect", config["network"]], deadline))
    require(isinstance(network, list) and len(network) == 1 and network[0].get("Driver") == "bridge"
            and network[0].get("Name") == config["network"], "docker-functional-network")
    root.mkdir(mode=0o700)
    before = source_seal(owner, deadline)
    artifacts = artifact_seal(fixtures, sdk)
    manifest_raw = bounded_read(Path(__file__).with_name("docker-required-suites.json"), 65536)
    required = parse_manifest(manifest_raw)
    require(set(required) == set(SUITES), "docker-functional-manifest")
    selected = environment({"socket":{"path":str(daemon)}, "namespaceSocketGid":int(config["socketGid"])},
                           endpoint, {"name":config["network"]}, fixtures, sdk, other)
    selected["RUSTACK_RUNTIME_GATEWAY_HOST"] = str(gateway)
    owner.environment.update(selected)
    ids = catalog(owner, endpoint, deadline, engine)
    images = {name:resources.captured(owner, endpoint, name, deadline, engine) for name in resources.FULL_SOURCES}
    built_path = safe_path(config["buildReceipt"])
    built = document(bounded_read(built_path, 65536))
    current_seal = hashlib.sha256(json.dumps(before, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    require(built.get("sourceSealSha256") == current_seal and built.get("engine") == engine
            and built.get("allChildGroupsJoined") is True
            and set(built.get("images", {})) == set(resources.REFRESH_SOURCES)
            and all(images[name] == image for name, image in built["images"].items())
            and hashlib.sha256(bounded_read(built_path.parent / "build.log", 16 * 1024 * 1024)).hexdigest() == built.get("logSha256"),
            "docker-functional-stale-production-images")
    source = retain_source_seal(root, before)
    storage.publish(root / "entry.json", {"schemaVersion":1, "engine":engine, **source, "environment":selected,
        "targetDirectory":str(target), "artifacts":artifacts, "images":images,
        "manifestSha256":hashlib.sha256(manifest_raw).hexdigest(), "configSha256":hashlib.sha256(raw).hexdigest()})
    initial = root / "before"; initial.mkdir(mode=0o700)
    observe_clean(owner, endpoint, engine, deadline, initial)
    rows = execute(owner, ["cargo", "test", "--locked", "--target-dir", str(target)], SUITES, required, required,
                   root, deadline, run_case, resources.verify_output,
                   lambda directory: observe_clean(owner, endpoint, engine, deadline, directory))
    require(source_seal(owner, deadline) == before and artifact_seal(fixtures, sdk) == artifacts
            and bounded_read(config_path, 65536) == raw and catalog(owner, endpoint, deadline, engine) == ids
            and all(resources.captured(owner, endpoint, name, deadline, engine) == image for name, image in images.items())
            and catalog(owner, "unix://" + str(other), deadline, peer_engine) == peer_ids,
            "docker-functional-source-changed")
    failed = [row["name"] for row in rows if row["code"] != 0]
    storage.publish(root / "report.json", {"schemaVersion":1, "engine":engine, "attempted":len(rows),
        "passed":len(rows)-len(failed), "failed":failed, "rows":rows, "scope":"complete-functional-inventory",
        "allChildGroupsJoined":not owner.unsettled})
    return 1 if failed else 0


def main():
    try:
        require(len(sys.argv) == 3, "docker-functional-arguments")
        owner = Runner(os.environ)
        with deadline_guard(3600, owner) as deadline:
            return run(safe_path(sys.argv[1]), safe_path(sys.argv[2]), owner, deadline)
    except (ActionError, OSError, ValueError, TypeError, KeyError):
        print("docker-functional-rejected", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
