"""Deterministic same-artifact packaging controls; no runtime substitution."""
import io
import json
from pathlib import Path
import tempfile
import unittest
import zipfile

from provided_zip_fixture import build, digest, linux_elf, package


class ProvidedZipControls(unittest.TestCase):
    def test_should_preserve_exact_bootstrap_bytes_and_stable_distinct_revisions(self):
        bootstrap = b"ELF fixture bytes" * 100
        a, b = package(bootstrap, b"A"), package(bootstrap, b"B")
        self.assertEqual(a, package(bootstrap, b"A"))
        self.assertNotEqual(a, b)
        self.assertNotEqual(digest(a), digest(b))
        for raw, marker in ((a,b"A"), (b,b"B")):
            with zipfile.ZipFile(io.BytesIO(raw)) as archive:
                self.assertEqual(archive.namelist(), ["bootstrap", "revision.txt"])
                self.assertEqual(archive.read("bootstrap"), bootstrap)
                self.assertEqual(archive.read("revision.txt"), marker)
                self.assertEqual(archive.getinfo("bootstrap").external_attr >> 16, 0o100755)
                self.assertEqual(archive.getinfo("revision.txt").external_attr >> 16, 0o100644)

    def test_should_reject_wrong_machine_or_incomplete_elf_before_packaging(self):
        raw = bytearray(64)
        raw[:7] = b"\x7fELF\x02\x01\x01"
        for a, b, value in ((16,18,2),(18,20,183),(20,24,1),(52,54,64)):
            raw[a:b] = value.to_bytes(b-a,"little")
        linux_elf(raw,"arm64")
        for data, arch in ((raw,"amd64"),(raw[:63],"arm64"),(b"bad", "arm64")):
            with self.subTest(architecture=arch), self.assertRaises(AssertionError):
                linux_elf(data,arch)
        with tempfile.TemporaryDirectory() as root:
            source = Path(root)/"bootstrap"
            source.write_bytes(raw)
            target = Path(root)/"provided"
            build(source,"arm64",target)
            manifest = json.loads((target/"provided-zip-manifest.json").read_text())
            for info in manifest["artifacts"].values():
                data = (target/info["file"]).read_bytes()
                self.assertEqual(info["bytes"],len(data))
                self.assertEqual(info["sha256"],digest(data))
            self.assertFalse(list(target.glob("*.pending")))


if __name__ == "__main__":
    unittest.main()
