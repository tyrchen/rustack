"""Direct Linux SDK measurements with exact inventories and joined process owners."""
import hashlib
from contextlib import contextmanager
import json
import os
from pathlib import Path
import platform
import re
import resource
import signal
import stat
import struct
import sys
import time

ROOT = Path(__file__).resolve().parents[2]
sys.path.insert(0, str(ROOT / "tools/github-action"))
from process_owner import Runner
from metadata import document as bounded_document
from settings import ActionError

MIB = 1024 * 1024
CASES = (
    "performance::test_should_measure_one_hundred_fifty_state_signed_sdk_executions",
    "performance::test_should_keep_health_and_stop_responsive_and_reuse_one_execution_slot",
)
CONTROLS = tuple("performance::tests::" + name for name in (
    "test_should_use_exact_nearest_ranks_and_preserve_sample_order",
    "test_should_round_up_and_enforce_exact_p95_budget",
    "test_should_reject_ready_results_at_the_expired_observation_boundary",
    "test_should_reject_exhaustion_readiness_at_the_two_second_boundary",
    "test_should_reject_broken_predecessors_and_wrong_or_missing_state_exits",
    "test_should_refuse_post_terminal_probes_as_live_load_evidence",
))
LIMITS = {"openExecutions": 1, "transitions": 1000000, "historyEvents": 25000,
          "historyBytes": 33554432, "totalHistoryBytes": 134217728}


class QualificationError(ValueError):
    """Only fixed locally defined codes may reach public diagnostics."""


def require(condition, code):
    if not condition:
        raise QualificationError(code)


def check_deadline(deadline):
    require(time.monotonic() < deadline, "qualification-deadline")


@contextmanager
def deadline_guard(seconds, owner=None):
    """Cover local file IO and report publication as well as owned children."""
    require(signal.getitimer(signal.ITIMER_REAL) == (0.0, 0.0), "existing-deadline")
    def expired(number, _frame):
        if owner is not None and owner.retiring:
            # Retirement has its own original three-second physical join bound.
            # Interrupting that join would discard ownership of the child group.
            return
        raise QualificationError("qualification-deadline" if number == signal.SIGALRM
                                 else "qualification-interrupted")
    previous = {number: signal.signal(number, expired)
                for number in (signal.SIGALRM, signal.SIGTERM, signal.SIGINT, signal.SIGHUP)}
    cutoff = time.monotonic() + seconds
    signal.setitimer(signal.ITIMER_REAL, seconds)
    try:
        yield cutoff
        check_deadline(cutoff)
    finally:
        signal.setitimer(signal.ITIMER_REAL, 0)
        for number, handler in previous.items():
            signal.signal(number, handler)


def document(raw, limit=65536):
    require(isinstance(raw, bytes) and 0 < len(raw) <= limit, "document-bound")
    try:
        return bounded_document(raw, maximum=limit, string_limit=16384)
    except ActionError:
        raise QualificationError("document-shape") from None


def safe_path(value):
    require(isinstance(value, str) and 0 < len(value.encode()) <= 4096
            and re.fullmatch(r"/[A-Za-z0-9_./-]+", value) is not None
            and all(part not in ("", ".", "..") for part in value[1:].split("/")), "unsafe-path")
    return Path(value)


def profile(value):
    require(isinstance(value, str)
            and re.fullmatch(r"rustack-workflows-[a-z0-9-]{1,40}", value) is not None,
            "unsafe-profile")
    return value


def bounded_read(path, limit):
    # No device/FIFO/symlink may enter the reader. All inputs are selected artifacts.
    descriptor = os.open(path, os.O_RDONLY | os.O_NOFOLLOW | os.O_NONBLOCK)
    with os.fdopen(descriptor, "rb") as stream:
        metadata = os.fstat(stream.fileno())
        require(stat.S_ISREG(metadata.st_mode) and metadata.st_size <= limit, "file-bound")
        raw = stream.read(limit + 1)
        require(len(raw) <= limit, "file-bound")
        return raw


def fingerprint(path):
    raw = bounded_read(safe_path(str(path)), 256 * MIB)
    require(64 <= len(raw) <= 256 * MIB and raw[:6] == b"\x7fELF\x02\x01"
            and struct.unpack_from("<H", raw, 18)[0] == 183, "arm64-elf")
    return hashlib.sha256(raw).hexdigest()


def artifact(raw):
    require(0 < len(raw) <= 4 * MIB, "cargo-stream-bound")
    require(raw.count(b"\n") <= 4096, "cargo-record-budget")
    lines = raw.split(b"\n")
    if lines[-1] == b"":
        lines.pop()
    require(1 <= len(lines) <= 4096, "cargo-record-budget")
    records = [document(line, 262144) for line in lines]
    require(records and all(isinstance(row, dict) for row in records), "cargo-stream-shape")
    require(records[-1] == {"reason": "build-finished", "success": True}, "cargo-build-result")
    selected = []
    for row in records:
        if row.get("reason") == "compiler-artifact":
            require(isinstance(row.get("target"), dict) and isinstance(row.get("profile"), dict),
                    "cargo-artifact-shape")
        if row.get("reason") == "compiler-artifact" and row.get("target", {}).get("name") == "workflows_sdk":
            require(row.get("profile", {}).get("test") is True
                    and row.get("target", {}).get("kind") == ["test"], "cargo-test-artifact")
            selected.append(safe_path(row.get("executable")))
    require(len(selected) == 1, "cargo-test-artifact-count")
    binary = selected[0]
    require(binary.parent.name == "deps" and binary.parent.parent.name == "debug"
            and binary.parent.parent.parent.name == "aarch64-unknown-linux-gnu"
            and re.fullmatch(r"workflows_sdk-[0-9a-f]{16}", binary.name) is not None,
            "cargo-target-path")
    server = binary.parent.parent / "rustack"
    servers = [row for row in records if row.get("reason") == "compiler-artifact"
               and row.get("target", {}).get("name") == "rustack"
               and row.get("target", {}).get("kind") == ["bin"]
               and row.get("executable") == str(server)]
    require(len(servers) == 1, "cargo-server-artifact-count")
    return binary, server


def inventory(raw):
    text = raw.decode("utf-8")
    lines = text.splitlines()
    names = [line[:-6] for line in lines if line.endswith(": test")]
    require(len(names) == len(set(names)) == 8
            and set(names) == set(CASES + CONTROLS)
            and len(lines) == 8, "test-inventory")


def summary(raw, passed, ignored):
    text = raw.decode("utf-8")
    found = re.findall(r"^test result: ok\. (\d+) passed; (\d+) failed; (\d+) ignored; "
                       r"(\d+) measured; (\d+) filtered out; finished in [0-9.]+s$", text, re.M)
    require(len(found) == 1 and found[0][:4] == (str(passed), "0", str(ignored), "0"),
            "test-terminal-summary")
    return text


def integer(value, low, high):
    return type(value) is int and low <= value <= high


def samples(value, ceiling):
    require(isinstance(value, dict) and set(value) == {
        "count", "observedMicros", "p50Micros", "p95Micros", "maxMicros"}, "sample-shape")
    raw = value["observedMicros"]
    require(type(value["count"]) is int and value["count"] == 100
            and isinstance(raw, list) and len(raw) == 100
            and all(integer(item, 1, ceiling) for item in raw), "sample-bound")
    ranked = sorted(raw)
    for key, rank in (("p50Micros", 49), ("p95Micros", 94), ("maxMicros", 99)):
        require(type(value[key]) is int and value[key] == ranked[rank], "sample-rank")


def measurement(raw, kind):
    text = summary(raw, 1, 0)
    reports = [line.encode() for line in re.findall(
        r"(?:^|\.\.\. )WORKFLOWS_PERFORMANCE ([^\r\n]*)$", text, re.M)]
    require(len(reports) == 1, "measurement-count")
    return validate_report(document(reports[0]), kind)


def validate_report(value, kind):
    require(isinstance(value, dict) and set(value) == {
        "schemaVersion", "host", "limits", "nativeProcessesObserved", "serverJoined", "measurement"},
        "report-shape")
    require(type(value["schemaVersion"]) is int and value["schemaVersion"] == 1
            and type(value["nativeProcessesObserved"]) is int and value["nativeProcessesObserved"] == 0
            and value["serverJoined"] is True and value["limits"] == LIMITS
            and all(type(number) is int for number in value["limits"].values()), "report-cleanup-limits")
    host = value["host"]
    require(isinstance(host, dict) and set(host) == {"os", "architecture", "availableLogicalCpus",
            "debugAssertions", "referenceQualification"} and host["os"] == "linux"
            and host["architecture"] == "aarch64" and type(host["availableLogicalCpus"]) is int
            and host["availableLogicalCpus"] == 4 and host["debugAssertions"] is True
            and host["referenceQualification"] == "pending", "measurement-host")
    measured = value["measurement"]
    require(isinstance(measured, dict) and measured.get("kind") == kind, "measurement-kind")
    if kind == "pureLatency":
        require(set(measured) == {"kind", "stateCount", "inputBytes", "executions", "budgetMicros", "latency"}
                and all(type(measured[key]) is int and measured[key] == expected for key, expected in
                        (("stateCount", 50), ("inputBytes", 1024), ("executions", 100), ("budgetMicros", 100000))),
                "latency-contract")
        samples(measured["latency"], 1999999)
        require(measured["latency"]["p95Micros"] <= 100000, "latency-budget")
    else:
        require(set(measured) == {"kind", "openExecutionSlots", "completedCycles", "inputBytes",
                "historyEventHighWater", "exhaustion", "readiness", "stop"}
                and all(type(measured[key]) is int and measured[key] == expected for key, expected in
                        (("openExecutionSlots", 1), ("completedCycles", 100), ("inputBytes", 1024)))
                and integer(measured["historyEventHighWater"], 1, 25000), "loop-contract")
        for key in ("readiness", "stop"):
            samples(measured[key], 1999999)
        exhaustion = measured["exhaustion"]
        require(isinstance(exhaustion, dict) and set(exhaustion) == {
                "probeCount", "liveProbeCount", "maxReadinessMicros"}
                and integer(exhaustion["probeCount"], 1, 1000000)
                and integer(exhaustion["liveProbeCount"], 1, exhaustion["probeCount"])
                and integer(exhaustion["maxReadinessMicros"], 1, 1999999), "exhaustion-contract")
    return value


def command(owner, arguments, deadline, budget):
    reply = owner.run(arguments, min(deadline, time.monotonic() + budget), stdout_limit=65536)
    require(reply.code == 0 and not reply.stderr, "command-failed")
    return reply.stdout


def guest(binary, server, deadline, owner):
    require(sys.platform == "linux" and platform.machine() == "aarch64"
            and len(os.sched_getaffinity(0)) == 4, "guest-platform")
    memory = bounded_read(Path("/proc/meminfo"), 65536).decode("ascii")
    match = re.search(r"^MemTotal:\s+(\d+) kB$", memory, re.M)
    require(match is not None and 7 * 1024 * 1024 <= int(match[1]) <= 8 * 1024 * 1024, "guest-memory")
    before = {"test": fingerprint(binary), "server": fingerprint(server)}
    inventory(command(owner, [str(binary), "performance::", "--list", "--format", "terse"], deadline, 10))
    summary(command(owner, [str(binary), "performance::", "--test-threads=1"], deadline, 30), 6, 2)
    reports = []
    for case, kind in zip(CASES, ("pureLatency", "loopResponsiveness")):
        reports.append(measurement(command(owner, [str(binary), case, "--ignored", "--exact",
                           "--nocapture", "--test-threads=1"], deadline, 220), kind))
    require(before == {"test": fingerprint(binary), "server": fingerprint(server)}
            and time.monotonic() < deadline and not owner.unsettled, "guest-artifact-or-deadline")
    return {"schemaVersion": 1, "referenceQualification": "pending", "binarySha256": before,
            "kernel": platform.release(), "python": platform.python_version(),
            "observedMemoryKiB": int(match[1]), "availableLogicalCpus": len(os.sched_getaffinity(0)),
            "childHighWaterRssKiB": resource.getrusage(resource.RUSAGE_CHILDREN).ru_maxrss,
            "controlsPassed": 6, "exactResourcesPassed": list(CASES), "reports": reports,
            "processGroupsJoined": True}


def status(owner, selected, deadline):
    value = document(command(owner, ["colima", "-p", selected, "status", "--json"], deadline, 10))
    require(isinstance(value, dict) and value.get("arch") == "aarch64" and value.get("cpu") == 4
            and value.get("memory") == 8 * 1024 * MIB and value.get("runtime") == "docker"
            and value.get("driver") == "macOS Virtualization.Framework", "profile-allocation")
    return value


def host(selected, build, deadline, owner):
    selected = profile(selected)
    raw = bounded_read(safe_path(build), 4 * MIB)
    binary, server = artifact(raw)
    before = {"test": fingerprint(binary), "server": fingerprint(server)}
    configuration = status(owner, selected, deadline)
    result = document(command(owner, ["colima", "-p", selected, "ssh", "--", "python3", "-I", "-B",
                      str(safe_path(str(Path(__file__).resolve()))), "guest", str(binary), str(server)],
                      deadline, 500))
    require(isinstance(result, dict) and set(result) == {"schemaVersion", "referenceQualification",
            "binarySha256", "kernel", "python", "observedMemoryKiB", "availableLogicalCpus",
            "childHighWaterRssKiB", "controlsPassed", "exactResourcesPassed", "reports", "processGroupsJoined"},
            "guest-result-shape")
    require(type(result["schemaVersion"]) is int and result["schemaVersion"] == 1
            and result["binarySha256"] == before and result["referenceQualification"] == "pending"
            and result["processGroupsJoined"] is True and type(result["controlsPassed"]) is int
            and result["controlsPassed"] == 6 and result["exactResourcesPassed"] == list(CASES)
            and type(result["availableLogicalCpus"]) is int and result["availableLogicalCpus"] == 4
            and integer(result["observedMemoryKiB"], 7 * 1024 * 1024, 8 * 1024 * 1024)
            and integer(result["childHighWaterRssKiB"], 1, 8 * 1024 * 1024), "guest-result-provenance")
    require(isinstance(result["reports"], list) and len(result["reports"]) == 2, "guest-report-count")
    for value, kind in zip(result["reports"], ("pureLatency", "loopResponsiveness")):
        validate_report(value, kind)
    require(isinstance(result["kernel"], str) and re.fullmatch(r"[A-Za-z0-9_.+-]{1,256}", result["kernel"])
            and isinstance(result["python"], str) and re.fullmatch(r"3\.[0-9]{1,3}\.[0-9]{1,3}", result["python"]),
            "guest-version-shape")
    require(before == {"test": fingerprint(binary), "server": fingerprint(server)}
            and configuration == status(owner, selected, deadline), "host-artifact-or-profile-change")
    result.update(profile=selected, profileConfiguration=configuration,
                  virtualization="native-aarch64-vz; physical CPU dedication unproven",
                  cargoArtifactStreamSha256=hashlib.sha256(raw).hexdigest())
    return result


def provision(selected, target, deadline, owner):
    selected = profile(selected)
    require(sys.platform == "darwin" and platform.machine() == "arm64", "provision-platform")
    target = safe_path(target)
    require(target.is_dir(), "target-directory")
    safe_path(str(ROOT))
    require(not os.path.lexists(Path.home() / ".colima" / selected), "existing-profile")
    # The new profile owns no user resources. Never activate or edit an existing VM.
    reply = owner.run(["colima", "start", selected, "--arch", "aarch64", "--vm-type", "vz",
                      "--cpus", "4", "--memory", "8", "--disk", "24", "--runtime", "docker",
                      "--activate=false", "--vz-rosetta=false", "--binfmt=false", "--ssh-config=false",
                      "--mount", str(ROOT), "--mount", str(target)], deadline, stdout_limit=65536)
    require(reply.code == 0, "provision-failed-profile-retained")
    return {"profile": selected, "configuration": status(owner, selected, deadline),
            "referenceQualification": "pending", "profileRetained": True}


def main():
    try:
        require(len(sys.argv) == 4 and sys.argv[1] in ("guest", "host", "provision"), "arguments")
        mode = sys.argv[1]
        owner = Runner({"PATH": "/usr/bin:/bin"} if mode == "guest" else os.environ)
        with deadline_guard({"guest": 450, "host": 530, "provision": 330}[mode], owner) as deadline:
            if mode == "guest":
                binary, server = safe_path(sys.argv[2]), safe_path(sys.argv[3])
                require(binary.parent.parent / "rustack" == server, "guest-server-path")
                result = guest(binary, server, deadline, owner)
            elif mode == "provision":
                result = provision(sys.argv[2], sys.argv[3], deadline, owner)
            else:
                result = host(sys.argv[2], sys.argv[3], deadline, owner)
            encoded = json.dumps(result, separators=(",", ":"), allow_nan=False)
            require(len(encoded.encode()) <= 65536, "result-bound")
            check_deadline(deadline)
            print(encoded, flush=True)
        return 0
    except (ActionError, QualificationError) as error:
        print("linux-performance-qualification-failed:" + str(error), file=sys.stderr)
        return 2
    except (OSError, ValueError, KeyError, TypeError, RecursionError, struct.error):
        print("linux-performance-qualification-failed", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
