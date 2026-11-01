"""Bound only command diagnostics; preserve the workload's filesystem limits."""
import os
from pathlib import Path
import shutil
import subprocess
import sys

LOG_LIMIT = 16 * 1024 * 1024


def run(path, arguments):
    if not arguments or arguments[0] not in ("cargo", "make"):
        raise ValueError("gate-command-policy")
    program = shutil.which(arguments[0])
    if program is None:
        raise ValueError("gate-command-unavailable")
    # This child inherits our process group. The outer Runner retains all
    # command/group retirement authority, including on diagnostic overflow.
    with path.open("xb") as log:
        os.chmod(path, 0o600)
        child = subprocess.Popen([program] + arguments[1:], stdin=subprocess.DEVNULL,
                                 stdout=subprocess.PIPE, stderr=subprocess.STDOUT)
        count = 0
        try:
            while True:
                block = child.stdout.read(min(8192, LOG_LIMIT - count + 1))
                if not block:
                    break
                count += len(block)
                if count > LOG_LIMIT:
                    raise ValueError("gate-log-bound")
                log.write(block)
            code = child.wait()
            log.flush()
            os.fsync(log.fileno())
            return code
        finally:
            child.stdout.close()


if __name__ == "__main__":
    try:
        if len(sys.argv) < 4:
            raise ValueError("gate-arguments")
        sys.exit(run(Path(sys.argv[1]), sys.argv[2:]))
    except (OSError, ValueError):
        print("gate-command-failed", file=sys.stderr)
        sys.exit(2)
