"""One source-sealed Rust verification sequence with bounded owned diagnostics."""
import hashlib
import json
import os
from pathlib import Path
import sys
import time

sys.path.insert(0, str(Path(__file__).resolve().parent))
import rootless_platform as storage
from rootless_functional import retain_source_seal, source_seal
from linux_performance import QualificationError, Runner, bounded_read, deadline_guard, require, safe_path
from settings import ActionError

GATES = (
    ("build", ["cargo", "build", "--locked", "--workspace", "--all-targets"]),
    ("feature-build", ["cargo", "build", "--locked", "--workspace", "--all-targets", "--features", "stepfunctions"]),
    ("test", ["cargo", "test", "--locked", "--workspace", "--all-targets"]),
    ("feature-test", ["cargo", "test", "--locked", "--workspace", "--all-targets", "--features", "stepfunctions"]),
    ("doctest", ["cargo", "test", "--locked", "--workspace", "--doc"]),
    ("feature-doctest", ["cargo", "test", "--locked", "--workspace", "--features", "stepfunctions", "--doc"]),
    ("fmt", ["cargo", "+nightly", "fmt", "--", "--check"]),
    ("clippy", ["cargo", "clippy", "--locked", "--workspace", "--all-targets", "--", "-D", "warnings"]),
    ("feature-clippy", ["cargo", "clippy", "--locked", "--workspace", "--all-targets", "--features", "stepfunctions", "--", "-D", "warnings"]),
    ("rustdoc", ["cargo", "doc", "--locked", "--workspace", "--no-deps"]),
    ("feature-rustdoc", ["cargo", "doc", "--locked", "--workspace", "--no-deps", "--features", "stepfunctions"]),
    ("strict-core", ["cargo", "clippy", "--locked", "-p", "rustack-lambda-core", "--lib", "--no-deps", "--",
                     "-D", "warnings", "-W", "clippy::pedantic", "-W", "clippy::unwrap_used",
                     "-W", "clippy::expect_used", "-W", "clippy::indexing_slicing", "-W", "clippy::panic"]),
    ("strict-consumer", ["cargo", "clippy", "--locked", "-p", "rustack-cli", "--bin", "rustack", "--test", "docker_startup",
                         "--no-default-features", "--features", "lambda,stepfunctions,s3,dynamodb", "--no-deps", "--",
                         "-D", "warnings", "-W", "clippy::pedantic"]),
    ("audit", ["cargo", "audit"]),
    ("deny", ["cargo", "deny", "check"]),
)


def run(directory, owner, deadline, gates=None):
    selected = GATES if gates is None else gates
    require(not os.path.lexists(directory), "rust-gates-evidence-exists")
    directory.mkdir(mode=0o700)
    before = source_seal(owner, deadline)
    source = retain_source_seal(directory, before)
    storage.publish(directory / "entry.json", {"schemaVersion": 1, "ownerPid": os.getpid(), **source,
        "gates": [{"name": name, "argv": argv} for name, argv in selected]})
    rows = []
    for name, arguments in selected:
        require(source_seal(owner, deadline) == before, "rust-gates-source-changed")
        if name in ("rustdoc", "feature-rustdoc"):
            owner.environment["RUSTDOCFLAGS"] = "-D warnings"
        else:
            owner.environment.pop("RUSTDOCFLAGS", None)
        log = directory / (name + ".log")
        cutoff = min(deadline, time.monotonic() + 900)
        invocation = [sys.executable, "-I", "-B", str(Path(__file__).with_name("gate_command.py")), str(log)] + arguments
        storage.publish(directory / (name + ".intent.json"), {"argv": invocation, "cutoffMonotonic": cutoff})
        started = time.monotonic()
        result = owner.run(invocation, cutoff)
        row = {"name": name, "argv": arguments, "code": result.code,
               "seconds": time.monotonic() - started,
               "logSha256": hashlib.sha256(bounded_read(log, 16 * 1024 * 1024)).hexdigest()}
        rows.append(row)
        storage.publish(directory / (name + ".json"), row)
        print(json.dumps({"gate": name, "code": result.code}), flush=True)
        require(result.code == 0 and result.stdout == result.stderr == b"", "rust-gate-failed")
    require(source_seal(owner, deadline) == before, "rust-gates-source-changed")
    report = {"schemaVersion": 1, **source, "gates": rows, "allChildGroupsJoined": not owner.unsettled}
    storage.publish(directory / "report.json", report)
    return report


def main():
    try:
        require(len(sys.argv) == 2, "rust-gates-arguments")
        owner = Runner(os.environ)
        with deadline_guard(5400, owner) as deadline:
            run(safe_path(sys.argv[1]), owner, deadline)
        return 0
    except (ActionError, QualificationError) as error:
        print("rust-gates-failed:" + str(error), file=sys.stderr)
        return 2
    except (OSError, ValueError, KeyError, TypeError):
        print("rust-gates-failed", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
