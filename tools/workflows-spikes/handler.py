"""Real official-RIC handler used by the isolated P0 transport experiment."""

import os
import sys
from pathlib import Path
import time

counter = 0


def handler(event, context):
    global counter
    counter += 1
    if event.get("logs"):
        print("WIRE_STDOUT_CAPTURE", flush=True)
        print("WIRE_STDERR_CAPTURE", file=sys.stderr, flush=True)
    if event.get("fail"):
        raise ValueError("intentional P0 failure")
    if event.get("sleep"):
        print("P0_BUSY_START=" + context.aws_request_id, flush=True)
        time.sleep(60)
    cgroup = Path("/sys/fs/cgroup")
    response = {
        "computed": event["value"] + 7,
        "counter": counter,
        "uid": os.getuid(),
        "requestId": context.aws_request_id,
        "memoryMax": (cgroup / "memory.max").read_text().strip(),
        "pidsMax": (cgroup / "pids.max").read_text().strip(),
        "rootReadOnly": bool(os.statvfs("/").f_flag & os.ST_RDONLY),
    }
    if "padding" in event:
        response["inputBytes"] = len(event["padding"].encode("utf-8"))
    if event.get("response_bytes"):
        response["large"] = "r" * event["response_bytes"]
    if event.get("policy"):
        temporary = os.statvfs("/tmp")
        status = dict(
            line.split(":", 1) for line in Path("/proc/self/status").read_text().splitlines()
            if ":" in line
        )
        response["tmpBytes"] = temporary.f_blocks * temporary.f_frsize
        response["tmpMode"] = os.stat("/tmp").st_mode & 0o7777
        response["swapMax"] = (cgroup / "memory.swap.max").read_text().strip()
        response["cpuMax"] = (cgroup / "cpu.max").read_text().strip()
        response["capEff"] = status["CapEff"].strip()
        response["noNewPrivs"] = status["NoNewPrivs"].strip()
        response["taskRoot"] = os.environ.get("LAMBDA_TASK_ROOT")
        response["functionName"] = os.environ.get("AWS_LAMBDA_FUNCTION_NAME")
        response["functionVersion"] = os.environ.get("AWS_LAMBDA_FUNCTION_VERSION")
        response["invokedArn"] = context.invoked_function_arn
        response["region"] = os.environ.get("AWS_REGION")
        response["configuredValue"] = os.environ.get("TEST_VALUE")
    return response
