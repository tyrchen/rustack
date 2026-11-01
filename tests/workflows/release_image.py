"""Bounded inspection of the actual release image; no container or extraction."""
import gzip
import hashlib
import io
import json
import os
from pathlib import Path
import re
import resource
import signal
import shutil
import stat
import struct
import sys
import tarfile
import tempfile
import time
import zlib

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "tools/github-action"))
from metadata import document, endpoint, source_id, engine_id
from process_owner import Runner
from settings import ActionError

MIB = 1024 * 1024
ARCHIVE_LIMIT = 128 * MIB
EXPANDED_LIMIT = 256 * MIB
BINARY_LIMIT = 128 * MIB
ENVIRONMENT = ["PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin",
               "GATEWAY_LISTEN=0.0.0.0:4566", "LOG_LEVEL=info", "SERVICES="]
INSPECT = ('{"id":{{json .Id}},"os":{{json .Os}},"architecture":{{json .Architecture}},'
           '"bytes":{{json .Size}},"config":{{json .Config}}}')
TREE = {
    "rustack": (0, 0, 0o755, "file"),
    "etc": (0, 0, 0o755, "directory"),
    "etc/ssl": (0, 0, 0o755, "directory"),
    "etc/ssl/certs": (0, 0, 0o755, "directory"),
    "etc/ssl/certs/ca-certificates.crt": (0, 0, 0o644, "file"),
    "tmp": (65532, 65532, 0o1777, "directory"),
    "data": (65532, 65532, 0o700, "directory"),
    "run": (0, 0, 0o555, "directory"),
    "run/rustack": (0, 0, 0o555, "directory"),
    "run/rustack/docker.sock": (0, 0, 0o444, "file"),
}


class ImageError(ValueError):
    """Fixed diagnostic codes, without candidate values or archive contents."""


def require(condition, code):
    if not condition:
        raise ImageError(code)


def digest(raw):
    return "sha256:" + hashlib.sha256(raw).hexdigest()


def configuration(value):
    require(isinstance(value, dict), "image-configuration")
    require(value.get("User") == "65532:65532" and value.get("WorkingDir") == "/data"
            and value.get("Entrypoint") == ["/rustack"] and value.get("Cmd") in (None, [])
            and value.get("Volumes") in (None, {}) and value.get("OnBuild") in (None, [])
            and value.get("Env") == ENVIRONMENT, "image-configuration")
    health = value.get("Healthcheck")
    require(isinstance(health, dict) and set(health) <= {
        "Test", "Interval", "Timeout", "StartPeriod", "StartInterval", "Retries"}, "image-health")
    require(health.get("Test") == ["CMD", "/rustack", "--health-check"]
            and health.get("Interval") == 2_000_000_000 and health.get("Timeout") == 4_000_000_000
            and health.get("StartPeriod") == 1_000_000_000 and health.get("Retries") == 3
            and health.get("StartInterval", 0) == 0
            and all(type(health.get(key, 0)) is int for key in
                    ("Interval", "Timeout", "StartPeriod", "StartInterval", "Retries")), "image-health")


def canonical(name):
    require(isinstance(name, str) and 0 < len(name.encode()) <= 256
            and re.fullmatch(r"[A-Za-z0-9_.+/-]+", name) is not None
            and all(part not in ("", ".", "..") for part in name.split("/")), "archive-path")
    return name


def _timing_pax(raw):
    require(0 < len(raw) <= 1024, "archive-pax-bytes")
    seen = set()
    while raw:
        space = raw.find(b" ")
        require(1 <= space <= 4 and raw[:space].isdigit(), "archive-pax-length")
        length = int(raw[:space])
        require(space + 3 <= length <= len(raw), "archive-pax-length")
        line, raw = raw[:length], raw[length:]
        require(line.endswith(b"\n"), "archive-pax-newline")
        key, separator, value = line[space + 1:-1].partition(b"=")
        require(separator and key in (b"mtime", b"atime", b"ctime") and key not in seen
                and 0 < len(value) <= 64 and re.fullmatch(rb"-?[0-9]+(?:\.[0-9]{1,9})?", value),
                "archive-pax-field")
        seen.add(key)
    require(1 <= len(seen) <= 3, "archive-pax-fields")


def _signature(member):
    name = member.name[:-1] if member.isdir() and member.name.endswith("/") else member.name
    return (canonical(name), member.uid, member.gid, member.mode, member.size, member.type, member.linkname)


def _physical(raw, maximum_members):
    offset, count, pending = 0, 0, False
    members, physical_names = {}, set()
    while offset + 512 <= len(raw):
        block = raw[offset:offset + 512]
        if not any(block):
            require(len(raw) - offset >= 1024 and not any(raw[offset:]) and not pending,
                    "archive-trailing")
            return members
        count += 1
        require(count <= maximum_members, "archive-physical-count")
        kind = block[156:157]
        require(kind in (tarfile.REGTYPE, tarfile.AREGTYPE, tarfile.DIRTYPE, tarfile.XHDTYPE),
                "archive-physical-type")
        member = tarfile.TarInfo.frombuf(block, "utf-8", "strict")
        name = _signature(member)[0]
        require(name not in physical_names and member.type == kind and not member.linkname
                and 0 <= member.size <= len(raw) and 0 <= member.uid <= 0xffffffff
                and 0 <= member.gid <= 0xffffffff and 0 <= member.mode <= 0o7777,
                "archive-physical-header")
        physical_names.add(name)
        start = offset + 512
        padded = ((member.size + 511) // 512) * 512
        require(start + padded <= len(raw), "archive-physical-size")
        require(not any(raw[start + member.size:start + padded]), "archive-padding")
        if kind == tarfile.XHDTYPE:
            require(not pending and 0 < member.size <= 1024, "archive-pax-chain-or-size")
            _timing_pax(raw[start:start + member.size])
            pending = True
        else:
            require(member.isfile() or (member.isdir() and member.size == 0), "archive-physical-type")
            members[name] = _signature(member)
            pending = False
        offset = start + padded
    raise ImageError("archive-terminator")


class Contents:
    """A small regular-member table over bounded bytes; never filesystem extraction."""
    def __init__(self, raw, maximum_members):
        require(isinstance(raw, bytes) and 1024 <= len(raw) <= EXPANDED_LIMIT
                and len(raw) % 512 == 0, "archive-bytes")
        physical = _physical(raw, maximum_members)
        self.archive = tarfile.open(fileobj=io.BytesIO(raw), mode="r:")
        self.members = {}
        for member in self.archive:
            name = _signature(member)[0]
            require(physical.get(name) == _signature(member), "archive-normalization")
            require(len(self.members) < maximum_members and name not in self.members
                    and (member.isfile() or (member.isdir() and member.size == 0)) and not member.linkname
                    and 0 <= member.size <= len(raw), "archive-member")
            require(set(member.pax_headers) <= {"mtime", "atime", "ctime"}
                    and all(isinstance(v, str) and len(v) <= 64
                            and re.fullmatch(r"-?[0-9]+(?:\.[0-9]{1,9})?", v)
                            for v in member.pax_headers.values()), "archive-pax")
            self.members[name] = member
        require(set(self.members) == set(physical), "archive-normalized-members")
        position = self.archive.fileobj.tell()
        require(len(raw) - position >= 512 and not any(raw[position:]), "archive-trailing")

    def read(self, name, maximum):
        member = self.members.get(canonical(name))
        require(member is not None and member.isfile() and member.size <= maximum, "archive-input")
        with self.archive.extractfile(member) as source:
            value = source.read(maximum + 1)
        require(len(value) == member.size, "archive-truncated")
        if name.startswith("blobs/sha256/"):
            require(name == "blobs/sha256/" + digest(value)[7:], "archive-blob-digest")
        return value


def elf(raw, architecture):
    require(len(raw) >= 64 and raw[:7] == b"\x7fELF\x02\x01\x01"
            and struct.unpack_from("<H", raw, 16)[0] in (2, 3)
            and struct.unpack_from("<I", raw, 20)[0] == 1, "image-elf")
    machine = struct.unpack_from("<H", raw, 18)[0]
    require(machine == {"arm64": 183, "amd64": 62}[architecture], "image-machine")
    offset = struct.unpack_from("<Q", raw, 32)[0]
    size, count = struct.unpack_from("<HH", raw, 54)
    require(size == 56 and 1 <= count <= 128 and offset >= 64
            and offset + size * count <= len(raw), "image-program-headers")
    require(all(struct.unpack_from("<I", raw, offset + size * i)[0] != 3
                for i in range(count)), "image-interpreter")
    return machine


def verify(raw, architecture, expected_id):
    """Bind the saved graph and exact scratch filesystem to a captured image ID."""
    try:
        require(architecture in ("arm64", "amd64") and len(raw) <= ARCHIVE_LIMIT, "image-input")
        source_id(expected_id)
        outer = Contents(raw, 128)
        manifests = document(outer.read("manifest.json", 16384))
        require(isinstance(manifests, list) and len(manifests) == 1
                and isinstance(manifests[0], dict), "image-manifest")
        manifest = manifests[0]
        config_raw = outer.read(manifest.get("Config"), 65536)
        config_digest = digest(config_raw)
        config = document(config_raw, maximum=65536, string_limit=4096)
        require(isinstance(config, dict) and config.get("os") == "linux"
                and config.get("architecture") == architecture, "image-platform")
        configuration(config.get("config"))
        root = config.get("rootfs")
        layers = manifest.get("Layers")
        require(isinstance(root, dict) and root.get("type") == "layers"
                and isinstance(layers, list) and 1 <= len(layers) <= 8
                and isinstance(root.get("diff_ids"), list)
                and len(root["diff_ids"]) == len(layers), "image-layers")
        graph = None
        if expected_id != config_digest:
            graph = document(outer.read("blobs/sha256/" + expected_id[7:], 65536), maximum=65536)
            require(isinstance(graph, dict) and graph.get("schemaVersion") == 2
                    and isinstance(graph.get("config"), dict)
                    and graph["config"].get("digest") == config_digest
                    and graph["config"].get("size") == len(config_raw)
                    and isinstance(graph.get("layers"), list)
                    and len(graph["layers"]) == len(layers), "image-identity")
        filesystem, binary, total = {}, None, 0
        for index, (name, expected) in enumerate(zip(layers, root["diff_ids"])):
            stored = outer.read(name, ARCHIVE_LIMIT)
            if graph is not None:
                descriptor = graph["layers"][index]
                require(isinstance(descriptor, dict) and descriptor.get("digest") == digest(stored)
                        and descriptor.get("size") == len(stored), "image-layer-identity")
            layer_raw = stored
            if stored.startswith(b"\x1f\x8b"):
                with gzip.GzipFile(fileobj=io.BytesIO(stored)) as compressed:
                    layer_raw = compressed.read(EXPANDED_LIMIT - total + 1)
            total += len(layer_raw)
            require(total <= EXPANDED_LIMIT and digest(layer_raw) == expected, "image-layer-digest")
            layer = Contents(layer_raw, 32)
            for path, member in layer.members.items():
                require(path in TREE, "image-extra-path")
                kind = "file" if member.isfile() else "directory"
                require((member.uid, member.gid, member.mode, kind) == TREE[path], "image-file-policy")
                require(member.size == 0 if kind == "directory" or path.endswith("docker.sock")
                        else member.size > 0, "image-file-size")
                filesystem[path] = dict(uid=member.uid, gid=member.gid, mode=member.mode,
                                        bytes=member.size, kind=kind)
                if path == "rustack":
                    binary = layer.read(path, BINARY_LIMIT)
                elif path.endswith("ca-certificates.crt"):
                    layer.read(path, MIB)
        require(set(filesystem) == set(TREE) and binary is not None, "image-filesystem")
        machine = elf(binary, architecture)
        return dict(schemaVersion=1, image=expected_id, architecture=architecture,
                    binarySha256=digest(binary), binaryBytes=len(binary), elfMachine=machine,
                    elfInterpreter=False, configurationSha256=config_digest,
                    archiveSha256=digest(raw), expandedBytes=total, filesystem=filesystem)
    except ImageError:
        raise
    except (ActionError, EOFError, OSError, ValueError, TypeError, KeyError, struct.error, tarfile.TarError, zlib.error):
        raise ImageError("image-archive-invalid") from None


def _limit_file_output():
    # This standalone fixture process and every exec child retain the kernel
    # bound. No preexec callback or second process supervisor is involved.
    soft, hard = resource.getrlimit(resource.RLIMIT_FSIZE)
    available = [ARCHIVE_LIMIT] + [value for value in (soft, hard)
                                 if value != resource.RLIM_INFINITY]
    maximum = min(available)
    require(maximum > 0, "image-file-limit")
    resource.setrlimit(resource.RLIMIT_FSIZE, (maximum, maximum))


def execute(architecture, socket, reference, program):
    require(architecture in ("arm64", "amd64") and isinstance(reference, str)
            and re.fullmatch(r"[A-Za-z0-9][A-Za-z0-9._/@:+-]{0,255}", reference), "image-arguments")
    selected = endpoint("unix://" + socket)
    require(isinstance(program, str) and 0 < len(os.fsencode(program)) <= 4096
            and all(32 <= ord(char) <= 126 for char in program)
            and (program == "docker" or program.startswith("/")), "image-cli")
    runner = Runner(os.environ)
    runner.pin_endpoint()
    original = time.monotonic() + 180
    def interrupted(_signal, _frame):
        if not runner.retiring:
            raise ImageError("image-observation-interrupted")
    for number in (signal.SIGALRM, signal.SIGINT, signal.SIGTERM):
        signal.signal(number, interrupted)
    signal.setitimer(signal.ITIMER_REAL, 180)
    def command(parts, cutoff):
        reply = runner.run(base + parts, min(cutoff, original))
        require(reply.code == 0, "image-command")
        return reply.stdout
    def engine(cutoff):
        return engine_id(document(command(["info", "--format", "{{json .ID}}"],
                                           min(cutoff, time.monotonic() + 5))))
    try:
        resolved = shutil.which(program)
        require(resolved is not None and Path(resolved).is_file(), "image-cli")
        base = [str(Path(resolved).resolve()), "--host", selected]
        _limit_file_output()
        root = Path(tempfile.mkdtemp(prefix="rustack-release-image-"))
        require(stat.S_IMODE(root.stat().st_mode) == 0o700, "image-private-root")
        print(json.dumps({"releaseImageEvidence": str(root)}), flush=True)
        captured = engine(original)
        def observed(parts, seconds):
            cutoff = min(original, time.monotonic() + seconds)
            require(engine(cutoff) == captured, "image-daemon-changed")
            result = command(parts, cutoff)
            require(engine(cutoff) == captured and time.monotonic() < cutoff, "image-late-observation")
            return result
        view_raw = observed(["image", "inspect", "--format", INSPECT, reference], 10)
        view = document(view_raw, string_limit=4096)
        require(isinstance(view, dict) and view.get("os") == "linux"
                and view.get("architecture") == architecture and type(view.get("bytes")) is int
                and 1 <= view["bytes"] <= 64 * MIB, "image-inspection")
        selected_id = source_id(view.get("id"))
        configuration(view.get("config"))
        path = root / "image.tar"
        observed(["image", "save", "--output", str(path), selected_id], 120)
        remaining = min(10, original - time.monotonic())
        require(remaining > 0, "image-original-deadline")
        signal.setitimer(signal.ITIMER_REAL, remaining)
        descriptor = os.open(path, os.O_RDONLY | os.O_NONBLOCK | os.O_NOFOLLOW)
        with os.fdopen(descriptor, "rb") as source:
            info = os.fstat(source.fileno())
            require(stat.S_ISREG(info.st_mode) and info.st_nlink == 1
                    and 0 < info.st_size <= ARCHIVE_LIMIT, "image-archive-file")
            raw = source.read(ARCHIVE_LIMIT + 1)
        require(len(raw) == info.st_size, "image-archive-observation")
        os.chmod(path, 0o600)
        report = verify(raw, architecture, selected_id)
        require(time.monotonic() < original, "image-original-deadline")
        report.update(engine=captured, reference=reference, inspectedImageBytes=view["bytes"],
                      inspectionSha256=digest(view_raw), privateArchive=str(path))
        target = root / "report.json"
        descriptor = os.open(target, os.O_WRONLY | os.O_CREAT | os.O_EXCL, 0o600)
        with os.fdopen(descriptor, "w") as output:
            json.dump(report, output, indent=2)
            output.write("\n")
        require(time.monotonic() < original, "image-original-deadline")
        print("Actual release image artifact verified: linux/" + architecture + "; " + selected_id)
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)


def main():
    try:
        require(len(sys.argv) == 5, "image-arguments")
        execute(*sys.argv[1:])
    except ImageError as error:
        print("Release image artifact rejected: " + str(error), file=sys.stderr)
        return 1
    except (ActionError, OSError, ValueError):
        print("Release image artifact rejected: command-or-io", file=sys.stderr)
        return 1
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
