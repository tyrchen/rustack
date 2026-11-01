"""Copy only validated, explicitly referenced release evidence into an artifact."""
import os
from pathlib import Path
import stat
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from linux_performance import ActionError
from linux_performance import Runner, deadline_guard, require
from release_qualification import assemble, member


def pack(source, destination, owner, deadline):
    metadata = source.stat()
    require(stat.S_ISDIR(metadata.st_mode) and metadata.st_uid == os.getuid()
            and stat.S_IMODE(metadata.st_mode) == 0o700, "bundle-private-source")
    require(not os.path.lexists(destination), "bundle-destination-exists")
    report = assemble(source, owner, deadline)
    require(report["candidateRunId"] == os.environ.get("CANDIDATE_RUN_ID"), "bundle-candidate-run")
    selected = {"bundle.json"} | set(report["candidateSeals"])
    for candidate in report["candidates"].values():
        selected.add(candidate["archive"])
    for lane in report["lanes"].values():
        selected.add(lane["path"])
        selected.update(lane["receipts"])
    require(len(selected) <= 2048, "bundle-file-count")
    destination.mkdir(mode=0o700)
    total = 0
    for name in sorted(selected):
        path = member(source, name, 2 * 1024 * 1024 * 1024, minimum=0)
        total += path.stat().st_size
        require(total <= 6 * 1024 * 1024 * 1024, "bundle-total-bound")
        target = destination / name
        target.parent.mkdir(mode=0o700, parents=True, exist_ok=True)
        with path.open("rb") as incoming, target.open("xb") as outgoing:
            os.chmod(target, 0o600)
            remaining = path.stat().st_size
            while remaining:
                block = incoming.read(min(remaining, 1024 * 1024))
                require(block, "bundle-copy-truncated")
                remaining -= len(block)
                outgoing.write(block)
            require(incoming.read(1) == b"", "bundle-copy-grew")
            outgoing.flush()
            os.fsync(outgoing.fileno())
    require(assemble(destination, owner, deadline) == report, "bundle-copy-changed")


def main():
    try:
        require(len(sys.argv) == 3, "bundle-arguments")
        owner = Runner(os.environ)
        with deadline_guard(600, owner) as deadline:
            pack(Path(sys.argv[1]).resolve(strict=True), Path(sys.argv[2]).absolute(), owner, deadline)
        return 0
    except (ActionError, OSError, ValueError, TypeError, KeyError):
        print("release-bundle-rejected", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
