"""Fenced rootless fixture transport and unchanged actual Rust kernel resources."""
import hashlib
import json
import os
from pathlib import Path
import re
import stat
import sys
import time
import uuid

sys.path.insert(0, str(Path(__file__).resolve().parent))
import rootless_platform as bootstrap
from linux_performance import (MIB, QualificationError, Runner, bounded_read, deadline_guard,
                               document, profile, require, safe_path, status, summary)
from image_performance_images import (LIMIT, cache_alias, catalog, fenced, hash_file, image, retain_content)
from required_suites import parse_inventory
from docker_required_suites import parse_manifest
from settings import ActionError
from metadata import engine_id, source_id

SCRIPT = Path(__file__).resolve()
SOURCES = ("rustack/lambda-runtime:integration-arm64", "rustack-p0-handler:arm64")
FULL_SOURCES = SOURCES + (
    "rustack-custom-ric:a-arm64", "rustack-custom-ric:b-arm64", "rustack-no-runtime:arm64",
    "rustack-invalid-os:arm64", "rustack-invalid-architecture:arm64",
    "public.ecr.aws/lambda/provided@sha256:612c98f126c2a95e77a016d56fed50a83055e11d636132ee6ebdd461f01b9617",
    "public.ecr.aws/lambda/nodejs@sha256:92b7cccd985332238af80ac8da42d6965d418552e88955e315da4e3c0c76ca79",
    "rustack-official-node:arm64", "rustack-handler-sdk:arm64", "rustack-callback:arm64",
    "rustack-pulumi:arm64", "rustack/workflows-server:integration-arm64", "rustack/workflows-release:integration-arm64")
REFRESH_SOURCES = (SOURCES[0], FULL_SOURCES[2], FULL_SOURCES[3], FULL_SOURCES[13], FULL_SOURCES[14])
LABEL = "rustack.workflows.rootless"


def sha(path):
    return hashlib.sha256(bounded_read(path, MIB)).hexdigest()


def attestation(path):
    path = safe_path(path)
    metadata = os.lstat(path)
    require(stat.S_ISDIR(metadata.st_mode) and metadata.st_uid == os.getuid()
            and stat.S_IMODE(metadata.st_mode) == 0o700, "resource-attestation-directory")
    value = document(bounded_read(path / "result.json", 65536))
    require(isinstance(value, dict) and value.get("profileRetained") is True,
            "resource-attestation")
    base = {key: item for key, item in value.items()
            if key not in ("profile", "profileConfiguration", "profileRetained")}
    bootstrap.result_proof(base, base.get("nonce"))
    profile(value["profile"])
    return value


def private(path):
    metadata = os.lstat(path)
    require(stat.S_ISDIR(metadata.st_mode) and metadata.st_uid == os.getuid()
            and stat.S_IMODE(metadata.st_mode) == 0o700, "resource-private-directory")


def socket_metadata(path):
    require(len(os.fsencode(path)) <= 100, "resource-socket-path-bound")
    private(path.parent)
    metadata = os.lstat(path)
    require(stat.S_ISSOCK(metadata.st_mode) and metadata.st_uid == os.getuid()
            and stat.S_IMODE(metadata.st_mode) & 0o007 == 0, "resource-forward-socket")
    return {"path": str(path), "inode": metadata.st_ino, "uid": metadata.st_uid}


def process_identity(pid, owner, deadline):
    require(type(pid) is int and 1 <= pid <= 999999999, "resource-forward-pid")
    raw = owner.run(["ps", "-p", str(pid), "-o", "lstart=", "-o", "uid="],
                    min(deadline, time.monotonic() + 10))
    require(raw.code == 0 and raw.stderr == b"", "resource-forward-process-observation")
    parts = raw.stdout.decode("ascii").split()
    require(len(parts) == 6 and re.fullmatch(r"[0-9]{1,10}", parts[-1])
            and re.fullmatch(r"[A-Za-z]{3} [A-Za-z]{3} [0-9]{1,2} [0-9]{2}:[0-9]{2}:[0-9]{2} [0-9]{4}",
                             " ".join(parts[:-1])), "resource-forward-process-identity")
    return {"start": " ".join(parts[:-1]), "uid": int(parts[-1])}


def require_process_identity(expected, actual):
    require(isinstance(expected, dict) and set(expected) == {"start", "uid"}
            and type(expected["uid"]) is int and expected["uid"] == os.getuid()
            and expected == actual, "resource-forward-process-reused")


def connection(control, proof, owner, deadline):
    control = safe_path(control)
    private(control)
    entry = document(bounded_read(control / "entry.json", 65536))
    require(isinstance(entry, dict) and set(entry) == {"profile", "engine", "ownerPid", "ownerIdentity", "nonce", "scriptSha256", "socket"}
            and entry["profile"] == proof["profile"] and entry["engine"] == proof["rootless"]["id"]
            and entry["scriptSha256"] == sha(SCRIPT) and type(entry["ownerPid"]) is int
            and entry["ownerPid"] > 0 and not os.path.lexists(control / "final.json"), "resource-forward-owner")
    bootstrap.token(entry["nonce"])
    require_process_identity(entry["ownerIdentity"], process_identity(entry["ownerPid"], owner, deadline))
    socket = control / "docker.sock"
    require(entry["socket"] == str(socket), "resource-forward-path")
    observed = socket_metadata(socket)
    record = control / "socket-observed.json"
    if not record.exists():
        bootstrap.publish(record, observed)
    else:
        require(document(bounded_read(record, 65536)) == observed, "resource-forward-socket-changed")
    endpoint = "unix://" + str(socket)
    engine = proof["rootless"]["id"]
    fenced(owner, endpoint, engine, ["docker", "--host", endpoint, "info", "--format", "{{.ID}}"], deadline)
    return endpoint, engine


def forward(proof, directory, owner, deadline):
    directory = safe_path(directory)
    require(directory.parent == Path("/tmp") and len(os.fsencode(directory / "docker.sock")) <= 100,
            "resource-forward-directory-policy")
    require(not os.path.lexists(directory), "resource-forward-directory-exists")
    directory.mkdir(mode=0o700)
    nonce = uuid.uuid4().hex
    socket = directory / "docker.sock"
    bootstrap.publish(directory / "entry.json", {"profile": proof["profile"], "engine": proof["rootless"]["id"],
        "ownerPid": os.getpid(), "ownerIdentity": process_identity(os.getpid(), owner, deadline),
        "nonce": nonce, "scriptSha256": sha(SCRIPT), "socket": str(socket)})
    config = Path.home() / ".colima/_lima" / ("colima-" + proof["profile"]) / "ssh.config"
    bounded_read(config, 65536)
    arguments = ["ssh", "-F", str(config), "-o", "ControlMaster=no", "-o", "ControlPath=none",
                 "-o", "ControlPersist=no", "-o", "ExitOnForwardFailure=yes", "-o", "ServerAliveInterval=5",
                 "-o", "ServerAliveCountMax=2", "-o", "LogLevel=ERROR", "-N", "-T", "-n",
                 "-L", str(socket) + ":" + proof["socket"]["path"], "lima-colima-" + proof["profile"]]
    bootstrap.publish(directory / "intent.json", {"argv": arguments, "cutoffMonotonic": deadline})
    try:
        reply = owner.run(arguments, deadline)
        require(reply.code == 0 and not reply.stderr, "resource-forward-ssh-exit")
    finally:
        removed = not os.path.lexists(socket)
        if not owner.unsettled and not removed and (directory / "socket-observed.json").exists():
            require(document(bounded_read(directory / "socket-observed.json", 65536)) == socket_metadata(socket),
                    "resource-forward-cleanup-substitution")
            socket.unlink()
            removed = True
        bootstrap.publish(directory / "final.json", {"ownerPid": os.getpid(), "nonce": nonce,
            "childGroupJoined": not owner.unsettled, "socketRemoved": removed})


def network(value, expected_name, nonce):
    require(isinstance(value, list) and len(value) == 1 and isinstance(value[0], dict), "resource-network-shape")
    row = value[0]
    require(row.get("Name") == expected_name and isinstance(row.get("Id"), str)
            and re.fullmatch(r"[0-9a-f]{64}", row["Id"]) and row.get("Driver") == "bridge"
            and row.get("Scope") == "local" and row.get("Labels") == {LABEL: nonce}
            and row.get("Containers") == {} and row.get("Internal") is False
            and isinstance(row.get("IPAM"), dict) and row["IPAM"].get("Driver") == "default"
            and isinstance(row["IPAM"].get("Config"), list) and len(row["IPAM"]["Config"]) == 1,
            "resource-network-ownership")
    return row


def captured(owner, endpoint, name, deadline, engine, expected_name=None):
    negative = {"rustack-invalid-os:arm64": ("windows", "amd64"),
                "rustack-invalid-architecture:arm64": ("linux", "amd64")}
    selected = expected_name if expected_name is not None else name
    if selected not in negative:
        return image(owner, endpoint, name, deadline, engine)
    raw = fenced(owner, endpoint, engine, ["docker", "--host", endpoint, "image", "inspect", name, "--format",
        '{"id":{{json .Id}},"os":{{json .Os}},"architecture":{{json .Architecture}},"bytes":{{json .Size}}}'], deadline)
    value = document(raw)
    require(isinstance(value, dict) and set(value) == {"id", "os", "architecture", "bytes"}
            and (value["os"], value["architecture"]) == negative[selected]
            and type(value["bytes"]) is int and 1 <= value["bytes"] <= 65536, "resource-negative-image")
    source_id(value["id"])
    return value


def tag_policy(present, immutable, old=None):
    source_id(immutable)
    if old is not None:
        source_id(old)
    permitted = (b"", (immutable + "\n").encode()) if old is None else ((old + "\n").encode(), (immutable + "\n").encode())
    require(present in permitted, "resource-target-tag-conflict")
    return present != (immutable + "\n").encode()


def prepare(proof, source_socket, control, directory, owner, deadline, full=False, previous=None, build=None):
    directory, source_socket = safe_path(directory), safe_path(source_socket)
    require(not os.path.lexists(directory), "resource-evidence-exists")
    directory.mkdir(mode=0o700)
    bootstrap.write_new(directory / "producer.py", bounded_read(SCRIPT, MIB))
    endpoint, engine = connection(control, proof, owner, deadline)
    source = "unix://" + str(source_socket)
    commands = bootstrap.Commands(owner, deadline, directory)
    source_engine = commands.run(["docker", "--host", source, "info", "--format", "{{.ID}}"], clean=True).decode().strip()
    require(source != endpoint and source_engine != engine, "resource-distinct-daemons")
    before_source = catalog(owner, source, deadline, source_engine)
    before_target = catalog(owner, endpoint, deadline, engine)
    selected = FULL_SOURCES if full else SOURCES
    budget = (4 if full else 2) * LIMIT
    expected = {name: captured(owner, source, name, deadline, source_engine) for name in selected}
    bootstrap.publish(directory / "entry.json", {"profile": proof["profile"], "engine": engine,
        "sourceEngine": source_engine, "sources": expected, "sourceIds": sorted(before_source),
        "targetIds": sorted(before_target), "scriptSha256": sha(SCRIPT)})
    prior = None
    if previous is not None:
        previous, build = safe_path(previous), safe_path(build)
        private(previous)
        private(build)
        prior_raw = bounded_read(previous / "images.json", 65536)
        prior = image_receipt(document(prior_raw), engine)
        require_producer(previous, document(bounded_read(previous / "entry.json", 65536)))
        require(full and prior["selection"] == "full" and prior["sourceEngine"] == source_engine
                and prior["sourceSocket"] == str(source_socket) and set(prior["sourceIds"]) <= before_source,
                "resource-refresh-affiliation")
        build_raw = bounded_read(build / "report.json", 65536)
        built = document(build_raw)
        from rootless_functional import source_seal
        current = hashlib.sha256(json.dumps(source_seal(owner, deadline), sort_keys=True, separators=(",", ":")).encode()).hexdigest()
        require(built.get("sourceSealSha256") == current, "resource-refresh-stale-build")
        allowed = REFRESH_SOURCES
        require(built.get("schemaVersion") == 1 and built.get("engine") == source_engine
                and built.get("sourceSocket") == str(source_socket) and built.get("allChildGroupsJoined") is True
                and set(built.get("images", {})) == set(allowed)
                and all(built["images"][name] == expected[name] for name in allowed), "resource-refresh-build")
        require(hashlib.sha256(bounded_read(build / "build.log", 16 * MIB)).hexdigest() == built.get("logSha256"), "resource-refresh-build-log")
        for row in prior["images"]:
            name = row["name"]
            require(captured(owner, endpoint, name, deadline, engine) == row["image"]
                    and captured(owner, endpoint, row["cacheAlias"], deadline, engine, name) == row["image"],
                    "resource-refresh-old-anchor")
            require(name in allowed or expected[name] == row["image"], "resource-refresh-nonallowlisted-image")
        bootstrap.write_new(directory / "build-receipt.json", build_raw)
        bootstrap.publish(directory / "refresh.json", {"previous": str(previous), "previousReceiptSha256":
            hashlib.sha256(prior_raw).hexdigest(), "buildReceiptSha256": hashlib.sha256(build_raw).hexdigest()})
    rows, total = [], 0
    for index, name in enumerate(selected):
        archive = directory / ("image-%d.tar" % index)
        bootstrap.write_new(archive, b"")
        immutable = expected[name]["id"]
        arguments = [sys.executable, "-I", "-B", str(SCRIPT.with_name("image_performance_images.py")),
                     "save", str(source_socket), str(archive), immutable, str(min(2 * LIMIT, budget - total)), "v1"]
        bootstrap.publish(directory / ("image-%d.intent.json" % index), {"argv": arguments})
        fenced(owner, source, source_engine, arguments, deadline, 120)
        size, digest = hash_file(archive, deadline)
        total += size
        require(total <= budget, "resource-archive-total-budget")
        fenced(owner, endpoint, engine, ["docker", "--host", endpoint, "image", "load", "--input", str(archive)], deadline, 120)
        require(captured(owner, endpoint, immutable, deadline, engine, name) == expected[name], "resource-import-differs")
        cache = retain_content(owner, endpoint, immutable, deadline, engine)
        require(captured(owner, endpoint, cache, deadline, engine, name) == expected[name], "resource-cache-differs")
        if "@sha256:" in name:
            pull = ["docker", "--host", endpoint, "image", "pull", "--platform", "linux/arm64", name]
            bootstrap.publish(directory / ("image-%d-digest.intent.json" % index), {"argv": pull, "expectedId": immutable})
            fenced(owner, endpoint, engine, pull, deadline, 120)
            require(image(owner, endpoint, name, deadline, engine) == expected[name], "resource-import-digest-differs")
            require(hash_file(archive, deadline) == (size, digest), "resource-import-changed")
            rows.append({"name": name, "image": expected[name], "archive": str(archive), "bytes": size,
                         "sha256": digest, "cacheAlias": cache})
            continue
        present = fenced(owner, endpoint, engine, ["docker", "--host", endpoint, "image", "ls", "--all", "--no-trunc",
            "--filter", "reference=" + name, "--format", "{{.ID}}"], deadline)
        old = next((row["image"]["id"] for row in prior["images"] if row["name"] == name), None) if prior else None
        if tag_policy(present, immutable, old):
            bootstrap.publish(directory / ("image-%d-tag.intent.json" % index),
                {"name": name, "oldId": old, "newId": immutable, "engine": engine})
            fenced(owner, endpoint, engine, ["docker", "--host", endpoint, "image", "tag", immutable, name], deadline)
        require(captured(owner, endpoint, name, deadline, engine) == expected[name]
                and hash_file(archive, deadline) == (size, digest), "resource-import-changed")
        rows.append({"name": name, "image": expected[name], "archive": str(archive), "bytes": size,
                     "sha256": digest, "cacheAlias": cache})
    require(catalog(owner, source, deadline, source_engine) == before_source
            and all(captured(owner, source, name, deadline, source_engine) == expected[name] for name in selected)
            and catalog(owner, endpoint, deadline, engine) == before_target | {row["image"]["id"] for row in rows},
            "resource-source-or-target-changed")
    bootstrap.publish(directory / "images.json", {"engine": engine, "sourceEngine": source_engine,
        "sourceSocket": str(source_socket), "sourceIds": sorted(before_source), "images": rows,
        "selection": "full" if full else "kernel"})
    nonce = uuid.uuid4().hex
    name = "rustack-rootless-d104-" + nonce[:12]
    bootstrap.publish(directory / "network-intent.json", {"engine": engine, "name": name, "nonce": nonce})
    identity = fenced(owner, endpoint, engine, ["docker", "--host", endpoint, "network", "create", "--driver", "bridge",
        "--label", LABEL + "=" + nonce, name], deadline).decode().strip()
    row = network(document(fenced(owner, endpoint, engine, ["docker", "--host", endpoint, "network", "inspect", identity], deadline)), name, nonce)
    require(row["Id"] == identity, "resource-network-id")
    bootstrap.publish(directory / "network.json", {"engine": engine, "name": name, "nonce": nonce, "id": identity,
        "inspection": row})
    return {"images": len(rows), "archiveBytes": total, "engine": engine, "network": identity}


def verify_output(raw, expected):
    summary(raw, len(expected), 0)
    text = raw.decode()
    require(not re.search(r"^test result: FAILED|^test .* \.\.\. FAILED$", text, re.MULTILINE), "resource-failed-status")
    rows = re.findall(r"^test ([A-Za-z0-9_:]+) \.\.\. (?!ignored\b)", text, re.MULTILINE)
    require(len(rows) == len(expected) and set(rows) == set(expected), "resource-exact-success-rows")


def private_aliases(raw):
    require(isinstance(raw, bytes) and len(raw) <= 32768, "resource-alias-inventory-bound")
    rows = raw.decode("ascii").splitlines()
    require(len(rows) <= 256 and all(0 < len(row) <= 1024 for row in rows), "resource-alias-inventory-shape")
    for row in rows:
        familiar = row
        for prefix in ("docker.io/", "index.docker.io/"):
            if familiar.startswith(prefix):
                familiar = familiar[len(prefix):]
                break
        if familiar.startswith("library/"):
            familiar = familiar[len("library/"):]
        require(not familiar.startswith(("rustack-internal/lambda-", "rustack-test-source")),
                "resource-private-alias-remnant")


def require_producer(preparation, entry):
    require(isinstance(entry, dict) and isinstance(entry.get("scriptSha256"), str)
            and re.fullmatch(r"[0-9a-f]{64}", entry["scriptSha256"]), "resource-producer-record")
    require(sha(preparation / "producer.py") == entry["scriptSha256"], "resource-producer-source")


def image_receipt(value, engine):
    require(isinstance(value, dict) and set(value) == {"engine", "sourceEngine", "sourceSocket", "sourceIds", "images", "selection"}
            and value["engine"] == engine and value["sourceEngine"] != engine, "resource-image-receipt")
    require(value["selection"] in ("kernel", "full"), "resource-image-selection")
    engine_id(value["sourceEngine"])
    safe_path(value["sourceSocket"])
    require(isinstance(value["sourceIds"], list) and len(value["sourceIds"]) <= 256
            and len(set(value["sourceIds"])) == len(value["sourceIds"]), "resource-source-id-list")
    for identity in value["sourceIds"]:
        source_id(identity)
    rows = value["images"]
    selected = FULL_SOURCES if value["selection"] == "full" else SOURCES
    require(isinstance(rows, list) and len(rows) == len(selected), "resource-image-count")
    total = 0
    for name, row in zip(selected, rows):
        require(isinstance(row, dict) and set(row) == {"name", "image", "archive", "bytes", "sha256", "cacheAlias"}
                and row["name"] == name and isinstance(row["image"], dict)
                and set(row["image"]) == {"id", "os", "architecture", "bytes"}, "resource-image-row")
        captured = row["image"]
        source_id(captured["id"])
        platform = ("windows", "amd64") if name == "rustack-invalid-os:arm64" else ("linux", "amd64") if name == "rustack-invalid-architecture:arm64" else ("linux", "arm64")
        require((captured["os"], captured["architecture"]) == platform
                and type(captured["bytes"]) is int and 1 <= captured["bytes"] <= LIMIT
                and row["cacheAlias"] == cache_alias(captured["id"])
                and type(row["bytes"]) is int and 1 <= row["bytes"] <= LIMIT
                and isinstance(row["sha256"], str) and re.fullmatch(r"[0-9a-f]{64}", row["sha256"]),
                "resource-image-policy")
        safe_path(row["archive"])
        total += row["bytes"]
    require(total <= (4 if value["selection"] == "full" else 2) * LIMIT, "resource-archive-total-budget")
    return value


def kernel(proof, control, directory, target, owner, deadline):
    directory, target = safe_path(directory), safe_path(target)
    private(directory)
    endpoint, engine = connection(control, proof, owner, deadline)
    images = image_receipt(document(bounded_read(directory / "images.json", 65536)), engine)
    entry = document(bounded_read(directory / "entry.json", 65536))
    require_producer(directory, entry)
    bridge = document(bounded_read(directory / "network.json", 65536))
    require(images["engine"] == bridge["engine"] == engine, "resource-receipt-engine")
    live = network(document(fenced(owner, endpoint, engine, ["docker", "--host", endpoint, "network", "inspect", bridge["id"]], deadline)), bridge["name"], bridge["nonce"])
    require(live["Id"] == bridge["id"], "resource-network-id")
    owner.environment.update(RUSTACK_RUNTIME_DOCKER_SOCKET=endpoint[7:], RUSTACK_RUNTIME_ARCH="arm64",
        RUSTACK_RUNTIME_IMAGE=SOURCES[0], RUSTACK_RUNTIME_NETWORK=bridge["name"])
    attempt = uuid.uuid4().hex
    commands = bootstrap.Commands(owner, deadline, directory / ("kernel-" + attempt))
    commands.evidence.mkdir(mode=0o700)
    arguments = ["cargo", "test", "--locked", "--target-dir", str(target), "-p", "rustack-lambda-core", "--test", "docker_probe", "--"]
    required = parse_manifest(bounded_read(SCRIPT.with_name("docker-required-suites.json"), 65536))["docker-probe"]
    actual = parse_inventory(commands.run(arguments + ["--ignored", "--list", "--color", "never"], 300))
    require(actual == required and len(required) == 2, "resource-kernel-inventory")
    raw = commands.run(arguments + ["--ignored", "--test-threads=1", "--nocapture"], 300)
    verify_output(raw, required)
    for tail in (["ps", "--all", "--quiet"], ["volume", "ls", "--quiet"]):
        require(fenced(owner, endpoint, engine, ["docker", "--host", endpoint] + tail, deadline) == b"", "resource-remnants")
    aliases = fenced(owner, endpoint, engine, ["docker", "--host", endpoint, "image", "ls", "--all", "--format", "{{.Repository}}:{{.Tag}}"], deadline)
    bootstrap.write_new(commands.evidence / "aliases.stdout", aliases)
    private_aliases(aliases)
    for row in images["images"]:
        require(captured(owner, endpoint, row["cacheAlias"], deadline, engine, row["name"]) == row["image"]
                and captured(owner, endpoint, row["name"], deadline, engine) == row["image"], "resource-source-image-lost")
    source = "unix://" + images["sourceSocket"]
    require(catalog(owner, source, deadline, images["sourceEngine"]) == set(images["sourceIds"])
            and all(captured(owner, source, row["name"], deadline, images["sourceEngine"]) == row["image"]
                    for row in images["images"]), "resource-original-source-changed")
    result = {"schemaVersion": 1, "engine": engine, "exactResourcesPassed": sorted(required),
              "kernelPolicyQualified": True, "platformQualification": "pending", "scriptSha256": sha(SCRIPT),
              "attempt": attempt, "rawEvidence": str(commands.evidence)}
    bootstrap.publish(directory / ("kernel-result-" + attempt + ".json"), result)
    return result


def main():
    mode = sys.argv[1] if len(sys.argv) > 1 else ""
    try:
        require((mode == "forward" and len(sys.argv) == 4)
                or (mode in ("prepare", "prepare-full", "kernel") and len(sys.argv) == 6)
                or (mode == "refresh-full" and len(sys.argv) == 8), "resource-arguments")
        proof = attestation(sys.argv[2])
        owner = Runner(os.environ)
        with deadline_guard(3600 if mode == "forward" else 700, owner) as deadline:
            require(status(owner, proof["profile"], deadline) == proof["profileConfiguration"], "resource-profile-changed")
            if mode == "forward":
                forward(proof, sys.argv[3], owner, deadline)
                result = {"forwardJoined": True}
            elif mode == "refresh-full":
                result = prepare(proof, *sys.argv[3:6], owner, deadline, full=True, previous=sys.argv[6], build=sys.argv[7])
            elif mode in ("prepare", "prepare-full"):
                result = prepare(proof, *sys.argv[3:], owner, deadline, full=mode == "prepare-full")
            else:
                result = kernel(proof, *sys.argv[3:], owner, deadline)
            print(json.dumps(result, separators=(",", ":")), flush=True)
        return 0
    except (ActionError, QualificationError) as error:
        if mode == "forward" and str(error) == "qualification-interrupted":
            result = document(bounded_read(safe_path(sys.argv[3]) / "final.json", 65536))
            if result["childGroupJoined"] is True and result["socketRemoved"] is True:
                return 0
        print("rootless-resource-failed:" + str(error), file=sys.stderr)
        return 2
    except (OSError, ValueError, KeyError, TypeError, RecursionError):
        print("rootless-resource-failed", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
