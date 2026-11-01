"""Bounded artifact, provenance and cache controls; never executes downloaded code."""

import gzip
import hashlib
import importlib.util
import io
import json
from pathlib import Path
import tarfile
import tempfile
import time
import unittest
from unittest.mock import patch


SPEC = importlib.util.spec_from_file_location("pulumi_tools", Path(__file__).resolve().parent.parent / "lambda-fixtures" / "pulumi_tools.py")
TOOLS = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(TOOLS)


def elf(machine=183):
    value = bytearray(64)
    value[:6] = b"\x7fELF\x02\x01"
    value[18:20] = machine.to_bytes(2, "little")
    return bytes(value)


def archive(root, label, entries):
    raw = io.BytesIO()
    with tarfile.open(fileobj=raw, mode="w", format=tarfile.USTAR_FORMAT) as out:
        for name, value, kind in entries:
            entry = tarfile.TarInfo(name)
            entry.type = kind
            entry.mode = 0o755
            entry.size = len(value) if kind not in (tarfile.SYMTYPE, tarfile.LNKTYPE) else 0
            if kind in (tarfile.SYMTYPE, tarfile.LNKTYPE):
                entry.linkname = "outside"
            out.addfile(entry, io.BytesIO(value))
    path = root / (label + ".tar.gz")
    path.write_bytes(gzip.compress(raw.getvalue()))
    return path


class PulumiToolControls(unittest.TestCase):
    def observe(self, source, destination, architecture="arm64", selected=None):
        return TOOLS.unpack(source, destination, selected or {"pulumi/pulumi": "bin/pulumi"}, architecture, time.monotonic() + 10)

    def test_should_extract_only_expected_matching_architecture_executables(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = archive(root, "valid", [("pulumi/pulumi", elf(), tarfile.REGTYPE), ("pulumi/extra", b"ignored", tarfile.REGTYPE)])
            result = self.observe(source, root / "out")
            self.assertEqual(result, [{"path": "bin/pulumi", "sha256": hashlib.sha256(elf()).hexdigest(), "bytes": 64}])
            self.assertEqual((root / "out/bin/pulumi").read_bytes(), elf())
            self.assertFalse((root / "out/extra").exists())
            self.assertEqual(self.observe(source, None), result)

    def test_should_accept_only_the_zero_sized_archive_root_directory_spelling(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = archive(root, "root", [("./", b"", tarfile.DIRTYPE), ("./pulumi/pulumi", elf(), tarfile.REGTYPE)])
            self.assertEqual(len(self.observe(source, root / "out")), 1)
            for spelling in ("././", "../", "/"):
                with self.subTest(spelling=spelling), self.assertRaises(TOOLS.ToolError):
                    TOOLS.name(spelling, True)

    def test_should_refuse_wrong_elf_architecture_before_publishing_tools(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = archive(root, "wrong", [("pulumi/pulumi", elf(62), tarfile.REGTYPE)])
            with self.assertRaises(TOOLS.ToolError):
                self.observe(source, root / "out")

    def test_should_reject_linked_special_and_traversal_entries(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            for index, (name, kind) in enumerate([
                ("pulumi/pulumi", tarfile.SYMTYPE), ("pulumi/pulumi", tarfile.LNKTYPE),
                ("pulumi/pulumi", tarfile.FIFOTYPE), ("../pulumi", tarfile.REGTYPE),
                ("/absolute", tarfile.REGTYPE), ("a/../pulumi", tarfile.REGTYPE), ("a\\pulumi", tarfile.REGTYPE),
            ]):
                with self.subTest(name=name, kind=kind):
                    source = archive(root, str(index), [(name, elf(), kind)])
                    with self.assertRaises(TOOLS.ToolError):
                        self.observe(source, root / ("out" + str(index)))

    def test_should_refuse_duplicate_missing_and_expired_selected_files(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            item = ("pulumi/pulumi", elf(), tarfile.REGTYPE)
            for label, entries in [("duplicate", [item, item]), ("missing", [("other", elf(), tarfile.REGTYPE)])]:
                with self.subTest(label=label), self.assertRaises(TOOLS.ToolError):
                    self.observe(archive(root, label, entries), root / label)
            source = archive(root, "expired", [item])
            with self.assertRaises(TOOLS.ToolError):
                TOOLS.unpack(source, root / "late", {"pulumi/pulumi": "bin/pulumi"}, "arm64", time.monotonic())

    def test_should_bound_plain_expansion_and_pax_before_metadata_allocation(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = archive(root, "size", [("pulumi/pulumi", elf() * 32, tarfile.REGTYPE)])
            with patch.object(TOOLS, "EXPANDED", 1024), self.assertRaises(TOOLS.ToolError):
                self.observe(source, root / "out")
            pax = archive(root, "pax", [("PaxHeaders/large", b"x" * 4097, tarfile.XHDTYPE)])
            with self.assertRaises(TOOLS.ToolError):
                self.observe(pax, root / "pax-out")
            with self.assertRaises(TOOLS.ToolError):
                TOOLS.pax(b"14 size=900000\n")

    def test_should_reject_truncated_wrong_crc_and_nonzero_trailing_archives(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            source = archive(root, "source", [("pulumi/pulumi", elf(), tarfile.REGTYPE)])
            encoded = source.read_bytes()
            for label, data in [("truncated", encoded[:-4]), ("crc", encoded[:-8] + bytes(8)),
                                ("trailing", gzip.compress(gzip.decompress(encoded) + b"not-padding"))]:
                with self.subTest(label=label):
                    broken = root / (label + ".gz")
                    broken.write_bytes(data)
                    with self.assertRaises((TOOLS.ToolError, EOFError, OSError)):
                        self.observe(broken, root / label)

    def test_should_anchor_cached_binary_hashes_to_verified_archive_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            cli = archive(root, "cli", [(value, elf(), tarfile.REGTYPE) for value in TOOLS.SELECTED])
            provider = archive(root, "aws", [("pulumi-resource-aws", elf(), tarfile.REGTYPE)])
            destination = root / "installed"
            def source(url, *unused):
                return cli if "get.pulumi.com" in url else provider
            with patch.object(TOOLS, "download", side_effect=source):
                TOOLS.prepare(destination, "arm64")
                manifest = destination / "manifest.json"
                data = json.loads(manifest.read_bytes())
                changed = elf()[:-1] + b"1"
                (destination / "bin/pulumi").write_bytes(changed)
                next(entry for entry in data["files"] if entry["path"] == "bin/pulumi")["sha256"] = hashlib.sha256(changed).hexdigest()
                manifest.write_text(json.dumps(data))
                with self.assertRaises(TOOLS.ToolError):
                    TOOLS.prepare(destination, "arm64")

    def test_should_reject_linked_cache_ancestors_even_for_unchanged_valid_bytes(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            cli = archive(root, "cli", [(value, elf(), tarfile.REGTYPE) for value in TOOLS.SELECTED])
            provider = archive(root, "aws", [("pulumi-resource-aws", elf(), tarfile.REGTYPE)])
            destination = root / "installed"
            with patch.object(TOOLS, "download", side_effect=lambda url, *args: cli if "get.pulumi.com" in url else provider):
                TOOLS.prepare(destination, "arm64")
                (destination / "bin").rename(root / "outside")
                (destination / "bin").symlink_to(root / "outside", target_is_directory=True)
                with self.assertRaises(TOOLS.ToolError):
                    TOOLS.prepare(destination, "arm64")

    def test_should_refuse_extra_linked_and_nonexecutable_cached_entries(self):
        with tempfile.TemporaryDirectory() as temporary:
            root = Path(temporary)
            cli = archive(root, "cli", [(value, elf(), tarfile.REGTYPE) for value in TOOLS.SELECTED])
            provider = archive(root, "aws", [("pulumi-resource-aws", elf(), tarfile.REGTYPE)])
            with patch.object(TOOLS, "download", side_effect=lambda url, *args: cli if "get.pulumi.com" in url else provider):
                for kind in ("extra", "link", "mode", "directory"):
                    with self.subTest(kind=kind):
                        destination = root / kind
                        TOOLS.prepare(destination, "arm64")
                        if kind == "extra":
                            with (destination / "unexpected").open("wb") as output:
                                output.truncate(TOOLS.EXPANDED + 1)
                        elif kind == "link":
                            (destination / "unexpected").symlink_to(root / "outside")
                        elif kind == "mode":
                            (destination / "bin/pulumi").chmod(0o644)
                        else:
                            (destination / "unexpected").mkdir()
                        with self.assertRaises(TOOLS.ToolError):
                            TOOLS.prepare(destination, "arm64")


if __name__ == "__main__":
    unittest.main()
