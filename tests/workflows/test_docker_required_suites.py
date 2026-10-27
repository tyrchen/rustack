"""Docker policy stays exact and cannot accept a native-only/release manifest."""
import copy
import json
from pathlib import Path
import unittest

from docker_required_suites import SUITES, parse_manifest, verify
from required_suites import MANIFEST_LIMIT, ManifestError, read_regular
from test_required_suites import encoded, listing


def document():
    return {"schemaVersion":1, "profile":"rustack-docker-image-v1", "qualification":"docker-image-functional",
            "suites":[{"id":suite, "tests":["test_should_execute_" + suite.replace("-", "_")]}
                      for suite in sorted(SUITES)]}


class DockerManifestControls(unittest.TestCase):
    def test_should_require_every_exact_suite_and_load_frozen_fifty_three_resources(self):
        required = parse_manifest(encoded(document()))
        self.assertEqual(verify(required, {key:listing(names) for key,names in required.items()}), 8)
        frozen = parse_manifest(read_regular(Path(__file__).with_name("docker-required-suites.json"), MANIFEST_LIMIT))
        self.assertEqual(sum(map(len, frozen.values())), 53)
        self.assertEqual(len(frozen["ordinary-startup"]), 24)
        self.assertEqual(len(frozen["docker-core"]), 8)
        self.assertEqual(len(frozen["relay-runtime"]), 5)

    def test_should_refuse_native_release_unknown_or_duplicate_scope(self):
        for change in ({"qualification":"full-release"}, {"qualification":"native-workflow-only"},
                       {"profile":"rustack-standard-jsonpath-v1"}, {"schemaVersion":True}, {"extra":1}):
            with self.subTest(change=change), self.assertRaises(ManifestError):
                parse_manifest(encoded(dict(document(), **change)))
        for mutate in (lambda row: row["suites"].pop(),
                       lambda row: row["suites"][0].update(id="native-bridge"),
                       lambda row: row["suites"][0].update(id=row["suites"][1]["id"])):
            value = document()
            mutate(value)
            with self.assertRaises(ManifestError):
                parse_manifest(encoded(value))
        native = Path(__file__).with_name("required-suites.json").read_bytes()
        with self.assertRaises(ManifestError):
            parse_manifest(native)

    def test_should_reject_drift_in_any_compiled_target_including_removed_ignored_tests(self):
        required = parse_manifest(encoded(document()))
        inventories = {key:listing(names) for key,names in required.items()}
        for suite in SUITES:
            for replacement in (listing([]), listing(["renamed"]),
                                listing(list(required[suite]) + ["unexpected_resource"])):
                changed = dict(inventories, **{suite:replacement})
                with self.subTest(suite=suite), self.assertRaises(ManifestError):
                    verify(required, changed)
        with self.assertRaises(ManifestError):
            verify(required, {key:value for key,value in inventories.items() if key != "relay-runtime"})

    def test_should_keep_strict_bounded_schema_and_inventory_decoding(self):
        for mutation in (lambda row: row["suites"][0].update(tests=["same", "same"]),
                         lambda row: row["suites"][0].update(tests=["bad;command"]),
                         lambda row: row["suites"][0].update(tests=["x"*257]),
                         lambda row: row["suites"][0].update(tests=["case"]*129)):
            value = copy.deepcopy(document())
            mutation(value)
            with self.assertRaises(ManifestError):
                parse_manifest(encoded(value))
        for value in (b" "*(MANIFEST_LIMIT+1), b"\xff", b"{", b'{"schemaVersion":1,"schemaVersion":1}'):
            with self.assertRaises(ManifestError):
                parse_manifest(value)
        required = parse_manifest(encoded(document()))
        inventories = {key:listing(names) for key,names in required.items()}
        for raw in (b"case: test\n2 tests, 0 benchmarks", b"case: test\n1 test, 1 benchmark", b"bad name: test", b"\xff"):
            with self.assertRaises(ManifestError):
                verify(required, dict(inventories, **{"docker-core":raw}))


if __name__ == "__main__":
    unittest.main()
