"""Record the complete existing Native gate and separate Action owner, unchanged."""
import os
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))
from linux_performance import ActionError, Runner, deadline_guard, require, safe_path
from rust_gates import run

GATES = (
    ("native", ["make", "--no-print-directory", "-j1", "verify-workflows-native"]),
    ("action", ["make", "--no-print-directory", "-j1", "test-workflows-action-fixture"]),
)
DOUBLED = (
    "persistence::test_should_restart_named_native_snapshots_without_replay_and_lose_only_unsaved_work",
    "performance::test_should_measure_one_hundred_fifty_state_signed_sdk_executions",
    "performance::test_should_keep_health_and_stop_responsive_and_reuse_one_execution_slot",
)
HTTP = "test_should_exchange_signed_boto3_requests_with_typed_transport"


def main():
    try:
        require(len(sys.argv) == 2, "native-gates-arguments")
        owner = Runner(os.environ)
        with deadline_guard(2400, owner) as deadline:
            run(safe_path(sys.argv[1]), owner, deadline, gates=GATES)
        return 0
    except (ActionError, OSError, ValueError, TypeError, KeyError):
        print("native-gates-rejected", file=sys.stderr)
        return 2


if __name__ == "__main__":
    sys.exit(main())
