"""Rootless resource guard controls without Docker, SSH or Cargo execution."""
import copy
import os
from pathlib import Path
import socket
import tempfile
import unittest
from unittest.mock import Mock, patch
from types import SimpleNamespace

import rootless_resources as subject
import rootless_functional as functional


ENGINE = "7bbe02f2-0571-42f9-a4a3-a36a81425357"
NONCE = "a" * 32


def network():
    return [{"Id": "f" * 64, "Name": "rustack-rootless-d104-" + NONCE[:12], "Driver": "bridge",
             "Scope": "local", "Internal": False, "Labels": {subject.LABEL: NONCE}, "Containers": {},
             "IPAM": {"Driver": "default", "Config": [{"Subnet": "172.18.0.0/16", "Gateway": "172.18.0.1"}]}}]


def images():
    rows = []
    for index, name in enumerate(subject.SOURCES):
        identity = "sha256:" + str(index) * 64
        rows.append({"name": name, "image": {"id": identity, "os": "linux", "architecture": "arm64", "bytes": 100},
                     "archive": "/tmp/image-%d.tar" % index, "bytes": 1024, "sha256": "c" * 64,
                     "cacheAlias": "rustack-workflows-cache:d102-" + str(index) * 64})
    return {"engine": ENGINE, "sourceEngine": "56b0fabd-452d-46a5-ac75-5b4063ab4061",
            "sourceSocket": "/tmp/source.sock", "sourceIds": ["sha256:" + "0" * 64], "images": rows, "selection": "kernel"}


def output(names):
    return ("\n".join("test " + name + " ... ok" for name in names)
            + "\ntest result: ok. 2 passed; 0 failed; 0 ignored; 0 measured; 0 filtered out; finished in 0.12s\n").encode()


class RootlessResourceControls(unittest.TestCase):
    def test_should_promote_only_the_exact_receipted_old_target_and_preserve_foreign_tags(self):
        old, new, foreign = ("sha256:" + character * 64 for character in "abc")
        self.assertTrue(subject.tag_policy((old + "\n").encode(), new, old))
        self.assertFalse(subject.tag_policy((new + "\n").encode(), new, old))
        self.assertTrue(subject.tag_policy(b"", new))
        for present, previous in ((b"", old), ((foreign + "\n").encode(), old), ((old + "\n").encode(), None)):
            with self.assertRaises(ValueError): subject.tag_policy(present, new, previous)

    def test_should_reject_reused_forward_pid_or_different_process_owner(self):
        expected = {"start": "Sat Oct 31 15:00:00 2026", "uid": os.getuid()}
        subject.require_process_identity(expected, expected.copy())
        with self.assertRaises(ValueError):
            subject.require_process_identity(expected, dict(expected, start="Sat Oct 31 15:01:00 2026"))
        with self.assertRaises(ValueError):
            subject.require_process_identity(expected, dict(expected, uid=os.getuid() + 1))

    def test_should_charge_delayed_forward_observation_to_its_original_cutoff(self):
        self.assertEqual(functional.observation_budget(100, 2700, 110), 2690)
        with self.assertRaises(ValueError):
            functional.observation_budget(100, 1205, 110)

    def test_should_detect_changed_python_consumer_and_local_artifact(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            consumer = root / "tests/workflows/consumer.py"
            consumer.parent.mkdir(parents=True)
            consumer.write_bytes(b"first consumer")
            with patch.object(functional, "ROOT", root), patch.object(functional, "source_paths", return_value={consumer}):
                before = functional.source_seal(Mock(), 1000)
                consumer.write_bytes(b"changed consumer")
                self.assertNotEqual(functional.source_seal(Mock(), 1000), before)
            for part in ("provided/provided-a.zip", "provided/provided-b.zip", "zip-code-bootstrap/bootstrap",
                         "native-verifier/rustack-lambda-runtime-verifier/bootstrap", "sdk-python"):
                path = root / part
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(b"original artifact")
            before = functional.artifact_seal(root, root / "sdk-python")
            (root / "provided/provided-a.zip").write_bytes(b"changed artifact")
            self.assertNotEqual(functional.artifact_seal(root, root / "sdk-python"), before)

    def test_should_retain_large_source_inventory_in_bounded_complete_shards(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            values = {"crates/example/src/file_%04d.rs" % index: "%064x" % index for index in range(1000)}
            proof = functional.retain_source_seal(root, values)
            restored = {}
            for name in proof["sourceSealFiles"]:
                raw = (root / name).read_bytes()
                self.assertLessEqual(len(raw), 65536)
                restored.update(subject.document(raw))
            self.assertEqual(restored, values)
            self.assertEqual(len(proof["sourceSealFiles"]), 8)

    def test_should_use_kernel_elapsed_age_without_comparing_process_local_monotonic_epochs(self):
        self.assertEqual(functional.forward_remaining(b" 00:10\n"), 3587)
        self.assertEqual(functional.forward_remaining(b" 10:00\n"), 2997)
        for raw in (b"01:00:00", b"39:57", b"00:60", b"bad", b"1-00:00:00"):
            with self.assertRaises(ValueError):
                functional.forward_remaining(raw)

    def test_should_preserve_failed_resource_exit_and_raw_streams_after_join(self):
        with tempfile.TemporaryDirectory() as temporary:
            owner = Mock()
            owner.run.return_value = SimpleNamespace(code=101, stdout=b"2 failed", stderr=b"CPU mismatch")
            root = Path(temporary)
            result = functional.run_case(owner, ["cargo", "test", "--exact", "fixed"], 1000, root)
            self.assertEqual(result.code, 101)
            self.assertEqual((root / "stdout").read_bytes(), b"2 failed")
            self.assertEqual((root / "stderr").read_bytes(), b"CPU mismatch")
            self.assertEqual(subject.document((root / "result.json").read_bytes())["code"], 101)

    def test_should_use_mapped_server_gid_and_only_fixed_fixture_environment(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for part in ("provided/provided-a.zip", "provided/provided-b.zip", "zip-code-bootstrap/bootstrap",
                         "native-verifier/rustack-lambda-runtime-verifier/bootstrap", "sdk-python"):
                path = root / part
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_bytes(b"fixture")
            proof = {"socket": {"path": "/run/user/501/docker.sock", "gid": 525278}, "namespaceSocketGid": 991}
            with patch.object(functional.shutil, "which", return_value="/usr/bin/docker"):
                value = functional.environment(proof, "unix:///tmp/owned.sock", {"name": "owned-bridge"},
                                               root, root / "sdk-python", Path("/tmp/other.sock"))
            self.assertEqual(value["RUSTACK_SERVER_SOCKET_GID"], "991")
            self.assertEqual(value["RUSTACK_SERVER_DAEMON_SOCKET"], "/run/user/501/docker.sock")
            self.assertEqual(value["RUSTACK_ZIP_BASE_IMAGE"], subject.FULL_SOURCES[7])
            self.assertEqual(value["RUSTACK_PULL_IMAGE"], subject.FULL_SOURCES[8])
            self.assertNotIn("AWS_SECRET_ACCESS_KEY", value)

    def test_should_bind_every_manifest_suite_to_fixed_source_owned_cargo_target(self):
        required = functional.parse_manifest((Path(__file__).parent / "docker-required-suites.json").read_bytes())
        self.assertEqual(set(functional.SUITES), set(required))
        self.assertEqual(sum(len(names) for names in required.values()), 83)
        self.assertEqual(functional.SUITES["docker-probe"], ["-p", "rustack-lambda-core", "--test", "docker_probe"])

    def test_should_reject_private_aliases_in_all_default_hub_spellings(self):
        subject.private_aliases(b"rustack-workflows-cache:d102-abc\n<none>:<none>\n")
        for prefix in ("", "docker.io/", "index.docker.io/", "docker.io/library/", "index.docker.io/library/", "library/"):
            for name in ("rustack-internal/lambda-install:pin", "rustack-test-source:fixture"):
                with self.subTest(prefix=prefix, name=name), self.assertRaises(ValueError):
                    subject.private_aliases((prefix + name + "\n").encode())

    def test_should_require_exact_full_fixture_set_and_only_declared_negative_platforms(self):
        value = images()
        value.update(selection="full", images=[])
        self.assertEqual(len(set(subject.FULL_SOURCES)), 15)
        for index, name in enumerate(subject.FULL_SOURCES):
            row = copy.deepcopy(images()["images"][0])
            row.update(name=name, archive="/tmp/image-%d.tar" % index)
            row["image"]["id"] = "sha256:" + ("%064x" % index)
            row["cacheAlias"] = "rustack-workflows-cache:d102-" + ("%064x" % index)
            if name == "rustack-invalid-os:arm64":
                row["image"].update(os="windows", architecture="amd64")
            if name == "rustack-invalid-architecture:arm64":
                row["image"].update(architecture="amd64")
            value["images"].append(row)
        self.assertEqual(subject.image_receipt(value, ENGINE), value)
        value["images"][0]["image"]["os"] = "windows"
        with self.assertRaises(ValueError):
            subject.image_receipt(value, ENGINE)

    def test_should_accept_exact_distinct_success_rows_and_reject_forged_or_ignored_results(self):
        names = {"probe::case_1_disconnected", "probe::case_2_operator_bridge"}
        raw = output(sorted(names))
        subject.verify_output(raw, names)
        subject.verify_output(raw.replace(b" ... ok", b" ... inherited SDK output\nok"), names)
        for changed in (raw.replace(b"2 passed", b"0 passed"), raw.replace(b"0 ignored", b"1 ignored"),
                        raw.replace(b"case_1_disconnected", b"case_2_operator_bridge"),
                        raw.replace(b"case_1_disconnected", b"case_3_extra"), raw + raw,
                        raw.replace(b" ... ok", b" ... ignored"),
                        raw + b"test unrelated ... FAILED\n", raw.replace(b"test probe::case_1", b"probe::case_1")):
            with self.assertRaises(ValueError):
                subject.verify_output(changed, names)

    def test_should_observe_cleanup_even_when_a_successful_child_transcript_is_rejected(self):
        from functional_inventory import execute
        owner, observe = Mock(), Mock()
        run = Mock(side_effect=[SimpleNamespace(code=0, stdout=b"sample: test\n1 test, 0 benchmarks\n"),
                                SimpleNamespace(code=0, stdout=b"malformed transcript")])
        verify = Mock(side_effect=ValueError("invalid transcript"))
        with tempfile.TemporaryDirectory() as temporary:
            with self.assertRaisesRegex(ValueError, "invalid transcript"):
                execute(owner, ["cargo"], {"suite":[]}, {"suite":{"sample"}}, {"suite":{"sample"}},
                        Path(temporary), 100, run, verify, observe)
            observe.assert_called_once_with(Path(temporary) / "case-001")

    def test_should_accept_owned_empty_bridge_and_reject_foreign_or_attached_network(self):
        value = network()
        name = value[0]["Name"]
        self.assertEqual(subject.network(value, name, NONCE), value[0])
        for key, replacement in (("Labels", {}), ("Id", "f" * 63), ("Name", "foreign"),
                                 ("Driver", "host"), ("Scope", "swarm"), ("Internal", True),
                                 ("Containers", {"container": {}}), ("IPAM", {"Config": []})):
            changed = copy.deepcopy(value)
            changed[0][key] = replacement
            with self.subTest(key=key), self.assertRaises(ValueError):
                subject.network(changed, name, NONCE)

    def test_should_accept_bounded_image_receipt_and_reject_engine_or_inventory_changes(self):
        value = images()
        self.assertEqual(subject.image_receipt(value, ENGINE), value)
        for key, replacement in (("engine", "other"), ("sourceEngine", ENGINE), ("sourceSocket", "/tmp/../socket"),
                                 ("sourceIds", ["bad"]), ("images", [])):
            changed = dict(value, **{key: replacement})
            with self.subTest(key=key), self.assertRaises((ValueError, subject.ActionError)):
                subject.image_receipt(changed, ENGINE)

    def test_should_reject_wrong_tag_platform_cache_archive_and_bool_sizes(self):
        for key, replacement in (("name", "foreign"), ("bytes", True), ("bytes", subject.LIMIT + 1),
                                 ("archive", "/tmp/../escape"), ("sha256", "f" * 63),
                                 ("cacheAlias", "foreign:tag")):
            changed = images()
            changed["images"][0][key] = replacement
            with self.subTest(key=key), self.assertRaises(ValueError):
                subject.image_receipt(changed, ENGINE)
        for key, replacement in (("os", "windows"), ("architecture", "amd64"), ("bytes", 0), ("id", "bad")):
            changed = images()
            changed["images"][0]["image"][key] = replacement
            with self.assertRaises((ValueError, subject.ActionError)):
                subject.image_receipt(changed, ENGINE)

    def test_should_reject_preexisting_forward_directory_without_dispatch(self):
        with tempfile.TemporaryDirectory(prefix="rootless-test-", dir="/tmp") as temporary:
            owner = Mock()
            with self.assertRaisesRegex(ValueError, "resource-forward-directory-exists"):
                subject.forward({"profile": "rustack-workflows-rootless-arm64-v2"}, temporary, owner, 100)
            owner.run.assert_not_called()

    def test_should_reject_socket_substitution_and_path_exhaustion(self):
        with tempfile.TemporaryDirectory(prefix="rootless-test-", dir="/tmp") as temporary:
            root = Path(temporary)
            path = root / "docker.sock"
            with socket.socket(socket.AF_UNIX) as channel:
                channel.bind(str(path))
                path.chmod(0o600)
                observed = subject.socket_metadata(path)
                self.assertEqual(observed["uid"], os.getuid())
                alias = root / "alias"
                alias.symlink_to(path)
                with self.assertRaises(ValueError):
                    subject.socket_metadata(alias)
                path.chmod(0o666)
                with self.assertRaises(ValueError):
                    subject.socket_metadata(path)
            with self.assertRaises(ValueError):
                subject.socket_metadata(root / ("x" * 101))

    def test_should_refuse_stale_forward_owner_before_connecting_to_daemon(self):
        with tempfile.TemporaryDirectory(prefix="rootless-test-", dir="/tmp") as temporary:
            root = Path(temporary)
            subject.bootstrap.publish(root / "entry.json", {"profile": "rustack-workflows-rootless-arm64-v2",
                "engine": ENGINE, "ownerPid": os.getpid(), "nonce": NONCE,
                "scriptSha256": "0" * 64, "socket": str(root / "docker.sock")})
            owner = Mock()
            with self.assertRaisesRegex(ValueError, "resource-forward-owner"):
                subject.connection(str(root), {"profile": "rustack-workflows-rootless-arm64-v2",
                                              "rootless": {"id": ENGINE}}, owner, 100)
            owner.run.assert_not_called()


if __name__ == "__main__":
    unittest.main()
