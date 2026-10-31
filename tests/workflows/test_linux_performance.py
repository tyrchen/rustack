"""Literal independent controls for Linux artifact, workload and cutoff evidence."""
import copy
import json
import os
from pathlib import Path
import signal
import struct
import subprocess
import sys
import tempfile
import threading
import time
import unittest
from unittest.mock import patch

import linux_performance as subject


def sample():
    return {"count": 100, "observedMicros": list(range(100, 0, -1)),
            "p50Micros": 50, "p95Micros": 95, "maxMicros": 100}


def report(kind="pureLatency"):
    measured = {"kind": "pureLatency", "stateCount": 50, "inputBytes": 1024,
                "executions": 100, "budgetMicros": 100000, "latency": sample()}
    if kind == "loopResponsiveness":
        measured = {"kind": kind, "openExecutionSlots": 1, "completedCycles": 100,
                    "inputBytes": 1024, "historyEventHighWater": 1000,
                    "exhaustion": {"probeCount": 10, "liveProbeCount": 9, "maxReadinessMicros": 100},
                    "readiness": sample(), "stop": sample()}
    return {"schemaVersion": 1, "host": {"os": "linux", "architecture": "aarch64",
            "availableLogicalCpus": 4, "debugAssertions": True, "referenceQualification": "pending"},
            "limits": {"openExecutions": 1, "transitions": 1000000, "historyEvents": 25000,
                       "historyBytes": 33554432, "totalHistoryBytes": 134217728},
            "nativeProcessesObserved": 0, "serverJoined": True, "measurement": measured}


def output(value, inline=False):
    prefix = "test performance::case ... " if inline else ""
    return (prefix + "WORKFLOWS_PERFORMANCE " + json.dumps(value) + "\n"
            "test result: ok. 1 passed; 0 failed; 0 ignored; 0 measured; 7 filtered out; finished in 1.00s\n").encode()


def cargo_records():
    base = "/tmp/target/aarch64-unknown-linux-gnu/debug/"
    return [{"reason": "compiler-artifact", "target": {"name": "rustack", "kind": ["bin"]},
             "profile": {"test": False}, "executable": base + "rustack"},
            {"reason": "compiler-artifact", "target": {"name": "workflows_sdk", "kind": ["test"]},
             "profile": {"test": True}, "executable": base + "deps/workflows_sdk-0123456789abcdef"},
            {"reason": "build-finished", "success": True}]


def cargo_stream(records):
    return b"\n".join(json.dumps(row).encode() for row in records)


class LinuxPerformanceControls(unittest.TestCase):
    def test_should_accept_literal_workloads_and_inline_rust_output(self):
        for kind in ("pureLatency", "loopResponsiveness"):
            for inline in (False, True):
                value = report(kind)
                self.assertEqual(subject.measurement(output(value, inline), kind), value)

    def test_should_reject_zero_test_success_duplicate_or_wrong_reports(self):
        raw = output(report())
        for changed in (raw.replace(b"1 passed", b"0 passed"),
                        raw.replace(b"0 ignored", b"1 ignored"),
                        raw + raw.splitlines()[0] + b"\n", raw.splitlines()[-1],
                        raw.replace(b"pureLatency", b"loopResponsiveness"),
                        raw.replace(b'"schemaVersion": 1', b'"schemaVersion": 1,"schemaVersion": 1')):
            with self.assertRaises(ValueError):
                subject.measurement(changed, "pureLatency")

    def test_should_reject_platform_limits_cleanup_and_reference_claim_mutations(self):
        for section, key, value in (("host", "os", "macos"), ("host", "architecture", "x86_64"),
                ("host", "availableLogicalCpus", 8), ("host", "debugAssertions", False),
                ("host", "referenceQualification", "qualified"), ("limits", "historyBytes", 1),
                (None, "nativeProcessesObserved", 1), (None, "serverJoined", False),
                (None, "schemaVersion", True)):
            changed = report()
            (changed[section] if section else changed)[key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                subject.measurement(output(changed), "pureLatency")

    def test_should_recompute_ranks_and_enforce_budget_without_losing_order(self):
        for key, value in (("count", 99), ("p95Micros", 96), ("observedMicros", [1] * 99),
                           ("observedMicros", [True] * 100), ("maxMicros", 99)):
            changed = report()
            changed["measurement"]["latency"][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                subject.measurement(output(changed), "pureLatency")
        changed = report()
        changed["measurement"]["latency"] = {"count": 100, "observedMicros": [100001] * 100,
                                             "p50Micros": 100001, "p95Micros": 100001, "maxMicros": 100001}
        with self.assertRaisesRegex(ValueError, "latency-budget"):
            subject.measurement(output(changed), "pureLatency")

    def test_should_reject_nonlive_exhaustion_and_two_second_health_results(self):
        original = report("loopResponsiveness")
        for key, value in (("liveProbeCount", 0), ("liveProbeCount", 11),
                           ("probeCount", True), ("maxReadinessMicros", 2000000)):
            changed = copy.deepcopy(original)
            changed["measurement"]["exhaustion"][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                subject.measurement(output(changed), "loopResponsiveness")

    def test_should_require_exact_complete_unique_inventory(self):
        names = subject.CASES + subject.CONTROLS
        raw = ("\n".join(name + ": test" for name in names) + "\n").encode()
        subject.inventory(raw)
        for changed in (raw.replace(names[0].encode(), b"unknown"), raw + raw,
                        raw.replace(names[0].encode(), names[1].encode()), raw[:-1] + b"extra"):
            with self.assertRaises(ValueError):
                subject.inventory(changed)

    def test_should_bind_one_test_and_its_actual_sibling_server_artifact(self):
        records = cargo_records()
        test, server = subject.artifact(cargo_stream(records))
        self.assertEqual(test.parent.parent / "rustack", server)
        mutations = [records[:-1], [*records[:-1], records[1], records[-1]], records[1:]]
        for change in ("target", "test", "path", "success"):
            changed = copy.deepcopy(records)
            if change == "target":
                changed[1]["target"]["kind"] = ["bin"]
            elif change == "test":
                changed[1]["profile"]["test"] = False
            elif change == "path":
                changed[1]["executable"] = str(test).replace("aarch64", "x86_64")
            else:
                changed[-1]["success"] = False
            mutations.append(changed)
        for changed in mutations:
            with self.assertRaises(ValueError):
                subject.artifact(cargo_stream(changed))

    def test_should_bound_cargo_record_count_before_decoding(self):
        records = [{"reason": "build-script-executed"}] * 4093 + cargo_records()
        subject.artifact(cargo_stream(records))
        with patch.object(subject, "document", side_effect=AssertionError("decoded before admission")):
            with self.assertRaisesRegex(ValueError, "cargo-record-budget"):
                subject.artifact(b"{}\n" * 4097)

    def test_should_select_only_an_explicit_allowlisted_linux_test_target(self):
        records = cargo_records()
        records[1]["target"]["name"] = "docker_startup"
        records[1]["executable"] = records[1]["executable"].replace("workflows_sdk", "docker_startup")
        selected, _ = subject.artifact(cargo_stream(records), "docker_startup")
        self.assertTrue(selected.name.startswith("docker_startup-"))
        for name in ("workflows_sdk", "untrusted", "docker_startup;id"):
            with self.assertRaises(ValueError): subject.artifact(cargo_stream(records), name)

    def test_should_reject_remote_shell_paths_profiles_and_document_expansion(self):
        for path in ("relative", "/tmp/a b", "/tmp/../test", "/tmp//test", "/tmp/$(id)",
                     "/tmp/a\n", "/tmp/" + "a" * 4096):
            with self.assertRaises(ValueError):
                subject.safe_path(path)
        for selected in ("default", "rustack-workflows-x;id", "rustack-workflows-" + "x" * 41):
            with self.assertRaises(ValueError):
                subject.profile(selected)
        for raw in (b'{"x":1,"x":2}', b'{"x":NaN}', b"x" * 65537,
                    b"[" * 1000 + b"0" + b"]" * 1000,
                    b'{"x":"' + b"x" * 16385 + b'"}', b"[" + b"0," * 512 + b"0]"):
            with self.assertRaises(ValueError):
                subject.document(raw)

    def test_should_reject_wrong_elf_symlinks_and_changed_hashes(self):
        with tempfile.TemporaryDirectory() as root:
            path = Path(root) / "test"
            elf = bytearray(64)
            elf[:6] = b"\x7fELF\x02\x01"
            struct.pack_into("<H", elf, 18, 183)
            path.write_bytes(elf)
            original = subject.fingerprint(path)
            elf[-1] = 1
            path.write_bytes(elf)
            self.assertNotEqual(subject.fingerprint(path), original)
            struct.pack_into("<H", elf, 18, 62)
            path.write_bytes(elf)
            with self.assertRaises(ValueError):
                subject.fingerprint(path)
            link = Path(root) / "link"
            link.symlink_to(path)
            with self.assertRaises(OSError):
                subject.fingerprint(link)
            fifo = Path(root) / "fifo"
            os.mkfifo(fifo)
            with self.assertRaises(ValueError):
                subject.bounded_read(fifo, 100)

    def test_should_reject_late_preflight_and_enforce_original_io_alarm(self):
        with patch.object(subject.time, "monotonic", return_value=100):
            with self.assertRaisesRegex(ValueError, "qualification-deadline"):
                subject.check_deadline(100)
        before = signal.getsignal(signal.SIGALRM)
        with self.assertRaisesRegex(ValueError, "qualification-deadline"):
            with subject.deadline_guard(0.01):
                # A blocked local observation must not escape the guest cutoff.
                signal.pause()
        self.assertEqual(signal.getsignal(signal.SIGALRM), before)
        self.assertEqual(signal.getitimer(signal.ITIMER_REAL), (0, 0))

    def test_should_join_held_child_when_alarm_arrives_during_retirement(self):
        with tempfile.TemporaryDirectory() as root:
            pidfile = Path(root) / "pid"
            script = ("import os,signal,time,pathlib;signal.signal(signal.SIGTERM,signal.SIG_IGN);"
                      "pathlib.Path(" + repr(str(pidfile)) + ").write_text(str(os.getpid()));time.sleep(10)")
            state = {"entered": None, "alarmDelivered": False}
            def clock():
                return 0 if state["entered"] is None else time.monotonic() - state["entered"]
            owner = subject.Runner({"PATH": "/usr/bin:/bin"}, clock=clock)
            def deliver_during_retirement():
                cutoff = time.monotonic() + 4
                while not pidfile.exists() and time.monotonic() < cutoff:
                    time.sleep(0.005)
                if not pidfile.exists():
                    return
                state["entered"] = time.monotonic()
                while not owner.retiring and time.monotonic() < cutoff:
                    time.sleep(0.005)
                if owner.retiring:
                    state["alarmDelivered"] = True
                    os.kill(os.getpid(), signal.SIGALRM)
            sender = threading.Thread(target=deliver_during_retirement)
            with subject.deadline_guard(9, owner):
                sender.start()
                try:
                    with self.assertRaises(subject.ActionError):
                        owner.run([sys.executable, "-I", "-B", "-c", script], 0.15)
                finally:
                    sender.join(timeout=5)
                self.assertFalse(sender.is_alive())
                self.assertTrue(state["alarmDelivered"])
            pid = int(pidfile.read_text())
            self.assertFalse(owner.unsettled)
            for observe in (os.kill, os.killpg):
                with self.assertRaises(ProcessLookupError):
                    observe(pid, 0)

    def test_should_join_actual_child_groups_on_owner_term_hup_and_int(self):
        for interruption in (signal.SIGTERM, signal.SIGHUP, signal.SIGINT):
            with self.subTest(interruption=interruption), tempfile.TemporaryDirectory() as root:
                pidfile = Path(root) / "pid"
                child_code = ("import os,time,pathlib;pathlib.Path(" + repr(str(pidfile))
                              + ").write_text(str(os.getpid()));time.sleep(10)")
                owner_code = ("import sys,time;sys.path.insert(0," + repr(str(Path(subject.__file__).parent))
                              + ");import linux_performance as m;owner=m.Runner({'PATH':'/usr/bin:/bin'})\n"
                              "try:\n with m.deadline_guard(9,owner):\n  owner.run("
                              + repr([sys.executable, "-I", "-B", "-c", child_code])
                              + ",time.monotonic()+8)\nexcept (m.ActionError,m.QualificationError):sys.exit(2)\n")
                parent = subprocess.Popen([sys.executable, "-I", "-B", "-c", owner_code],
                                          stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                                          start_new_session=True)
                child_pid = None
                try:
                    cutoff = time.monotonic() + 3
                    while not pidfile.exists() and time.monotonic() < cutoff and parent.poll() is None:
                        time.sleep(0.005)
                    self.assertTrue(pidfile.exists(), "child never entered")
                    child_pid = int(pidfile.read_text())
                    os.kill(parent.pid, interruption)
                    stdout, stderr = parent.communicate(timeout=5)
                    self.assertEqual((parent.returncode, stdout, stderr), (2, b"", b""))
                    for observe in (os.kill, os.killpg):
                        with self.assertRaises(ProcessLookupError):
                            observe(child_pid, 0)
                finally:
                    if parent.poll() is None:
                        os.killpg(parent.pid, signal.SIGKILL)
                    parent.communicate(timeout=5)
                    if child_pid is not None:
                        try:
                            os.killpg(child_pid, signal.SIGKILL)
                        except ProcessLookupError:
                            pass


if __name__ == "__main__":
    unittest.main()
