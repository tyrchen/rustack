"""Bounded exact-inventory decoder and native workflow resource entrypoint.

Make owns compilation, inventory creation and subsequent resource execution.
This module validates data only; manifest contents never become commands.
"""

import argparse
import json
import os
from pathlib import Path
import re
import stat
import sys


SUITES = frozenset(("native-bridge", "registered-sdk", "typed-http-sdk"))
MANIFEST_LIMIT = 65_536
INVENTORY_LIMIT = 32_768
NAME = re.compile(r"[A-Za-z0-9_]+(?:::[A-Za-z0-9_]+)*", re.ASCII)
SUMMARY = re.compile(r"([0-9]{1,4}) tests?, ([0-9]{1,4}) benchmarks?", re.ASCII)


class ManifestError(ValueError):
    """A fixed diagnostic with no untrusted input or filesystem contents."""


def read_regular(path, limit):
    """Read a small regular file without following a final symlink or FIFO."""
    try:
        if not stat.S_ISREG(path.lstat().st_mode):
            raise ManifestError("input must be a regular file")
        flags = os.O_RDONLY | os.O_NONBLOCK | getattr(os, "O_NOFOLLOW", 0)
        descriptor = os.open(path, flags)
        with os.fdopen(descriptor, "rb") as source:
            if not stat.S_ISREG(os.fstat(source.fileno()).st_mode):
                raise ManifestError("opened input must be a regular file")
            value = source.read(limit + 1)
        if len(value) > limit:
            raise ManifestError("input byte limit exceeded")
        return value
    except OSError:
        raise ManifestError("cannot read required inventory input") from None


def _object(pairs):
    result = {}
    for key, value in pairs:
        if key in result:
            raise ManifestError("duplicate manifest field")
        result[key] = value
    return result


def _name(value):
    if not isinstance(value, str) or len(value) > 256 or NAME.fullmatch(value) is None:
        raise ManifestError("invalid required test name")
    return value


def parse_manifest(raw, *, expected_suites=SUITES, profile="rustack-standard-jsonpath-v1",
                   qualification="native-workflow-only"):
    """Validate the complete bounded schema, suite set and exact test names."""
    if len(raw) > MANIFEST_LIMIT:
        raise ManifestError("manifest byte limit exceeded")
    try:
        document = json.loads(raw.decode("utf-8"), object_pairs_hook=_object)
    except (ValueError, RecursionError, UnicodeError):
        raise ManifestError("invalid manifest encoding or duplicate fields") from None
    if not isinstance(document, dict) or set(document) != {
        "schemaVersion", "profile", "qualification", "suites"
    }:
        raise ManifestError("invalid manifest fields")
    if type(document["schemaVersion"]) is not int or document["schemaVersion"] != 1:
        raise ManifestError("unsupported manifest schema")
    if document["profile"] != profile:
        raise ManifestError("unsupported workflow profile")
    if document["qualification"] != qualification:
        raise ManifestError("unsupported manifest qualification")
    suites = document["suites"]
    if not isinstance(suites, list) or len(suites) != len(expected_suites):
        raise ManifestError("incomplete required suite set")
    result = {}
    for suite in suites:
        if not isinstance(suite, dict) or set(suite) != {"id", "tests"}:
            raise ManifestError("invalid suite fields")
        identity = suite["id"]
        if not isinstance(identity, str) or identity not in expected_suites or identity in result:
            raise ManifestError("unknown or repeated required suite")
        tests = suite["tests"]
        if not isinstance(tests, list) or not 1 <= len(tests) <= 128:
            raise ManifestError("invalid required test count")
        names = frozenset(_name(name) for name in tests)
        if len(names) != len(tests):
            raise ManifestError("duplicate required test")
        if identity == "native-bridge" and any(
            not name.startswith("stepfunctions_bridge::tests::native::")
            or not name.rsplit("::", 1)[-1].startswith("native_e2e")
            for name in names
        ):
            raise ManifestError("bridge case is outside the Native selection")
        result[identity] = names
    if set(result) != expected_suites:
        raise ManifestError("incomplete required suite set")
    return result


def parse_inventory(raw):
    """Accept only exact ignored-test names followed by one consistent summary."""
    if len(raw) > INVENTORY_LIMIT:
        raise ManifestError("compiled inventory byte limit exceeded")
    try:
        lines = [line[:-1] if line.endswith("\r") else line for line in raw.decode("utf-8").split("\n")]
    except UnicodeError:
        raise ManifestError("invalid compiled inventory encoding") from None
    names = set()
    summary = None
    for line in lines:
        if not line:
            continue
        if summary is not None:
            raise ManifestError("unexpected data after inventory summary")
        if line.endswith(": test"):
            name = _name(line[:-6])
            if name in names or len(names) >= 128:
                raise ManifestError("duplicate or excessive compiled test names")
            names.add(name)
        else:
            match = SUMMARY.fullmatch(line)
            if match is None:
                raise ManifestError("unknown compiled inventory line")
            summary = (int(match[1]), int(match[2]))
    if not names or summary != (len(names), 0):
        raise ManifestError("missing or inconsistent compiled inventory summary")
    return frozenset(names)


def verify(required, inventories, *, expected_suites=SUITES):
    """Require exact complete agreement, including unexpected ignored cases."""
    if set(required) != expected_suites or set(inventories) != expected_suites:
        raise ManifestError("incomplete inventory inputs")
    for identity in sorted(expected_suites):
        if parse_inventory(inventories[identity]) != required[identity]:
            raise ManifestError("compiled inventory differs from required suite: " + identity)
    return sum(map(len, required.values()))


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("inventory_directory", type=Path)
    args = parser.parse_args()
    try:
        required = parse_manifest(read_regular(
            Path(__file__).with_name("required-suites.json"), MANIFEST_LIMIT
        ))
        inventories = {
            identity: read_regular(args.inventory_directory / (identity + ".txt"), INVENTORY_LIMIT)
            for identity in SUITES
        }
        count = verify(required, inventories)
    except ManifestError as error:
        print("Workflow resource manifest rejected: " + str(error), file=sys.stderr)
        return 1
    print("Workflow resource manifest verified: 3 suites, " + str(count)
          + " exact resource tests; native-workflow-only.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
