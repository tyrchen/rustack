"""Three explicit local images, bounded file-output saves and fenced imports."""
import hashlib
import os
from pathlib import Path
import resource
import shutil
import stat
import sys
import tempfile
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))

from linux_performance import MIB, Runner, deadline_guard, profile, safe_path, status
from image_performance_support import MeasurementError, decode, encode, require
from metadata import endpoint as local_endpoint, engine_id, source_id

SOURCES = ("rustack/lambda-runtime:integration-arm64", "rustack-p0-handler:arm64", "rustack-custom-ric:a-arm64")
LIMIT = 1024 * MIB


def call(owner, arguments, deadline, seconds=10):
    reply = owner.run(arguments, min(deadline, time.monotonic() + seconds))
    require(reply.code == 0 and reply.stderr == b"", "image-preparation-command")
    return reply.stdout


def fenced(owner, endpoint, engine, arguments, deadline, seconds=10):
    def identity():
        current = call(owner, ["docker", "--host", endpoint, "info", "--format", "{{.ID}}"], deadline).decode().strip()
        require(current == engine, "image-operation-daemon-changed")
    identity()
    try:
        return call(owner, arguments, deadline, seconds)
    finally:
        identity()


def image(owner, endpoint, name, deadline, engine):
    value = decode(fenced(owner, endpoint, engine, ["docker", "--host", endpoint, "image", "inspect", name,
        "--format", '{"id":{{json .Id}},"os":{{json .Os}},"architecture":{{json .Architecture}},"bytes":{{json .Size}}}'], deadline))
    require(isinstance(value, dict) and value.get("os") == "linux" and value.get("architecture") == "arm64"
            and type(value.get("bytes")) is int and 0 < value["bytes"] <= LIMIT, "source-platform")
    source_id(value["id"])
    return value


def cache_alias(immutable):
    source_id(immutable)
    return "rustack-workflows-cache:d102-" + immutable[7:]


def retain_content(owner, endpoint, immutable, deadline, engine):
    alias = cache_alias(immutable)
    raw = fenced(owner, endpoint, engine, ["docker", "--host", endpoint, "image", "ls", "--all",
        "--no-trunc", "--filter", "reference=" + alias, "--format", "{{.ID}}"], deadline)
    require(raw in (b"", (immutable + "\n").encode()), "operator-cache-alias-conflict")
    if not raw:
        fenced(owner, endpoint, engine, ["docker", "--host", endpoint, "image", "tag", immutable, alias], deadline)
    observed = fenced(owner, endpoint, engine, ["docker", "--host", endpoint, "image", "inspect", alias,
        "--format", "{{.Id}}"], deadline)
    require(observed == (immutable + "\n").encode(), "operator-cache-id-differs")
    return alias


def anchor(owner, endpoint, expected, deadline, engine):
    alias = retain_content(owner, endpoint, expected["id"], deadline, engine)
    require(image(owner, endpoint, alias, deadline, engine) == expected, "operator-cache-alias-differs")
    return alias


def source_references(architecture):
    require(architecture in ("arm64", "amd64"), "source-cache-architecture")
    return tuple(name + architecture for name in ("rustack/lambda-runtime:integration-", "rustack-p0-handler:",
        "rustack-custom-ric:a-", "rustack-custom-ric:b-", "rustack-no-runtime:",
        "rustack/workflows-server:integration-", "rustack/workflows-release:integration-"))


def retain_sources(socket, architecture, owner, deadline):
    references = source_references(architecture)
    endpoint = "unix://" + str(safe_path(socket))
    local_endpoint(endpoint)
    engine = call(owner, ["docker", "--host", endpoint, "info", "--format", "{{.ID}}"], deadline).decode().strip()
    engine_id(engine)
    preserved = []
    for reference in references:
        raw = fenced(owner, endpoint, engine, ["docker", "--host", endpoint, "image", "ls", "--all", "--no-trunc",
            "--filter", "reference=" + reference, "--format", "{{.ID}}"], deadline)
        ids = raw.decode("ascii").splitlines()
        require(len(ids) <= 1, "source-cache-reference-count")
        if ids:
            immutable = source_id(ids[0])
            alias = retain_content(owner, endpoint, immutable, deadline, engine)
            preserved.append({"reference": reference, "id": immutable, "cacheAlias": alias})
    for row in preserved:
        actual = fenced(owner, endpoint, engine, ["docker", "--host", endpoint, "image", "inspect", row["reference"],
            "--format", "{{.Id}}"], deadline)
        require(actual == (row["id"] + "\n").encode(), "source-cache-reference-changed")
    require(call(owner, ["docker", "--host", endpoint, "info", "--format", "{{.ID}}"], deadline).decode().strip() == engine,
            "source-cache-daemon-changed")
    return {"schemaVersion": 1, "engine": engine, "architecture": architecture,
            "referenceCount": len(references), "preserved": preserved}


def catalog(owner, endpoint, deadline, engine):
    raw = fenced(owner, endpoint, engine, ["docker", "--host", endpoint, "image", "ls", "--all", "--quiet", "--no-trunc"], deadline)
    values = raw.decode("ascii").splitlines()
    require(len(values) <= 256, "image-inventory-budget")
    for value in values: source_id(value)
    return set(values)


def hash_file(path, deadline):
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    digest, count = hashlib.sha256(), 0
    with os.fdopen(descriptor, "rb") as stream:
        metadata = os.fstat(stream.fileno())
        require(stat.S_ISREG(metadata.st_mode) and metadata.st_nlink == 1
                and 0 < metadata.st_size <= LIMIT, "image-archive-file")
        while True:
            require(time.monotonic() < deadline, "image-archive-deadline")
            block = stream.read(MIB)
            if not block: break
            count += len(block)
            require(count <= LIMIT, "image-archive-byte-budget")
            digest.update(block)
        require(count == metadata.st_size, "image-archive-changed")
    return count, digest.hexdigest()


def file_limit(soft, hard, remaining):
    require(type(remaining) is int and 1 <= remaining <= 2 * LIMIT, "image-remaining-budget")
    return min([LIMIT, remaining] + [value for value in (soft, hard) if value != resource.RLIM_INFINITY])


def save(socket, archive, immutable, remaining):
    socket, archive = safe_path(socket), safe_path(archive)
    source_id(immutable)
    program = shutil.which("docker")
    require(program is not None, "docker-executable")
    soft, hard = resource.getrlimit(resource.RLIMIT_FSIZE)
    cap = file_limit(soft, hard, int(remaining))
    resource.setrlimit(resource.RLIMIT_FSIZE, (cap, cap))
    os.execv(program, [program, "--host", "unix://" + str(socket), "image", "save", "--output", str(archive), immutable])


def prepare(selected, socket, directory, owner, deadline):
    selected, socket, directory = profile(selected), safe_path(socket), safe_path(directory)
    require(not os.path.lexists(directory / "image-preparation.json"), "preparation-receipt-exists")
    allocation = status(owner, selected, deadline)
    target, source = allocation["docker_socket"], "unix://" + str(socket)
    local_endpoint(target)
    local_endpoint(source)
    require(target != source, "distinct-image-daemons")
    identities = {}
    for key, endpoint in (("source", source), ("target", target)):
        identities[key] = call(owner, ["docker", "--host", endpoint, "info", "--format", "{{.ID}}"], deadline).decode().strip()
        engine_id(identities[key])
    require(identities["source"] != identities["target"], "distinct-image-daemon-identities")
    before = {key: catalog(owner, endpoint, deadline, identities[key]) for key, endpoint in (("source", source), ("target", target))}
    expected = {name: image(owner, source, name, deadline, identities["source"]) for name in SOURCES}
    require(len({row["id"] for row in expected.values()}) == 3, "distinct-source-images")
    directory.mkdir(parents=True, exist_ok=True)
    root = Path(tempfile.mkdtemp(prefix="image-transfer-", dir=directory))
    rows, total = [], 0
    for index, name in enumerate(SOURCES):
        remaining = 2 * LIMIT - total
        require(remaining > 0, "image-total-budget-before-save")
        archive = root / (str(index) + ".tar")
        with archive.open("xb"):
            archive.chmod(0o600)
        immutable = expected[name]["id"]
        fenced(owner, source, identities["source"], [sys.executable, "-I", "-B", str(Path(__file__).resolve()),
            "save", str(socket), str(archive), immutable, str(remaining), "v1"], deadline, 120)
        size, digest = hash_file(archive, deadline)
        total += size
        require(size <= remaining and total <= 2 * LIMIT
                and image(owner, source, immutable, deadline, identities["source"]) == expected[name], "source-changed-or-total-budget")
        # The only accepted archives are produced here by this fenced local CLI.
        # No caller-supplied TAR is extracted or interpreted by Python.
        fenced(owner, target, identities["target"], ["docker", "--host", target, "image", "load", "--input", str(archive)], deadline, 120)
        require(image(owner, target, immutable, deadline, identities["target"]) == expected[name], "imported-image-differs")
        alias = anchor(owner, target, expected[name], deadline, identities["target"])
        require(hash_file(archive, deadline) == (size, digest), "import-archive-changed")
        rows.append({"name": name, "image": expected[name], "cacheAlias": alias, "archive": str(archive), "bytes": size, "sha256": digest})
    for key, endpoint in (("source", source), ("target", target)):
        observed = call(owner, ["docker", "--host", endpoint, "info", "--format", "{{.ID}}"], deadline).decode().strip()
        require(observed == identities[key], "image-daemon-changed")
        after = catalog(owner, endpoint, deadline, identities[key])
        require(after == before[key] | ({row["id"] for row in expected.values()} if key == "target" else set()),
                "image-inventory-changed")
    result = {"schemaVersion": 1, "profile": selected, "engines": identities, "images": rows,
              "sourceIdsPreserved": sorted(before["source"]), "targetIdsPreserved": sorted(before["target"])}
    (directory / "image-preparation.json").write_bytes(encode(result) + b"\n")
    return result


def main():
    try:
        require(len(sys.argv) in (5, 6, 7) and sys.argv[1] in ("prepare", "save", "retain"), "image-preparation-arguments")
        require(sys.argv[-1] == "v1", "preparation-schema")
        if sys.argv[1] == "save":
            require(len(sys.argv) == 7, "save-arguments")
            save(*sys.argv[2:6])
        elif sys.argv[1] == "prepare":
            require(len(sys.argv) == 6, "preparation-arguments")
            owner = Runner(os.environ)
            with deadline_guard(600, owner) as deadline:
                print(encode(prepare(*sys.argv[2:5], owner, deadline)).decode(), flush=True)
        else:
            require(len(sys.argv) == 5, "source-cache-arguments")
            owner = Runner(os.environ)
            with deadline_guard(120, owner) as deadline:
                result = retain_sources(*sys.argv[2:4], owner, deadline)
                print("WORKFLOWS_SOURCE_CACHE " + encode(result).decode(), flush=True)
        return 0
    except MeasurementError as error:
        print("image-performance-preparation-failed:" + str(error), file=sys.stderr)
        return 2
    except Exception:
        print("image-performance-preparation-failed", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
