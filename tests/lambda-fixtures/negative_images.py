"""Deterministic, non-executable Docker archives with genuine platform metadata.

These explicit fixture caches test preparation validation, not container execution.
No filesystem paths or archive member names come from image input.
"""
import argparse
import hashlib
import io
import json
from pathlib import Path
import tarfile


def encoded(value):
    return json.dumps(value, sort_keys=True, separators=(",", ":")).encode("ascii")


def archive(members):
    output = io.BytesIO()
    with tarfile.open(fileobj=output, mode="w", format=tarfile.USTAR_FORMAT) as target:
        for name, value in members:
            info = tarfile.TarInfo(name)
            info.size, info.uid, info.gid, info.mode, info.mtime = len(value), 1000, 1000, 0o444, 0
            target.addfile(info, io.BytesIO(value))
    result = output.getvalue()
    assert len(result) <= 65536
    return result


def image(selected, kind):
    if selected not in ("arm64", "amd64") or kind not in ("os", "architecture"):
        raise ValueError("unsupported fixed negative fixture")
    system = "windows" if kind == "os" else "linux"
    architecture = "amd64" if kind == "os" or selected == "arm64" else "arm64"
    layer = archive([("fixture.txt", b"Rustack platform validation fixture\n")])
    config = encoded({"architecture":architecture, "os":system,
                      "config":{"User":"1000:1000", "Entrypoint":["/not-a-runtime"], "WorkingDir":"/"},
                      "rootfs":{"type":"layers", "diff_ids":["sha256:" + hashlib.sha256(layer).hexdigest()]}})
    identity = hashlib.sha256(config).hexdigest()
    manifest = [{"Config":identity + ".json", "RepoTags":[f"rustack-invalid-{kind}:{selected}"],
                 "Layers":["layer/layer.tar"]}]
    return archive([(identity + ".json", config), ("layer/layer.tar", layer),
                    ("manifest.json", encoded(manifest))])


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("architecture", choices=("arm64", "amd64"))
    parser.add_argument("output", type=Path)
    args = parser.parse_args()
    args.output.mkdir(parents=True, exist_ok=True)
    for kind in ("os", "architecture"):
        (args.output / (kind + ".tar")).write_bytes(image(args.architecture, kind))


if __name__ == "__main__":
    main()
