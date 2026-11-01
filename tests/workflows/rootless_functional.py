"""Complete preloaded rootless resource inventory; any failed case fails the gate."""
import hashlib
import json
import os
from pathlib import Path
import re
import shutil
import stat
import sys
import time
import uuid

sys.path.insert(0, str(Path(__file__).resolve().parent))
import rootless_platform as bootstrap
import rootless_resources as resources
from functional_inventory import execute as execute_inventory
from linux_performance import (MIB, QualificationError, Runner, bounded_read, deadline_guard,
                               document, require, safe_path)
from image_performance_images import catalog, fenced
from docker_required_suites import parse_manifest
from required_suites import parse_inventory
from settings import ActionError

SCRIPT = Path(__file__).resolve()
ROOT = SCRIPT.parents[2]
SUITES = {
    "ordinary-startup": ["-p", "rustack-cli", "--test", "docker_startup", "--no-default-features",
                         "--features", "lambda,stepfunctions,s3,dynamodb"],
    "docker-core": ["-p", "rustack-lambda-core", "--lib"],
    "docker-environment": ["-p", "rustack-lambda-core", "--test", "docker_environment"],
    "docker-executor": ["-p", "rustack-lambda-core", "--test", "docker_executor"],
    "docker-endpoint": ["-p", "rustack-lambda-core", "--test", "docker_endpoint"],
    "docker-probe": ["-p", "rustack-lambda-core", "--test", "docker_probe"],
    "docker-wire": ["-p", "rustack-lambda-core", "--test", "docker_wire"],
    "relay-runtime": ["-p", "rustack-lambda-runtime", "--test", "docker"],
}
SERVER_CASES = frozenset((
    "test_should_execute_image_sdk_and_nested_asl_from_a_server_container",
    "test_should_execute_image_sdk_and_nested_asl_from_the_release_image",
    "test_should_refuse_unreachable_server_container_gateway_and_join_startup_cleanup",
    "test_should_refuse_unreachable_release_image_gateway_and_join_startup_cleanup",
))


def forward_remaining(raw):
    text = raw.decode("ascii").strip()
    require(re.fullmatch(r"(?:[0-9]{1,2}:)?[0-9]{2}:[0-9]{2}", text) is not None,
            "functional-forward-age")
    parts = [int(part) for part in text.split(":")]
    require(parts[-1] < 60 and parts[-2] < 60, "functional-forward-age")
    age = parts[-1] + parts[-2] * 60 + (parts[0] * 3600 if len(parts) == 3 else 0)
    # Kernel elapsed age starts at process creation, before its first deadline.
    # Three seconds cover ps's whole-second truncation and observation transit.
    remaining = 3600 - age - 3
    require(remaining > 1200, "functional-forward-lifetime")
    return remaining


def source_paths(owner, deadline):
    cutoff = min(deadline, time.monotonic() + 30)
    result = set()
    roots = ("apps", "crates", "tests", "tools", "patches", "assets", ".github", ".cargo", "examples", "codegen/src", "codegen/services/stepfunctions.toml",
             "codegen/smithy-model/stepfunctions.json", "codegen/smithy-model/stepfunctions.provenance.json")
    for directory in roots:
        reply = owner.run(["git", "-C", str(ROOT), "ls-files", "--cached", "--others",
                           "--exclude-standard", "-z", "--", directory],
                          min(cutoff, time.monotonic() + 10), stdout_limit=65536)
        require(reply.code == 0 and reply.stderr == b"", "functional-source-inventory")
        for raw in reply.stdout.split(b"\0"):
            if not raw:
                continue
            name = raw.decode("utf-8")
            require(re.fullmatch(r"[A-Za-z0-9_@./+-]{1,512}", name) is not None
                    and all(part not in ("", ".", "..") for part in name.split("/")), "functional-source-path")
            result.add(ROOT / name)
            require(len(result) <= 4096, "functional-source-count")
    return result


def source_seal(owner, deadline):
    paths = set()
    suffixes = {".rs", ".py", ".js", ".json", ".yaml", ".yml", ".toml", ".lock", ".txt", ".Dockerfile", ".sh"}
    for path in source_paths(owner, deadline):
        if path.suffix in suffixes or path.name == "Dockerfile":
            paths.add(path)
    paths.update(ROOT / name for name in ("Makefile", "Dockerfile", "Cargo.toml", "Cargo.lock", "rust-toolchain.toml", "action.yml", "rustfmt.toml", ".rustfmt.toml", "deny.toml", ".dockerignore"))
    return {str(path.relative_to(ROOT)): hashlib.sha256(bounded_read(path, 4 * MIB)).hexdigest()
            for path in sorted(paths) if path.is_file()}


def artifact_seal(fixtures, sdk):
    selected = {"provided-a": fixtures / "provided/provided-a.zip", "provided-b": fixtures / "provided/provided-b.zip",
                "code-bootstrap": fixtures / "zip-code-bootstrap/bootstrap",
                "native-verifier": fixtures / "native-verifier/rustack-lambda-runtime-verifier/bootstrap",
                "sdk-interpreter": sdk}
    result = {}
    for role, path in selected.items():
        resolved = path.resolve(strict=True)
        metadata = os.lstat(resolved)
        require(stat.S_ISREG(metadata.st_mode) and 1 <= metadata.st_size <= 256 * MIB,
                "functional-artifact-bound")
        raw = bounded_read(resolved, 256 * MIB)
        result[role] = {"path": str(path), "resolved": str(resolved), "bytes": len(raw),
                        "mode": stat.S_IMODE(metadata.st_mode), "sha256": hashlib.sha256(raw).hexdigest()}
    return result


def retain_source_seal(root, values):
    require(isinstance(values, dict) and 1 <= len(values) <= 4096, "functional-source-seal-count")
    ordered = sorted(values.items())
    files = []
    for offset in range(0, len(ordered), 128):
        name = "source-%03d.json" % (offset // 128)
        bootstrap.publish(root / name, dict(ordered[offset:offset + 128]))
        files.append(name)
    encoded = json.dumps(values, sort_keys=True, separators=(",", ":")).encode()
    require(len(encoded) <= MIB, "functional-source-seal-bound")
    return {"sourceSealFiles": files, "sourceSealSha256": hashlib.sha256(encoded).hexdigest()}


def observation_budget(started, remaining, observed):
    require(isinstance(started, (int, float)) and isinstance(observed, (int, float))
            and observed >= started, "functional-forward-clock")
    seconds = min(2700, remaining) - (observed - started)
    require(seconds > 1200, "functional-forward-observation-expired")
    return seconds


def environment(proof, endpoint, bridge, fixtures, sdk, other):
    docker = shutil.which("docker")
    require(docker is not None and sdk.is_file(), "functional-tools")
    for part in ("provided/provided-a.zip", "provided/provided-b.zip", "zip-code-bootstrap/bootstrap",
                 "native-verifier/rustack-lambda-runtime-verifier/bootstrap"):
        require((fixtures / part).is_file(), "functional-fixture-artifact")
    return {
        "RUSTACK_RUNTIME_DOCKER_SOCKET": endpoint[7:], "RUSTACK_OTHER_DOCKER_SOCKET": str(other),
        "RUSTACK_RUNTIME_ARCH": "arm64", "RUSTACK_RUNTIME_IMAGE": resources.SOURCES[0],
        "RUSTACK_RUNTIME_NETWORK": bridge["name"], "RUSTACK_RUNTIME_GATEWAY_HOST": "192.168.5.2",
        "RUSTACK_PROVIDED_ZIP_A": str(fixtures / "provided/provided-a.zip"),
        "RUSTACK_PROVIDED_ZIP_B": str(fixtures / "provided/provided-b.zip"),
        "RUSTACK_ZIP_NATIVE_VERIFIER": str(fixtures / "native-verifier/rustack-lambda-runtime-verifier/bootstrap"),
        "RUSTACK_ZIP_CODE_BOOTSTRAP": str(fixtures / "zip-code-bootstrap/bootstrap"),
        "RUSTACK_ZIP_BASE_IMAGE": resources.FULL_SOURCES[7], "RUSTACK_PULL_IMAGE": resources.FULL_SOURCES[8],
        "RUSTACK_CUSTOM_IMAGE_A": "rustack-custom-ric:a-arm64", "RUSTACK_CUSTOM_IMAGE_B": "rustack-custom-ric:b-arm64",
        "RUSTACK_NODE_IMAGE": "rustack-official-node:arm64", "RUSTACK_HANDLER_SDK_IMAGE": "rustack-handler-sdk:arm64",
        "RUSTACK_CALLBACK_IMAGE": "rustack-callback:arm64", "RUSTACK_PULUMI_IMAGE": "rustack-pulumi:arm64",
        "RUSTACK_SERVER_IMAGE": "rustack/workflows-server:integration-arm64",
        "RUSTACK_RELEASE_IMAGE": "rustack/workflows-release:integration-arm64",
        "RUSTACK_SERVER_DAEMON_SOCKET": proof["socket"]["path"],
        "RUSTACK_SERVER_SOCKET_GID": str(proof["namespaceSocketGid"]),
        "RUSTACK_DOCKER_BIN": docker, "RUSTACK_WORKFLOWS_PYTHON": str(sdk),
    }


def observe_clean(owner, endpoint, engine, deadline, evidence):
    for kind, tail in (("containers", ["ps", "--all", "--quiet"]), ("volumes", ["volume", "ls", "--quiet"])):
        raw = fenced(owner, endpoint, engine, ["docker", "--host", endpoint] + tail, deadline)
        bootstrap.write_new(evidence / (kind + ".stdout"), raw)
        require(raw == b"", "functional-resource-remnant")
    raw = fenced(owner, endpoint, engine, ["docker", "--host", endpoint, "image", "ls", "--all", "--format", "{{.Repository}}:{{.Tag}}"], deadline)
    bootstrap.write_new(evidence / "aliases.stdout", raw)
    resources.private_aliases(raw)
    require(not any(row.startswith("public.ecr.aws:443/lambda/nodejs") for row in raw.decode().splitlines()),
            "functional-pull-alias-remnant")


def run_case(owner, arguments, cutoff, directory):
    bootstrap.publish(directory / "intent.json", {"argv": arguments, "callerCutoffMonotonic": cutoff,
                      "ownerPid": os.getpid()})
    result = owner.run(arguments, cutoff, stdout_limit=65536)
    bootstrap.write_new(directory / "stdout", result.stdout)
    bootstrap.write_new(directory / "stderr", result.stderr)
    bootstrap.publish(directory / "result.json", {"code": result.code,
                      "stdoutSha256": hashlib.sha256(result.stdout).hexdigest(),
                      "stderrSha256": hashlib.sha256(result.stderr).hexdigest()})
    return result


def execute(proof, control, preparation, target, fixtures, sdk, other, owner, deadline, server_focus=False):
    endpoint, engine = resources.connection(str(control), proof, owner, deadline)
    image_raw = bounded_read(preparation / "images.json", 65536)
    images = resources.image_receipt(document(image_raw), engine)
    require(images["selection"] == "full" and images["sourceSocket"] == str(other), "functional-full-fixtures")
    bridge = document(bounded_read(preparation / "network.json", 65536))
    require(bridge["engine"] == engine, "functional-network-engine")
    actual = resources.network(document(fenced(owner, endpoint, engine, ["docker", "--host", endpoint,
        "network", "inspect", bridge["id"]], deadline)), bridge["name"], bridge["nonce"])
    require(actual["Id"] == bridge["id"], "functional-network-id")
    entry = document(bounded_read(preparation / "entry.json", 65536))
    resources.require_producer(preparation, entry)
    manifest_raw = bounded_read(SCRIPT.with_name("docker-required-suites.json"), 65536)
    required = parse_manifest(manifest_raw)
    require(set(required) == set(SUITES) and all(required.values()),
            "functional-complete-inventory")
    root = preparation / ("functional-" + uuid.uuid4().hex)
    root.mkdir(mode=0o700)
    before = source_seal(owner, deadline)
    build_raw = bounded_read(preparation / "build-receipt.json", 65536)
    built = document(build_raw)
    current_seal = hashlib.sha256(json.dumps(before, sort_keys=True, separators=(",", ":")).encode()).hexdigest()
    require(built.get("sourceSealSha256") == current_seal and built.get("allChildGroupsJoined") is True,
            "functional-stale-production-images")
    artifacts = artifact_seal(fixtures, sdk)
    selected_environment = environment(proof, endpoint, bridge, fixtures, sdk, other)
    owner.environment.update(selected_environment)
    source_proof = retain_source_seal(root, before)
    bootstrap.publish(root / "entry.json", {"schemaVersion":1, "engine": engine, "profile": proof["profile"], "ownerPid": os.getpid(),
        **source_proof, "artifacts": artifacts, "manifestSha256": hashlib.sha256(manifest_raw).hexdigest(),
        "imageReceiptSha256": hashlib.sha256(image_raw).hexdigest(), "controllerSha256": resources.sha(SCRIPT),
        "resourceControllerSha256": resources.sha(resources.SCRIPT), "environment": selected_environment, "targetDirectory":str(target)})
    bootstrap.write_new(root / "controller.py", bounded_read(SCRIPT, MIB))
    bootstrap.write_new(root / "resource-controller.py", bounded_read(resources.SCRIPT, MIB))
    baseline = root / "before"
    baseline.mkdir(mode=0o700)
    observe_clean(owner, endpoint, engine, deadline, baseline)
    cargo = ["cargo", "test", "--locked", "--target-dir", str(target)]
    selected = {"ordinary-startup": SERVER_CASES} if server_focus else required
    require(SERVER_CASES <= required["ordinary-startup"], "functional-server-case-inventory")
    expected_count = sum(len(names) for names in selected.values())
    rows = execute_inventory(owner, cargo, SUITES, required, selected, root, deadline, run_case,
        resources.verify_output, lambda directory: observe_clean(owner, endpoint, engine, deadline, directory))
    require(len(rows) == expected_count and source_seal(owner, deadline) == before and artifact_seal(fixtures, sdk) == artifacts
            and bounded_read(SCRIPT.with_name("docker-required-suites.json"), 65536) == manifest_raw,
            "functional-source-or-inventory-changed")
    require(catalog(owner, "unix://" + str(other), deadline, images["sourceEngine"]) == set(images["sourceIds"]),
            "functional-original-source-ids-changed")
    for row in images["images"]:
        require(resources.captured(owner, endpoint, row["name"], deadline, engine) == row["image"]
                and resources.captured(owner, endpoint, row["cacheAlias"], deadline, engine, row["name"]) == row["image"]
                and resources.captured(owner, "unix://" + str(other), row["name"], deadline, images["sourceEngine"]) == row["image"],
                "functional-source-image-changed")
    failed = [row["name"] for row in rows if row["code"] != 0]
    report = {"schemaVersion": 1, "engine": engine, "attempted": len(rows), "passed": expected_count - len(failed),
              "scope": "server-focused-only" if server_focus else "complete-functional-inventory",
              "failed": failed, "platformQualification": "failed" if failed else "pending",
              "rows": rows, "evidence": str(root), "allChildGroupsJoined": not owner.unsettled}
    bootstrap.publish(root / "report.json", report)
    print(json.dumps(report, separators=(",", ":")), flush=True)
    return 1 if failed else 0


def main():
    try:
        server_focus = len(sys.argv) == 9 and sys.argv[1] == "--server-focus"
        arguments = sys.argv[2:] if server_focus else sys.argv[1:]
        require(len(arguments) == 7, "functional-arguments")
        proof = resources.attestation(arguments[0])
        control, preparation, target, fixtures, sdk, other = (safe_path(value) for value in arguments[1:])
        owner = Runner(os.environ)
        forward_entry = document(bounded_read(control / "entry.json", 65536))
        pid = forward_entry.get("ownerPid")
        require(type(pid) is int and 1 <= pid <= 999999999, "functional-forward-pid")
        started = time.monotonic()
        resources.require_process_identity(forward_entry.get("ownerIdentity"),
            resources.process_identity(pid, owner, started + 10))
        elapsed = owner.run(["ps", "-p", str(pid), "-o", "etime="], started + 10)
        require(elapsed.code == 0 and elapsed.stderr == b"", "functional-forward-observation")
        remaining = forward_remaining(elapsed.stdout)
        seconds = observation_budget(started, remaining, time.monotonic())
        with deadline_guard(seconds, owner) as deadline:
            return execute(proof, control, preparation, target, fixtures, sdk, other, owner, deadline, server_focus=server_focus)
    except (ActionError, QualificationError) as error:
        print("rootless-functional-failed:" + str(error), file=sys.stderr)
        return 2
    except (OSError, ValueError, KeyError, TypeError, RecursionError):
        print("rootless-functional-failed", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
