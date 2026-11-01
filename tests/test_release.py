"""Exercise publication closure and fail-before-write Action release fences."""
import copy
import importlib.util
import json
from pathlib import Path
import tempfile
import unittest
from unittest.mock import patch

SPEC = importlib.util.spec_from_file_location("release", Path(__file__).resolve().parents[1] / "tools/release.py")
release = importlib.util.module_from_spec(SPEC)
SPEC.loader.exec_module(release)
COMMIT = "a" * 40


class ReleaseTests(unittest.TestCase):
    def metadata(self):
        return {"packages": [
            {"name": "squib-loader", "version": "0.2.1", "dependencies": []},
            {"name": "rustack-lambda-core", "version": "0.12.0", "dependencies": [
                {"name": "squib-loader", "req": "~0.2.1", "kind": None}]},
            {"name": "rustack-cli", "version": "0.12.0", "dependencies": [
                {"name": "rustack-lambda-core", "req": "^0.12.0", "kind": None}]},
            {"name": "fixture", "version": "0.9.0", "publish": [], "dependencies": [
                {"name": "rustack-cli", "kind": None}]}]}

    def test_should_require_complete_publishable_dependency_closure(self):
        metadata = self.metadata()
        selected = ["squib-loader", "rustack-lambda-core", "rustack-cli"]
        self.assertEqual(release.check(metadata, selected), "0.12.0")
        for invalid in ([], selected + selected, selected + ["missing"], selected + ["fixture"], selected[:-1]):
            with self.subTest(invalid=invalid), self.assertRaises(ValueError):
                release.check(metadata, invalid)

    def test_should_require_security_floor_outside_workspace_patches(self):
        for constraint in ("^0.2.0", "~0.2.0"):
            metadata = self.metadata()
            metadata["packages"][1]["dependencies"][0]["req"] = constraint
            with self.assertRaisesRegex(ValueError, "security floor"):
                release.check(metadata, [p["name"] for p in metadata["packages"][:3]])

    def aliases(self, remote=COMMIT, values=None):
        environment = {"GITHUB_REPOSITORY": "tyrchen/rustack", "GITHUB_SHA": COMMIT,
                       "RELEASE_VERSION": "0.12.0"}
        environment.update(values or {})
        writes = []

        def execute(args):
            if args[:2] == ["git", "rev-parse"]:
                return COMMIT
            if "--method" in args:
                writes.append(args)
                return "{}"
            if "/commits/" in args[-1]:
                return json.dumps({"sha": remote})
            return "[]"

        return environment, execute, writes

    def test_should_create_only_major_and_minor_aliases_after_exact_source_check(self):
        environment, execute, writes = self.aliases()
        release.action_aliases("0.12.0", environment, execute)
        self.assertEqual([args[-3] for args in writes], ["ref=refs/tags/v0", "ref=refs/tags/v0.12"])

    def test_should_reject_mismatched_promotion_before_any_tag_write(self):
        for remote, values in (("b" * 40, {}), (COMMIT, {"RELEASE_VERSION": "0.11.0"}),
                               (COMMIT, {"GITHUB_SHA": "b" * 40}),
                               (COMMIT, {"GITHUB_REPOSITORY": "../other"})):
            environment, execute, writes = self.aliases(remote, values)
            with self.subTest(values=values), self.assertRaises(ValueError):
                release.action_aliases("0.12.0", environment, execute)
            self.assertEqual(writes, [])

    def test_should_reject_noncanonical_versions(self):
        for version in ("v0.12.0", "0.12", "00.12.0", "0.12.0;echo", "0.12.0-rc.1"):
            metadata = copy.deepcopy(self.metadata())
            metadata["packages"][2]["version"] = version
            with self.subTest(version=version), self.assertRaises(ValueError):
                release.release_version(metadata)

    def test_should_preserve_full_version_tags_when_updating_existing_aliases(self):
        environment, base, writes = self.aliases()

        def execute(args):
            if "/git/matching-refs/" in args[-1]:
                alias = args[-1].rsplit("/", 1)[-1]
                return json.dumps([{"ref": "refs/tags/" + alias, "object": {"sha": "b" * 40}},
                                   {"ref": "refs/tags/v0.12.0", "object": {"sha": COMMIT}}])
            return base(args)

        release.action_aliases("0.12.0", environment, execute)
        self.assertEqual([args[4] for args in writes], [
            "repos/tyrchen/rustack/git/refs/tags/v0",
            "repos/tyrchen/rustack/git/refs/tags/v0.12"])
        self.assertTrue(all(args[3] == "PATCH" for args in writes))

    def test_should_refuse_dirty_release_or_retargeting_an_existing_version(self):
        for dirty, tagged in ((True, COMMIT), (False, "b" * 40)):
            writes = []

            def execute(args):
                if args[1] == "status":
                    return " M Cargo.toml" if dirty else ""
                if args[1] == "branch":
                    return "master"
                if args[1] == "rev-parse":
                    return tagged if args[-1].endswith("^{commit}") else COMMIT
                if args[1:3] == ["tag", "--list"]:
                    return "v0.12.0"
                writes.append(args)
                return ""

            with tempfile.TemporaryDirectory() as temporary:
                root = Path(temporary)
                (root / "CHANGELOG.md").write_text("## [0.12.0]\n")
                with patch.object(release, "ROOT", root), self.assertRaises(ValueError):
                    release.tag("0.12.0", execute)
            self.assertEqual(writes, [])


    def test_should_require_the_qualified_image_even_if_the_version_tag_is_replaced(self):
        image = "sha256:" + "c" * 64
        receipt = {"schemaVersion": 1, "commit": COMMIT, "version": "0.12.0",
                   "registryManifests": ["ghcr.io/tyrchen/rustack@" + image] * 2,
                   "sourceImageIds": {"arm64": image, "amd64": image},
                   "allChildGroupsJoined": True}
        release.promoted_image("0.12.0", COMMIT, receipt, lambda _: image)
        with self.assertRaisesRegex(ValueError, "container differs"):
            release.promoted_image("0.12.0", COMMIT, receipt, lambda _: "sha256:" + "d" * 64)
        for field, value in (("commit", "b" * 40), ("version", "0.11.0"),
                             ("allChildGroupsJoined", False)):
            changed = dict(receipt, **{field: value})
            with self.subTest(field=field), self.assertRaisesRegex(ValueError, "receipt differs"):
                release.promoted_image("0.12.0", COMMIT, changed, lambda _: image)


if __name__ == "__main__":
    unittest.main()
