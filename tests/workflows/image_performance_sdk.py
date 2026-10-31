"""Signed real-image workloads; private owner joins this child before publication."""
import concurrent.futures
import contextlib
import http.client
import importlib.metadata
import json
import os
from pathlib import Path
import platform
import re
import sys
import time
import urllib.parse

sys.path.insert(0, str(Path(__file__).resolve().parent))

from image_performance_support import MeasurementError, canonical, encode, integer, live, micros, overhead, require, samples, wait_task_baseline


def capability(endpoint, connection=None, path="/_rustack/capabilities"):
    address = urllib.parse.urlsplit(endpoint)
    require(address.scheme == "http" and address.hostname in ("127.0.0.1", "localhost"), "sdk-endpoint")
    owned = connection is None
    if owned:
        connection = http.client.HTTPConnection(address.hostname, address.port, timeout=2)
    began = time.monotonic_ns()
    try:
        require(path in ("/_rustack/capabilities", "/_health/ready"), "health-path")
        connection.request("GET", path)
        response = connection.getresponse()
        raw = response.read(65537)
        require(response.status == 200 and len(raw) <= 65536, "health-response")
        value = json.loads(raw)
        require(value["ready"] is True, "unready")
        if path == "/_rustack/capabilities":
            require(value["workers"]["docker"]["ready"] is True, "docker-unready")
        else:
            require(isinstance(value.get("services"), dict)
                    and value["services"]
                    and all(state in ("running", "disabled") for state in value["services"].values()), "health-services")
        elapsed = micros(time.monotonic_ns() - began)
        require(elapsed < 2000000, "late-health")
        return value, elapsed
    finally:
        if owned:
            connection.close()


def flood_failure(collector, phase, expected, future):
    with collector.condition:
        result = {"phase": phase, "expected": expected, "begun": len(collector.started),
                  "released": len(collector.released), "ended": len(collector.ended), "outcome": "pending"}
    if future.done():
        try:
            future.result(timeout=0)
            result["outcome"] = "unexpected-success"
        except Exception as error:
            response = getattr(error, "response", {})
            detail = response.get("Error", {}) if isinstance(response, dict) else {}
            if not isinstance(detail, dict): detail = {}
            message = detail.get("Message", "")
            if not isinstance(message, str): message = ""
            if detail.get("Code") == "ServiceException":
                result["outcome"] = ("handler-timeout" if "function timed out after 1s" in message else
                    "function-exit" if "function exited with status" in message else "service-error")
            else:
                result["outcome"] = "sdk-or-response-error"
    return result


def observe_flood(collector, rows, count, phase, future):
    began = time.monotonic_ns()
    try:
        return collector.wait_for(rows, count)
    except MeasurementError:
        diagnostic = flood_failure(collector, phase, count, future)
        diagnostic["elapsedMicros"] = micros(time.monotonic_ns() - began)
        print("image-performance-flood-failure " + encode(diagnostic).decode(), file=sys.stderr, flush=True)
        raise


def process_stats(pid):
    if sys.platform != "linux":
        return {"rssKiB": None, "tasks": None}
    raw = Path("/proc") / str(pid) / "status"
    with raw.open("rb") as stream:
        text = stream.read(65537)
    require(len(text) <= 65536, "process-status-bound")
    rss = re.search(rb"^VmRSS:\s+(\d+) kB$", text, re.M)
    tasks = re.search(rb"^Threads:\s+(\d+)$", text, re.M)
    require(rss is not None and tasks is not None, "process-status-shape")
    return {"rssKiB": int(rss[1]), "tasks": int(tasks[1])}


def exercise(endpoint, image, architecture, phase, socket, owner, engine, gateway, pid):
    from custom_ric_sdk import Collector, Matrix, collecting
    from lambda_fault_support import Metadata
    from botocore.exceptions import ClientError

    class Flood(Collector):
        def __init__(self):
            self.started, self.ended, self.released = {}, {}, set()
            super().__init__()
            self.deadline = time.monotonic() + 400

        def exchange(self, path, event):
            with self.condition:
                request = canonical(event["requestId"])
                require(not self.aborted and time.monotonic() < self.deadline, "flood-expired")
                require(isinstance(event["functionName"], str) and len(event["functionName"]) <= 64,
                        "flood-function")
                if path == "/flood-begin":
                    require(set(event) == {"functionName", "requestId", "beforeBytes"}
                            and request not in self.started and len(self.started) < 100
                            and integer(event["beforeBytes"], 1, 1048575), "flood-begin")
                    self.started[request] = event
                    self.condition.notify_all()
                    cutoff = min(self.deadline, time.monotonic() + 2)
                    while request not in self.released and not self.aborted:
                        remaining = cutoff - time.monotonic()
                        require(remaining > 0, "flood-release-expired")
                        self.condition.wait(remaining)
                    require(not self.aborted, "flood-aborted")
                    return {"release": True}
                require(path == "/flood-end" and request in self.released and request not in self.ended
                        and set(event) == {"functionName", "requestId", "beforeBytes", "afterBytes"}
                        and event["functionName"] == self.started[request]["functionName"]
                        and event["beforeBytes"] == self.started[request]["beforeBytes"], "flood-end")
                result = {key: event[key] for key in ("requestId", "beforeBytes", "afterBytes")}
                live(1, 1, 1, False, result)
                self.ended[request] = result
                self.condition.notify_all()
                return {"recorded": True}

        def wait_for(self, rows, count):
            cutoff = time.monotonic() + 3
            with self.condition:
                while len(rows) < count and not self.aborted:
                    remaining = cutoff - time.monotonic()
                    require(remaining > 0, "flood-observation-expired")
                    self.condition.wait(remaining)
                require(not self.aborted and len(rows) == count, "flood-observation-count")
                return list(rows)[-1]

    matrix = Matrix(endpoint, image, architecture)
    metadata = Metadata(socket, owner, engine)
    address = urllib.parse.urlsplit(endpoint)
    health_connection = http.client.HTTPConnection(address.hostname, address.port, timeout=2)
    request_ids, containers, environments = set(), set(), []

    def pair():
        rows = metadata.rows()
        require(len(rows) == 2 and {row["Labels"]["rustack.lambda.role"] for row in rows}
                == {"function", "relay"}, "environment-pair")
        identities = {row["Labels"]["rustack.lambda.environmentId"] for row in rows}
        require(len(identities) == 1, "pair-affiliation")
        return {row["Id"] for row in rows}, canonical(next(iter(identities)))

    def create(**fields):
        name = matrix.create(**fields)
        matrix.client.put_function_concurrency(FunctionName=name, ReservedConcurrentExecutions=1)
        return name

    def invoke(name, counter, delay=0):
        started = time.monotonic_ns()
        output, _ = matrix.invoke(name, {"mode": "timed", "value": 35, "delayMillis": delay})
        elapsed = time.monotonic_ns() - started
        require(output["computed"] == 42 and output["counter"] == counter
                and output["imageRevision"] == "A", "handler-computation")
        request = canonical(output["requestId"])
        require(request not in request_ids, "duplicate-request")
        request_ids.add(request)
        handler = output["handlerNanos"]
        overhead(elapsed, handler)
        return elapsed, handler

    def delete(name):
        cutoff = time.monotonic() + 15
        matrix.client.delete_function(FunctionName=name)
        matrix.functions.remove(name)
        metadata.wait_ids(set(), cutoff)
        while True:
            value, _ = ready()
            require(time.monotonic() < cutoff, "idle-retirement-capacity-expired")
            capacity = value["workers"]["docker"]["capacity"]
            if capacity["allocated"] == capacity["memoryBytes"] == capacity["quarantined"] == 0:
                break
            time.sleep(0.01)

    def admission(value):
        return value["workers"]["lambdaAdmission"]["activeExecutions"]

    def ready(path="/_rustack/capabilities"):
        return capability(endpoint, health_connection, path)

    def settled_tasks():
        settled, _ = ready()
        require(admission(settled) == 0 and settled["workers"]["lambdaAdmission"]["acceptedInvocations"] == 0,
                "invocation-capacity-leak")
        capacity = settled["workers"]["docker"]["capacity"]
        require(capacity["allocated"] == capacity["memoryBytes"] == capacity["quarantined"] == 0
                and capacity["accepting"] is True and capacity["workerFailed"] is False,
                "environment-capacity-leak")
        return settled["workers"]["runtimeTasks"]

    try:
        require(not metadata.ids(), "nonempty-initial-namespace")
        baseline = process_stats(pid)
        if phase == "warm":
            name = create()
            invoke(name, 1)
            original, environment = pair()
            total, handler, adapter = [], [], []
            for counter in range(2, 102):
                full, user = invoke(name, counter, 10)
                total.append(full); handler.append(user); adapter.append(overhead(full, user))
                require(pair() == (original, environment), "warm-environment-changed")
            measured = {"totalNanos": total, "handlerNanos": handler, "overhead": samples(adapter, 100)}
            require(measured["overhead"]["p95Micros"] <= 50000, "warm-overhead-budget")
            delete(name)
        elif phase == "cold":
            durations = []
            for _ in range(20):
                require(not metadata.ids(), "cold-environment-reused")
                name = create()
                elapsed, _ = invoke(name, 1)
                selected, environment = pair()
                require(not selected & containers and environment not in environments, "cold-identity-reused")
                containers.update(selected); environments.append(environment); durations.append(micros(elapsed))
                delete(name)
            measured = {"latency": samples(durations, 20), "environments": environments,
                        "uniqueContainers": len(containers)}
            require(measured["latency"]["p95Micros"] <= 5000000, "cold-budget")
        else:
            require(phase == "cycles", "workload-phase")
            health, timeouts, floods, stats, quiescence, brackets = [], [], [], [], [], []
            with collecting(Flood) as collector, concurrent.futures.ThreadPoolExecutor(max_workers=1) as workers:
                priming = create()
                invoke(priming, 1)
                delete(priming)
                name = create(Timeout=1, Environment={"Variables": {
                    "IMAGE_FIXTURE_COLLECTOR": gateway + ":" + str(collector.server_port),
                    "IMAGE_FIXTURE_TOKEN": collector.token}})
                initial, _ = ready()
                task_baseline = initial["workers"]["runtimeTasks"]
                for index in range(100):
                    invoke(name, 1)
                    selected, _ = pair()
                    started = time.monotonic_ns()
                    future = workers.submit(matrix.invoke, name, {"mode": "timed-log-flood", "delayMillis": 5000})
                    request = observe_flood(collector, collector.started, index + 1, "begin", future)
                    before, _ = ready()
                    _, observed = ready("/_health/ready")
                    after, _ = ready()
                    completed = future.done()
                    require(admission(before) == admission(after) == 1 and not completed
                            and pair()[0] == selected, "health-not-during-live-flood")
                    with collector.condition:
                        collector.released.add(request)
                        collector.condition.notify_all()
                    observe_flood(collector, collector.ended, index + 1, "end", future)
                    flood = collector.ended[request]
                    live(admission(before), admission(after), observed, completed, flood)
                    brackets.append([admission(before), admission(after), completed])
                    try:
                        future.result(timeout=65)
                    except ClientError as error:
                        returned = time.monotonic_ns()
                        require(error.response["Error"]["Code"] == "ServiceException"
                                and "function timed out after 1s" in error.response["Error"].get("Message", ""),
                                "timeout-classification")
                    else:
                        raise MeasurementError("timeout-returned-success")
                    timeouts.append(micros(returned - started)); health.append(observed); floods.append(flood)
                    require(not metadata.ids(), "timeout-returned-before-retirement")
                    elapsed, tasks = wait_task_baseline(settled_tasks, task_baseline, returned)
                    quiescence.append(elapsed)
                    current = process_stats(pid)
                    current["runtimeTasks"] = tasks
                    stats.append(current)
                delete(name)
            measured = {"readiness": samples(health, 100), "timeout": samples(timeouts, 100),
                        "floods": floods, "processStats": stats, "completedCycles": 100,
                        "containerHighWater": 2, "emptyAfterEachTimeout": True,
                        "runtimeTaskBaseline": task_baseline, "taskQuiescence": samples(quiescence, 100),
                        "liveBrackets": brackets, "primingInvocations": 1, "primingCleanupConfirmed": True}
        return {"schemaVersion": 1, "phase": phase, "referenceQualification": "pending",
                "os": sys.platform, "architecture": platform.machine(), "python": platform.python_version(),
                "sdkVersion": importlib.metadata.version("boto3"), "baselineProcess": baseline,
                "measurement": measured, "emptyAfterWorkload": True}
    finally:
        matrix.cleanup()
        matrix.client.close()
        health_connection.close()


def main():
    require(len(sys.argv) == 12, "workload-arguments")
    print(os.getpid(), flush=True)
    sdk_root = sys.argv[11]
    if sdk_root != "-":
        from linux_performance import safe_path
        sys.path.insert(0, str(safe_path(sdk_root)))
    from linux_performance import deadline_guard
    with deadline_guard(400):
        require(importlib.metadata.version("boto3") == importlib.metadata.version("botocore") == "1.43.118",
                "sdk-version")
        result = exercise(*sys.argv[1:9], int(sys.argv[9]))
        require(sys.argv[10] == "v1", "workload-schema")
        print(encode(result).decode(), flush=True)


if __name__ == "__main__":
    try:
        main()
    except MeasurementError as error:
        print("image-performance-workload-failed:" + str(error), file=sys.stderr)
        raise SystemExit(2) from None
    except Exception:
        print("image-performance-workload-failed", file=sys.stderr)
        raise SystemExit(2) from None
