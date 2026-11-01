"""Validate retained raw measurement evidence after physical server shutdown."""
import os
from pathlib import Path
import sys

sys.path.insert(0, str(Path(__file__).resolve().parent))

from image_performance_support import MeasurementError, cold_stages, decode, encode, exact_samples, integer, live, overhead, require, samples, verify_stages
from linux_performance import bounded_read, deadline_guard
from settings import ActionError


def validate(value, phase, log=None, baseline=b""):
    fields = {"schemaVersion", "phase", "referenceQualification", "os", "architecture", "python",
              "sdkVersion", "baselineProcess", "measurement", "emptyAfterWorkload"}
    require(isinstance(value, dict) and set(value) in (fields, fields | {"debugAssertions"}), "measurement-fields")
    if "debugAssertions" in value:
        require(type(value["debugAssertions"]) is bool, "measurement-build-mode")
    require(type(value["schemaVersion"]) is int and value["schemaVersion"] == 1 and value["phase"] == phase
            and value["referenceQualification"] == "pending" and value["emptyAfterWorkload"] is True
            and value["sdkVersion"] == "1.43.118" and value["os"] in ("linux", "darwin")
            and value["architecture"] in ("aarch64", "arm64", "x86_64"), "measurement-identity")
    measured = value["measurement"]
    require(isinstance(measured, dict), "measurement-shape")
    if phase == "warm":
        require(set(measured) == {"totalNanos", "handlerNanos", "overhead"}
                and isinstance(measured["totalNanos"], list) and isinstance(measured["handlerNanos"], list)
                and len(measured["totalNanos"]) == len(measured["handlerNanos"]) == 100, "warm-count")
        values = [overhead(full, user) for full, user in zip(measured["totalNanos"], measured["handlerNanos"])]
        require(exact_samples(measured["overhead"], 100) == samples(values, 100)
                and measured["overhead"]["p95Micros"] <= 50000, "warm-ranks-or-budget")
    elif phase == "cold":
        require(set(measured) in ({"latency", "environments", "uniqueContainers"},
                                 {"latency", "environments", "uniqueContainers", "initialization"})
                and type(measured["uniqueContainers"]) is int and measured["uniqueContainers"] == 40,
                "cold-shape")
        require(exact_samples(measured["latency"], 20)
                and measured["latency"]["p95Micros"] <= 5000000, "cold-ranks-or-budget")
        if log is None:
            require("initialization" in measured, "cold-log-required")
            verify_stages(measured["initialization"], measured["environments"], measured["latency"]["observedMicros"])
        else:
            stages = cold_stages(log, measured["environments"], measured["latency"]["observedMicros"], baseline)
            if "initialization" in measured:
                require(stages == measured["initialization"], "cold-stages-changed")
            measured["initialization"] = stages
    else:
        require(phase == "cycles" and set(measured) == {"readiness", "timeout", "floods", "processStats",
            "completedCycles", "containerHighWater", "emptyAfterEachTimeout", "runtimeTaskBaseline",
            "taskQuiescence", "liveBrackets", "primingInvocations", "primingCleanupConfirmed", "controlChannels"}, "cycle-shape")
        require(type(measured["completedCycles"]) is int and measured["completedCycles"] == 100
                and type(measured["containerHighWater"]) is int and measured["containerHighWater"] == 2
                and measured["emptyAfterEachTimeout"] is True
                and type(measured["primingInvocations"]) is int and measured["primingInvocations"] == 1
                and measured["primingCleanupConfirmed"] is True
                and integer(measured["runtimeTaskBaseline"], 1, 4096), "cycle-cleanup")
        channels = measured["controlChannels"]
        require(isinstance(channels, dict) and set(channels) == {"prepared", "readyBeforeInvoke", "sameConnection", "closed"}
                and all(integer(value, 100, 100) for value in channels.values()), "cycle-control-channels")
        for key in ("readiness", "timeout", "taskQuiescence"):
            exact_samples(measured[key], 100)
        require(measured["taskQuiescence"]["maxMicros"] < 2000000, "task-quiescence-budget")
        require(isinstance(measured["floods"], list) and isinstance(measured["liveBrackets"], list)
                and len(measured["floods"]) == len(measured["liveBrackets"]) == 100, "flood-count")
        requests = set()
        for flood, bracket, duration in zip(measured["floods"], measured["liveBrackets"],
                                          measured["readiness"]["observedMicros"]):
            require(isinstance(bracket, list) and len(bracket) == 3, "live-bracket-shape")
            live(bracket[0], bracket[1], duration, bracket[2], flood)
            require(flood["requestId"] not in requests, "flood-request-reused")
            requests.add(flood["requestId"])
        require(isinstance(measured["processStats"], list) and len(measured["processStats"]) == 100,
                "process-stats-count")
        for row in measured["processStats"]:
            require(isinstance(row, dict) and set(row) == {"rssKiB", "tasks", "runtimeTasks"}
                    and type(row["runtimeTasks"]) is int and row["runtimeTasks"] == measured["runtimeTaskBaseline"],
                    "runtime-task-growth")
            if value["os"] == "linux":
                require(integer(row["rssKiB"], 1, 8 * 1024 * 1024) and integer(row["tasks"], 1, 4096),
                        "linux-process-stats")
            else:
                require(row["rssKiB"] is None and row["tasks"] is None, "host-process-stats")
    encode(value)
    return value


def main():
    try:
        require(len(sys.argv) == 5, "report-arguments")
        print(os.getpid(), flush=True)
        with deadline_guard(8):
            value = decode(bounded_read(Path(sys.argv[1]), 32768))
            raw = bounded_read(Path(sys.argv[2]), 32768)
            baseline = bounded_read(Path(sys.argv[4]), 32768)
            print(encode(validate(value, sys.argv[3], raw, baseline)).decode(), flush=True)
        return 0
    except MeasurementError as error:
        print("image-performance-report-rejected:" + str(error), file=sys.stderr)
        return 2
    except (ActionError, OSError, ValueError, TypeError, KeyError, RecursionError):
        print("image-performance-report-rejected", file=sys.stderr)
        return 2


if __name__ == "__main__":
    raise SystemExit(main())
