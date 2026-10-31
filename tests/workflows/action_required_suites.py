"""One independent Action process suite; not Native or Docker qualification."""
import argparse
from pathlib import Path
import sys

from required_suites import MANIFEST_LIMIT, INVENTORY_LIMIT, ManifestError, parse_manifest, read_regular, verify

SUITES = frozenset(("composite-action",))
CASE = "test_should_execute_actual_action_process_controls"


def decode(raw):
    result = parse_manifest(raw, expected_suites=SUITES, profile="rustack-composite-action-v1",
                            qualification="action-process-controls")
    if result["composite-action"] != frozenset((CASE,)):
        raise ManifestError("Action process case changed")
    return result


def main():
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("inventory_directory", type=Path)
    arguments = parser.parse_args()
    try:
        required = decode(read_regular(Path(__file__).with_name("action-required-suites.json"), MANIFEST_LIMIT))
        inventory = read_regular(arguments.inventory_directory / "composite-action.txt", INVENTORY_LIMIT)
        count = verify(required, {"composite-action": inventory}, expected_suites=SUITES)
    except ManifestError as error:
        print("Action resource manifest rejected: " + str(error), file=sys.stderr)
        return 1
    print("Action resource manifest verified: " + str(count) + " exact independent process suite")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
