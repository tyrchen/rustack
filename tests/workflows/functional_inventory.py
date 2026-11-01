"""Shared exact compiled inventory and retained one-resource command sequence."""
import time
import json

from linux_performance import require
from required_suites import parse_inventory
import rootless_platform as storage


def execute(owner, cargo, suites, required, selected, root, deadline, run_case, verify_output, clean):
    for identity, selector in suites.items():
        directory = root / ("inventory-" + identity)
        directory.mkdir(mode=0o700)
        result = run_case(owner, cargo + selector + ["--", "--ignored", "--list", "--color", "never"],
                          min(deadline, time.monotonic() + 300), directory)
        require(result.code == 0 and parse_inventory(result.stdout) == required[identity], "functional-compiled-inventory")
    rows = []
    for identity, selector in suites.items():
        for name in sorted(selected.get(identity, ())):
            directory = root / ("case-%03d" % (len(rows) + 1))
            directory.mkdir(mode=0o700)
            result = run_case(owner, cargo + selector + ["--", "--ignored", "--exact", name, "--show-output", "--test-threads=1"],
                              min(deadline, time.monotonic() + 1200), directory)
            try:
                if result.code == 0:
                    verify_output(result.stdout, {name})
            finally:
                clean(directory)
            rows.append({"suite":identity, "name":name, "code":result.code, "evidence":str(directory)})
            storage.publish(root / ("progress-%03d.json" % len(rows)), rows[-1])
            print(json.dumps({"resource":len(rows), "name":name, "code":result.code}), flush=True)
    return rows
