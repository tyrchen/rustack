"""Real official-RIC handler used by the isolated P0 transport experiment."""

import os
from pathlib import Path
import time

counter = 0


def handler(event, context):
    global counter
    counter += 1
    if event.get("fail"):
        raise ValueError("intentional P0 failure")
    if event.get("sleep"):
        print("P0_BUSY_START=" + context.aws_request_id, flush=True)
        time.sleep(60)
    cgroup = Path("/sys/fs/cgroup")
    return {
        "computed": event["value"] + 7,
        "counter": counter,
        "uid": os.getuid(),
        "requestId": context.aws_request_id,
        "memoryMax": (cgroup / "memory.max").read_text().strip(),
        "pidsMax": (cgroup / "pids.max").read_text().strip(),
        "rootReadOnly": bool(os.statvfs("/").f_flag & os.ST_RDONLY),
    }
