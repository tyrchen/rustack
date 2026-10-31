"""The separate Action inventory cannot drift or impersonate another profile."""
import copy
import json
from pathlib import Path
import unittest

from action_required_suites import CASE, SUITES, decode
from required_suites import MANIFEST_LIMIT, ManifestError, read_regular, verify
from test_required_suites import encoded, listing


def document():
    return json.loads(read_regular(Path(__file__).with_name("action-required-suites.json"), MANIFEST_LIMIT))


class ActionManifestControls(unittest.TestCase):
    def test_should_require_the_one_exact_independent_process_case(self):
        required = decode(encoded(document()))
        self.assertEqual(required, {"composite-action": frozenset((CASE,))})
        self.assertEqual(verify(required, {"composite-action": listing([CASE])}, expected_suites=SUITES), 1)
        for names in ([], ["renamed"], [CASE, "unexpected"], [CASE, CASE]):
            with self.subTest(names=names), self.assertRaises(ManifestError):
                verify(required, {"composite-action": listing(names)}, expected_suites=SUITES)

    def test_should_reject_manifest_and_resource_scope_drift(self):
        for change in ({"qualification": "docker-image-functional"}, {"qualification": "native-workflow-only"},
                       {"profile": "rustack-standard-jsonpath-v1"}, {"schemaVersion": True}, {"extra": 1}):
            with self.subTest(change=change), self.assertRaises(ManifestError):
                decode(encoded(dict(document(), **change)))
        for mutate in (lambda row: row["suites"].clear(),
                       lambda row: row["suites"][0].update(id="ordinary-startup"),
                       lambda row: row["suites"][0].update(tests=["renamed"]),
                       lambda row: row["suites"][0].update(tests=[CASE, "unexpected"]),
                       lambda row: row["suites"].append(copy.deepcopy(row["suites"][0]))):
            value = document()
            mutate(value)
            with self.assertRaises(ManifestError):
                decode(encoded(value))
        for name in ("required-suites.json", "docker-required-suites.json"):
            with self.assertRaises(ManifestError):
                decode(read_regular(Path(__file__).with_name(name), MANIFEST_LIMIT))


if __name__ == "__main__":
    unittest.main()
