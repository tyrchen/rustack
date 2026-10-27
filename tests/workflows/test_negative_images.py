"""The incompatible artifacts have deterministic real image identities/platforms."""
import hashlib
import importlib.util
import io
import json
import os
from pathlib import Path
import subprocess
import tarfile
import tempfile
import unittest

spec = importlib.util.spec_from_file_location("negative_images", Path(__file__).parents[1] / "lambda-fixtures/negative_images.py")
negative = importlib.util.module_from_spec(spec)
spec.loader.exec_module(negative)


class NegativeImageControls(unittest.TestCase):
    def test_should_reject_invalid_provisioning_before_cli_with_optimization_enabled(self):
        environment = {"PATH":os.environ.get("PATH", "/usr/bin:/bin"),
                       "PYTHONOPTIMIZE":"1", "PYTHONPATH":"/explicit-untrusted-module-path"}
        with tempfile.TemporaryDirectory(prefix="rustack-cache-guard-") as root:
            executable = str(Path(root) / "must-not-exist")
            for option, message in (("WORKFLOWS_ARCH=invalid", "WORKFLOWS_ARCH must be arm64 or amd64"),
                                    ("WORKFLOWS_PULL_IMAGE=public.ecr.aws/lambda/nodejs:latest", "immutable public Node fixture required"),
                                    ("WORKFLOWS_PULL_IMAGE=foreign.example/private@sha256:" + "a"*64, "immutable public Node fixture required")):
                with self.subTest(option=option):
                    result = subprocess.run(["make", "lambda-pull-fixture-image", "WORKFLOWS_ARCH=arm64",
                                             "WORKFLOWS_DOCKER_BIN=" + executable,
                                             "WORKFLOWS_DOCKER_SOCKET=/unused/fixture.sock", option],
                                            cwd=Path(__file__).resolve().parents[2], env=environment,
                                            capture_output=True, text=True, timeout=10)
                    self.assertNotEqual(result.returncode, 0)
                    self.assertIn(message, result.stderr)
                    self.assertNotIn("FileNotFoundError", result.stderr)

    def test_should_build_deterministic_nonempty_content_addressed_platforms(self):
        for selected in ("arm64", "amd64"):
            for kind in ("os", "architecture"):
                raw = negative.image(selected, kind)
                self.assertEqual(raw, negative.image(selected, kind))
                self.assertLessEqual(len(raw), 65536)
                with tarfile.open(fileobj=io.BytesIO(raw)) as archive:
                    manifest = json.load(archive.extractfile("manifest.json"))[0]
                    config = archive.extractfile(manifest["Config"]).read()
                    self.assertEqual(manifest["Config"], hashlib.sha256(config).hexdigest() + ".json")
                    metadata = json.loads(config)
                    self.assertEqual(metadata["os"], "windows" if kind == "os" else "linux")
                    if kind == "architecture":
                        self.assertNotEqual(metadata["architecture"], selected)
                    layer = archive.extractfile(manifest["Layers"][0]).read()
                    self.assertEqual(metadata["rootfs"]["diff_ids"], ["sha256:" + hashlib.sha256(layer).hexdigest()])
                    self.assertTrue(all(member.isfile() and member.uid == 1000 and member.mtime == 0 for member in archive))

    def test_should_refuse_unrecognized_platform_or_artifact_kind(self):
        for selected, kind in (("../arm64", "os"), ("arm64", "../os"), ("x86_64", "architecture")):
            with self.assertRaises(ValueError):
                negative.image(selected, kind)


if __name__ == "__main__":
    unittest.main()
