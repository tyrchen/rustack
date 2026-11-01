"""Deterministic immutable Linux ZIP bytes shared by Docker and Native resources."""
import base64
import hashlib
import io
import json
from pathlib import Path
import stat
import sys
import zipfile

LIMIT = 50 * 1024 * 1024


def digest(raw):
    return base64.b64encode(hashlib.sha256(raw).digest()).decode("ascii")


def package(bootstrap, marker):
    assert marker in (b"A", b"B")
    target = io.BytesIO()
    with zipfile.ZipFile(target, "w", compression=zipfile.ZIP_STORED) as archive:
        for name, raw, mode in (("bootstrap", bootstrap, 0o755), ("revision.txt", marker, 0o644)):
            info = zipfile.ZipInfo(name, (1980, 1, 1, 0, 0, 0))
            info.create_system = 3
            info.external_attr = (stat.S_IFREG | mode) << 16
            info.compress_type = zipfile.ZIP_STORED
            archive.writestr(info, raw)
    raw = target.getvalue()
    assert len(raw) <= LIMIT
    return raw


def linux_elf(raw, architecture):
    machine = {"arm64":183, "amd64":62}[architecture]
    assert len(raw) >= 64 and raw[:7] == b"\x7fELF\x02\x01\x01"
    assert int.from_bytes(raw[16:18], "little") in (2, 3)
    assert int.from_bytes(raw[18:20], "little") == machine
    assert int.from_bytes(raw[20:24], "little") == 1 and int.from_bytes(raw[52:54], "little") == 64


def read(path):
    with path.open("rb") as stream:
        raw = stream.read(LIMIT + 1)
    assert raw and len(raw) <= LIMIT
    return raw


def build(source, architecture, directory):
    raw = read(source)
    linux_elf(raw, architecture)
    directory.mkdir(parents=True, exist_ok=True)
    outputs = {}
    for marker in (b"A", b"B"):
        code = package(raw, marker)
        name = "provided-" + marker.decode("ascii").lower() + ".zip"
        pending = directory / (name + ".pending")
        pending.write_bytes(code)
        pending.replace(directory / name)
        outputs[marker.decode("ascii")] = {"file":name, "bytes":len(code), "sha256":digest(code)}
    manifest = {"schemaVersion":1, "architecture":architecture, "bootstrapSha256":digest(raw), "artifacts":outputs}
    (directory / "provided-zip-manifest.json").write_text(json.dumps(manifest, sort_keys=True) + "\n")
    print(json.dumps(manifest, sort_keys=True))


if __name__ == "__main__":
    assert len(sys.argv) == 4
    build(Path(sys.argv[1]), sys.argv[2], Path(sys.argv[3]))
