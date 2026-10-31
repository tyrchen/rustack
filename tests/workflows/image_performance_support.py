"""Exact sample, cold-stage and live-flood oracles; no SDK or network dependency."""
import json
import hashlib
from pathlib import Path
import re
import sys
import time
import uuid

sys.path.insert(0, str(Path(__file__).resolve().parents[2] / "tools/github-action"))
from metadata import document


class MeasurementError(ValueError):
    """Fixed local diagnostic codes, without candidate or SDK payload values."""


def require(condition, code):
    if not condition:
        raise MeasurementError(code)


def integer(value, low, high):
    return type(value) is int and low <= value <= high


def micros(nanos):
    require(integer(nanos, 0, 480000000000), "duration-bound")
    return (nanos + 999) // 1000


def samples(values, count):
    require(count in (20, 100) and isinstance(values, list) and len(values) == count
            and all(integer(value, 0, 480000000) for value in values), "sample-count-or-range")
    ranked = sorted(values)
    return {"observedMicros": values, "p50Micros": ranked[(count - 1) // 2],
            "p95Micros": ranked[(95 * count + 99) // 100 - 1], "maxMicros": ranked[-1]}


def overhead(total, handler):
    require(integer(total, 1, 60000000000) and integer(handler, 1, total), "handler-clock")
    return micros(total - handler)


def exact_samples(value, count):
    require(isinstance(value, dict) and set(value) == {"observedMicros", "p50Micros", "p95Micros", "maxMicros"}
            and all(integer(value[key], 0, 480000000) for key in ("p50Micros", "p95Micros", "maxMicros"))
            and value == samples(value["observedMicros"], count), "sample-ranks")
    return value


def wait_task_baseline(read, baseline, started, clock=time.monotonic_ns, pause=time.sleep):
    require(integer(baseline, 1, 4096), "runtime-task-baseline")
    cutoff = started + 2000000000
    while True:
        require(clock() < cutoff, "task-quiescence-before-observation")
        value = read()
        observed = clock()
        require(observed < cutoff and integer(value, 1, 4096), "task-quiescence-late-observation")
        if value == baseline:
            return micros(observed - started), value
        pause(min(0.005, (cutoff - observed) / 1000000000))


def canonical(value):
    require(isinstance(value, str) and len(value) == 36 and str(uuid.UUID(value)) == value,
            "environment-identity")
    return value


def cold_stages(raw, environments, total_micros, baseline=b""):
    require(isinstance(raw, bytes) and len(raw) <= 32768, "initialization-log-bound")
    require(isinstance(baseline, bytes) and len(baseline) <= 32768 and raw.startswith(baseline)
            and (not baseline or baseline.endswith(b"\n")), "initialization-baseline")
    require(len(environments) == len(set(environments)) == len(total_micros) == 20,
            "cold-identity-count")
    expected = {canonical(name): value for name, value in zip(environments, total_micros)}
    observed = {}
    text = re.sub(rb"\x1b\[[0-9;]*m", b"", raw[len(baseline):]).decode("utf-8")
    for line in text.splitlines():
        if "rustack_lambda_core::docker::initialization" not in line:
            continue
        require(len(line.encode()) <= 1024, "initialization-line-bound")
        match = re.search(r"Image environment ready environment_id=([0-9a-f-]{36}) "
                          r"relay_micros=([0-9]{1,12}) ric_micros=([0-9]{1,12})$", line)
        require(match is not None, "initialization-event-shape")
        name, relay, ric = match[1], int(match[2]), int(match[3])
        canonical(name)
        require(name in expected and name not in observed and relay + ric <= expected[name] + 2,
                "initialization-affiliation-or-clock")
        observed[name] = (relay, ric)
    require(set(observed) == set(expected), "initialization-event-count")
    return {"relay": samples([observed[name][0] for name in environments], 20),
            "functionRic": samples([observed[name][1] for name in environments], 20),
            "startupBaseline": {"bytes": len(baseline), "sha256": hashlib.sha256(baseline).hexdigest()},
            "events": [{"environmentId": name, "relayMicros": row[0], "ricMicros": row[1]}
                       for name, row in observed.items()]}


def verify_stages(value, environments, total_micros):
    require(isinstance(value, dict) and set(value) == {"relay", "functionRic", "startupBaseline", "events"}
            and isinstance(value["events"], list) and len(value["events"]) == 20, "cold-normalized-events")
    before = value["startupBaseline"]
    require(isinstance(before, dict) and set(before) == {"bytes", "sha256"}
            and integer(before["bytes"], 0, 32768) and isinstance(before["sha256"], str)
            and re.fullmatch(r"[0-9a-f]{64}", before["sha256"]) is not None, "cold-baseline-proof")
    rows = []
    for event in value["events"]:
        require(isinstance(event, dict) and set(event) == {"environmentId", "relayMicros", "ricMicros"}
                and integer(event["relayMicros"], 0, 480000000) and integer(event["ricMicros"], 0, 480000000),
                "cold-normalized-event")
        name = canonical(event["environmentId"])
        rows.append(("rustack_lambda_core::docker::initialization: Image environment ready environment_id="
                     + name + " relay_micros=" + str(event["relayMicros"]) + " ric_micros=" + str(event["ricMicros"])).encode())
    normalized = cold_stages(b"\n".join(rows), environments, total_micros)
    require(exact_samples(value["relay"], 20) == normalized["relay"]
            and exact_samples(value["functionRic"], 20) == normalized["functionRic"],
            "cold-normalized-ranks")
    return value


def live(before, after, ready_micros, completed, flood):
    require(before == after == 1 and integer(before, 1, 1) and integer(after, 1, 1)
            and integer(ready_micros, 0, 1999999) and completed is False, "nonlive-health")
    require(isinstance(flood, dict) and set(flood) == {"requestId", "beforeBytes", "afterBytes"},
            "flood-evidence-shape")
    canonical(flood["requestId"])
    require(integer(flood["beforeBytes"], 1, 1048575)
            and integer(flood["afterBytes"], flood["beforeBytes"] + 1, 1048575), "inactive-flood")


def decode(raw, maximum=32768):
    require(maximum in (32768, 65536), "measurement-byte-policy")
    return document(raw, maximum=maximum, string_limit=4096, member_limit=4096)


def encode(value):
    raw = json.dumps(value, separators=(",", ":"), allow_nan=False).encode()
    require(len(raw) <= 32768, "measurement-report-bound")
    decode(raw)
    return raw
