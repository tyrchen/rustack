"""Exact functional Docker resources; this policy cannot certify a full release."""
import argparse
from pathlib import Path
import sys

from required_suites import (
    INVENTORY_LIMIT, MANIFEST_LIMIT, ManifestError, parse_manifest as decode_manifest,
    read_regular, verify as verify_exact,
)


SUITES = frozenset(("ordinary-startup", "docker-core", "docker-environment", "docker-executor",
                    "docker-endpoint", "docker-probe", "docker-wire", "relay-runtime"))


def parse_manifest(raw):
    return decode_manifest(raw, expected_suites=SUITES, profile="rustack-docker-image-v1",
                           qualification="docker-image-functional")


def verify(required, inventories):
    return verify_exact(required, inventories, expected_suites=SUITES)


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("inventory_directory", type=Path)
    args = parser.parse_args()
    try:
        required = parse_manifest(read_regular(
            Path(__file__).with_name("docker-required-suites.json"), MANIFEST_LIMIT))
        inventories = {identity:read_regular(args.inventory_directory / (identity + ".txt"), INVENTORY_LIMIT)
                       for identity in SUITES}
        count = verify(required, inventories)
    except ManifestError as error:
        print("Docker resource manifest rejected: " + str(error), file=sys.stderr)
        return 1
    print("Docker resource manifest verified: 8 suites, " + str(count)
          + " exact resource tests; docker-image-functional.")
    return 0


if __name__ == "__main__":
    sys.exit(main())
