"""Negative controls for external oracle integrity and ownership boundaries."""
import json
import os
from pathlib import Path
import tempfile
import sys
from types import SimpleNamespace
import unittest
from unittest.mock import patch

from localstack_snapshot.snapshots import SnapshotAssertionError

from localstack_policy import exact, guards, manifest, outcomes, private, relative
from localstack_fixtures import Fixtures, RecordedSession
import localstack_images as images
from localstack_policy import digest, publish, read


class PolicyTests(unittest.TestCase):
    def test_should_pin_every_required_node_to_a_recorded_upstream_baseline(self):
        value, hashed = manifest()
        self.assertEqual(len(hashed), 64)
        self.assertEqual({key:len(nodes) for key, nodes in value["groups"].items()},
                         {"pure":91, "docker":32})

    def test_should_reject_missing_extra_and_repeated_collection(self):
        for nodes in (["a"], ["a", "b", "c"], ["a", "a"]):
            with self.subTest(nodes=nodes), self.assertRaisesRegex(ValueError, "collection"):
                exact(["a", "b"], nodes)

    def test_should_reject_failed_skipped_xfailed_and_missing_teardown(self):
        good = {"setup":"passed", "call":"passed", "teardown":"passed", "xfail":False}
        for changes in ({"call":"failed"}, {"call":"skipped"}, {"xfail":True}, {"teardown":None}):
            with self.subTest(changes=changes), self.assertRaisesRegex(ValueError, "unsuccessful"):
                outcomes(["a"], {"a":{**good, **changes}})

    def test_should_forbid_snapshot_updates_skip_all_and_test_selection_flags(self):
        for flag in ("--snapshot-update", "--snapshot-raw", "--snapshot-skip-all", "--collect-only",
                     "--maxfail=1", "--lf", "-x"):
            with self.subTest(flag=flag), self.assertRaisesRegex(ValueError, "forbidden"):
                guards([flag], {})

    def test_should_forbid_ambient_cloud_and_pytest_configuration(self):
        for name in ("AWS_PROFILE", "AWS_DEFAULT_PROFILE", "TEST_TARGET", "PYTEST_ADDOPTS", "PYTEST_PLUGINS",
                     "SNAPSHOT_UPDATE", "SNAPSHOT_RAW"):
            with self.subTest(name=name), self.assertRaisesRegex(ValueError, "ambient"):
                guards([], {name:"1"})

    def test_should_reject_traversal_and_absolute_vendor_references(self):
        for value in ("../secret", "/tmp/file", "a//b", "a/./b", "a/../b", "a\\b"):
            with self.subTest(value=value), self.assertRaisesRegex(ValueError, "relative"):
                relative(value)

    def test_should_reject_nonprivate_and_symlinked_evidence(self):
        with tempfile.TemporaryDirectory() as directory:
            path = Path(directory).resolve()
            os.chmod(path, 0o755)
            with self.assertRaisesRegex(ValueError, "ownership"):
                private(path)
            os.chmod(path, 0o700)
            link = path / "link"
            link.symlink_to(path, target_is_directory=True)
            with self.assertRaisesRegex(ValueError, "evidence"):
                private(link)

    def test_should_preserve_upstream_semantic_assertions_and_baseline_bytes(self):
        with tempfile.TemporaryDirectory() as directory:
            base = Path(directory) / "oracle"
            baseline = base.with_suffix(".snapshot.json")
            original = json.dumps({"node":{"recorded-date":"upstream", "recorded-content":{
                "result":{"output":42, "error":"OriginalError"}}}}).encode()
            baseline.write_bytes(original)
            for wrong in ({"output":43, "error":"OriginalError"}, {"output":42, "error":"ChangedError"}):
                session = RecordedSession(base_file_path=str(base), scope_key="node", update=False, verify=True)
                session.match("result", wrong)
                with self.assertRaises(SnapshotAssertionError):
                    session._assert_all()
                self.assertTrue(session.checked)
                self.assertFalse(session.passed)
                self.assertEqual(baseline.read_bytes(), original)

    def test_should_not_accept_a_session_that_compares_nothing(self):
        with tempfile.TemporaryDirectory() as directory:
            session = RecordedSession(base_file_path=str(Path(directory) / "empty"),
                                      scope_key="node", update=False, verify=True)
            self.assertEqual(session._assert_all(), [])
            self.assertFalse(session.passed)

    def test_should_install_the_original_aws_snapshot_transformers(self):
        with tempfile.TemporaryDirectory() as directory, patch.object(sys, "path", [str(images.VENDOR), *sys.path]):
            base = Path(directory) / "oracle"
            session = RecordedSession(base_file_path=str(base), scope_key="node", update=False, verify=True)
            request = SimpleNamespace(fspath=SimpleNamespace(dirname="tests/aws/services/stepfunctions/v2"))
            plugin = Fixtures(Path(directory), "http://127.0.0.1:1", "pure", [], {}, {})
            observed = Fixtures.snapshot.__wrapped__(plugin, request, session)
            self.assertIs(observed, session)
            self.assertTrue(callable(observed.transform.stepfunctions_api))
            self.assertTrue(callable(observed.transform.sfn_sm_create_arn))

    def test_should_fail_the_case_when_machine_cleanup_cannot_be_confirmed(self):
        with tempfile.TemporaryDirectory() as directory:
            plugin = Fixtures(Path(directory), "http://127.0.0.1:1", "pure", [], {}, {})
            client = SimpleNamespace(create_state_machine=lambda **_: {"stateMachineArn":"arn"},
                list_executions=lambda **_: {"executions":[]}, delete_state_machine=lambda **_: None)
            owner = Fixtures.create_state_machine.__wrapped__(plugin, None, None)
            create = next(owner)
            create(SimpleNamespace(stepfunctions=client), name="test")
            with patch.object(client, "delete_state_machine", side_effect=OSError("unavailable")):
                with self.assertRaisesRegex(ValueError, "cleanup-failed"):
                    next(owner)

    def test_should_accept_upstream_positional_and_keyword_role_factory_calls(self):
        with tempfile.TemporaryDirectory() as directory:
            plugin = Fixtures(Path(directory), "http://127.0.0.1:1", "pure", [], {}, {})
            factory = Fixtures.create_state_machine_iam_role.__wrapped__(plugin)
            self.assertEqual(factory(None), factory(target_aws_client=None))

    def test_should_reject_substituted_image_receipt_and_build_metadata(self):
        source = images.HANDLERS[0]
        with tempfile.TemporaryDirectory() as directory, patch.object(images, "HANDLERS", (source,)):
            root = Path(directory).resolve()
            context = root / "handler-0"
            context.mkdir()
            raw = read(images.VENDOR / source)
            (context / "handler.py").write_bytes(raw)
            (context / "Dockerfile").write_bytes(images.recipe())
            (context / "build.stdout").write_bytes(b"build complete")
            image_id = "sha256:" + "a" * 64
            metadata = {"containerimage.digest":image_id, "containerimage.config.digest":"sha256:" + "b" * 64}
            publish(context / "metadata.json", metadata)
            name = "rustack-localstack-compat:nonce-0"
            publish(context / "build-intent.json", {"argv":images.build_arguments(Path("/builder"), context, name),
                "engine":"engine", "source":source, "handlerSha256":digest(raw),
                "dockerfileSha256":digest(images.recipe())})
            row = {"imageId":image_id, "handlerSha256":digest(raw),
                "buildIntentSha256":digest(read(context / "build-intent.json")),
                "metadataSha256":digest(read(context / "metadata.json")),
                "buildStdoutSha256":digest(read(context / "build.stdout"))}
            publish(context / "result.json", row)
            row["buildResultSha256"] = digest(read(context / "result.json"))
            value = {"engine":"engine", "endpoint":"unix:///socket", "base":images.BASE,
                "manifestSha256":"seal", "builderExecutable":"/builder", "handlers":{source:row}}
            publish(root / "intent.json", {"engine":"engine", "endpoint":"unix:///socket",
                "base":images.BASE, "manifestSha256":"seal", "nonce":"nonce"})
            images.validate_builds(root, value)
            row["imageId"] = "sha256:" + "c" * 64
            with self.assertRaisesRegex(ValueError, "result-affiliation"):
                images.validate_builds(root, value)
            row["imageId"] = image_id
            (context / "metadata.json").write_text(json.dumps({"containerimage.digest":"sha256:" + "d" * 64}))
            with self.assertRaisesRegex(ValueError, "build-output"):
                images.validate_builds(root, value)


if __name__ == "__main__":
    unittest.main()
