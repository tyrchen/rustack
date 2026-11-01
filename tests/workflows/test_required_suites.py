"""Positive and drift/failure controls for exact resource qualification."""

import copy
import json
import os
from pathlib import Path
import tempfile
import unittest

from required_suites import (
    INVENTORY_LIMIT, MANIFEST_LIMIT, ManifestError, SUITES, parse_inventory,
    parse_manifest, read_regular, verify,
)


def document():
    return {
        "schemaVersion": 1,
        "profile": "rustack-standard-jsonpath-v1",
        "qualification": "native-workflow-only",
        "suites": [
            {"id": "native-bridge", "tests": ["stepfunctions_bridge::tests::native::native_e2e_real"]},
            {"id": "registered-sdk", "tests": ["test_should_use_real_sdk"]},
            {"id": "typed-http-sdk", "tests": ["test_should_use_typed_http"]},
        ],
    }


def encoded(value):
    return json.dumps(value).encode()


def listing(names):
    return ("\n".join(name + ": test" for name in names)
            + "\n\n" + str(len(names)) + " tests, 0 benchmarks\n").encode()


class RequiredSuitesTests(unittest.TestCase):
    def test_should_accept_complete_exact_inventories(self):
        required = parse_manifest(encoded(document()))
        inventories = {identity: listing(names) for identity, names in required.items()}
        self.assertEqual(verify(required, inventories), 3)
        actual = parse_manifest(read_regular(Path(__file__).with_name("required-suites.json"), MANIFEST_LIMIT))
        self.assertEqual(set(actual), SUITES)

    def test_should_reject_missing_renamed_extra_or_no_longer_ignored_cases(self):
        required = parse_manifest(encoded(document()))
        inventories = {identity: listing(names) for identity, names in required.items()}
        for names in [[], ["renamed"], ["test_should_use_real_sdk", "unexpected"]]:
            changed = dict(inventories, **{"registered-sdk": listing(names)})
            with self.subTest(names=names), self.assertRaises(ManifestError):
                verify(required, changed)
        with self.assertRaises(ManifestError):
            verify(required, {"registered-sdk": inventories["registered-sdk"]})

    def test_should_reject_schema_profile_scope_and_field_drift(self):
        changes = [
            {"schemaVersion": True}, {"schemaVersion": 2}, {"profile": "other"},
            {"qualification": "full-release"}, {"extra": 1}, {"suites": []},
            {"suites": {}},
        ]
        for change in changes:
            value = dict(document(), **change)
            with self.subTest(change=change), self.assertRaises(ManifestError):
                parse_manifest(encoded(value))
        for value in [b"{", b"\xff", b"[]", encoded(document()).decode().encode("utf-16"), b'{' + b'"schemaVersion":1,' * 2 + b'"suites":[]}']:
            with self.subTest(raw=value), self.assertRaises(ManifestError):
                parse_manifest(value)

    def test_should_reject_duplicate_unknown_and_unbounded_suite_members(self):
        mutations = [
            lambda value: value["suites"].append(copy.deepcopy(value["suites"][0])),
            lambda value: value["suites"][0].update(id="other"),
            lambda value: value["suites"][1].update(id="native-bridge"),
            lambda value: value["suites"][0].update(extra=True),
            lambda value: value["suites"][0].update(tests=["ordinary_case"]),
            lambda value: value["suites"][1].update(tests=[]),
            lambda value: value["suites"][1].update(tests=["same", "same"]),
            lambda value: value["suites"][1].update(tests=["case"] * 129),
            lambda value: value["suites"][1].update(tests=["a" * 257]),
            lambda value: value["suites"][1].update(tests=["bad;command"]),
            lambda value: value["suites"][1].update(tests=["café"]),
            lambda value: value["suites"][1].update(tests=[7]),
        ]
        for index, mutate in enumerate(mutations):
            value = document()
            mutate(value)
            with self.subTest(index=index), self.assertRaises(ManifestError):
                parse_manifest(encoded(value))

    def test_should_reject_forged_and_ambiguous_inventory_text(self):
        for raw in [
            b"", b"0 tests, 0 benchmarks", b"case: test\n2 tests, 0 benchmarks",
            b"case: test\n1 test, 1 benchmark", b"case: test\ncase: test\n2 tests, 0 benchmarks",
            b"case: test\n1 test, 0 benchmarks\nextra: test", b"unexpected line",
            b"case: test\n", b"\xff", b"bad name: test\n1 test, 0 benchmarks",
            b"case: test\n1 test, 0 benchmarks\n1 test, 0 benchmarks",
            b"case: test\v1 test, 0 benchmarks",
        ]:
            with self.subTest(raw=raw), self.assertRaises(ManifestError):
                parse_inventory(raw)
        self.assertEqual(parse_inventory(b"case: test\n1 test, 0 benchmarks\n"), frozenset(["case"]))

    def test_should_enforce_byte_limits_and_reject_nonregular_files(self):
        for parse, maximum in [(parse_manifest, MANIFEST_LIMIT), (parse_inventory, INVENTORY_LIMIT)]:
            with self.assertRaises(ManifestError):
                parse(b" " * (maximum + 1))
        with tempfile.TemporaryDirectory() as name:
            root = Path(name)
            regular = root / "regular"
            regular.write_bytes(b"1234")
            self.assertEqual(read_regular(regular, 4), b"1234")
            with self.assertRaises(ManifestError):
                read_regular(regular, 3)
            link = root / "link"
            link.symlink_to(regular)
            fifo = root / "fifo"
            os.mkfifo(fifo)
            for path in [root, link, fifo, root / "missing"]:
                with self.subTest(path=path.name), self.assertRaises(ManifestError):
                    read_regular(path, 4)


if __name__ == "__main__":
    unittest.main()
