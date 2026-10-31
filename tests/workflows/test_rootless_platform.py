"""Independent rootless identity/ownership negatives; no Docker or VM is used."""
import os
import json
from pathlib import Path
import socket
import tempfile
from types import SimpleNamespace
import unittest
from unittest.mock import Mock, patch

import rootless_platform as subject


NONCE = "0123456789abcdef" * 2


def report():
    return {"schemaVersion": 1, "platformQualification": "pending", "uid": 501, "gid": 1000,
            "namespaceSocketGid": 0,
            "socket": {"path": "/run/user/501/docker.sock", "uid": 501, "gid": 1000,
                       "mode": 0o660, "inode": 1234},
            "serviceMainPid": 2020, "dockerdPid": 2021,
            "uidMap": [[0, 501, 1], [1, 524288, 65536]],
            "gidMap": [[0, 1000, 1], [1, 524288, 65536]],
            "controllers": ["cpu", "cpuset", "io", "memory", "pids"],
            "rootless": {"id": "first-engine", "os": "linux", "arch": "aarch64", "version": "29.5.2",
                         "security": ["name=seccomp,profile=builtin", "name=rootless", "name=cgroupns"],
                         "cgroupVersion": "2", "cgroupDriver": "systemd", "warnings": None},
            "rootfulEngineId": "second-engine", "kernel": "6.8.0-117-generic",
            "referenceQualification": "pending", "scriptSha256": subject.script_hash(), "nonce": NONCE,
            "rootlessExtrasVersion": "5:29.5.2-1~ubuntu.24.04~noble",
            "delegation": {kind: {"declared": ["cpu", "cpuset", "io", "memory", "pids"],
                                  "available": ["cpu", "cpuset", "io", "memory", "pids"],
                                  "enabled": ["cpu", "memory", "pids"] if kind == "manager" else [],
                                  "controlUid": 501, "controlWritable": True} for kind in ("manager", "service")},
            "packages": {"uidmap": "1:4.13+dfsg1", "dbus-user-session": "1.14.10-4ubuntu4.1",
                         "slirp4netns": "1.2.1-1build2",
                         "docker-ce-rootless-extras": "5:29.5.2-1~ubuntu.24.04~noble",
                         "repositoryCandidate": "5:29.9.2-1~ubuntu.24.04~noble"}}


class RootlessPlatformControls(unittest.TestCase):
    def test_should_accept_literal_independent_identity_proof(self):
        value = report()
        self.assertEqual(subject.result_proof(value, NONCE), value)

    def test_should_reject_rootful_masquerade_and_missing_resource_controls(self):
        for key, replacement in (("security", ["name=seccomp"]), ("cgroupVersion", "1"),
                                 ("cgroupDriver", "none"), ("warnings", ["No memory limit support"]),
                                 ("arch", "x86_64"), ("os", "windows"), ("version", True)):
            changed = report()
            changed["rootless"][key] = replacement
            with self.subTest(key=key), self.assertRaises(ValueError):
                subject.result_proof(changed, NONCE)

    def test_should_reject_missing_unknown_or_duplicate_receipt_fields(self):
        for changed in ({}, dict(report(), unknown=1), {k: v for k, v in report().items() if k != "nonce"}):
            with self.assertRaises(ValueError):
                subject.result_proof(changed, NONCE)
        with self.assertRaises(ValueError):
            subject.document(b'{"schemaVersion":1,"schemaVersion":1}')

    def test_should_reject_forged_process_socket_and_qualification_claims(self):
        for key, value in (("uid", 0), ("uid", True), ("serviceMainPid", 0), ("dockerdPid", True),
                           ("nonce", "0" * 32), ("scriptSha256", "0" * 64),
                           ("platformQualification", "qualified"), ("referenceQualification", "qualified"),
                           ("rootfulEngineId", "first-engine"), ("kernel", "kernel\nforged"),
                           ("rootlessExtrasVersion", "5:29.9.2-1~ubuntu.24.04~noble")):
            changed = dict(report(), **{key: value})
            with self.subTest(key=key), self.assertRaises(ValueError):
                subject.result_proof(changed, NONCE)
        for key, value in (("uid", True), ("uid", 0), ("gid", True), ("mode", 0o666),
                           ("inode", 0), ("path", "/var/run/docker.sock")):
            changed = report()
            changed["socket"][key] = value
            with self.subTest(key=key), self.assertRaises(ValueError):
                subject.result_proof(changed, NONCE)

    def test_should_require_all_delegated_controllers_and_no_duplicates(self):
        for value in (["memory", "pids"], ["cpu", "memory"], ["cpu", "memory", "pids", "pids"],
                      ["cpu", "memory", True], ["pids", "memory", "cpu"]):
            changed = dict(report(), controllers=value)
            with self.assertRaises(ValueError):
                subject.result_proof(changed, NONCE)

    def test_should_reject_available_but_undelegated_or_unwritable_controllers(self):
        for kind, key, value in (("manager", "enabled", ["memory", "pids"]),
                                  ("manager", "declared", ["memory", "pids"]),
                                  ("service", "declared", ["memory", "pids"]),
                                  ("service", "controlUid", 0), ("manager", "controlWritable", False)):
            changed = report()
            changed["delegation"][kind][key] = value
            with self.subTest(kind=kind, key=key), self.assertRaises(ValueError):
                subject.result_proof(changed, NONCE)

    def test_should_reject_identity_maps_root_mappings_and_unbounded_ranges(self):
        for raw in (b"0 0 4294967295\n", b"0 0 1\n1 524288 65536\n", b"0 501 1\n1 5 65536\n",
                    b"0 501 1\n1 524288 65535\n", b"0 501 1\n1 4294967295 65536\n",
                    b"0 501 1\n1 524288 65536\n2 700000 1\n", b"0 x 1\n1 524288 65536\n"):
            with self.subTest(raw=raw), self.assertRaises(ValueError):
                subject.mappings(raw, 501)
        self.assertEqual(subject.mappings(b"0 501 1\n1 524288 65536\n", 501),
                         [[0, 501, 1], [1, 524288, 65536]])

    def test_should_reject_boolean_or_wrong_uid_and_gid_map_receipts(self):
        for key in ("uidMap", "gidMap"):
            for value in ([[False, 501, 1], [1, 524288, 65536]], [[0, 0, 1], [1, 524288, 65536]], []):
                changed = dict(report(), **{key: value})
                with self.subTest(key=key), self.assertRaises(ValueError):
                    subject.result_proof(changed, NONCE)

    def test_should_require_every_process_uid_to_be_unprivileged_owner(self):
        subject.process_uid(b"Name:\tdockerd\nUid:\t501\t501\t501\t501\n", 501)
        for raw in (b"Uid:\t0 0 0 0\n", b"Uid:\t501 0 501 501\n", b"Uid:\t501 501 501\n",
                    b"Uid:\t501 501 501 501\nUid:\t501 501 501 501\n", b"Name:\tdockerd\n"):
            with self.assertRaises(ValueError):
                subject.process_uid(raw, 501)

    def test_should_pin_key_primary_fingerprint_and_exact_engine_version(self):
        valid = b"pub:-:4096:1:key:0:0::-:::scSC:\nfpr:::::::::" + subject.KEY_FINGERPRINT.encode() + b":\n"
        subject.key_fingerprint(valid)
        for raw in (valid.replace(b"9DC8", b"0000"), valid + valid, b"fpr:::::::::" + subject.KEY_FINGERPRINT.encode()):
            with self.assertRaises(ValueError):
                subject.key_fingerprint(raw)
        self.assertEqual(subject.version(b"29.5.2\n"), "29.5.2")
        for raw in (b"29.5.2;unsafe", b"", b"29.5.2-dev", b"29.5"):
            with self.assertRaises(ValueError):
                subject.version(raw)

    def test_should_reject_mismatched_installed_package_and_bad_candidate(self):
        for key, value in (("docker-ce-rootless-extras", "5:29.9.2-1~ubuntu.24.04~noble"),
                           ("repositoryCandidate", "(none)"), ("uidmap", "bad\nline")):
            changed = report()
            changed["packages"][key] = value
            with self.assertRaises(ValueError):
                subject.result_proof(changed, NONCE)

    def test_should_bind_entry_to_profile_nonce_and_current_source(self):
        valid = {"schemaVersion": 1, "profile": "rustack-workflows-rootless-arm64", "nonce": NONCE,
                 "scriptSha256": subject.script_hash(), "targetDirectory": "/tmp/target"}
        subject.entry(valid, valid["profile"])
        for key, value in (("schemaVersion", True), ("profile", "rustack-workflows-arm64"),
                           ("nonce", "../unsafe"), ("scriptSha256", "0" * 64), ("targetDirectory", "/tmp/../target")):
            changed = dict(valid, **{key: value})
            with self.assertRaises(ValueError):
                subject.entry(changed, valid["profile"])

    def test_should_refuse_existing_vm_before_creating_evidence_or_dispatch(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            selected = "rustack-workflows-rootless-arm64"
            (root / ".colima" / selected).mkdir(parents=True)
            evidence = root / "evidence"
            owner = Mock()
            with patch.object(subject.Path, "home", return_value=root), \
                    patch.object(subject.sys, "platform", "darwin"), \
                    patch.object(subject.platform, "machine", return_value="arm64"):
                with self.assertRaisesRegex(ValueError, "existing-profile"):
                    subject.host("provision", selected, str(root), str(evidence), owner, 100)
            self.assertFalse(evidence.exists())
            owner.run.assert_not_called()

    def test_should_refuse_existing_guest_service_before_installing_packages(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            service = root / ".config/systemd/user/docker.service"
            service.parent.mkdir(parents=True)
            service.write_text("unrelated unit")
            owner = Mock()
            with patch.object(subject.Path, "home", return_value=root), \
                    patch.object(subject, "guest_base", return_value=(501, root / "run", "tchen")):
                with self.assertRaisesRegex(ValueError, "rootless-guest-not-fresh"):
                    subject.guest_setup(NONCE, owner, 100)
            self.assertEqual(service.read_text(), "unrelated unit")
            owner.run.assert_not_called()

    def test_should_keep_raw_command_receipts_and_refuse_nonzero_or_stderr(self):
        with tempfile.TemporaryDirectory() as temporary:
            owner = Mock()
            owner.run.return_value = SimpleNamespace(code=0, stdout=b"answer", stderr=b"")
            command = subject.Commands(owner, 12345, Path(temporary))
            with patch.object(subject.time, "monotonic", return_value=12300):
                self.assertEqual(command.run(["fixed", "argument"], 10, clean=True), b"answer")
            owner.run.assert_called_once_with(["fixed", "argument"], 12310, stdout_limit=65536)
            self.assertEqual((Path(temporary) / "command-001.stdout").read_bytes(), b"answer")
            owner.run.return_value = SimpleNamespace(code=0, stdout=b"", stderr=b"warning")
            with self.assertRaises(ValueError):
                command.run(["fixed"], clean=True)
            owner.run.return_value = SimpleNamespace(code=1, stdout=b"", stderr=b"")
            with self.assertRaises(ValueError):
                command.run(["fixed"])

    def test_should_independently_bound_privileged_service_cgroup_before_caller_cutoff(self):
        owner = Mock()
        owner.run.return_value = SimpleNamespace(code=0, stdout=b"", stderr=b"")
        temporary = tempfile.TemporaryDirectory()
        self.addCleanup(temporary.cleanup)
        command = subject.Commands(owner, 1000, Path(temporary.name))
        with patch.object(subject.time, "monotonic", return_value=500):
            command.run(["sudo", "-n", "env", "DEBIAN_FRONTEND=noninteractive", "apt-get", "update"], 300)
        arguments, cutoff = owner.run.call_args[0]
        self.assertEqual(cutoff, 800)
        self.assertEqual(arguments[:7], ["sudo", "-n", "systemd-run", "--quiet", "--wait", "--pipe", "--collect"])
        for setting in ("RuntimeMaxSec=294s", "TimeoutStopSec=2s", "TimeoutStartSec=2s",
                        "KillMode=control-group", "SendSIGKILL=yes", "Restart=no"):
            self.assertIn("--property=" + setting, arguments)
        self.assertEqual(arguments[-4:], ["/usr/bin/env", "DEBIAN_FRONTEND=noninteractive", "apt-get", "update"])
        with patch.object(subject.time, "monotonic", return_value=995):
            with self.assertRaisesRegex(ValueError, "rootless-privileged-command-budget"):
                command.run(["sudo", "-n", "env", "apt-get", "update"], 300)

    def test_should_persist_exact_privileged_intent_before_dispatch_and_preserve_on_lost_result(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            def interrupted(arguments, *_args, **_keywords):
                intent = subject.document((root / "command-001.intent.json").read_bytes())
                self.assertEqual(intent["argv"], arguments)
                self.assertEqual(intent["ownerPid"], os.getpid())
                self.assertTrue(any(item.startswith("--unit=rustack-d103-") for item in arguments))
                raise OSError("lost SSH observation")
            owner = Mock()
            owner.run.side_effect = interrupted
            command = subject.Commands(owner, 1000, root)
            with patch.object(subject.time, "monotonic", return_value=500):
                with self.assertRaises(OSError):
                    command.run(["sudo", "-n", "install", "--version"])
            self.assertTrue((root / "command-001.intent.json").is_file())
            self.assertFalse((root / "command-001.json").exists())
            self.assertEqual((root / "command-001.intent.json").stat().st_mode & 0o777, 0o600)

    def test_should_refuse_privileged_effect_when_intent_io_consumes_retirement_slack(self):
        with tempfile.TemporaryDirectory() as temporary:
            owner = Mock()
            command = subject.Commands(owner, 1000, Path(temporary))
            clock = [500]
            publish = subject.publish
            def delayed(path, value):
                publish(path, value)
                clock[0] += 2
            with patch.object(subject.time, "monotonic", side_effect=lambda: clock[0]), \
                    patch.object(subject, "publish", side_effect=delayed):
                with self.assertRaisesRegex(ValueError, "rootless-privileged-intent-late"):
                    command.run(["sudo", "-n", "install", "--version"])
            owner.run.assert_not_called()

    def test_should_refuse_fresh_setup_resumption_after_any_guest_entry(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            state = root / ".local/state/rustack-rootless-d103"
            state.mkdir(parents=True)
            with patch.object(subject.Path, "home", return_value=root), \
                    patch.object(subject, "guest_base", return_value=(501, root / "run", "tchen")):
                with self.assertRaisesRegex(ValueError, "rootless-guest-not-fresh"):
                    subject.guest_fresh(NONCE)

    def test_should_reject_replacing_system_key_or_repository_file(self):
        with tempfile.TemporaryDirectory() as temporary:
            source, target = Path(temporary) / "source", Path(temporary) / "target"
            source.write_bytes(b"expected")
            target.write_bytes(b"unrelated")
            commands = Mock()
            with self.assertRaisesRegex(ValueError, "rootless-system-file-conflict"):
                subject.install_file(commands, source, target, "0644")
            self.assertEqual(target.read_bytes(), b"unrelated")
            commands.run.assert_not_called()

    def test_should_reuse_verified_canonical_repo_and_reject_conflicting_sources(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            sources = root / "sources"
            sources.mkdir()
            main, key = root / "main", root / "docker.asc"
            key.write_bytes(b"public key")
            key.chmod(0o644)
            canonical = ("deb [arch=arm64 signed-by=" + str(key)
                         + "] https://download.docker.com/linux/ubuntu noble stable\n")
            source = sources / "docker.list"
            source.write_text(canonical)
            source.chmod(0o644)
            commands = Mock()
            commands.run.return_value = (b"pub:-:4096:1:key:0:0::-:::scSC:\nfpr:::::::::"
                                         + subject.KEY_FINGERPRINT.encode() + b":\n")
            original = os.lstat
            def root_metadata(*arguments, **keywords):
                value = original(*arguments, **keywords)
                return SimpleNamespace(st_mode=value.st_mode, st_uid=0, st_size=value.st_size)
            with patch.object(os, "lstat", side_effect=root_metadata):
                self.assertTrue(subject.existing_repository(commands, sources, main, key))
                duplicate = sources / "duplicate.list"
                duplicate.write_text(canonical)
                duplicate.chmod(0o644)
                with self.assertRaisesRegex(ValueError, "rootless-existing-source-shape"):
                    subject.existing_repository(commands, sources, main, key)
                duplicate.unlink()
                source.write_text(canonical.replace("noble stable", "noble test"))
                with self.assertRaisesRegex(ValueError, "rootless-existing-source-shape"):
                    subject.existing_repository(commands, sources, main, key)

    def test_should_preserve_concurrent_publication_and_remove_only_private_temporary(self):
        # Exercise the actual publication algorithm in a local directory. Only
        # the fixed destination and root UID metadata are adapted for this test.
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            root.chmod(0o755)
            source, target = root / "source", root / "target"
            source.write_bytes(b"expected")
            code = subject.SYSTEM_FILE_PUBLISHER.replace("/etc/apt/keyrings/rustack-d103-docker.asc", str(target))
            original_lstat, original_fstat = os.lstat, os.fstat
            def root_metadata(value):
                return SimpleNamespace(st_mode=value.st_mode, st_uid=0, st_size=value.st_size)
            def concurrent_link(*_arguments, **_keywords):
                target.write_bytes(b"other publisher")
                raise FileExistsError()
            arguments = ["publisher", str(source), str(target),
                         subject.hashlib.sha256(source.read_bytes()).hexdigest(), "0"]
            with patch.object(subject.sys, "argv", arguments), \
                    patch.object(os, "lstat", side_effect=lambda *a, **k: root_metadata(original_lstat(*a, **k))), \
                    patch.object(os, "fstat", side_effect=lambda *a, **k: root_metadata(original_fstat(*a, **k))), \
                    patch.object(os, "link", side_effect=concurrent_link):
                with self.assertRaises(SystemExit) as error:
                    exec(compile(code, "rootless-system-file-publisher", "exec"), {})
            self.assertEqual(error.exception.code, 2)
            self.assertEqual(target.read_bytes(), b"other publisher")
            self.assertEqual(sorted(path.name for path in root.iterdir()), ["source", "target"])

    def test_should_require_private_actual_socket_and_reject_symlink_or_world_access(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            endpoint = root / "docker.sock"
            with socket.socket(socket.AF_UNIX) as channel:
                channel.bind(str(endpoint))
                endpoint.chmod(0o600)
                self.assertEqual(subject.socket_proof(endpoint, os.getuid())["mode"], 0o600)
                endpoint.chmod(0o666)
                with self.assertRaises(ValueError):
                    subject.socket_proof(endpoint, os.getuid())
                endpoint.chmod(0o600)
                alias = root / "alias"
                alias.symlink_to(endpoint)
                with self.assertRaises(ValueError):
                    subject.socket_proof(alias, os.getuid())

    def test_should_preserve_sticky_socket_mode_and_map_daemon_group_through_subgids(self):
        value = report()
        value["socket"].update(gid=525278, mode=0o1660)
        value["namespaceSocketGid"] = 991
        self.assertEqual(subject.result_proof(value, NONCE), value)
        value["namespaceSocketGid"] = 525278
        with self.assertRaisesRegex(ValueError, "rootless-socket-gid-map"):
            subject.result_proof(value, NONCE)

    def test_should_recover_only_complete_joined_install_proof_and_reject_missing_raw_or_live_job(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            state = root / ".local/state/rustack-rootless-d103"
            state.mkdir(parents=True, mode=0o700)
            historical = b"historical bootstrap source\n"
            owner_hash = subject.hashlib.sha256(historical).hexdigest()
            subject.write_new(state / "source.py", historical)
            subject.publish(state / "entry.json", {"nonce": NONCE, "scriptSha256": owner_hash, "uid": 501})
            subject.publish(state / "failure.json", {"nonce": NONCE, "outcome": "failed-profile-retained"})
            value = report()
            unit = "rustack-d103-" + "a" * 32 + "-001"
            data = [( ["sudo", "-n", "systemd-run", "--unit=" + unit, "/usr/bin/env", "apt-get", "update"], b""),
                    (["env", "DBUS_SESSION_BUS_ADDRESS=unix:path=/run/user/501/bus", "dockerd-rootless-setuptool.sh", "install", "--force"], b""),
                    (["docker", "--host", "unix:///run/user/501/docker.sock", "info", "--format", "fixed"], json.dumps(value["rootless"]).encode()),
                    (["docker", "--host", "unix:///run/user/501/docker.sock", "info", "--format", "fixed"], json.dumps(value["rootless"]).encode()),
                    (["docker", "--host", "unix:///var/run/docker.sock", "info", "--format", "{{.ID}}"], b"second-engine\n")]
            for number, (arguments, raw) in enumerate(data, 1):
                base = state / ("command-%03d" % number)
                subject.publish(state / (base.name + ".intent.json"),
                                {"argv": arguments, "callerCutoffMonotonic": 1000, "ownerPid": 999})
                subject.publish(base.with_suffix(".json"), {"argv": arguments, "code": 0,
                    "stdoutSha256": subject.hashlib.sha256(raw).hexdigest(),
                    "stderrSha256": subject.hashlib.sha256(b"").hexdigest()})
                subject.write_new(base.with_suffix(".stdout"), raw)
                subject.write_new(base.with_suffix(".stderr"), b"")
            commands = Mock()
            commands.run.return_value = b""
            original_lstat = os.lstat
            def metadata(*arguments, **keywords):
                stat = original_lstat(*arguments, **keywords)
                return SimpleNamespace(st_mode=stat.st_mode, st_uid=501, st_size=stat.st_size)
            with patch.object(subject.Path, "home", return_value=root), \
                    patch.object(subject, "guest_base", return_value=(501, Path("/run/user/501"), "tchen")), \
                    patch.object(os, "lstat", side_effect=metadata), \
                    patch.object(subject, "guest_attest", side_effect=lambda _commands: report()):
                self.assertEqual(subject.guest_recover(NONCE, owner_hash, commands), report())
                commands.run.return_value = b"exact service still active\n"
                with self.assertRaisesRegex(ValueError, "rootless-recovery-job-not-retired"):
                    subject.guest_recover(NONCE, owner_hash, commands)
                commands.run.return_value = b""
                raw_path = state / "command-001.stdout"
                raw_path.write_bytes(b"changed")
                with self.assertRaisesRegex(ValueError, "rootless-recovery-raw-command"):
                    subject.guest_recover(NONCE, owner_hash, commands)
                raw_path.write_bytes(b"")
                (state / "command-001.intent.json").unlink()
                with self.assertRaisesRegex(ValueError, "rootless-recovery-command-sequence"):
                    subject.guest_recover(NONCE, owner_hash, commands)


if __name__ == "__main__":
    unittest.main()
