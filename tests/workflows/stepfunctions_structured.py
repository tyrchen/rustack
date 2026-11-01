"""Independent SDK checks for structured frames and real consumed Native calls."""

import io
import json
import os
import platform
import time
import uuid
import zipfile
from pathlib import Path


def check_history(sfn, arn, round_kind, history):
    events = history(sfn, arn)
    assert events
    for index, event in enumerate(events, 1):
        assert event["id"] == index and 0 <= event.get("previousEventId", 0) < index
    assert any(event["type"] == round_kind for event in events), round_kind
    reverse = history(sfn, arn, reverse=True, data=False)
    assert [(e["id"], e.get("previousEventId", 0)) for e in reverse] == [(e["id"], e.get("previousEventId", 0)) for e in reversed(events)]
    by_id = {event["id"]: event for event in events}
    for event in events:
        if event["type"] == "MapIterationStarted":
            assert by_id[event["previousEventId"]]["type"] in ("MapStateStarted", "PassStateExited", "WaitStateExited", "TaskStateExited", "ParallelStateExited", "MapStateExited", "SucceedStateExited")
    return events


def catalog(sfn, create, call, terminal, history):
    fixture = json.loads(Path(__file__).with_name("structured-cases.json").read_text())
    assert fixture["schemaVersion"] == 1 and fixture["profile"] == "rustack-standard-jsonpath-structured-v1"
    assert len(fixture["cases"]) == 6 and len({case["id"] for case in fixture["cases"]}) == 6
    for case in fixture["cases"]:
        assert {"id", "definition", "input", "expectedOutput", "round"} <= set(case)
        machine = create(sfn, "SdkStructured-" + case["id"], case["definition"])
        started = call(sfn, "start_execution", stateMachineArn=machine, name="Run", input=json.dumps(case["input"]))
        result = terminal(sfn, started["executionArn"])
        assert result["status"] == "SUCCEEDED", (case["id"], result)
        assert json.loads(result["output"]) == case["expectedOutput"], (case["id"], result)
        events = check_history(sfn, started["executionArn"], case["round"], history)
        if "taskAttempts" in case:
            assert sum(event["type"].endswith("Scheduled") for event in events) == case["taskAttempts"]
    return len(fixture["cases"])


def deploy_gate(lam, bootstrap, markers, stop):
    directory = Path(markers).parent / ("fanout-stop" if stop else "fanout-overlap")
    directory.mkdir()
    (directory / "consumed").mkdir()
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w") as archive:
        info = zipfile.ZipInfo("bootstrap")
        info.external_attr = 0o755 << 16
        archive.writestr(info, Path(bootstrap).read_bytes())
    function = "SdkFanoutStop" if stop else "SdkFanoutGate"
    lam.create_function(FunctionName=function, Runtime="provided.al2023", Role="arn:aws:iam::000000000000:role/Workflows",
                        Handler="bootstrap", Code={"ZipFile": buffer.getvalue()}, Timeout=900,
                        Architectures=["arm64" if platform.machine().lower() in ("arm64", "aarch64") else "x86_64"],
                        Environment={"Variables": {"WORKFLOW_MODE": "1", "PROCESS_MARKER_DIR": str(markers),
                                                   "WORKFLOW_INVOCATION_DIR": str(directory / "consumed"),
                                                   "WORKFLOW_GATE_FILE": str(directory / "release")}})
    return directory, function


def consumed(directory):
    deadline = time.monotonic() + 10
    while time.monotonic() < deadline:
        files = list((directory / "consumed").glob("*.json"))
        assert len(files) <= 2 and all(path.stat().st_size <= 1024 for path in files)
        markers = set()
        for path in files:
            value = json.loads(path.read_bytes())
            assert value["pid"] > 1 and uuid.UUID(value["request_id"]).version == 4
            markers.add((value["pid"], value["request_id"]))
        if len(markers) == 2:
            assert len({pid for pid, _ in markers}) == 2 and len({request for _, request in markers}) == 2
            return markers
        time.sleep(0.005)
    raise AssertionError("two Native handlers did not consume before release")


def absent(pid):
    deadline = time.monotonic() + 5
    while time.monotonic() < deadline:
        try:
            os.kill(pid, 0)
        except ProcessLookupError:
            return
        time.sleep(0.02)
    raise AssertionError("stopped Native handler PID survived")


def gated(sfn, lam, bootstrap, markers, stop, create, call, terminal, history):
    directory, function = deploy_gate(lam, bootstrap, markers, stop)
    task = {"StartAt": "Call", "States": {"Call": {"Type": "Task", "Resource": "arn:aws:lambda:us-east-1:000000000000:function:" + function, "End": True}}}
    branches = [{"StartAt": "M", "States": {"M": {"Type": "Map", "MaxConcurrency": 2, "ItemProcessor": task, "End": True}}}] if stop else [task, task]
    definition = {"StartAt": "P", "States": {"P": {"Type": "Parallel", "Branches": branches, "End": True}}}
    machine = create(sfn, "SdkStructuredStop" if stop else "SdkStructuredOverlap", definition)
    started = call(sfn, "start_execution", stateMachineArn=machine, name="Run", input=json.dumps([{"n": 21}, {"n": 7}] if stop else {"n": 21}))
    observed = consumed(directory)
    if stop:
        call(sfn, "stop_execution", executionArn=started["executionArn"], error="SdkStructuredStop", cause="consumed nested tasks")
        result = terminal(sfn, started["executionArn"])
        assert result["status"] == "ABORTED" and result["error"] == "SdkStructuredStop"
        for pid, _ in observed:
            absent(pid)
        events = check_history(sfn, started["executionArn"], "MapIterationAborted", history)
        aborted = [event for event in events if event["type"] == "MapIterationAborted"]
        assert len(aborted) == 2 and {event["mapIterationAbortedEventDetails"]["index"] for event in aborted} == {0, 1}
        assert events[-1]["type"] == "ExecutionAborted"
    else:
        (directory / "release").write_bytes(b"release")
        result = terminal(sfn, started["executionArn"])
        assert result["status"] == "SUCCEEDED", result
        output = json.loads(result["output"])
        assert len(output) == 2 and all(value["computed"] == 42 for value in output)
        assert {(value["pid"], value["request_id"]) for value in output} == observed
        events = check_history(sfn, started["executionArn"], "ParallelStateSucceeded", history)
        fork = next(event["id"] for event in events if event["type"] == "ParallelStateStarted")
        entries = [event for event in events if event["type"] == "TaskStateEntered"]
        assert len(entries) == 2 and all(event["previousEventId"] == fork for event in entries)


def exercise(sfn, lam, bootstrap, markers, create, call, terminal, history, assert_omitted):
    original_history = history
    def checked_history(*args, **kwargs):
        events = original_history(*args, **kwargs)
        if kwargs.get("data") is False:
            assert_omitted(events)
        return events
    cases = catalog(sfn, create, call, terminal, checked_history)
    for stop in [False, True]:
        gated(sfn, lam, bootstrap, markers, stop, create, call, terminal, checked_history)
    return cases
