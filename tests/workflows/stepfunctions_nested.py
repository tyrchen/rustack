"""Nested ASL acceptance through the actual service and ordinary boto3."""

import io
import json
import platform
import time
import zipfile
from pathlib import Path

from stepfunctions_structured import absent


def definition(child, suffix, value):
    return {"StartAt": "T", "States": {"T": {"Type": "Task", "Resource": "arn:aws:states:::states:startExecution" + suffix,
                                           "Parameters": {"StateMachineArn": child, "Input": value}, "End": True}}}


def check_history(sfn, arn, suffix, child, history, omitted):
    events = history(sfn, arn)
    kinds = [e["type"] for e in events]
    expected = ["ExecutionStarted", "TaskStateEntered", "TaskScheduled", "TaskStarted"]
    if suffix:
        expected.append("TaskSubmitted")
    if kinds[-1] == "ExecutionSucceeded":
        expected += ["TaskSucceeded", "TaskStateExited", "ExecutionSucceeded"]
        assert kinds == expected, kinds
    for index, event in enumerate(events):
        assert event["id"] == index + 1 and event.get("previousEventId", 0) == (0 if index == 1 else index)
        if event["type"] == "TaskSubmitted":
            details = event["taskSubmittedEventDetails"]
            assert details["resourceType"] == "states" and details["resource"] == "startExecution" + suffix
            assert json.loads(details["output"])["ExecutionArn"] == child
    reverse = history(sfn, arn, reverse=True, data=False)
    omitted(reverse)
    assert [(e["id"], e.get("previousEventId", 0)) for e in reverse] == [(e["id"], e.get("previousEventId", 0)) for e in reversed(events)]


def exercise(sfn, lam, bootstrap, markers, create, call, terminal, history, omitted):
    catalog = json.loads(Path(__file__).with_name("nested-cases.json").read_text())
    assert catalog["schemaVersion"] == 1 and catalog["profile"] == "rustack-standard-jsonpath-nested-v1"
    assert len(catalog["cases"]) == 6 and len({c["id"] for c in catalog["cases"]}) == 6
    child = create(sfn, "SdkNestedEcho", {"StartAt": "D", "States": {"D": {"Type": "Pass", "End": True}}})
    for case in catalog["cases"]:
        suffix = case["suffix"]
        machine = create(sfn, "SdkNested-" + case["id"], definition(child, suffix, case["input"]))
        arn = call(sfn, "start_execution", stateMachineArn=machine, input="{}", name="Run")["executionArn"]
        result = terminal(sfn, arn)
        assert result["status"] == "SUCCEEDED", result
        output = json.loads(result["output"])
        nested = terminal(sfn, output["ExecutionArn"])
        assert json.loads(nested["output"]) == case["expected"]
        assert isinstance(output["StartDate"], int) and abs(output["StartDate"] - int(nested["startDate"].timestamp() * 1000)) <= 1
        if suffix:
            assert output["Status"] == "SUCCEEDED" and output["StateMachineArn"] == child
            assert output["InputDetails"] == output["OutputDetails"] == {"Included": True}
            if suffix == ".sync":
                assert isinstance(output["Input"], str) and isinstance(output["Output"], str)
                assert json.loads(output["Input"]) == json.loads(output["Output"]) == case["expected"]
                if isinstance(case["input"], str):
                    assert output["Input"] == case["input"]
            else:
                assert output["Input"] == output["Output"] == case["expected"]
        check_history(sfn, arn, suffix, output["ExecutionArn"], history, omitted)
    for stop in [False, True]:
        gated(sfn, lam, bootstrap, markers, stop, create, call, terminal, history, omitted)
    return len(catalog["cases"])


def gated(sfn, lam, bootstrap, markers, stop, create, call, terminal, history, omitted):
    name = "SdkNestedStop" if stop else "SdkNestedCompute"
    directory = Path(markers).parent / name
    directory.mkdir()
    (directory / "consumed").mkdir()
    buffer = io.BytesIO()
    with zipfile.ZipFile(buffer, "w") as archive:
        info = zipfile.ZipInfo("bootstrap")
        info.external_attr = 0o755 << 16
        archive.writestr(info, Path(bootstrap).read_bytes())
    lam.create_function(FunctionName=name, Runtime="provided.al2023", Role="arn:aws:iam::000000000000:role/Workflows",
                        Handler="bootstrap", Code={"ZipFile": buffer.getvalue()}, Timeout=900,
                        Architectures=["arm64" if platform.machine().lower() in ("arm64", "aarch64") else "x86_64"],
                        Environment={"Variables": {"WORKFLOW_MODE": "1", "PROCESS_MARKER_DIR": str(markers),
                                                   "WORKFLOW_INVOCATION_DIR": str(directory / "consumed"),
                                                   "WORKFLOW_GATE_FILE": str(directory / "release")}})
    child = create(sfn, name + "Child", {"StartAt": "L", "States": {"L": {"Type": "Task", "Resource": "arn:aws:lambda:us-east-1:000000000000:function:" + name, "End": True}}})
    machine = create(sfn, name + "Parent", definition(child, ".sync:2", {"n": 21}))
    arn = call(sfn, "start_execution", stateMachineArn=machine, input="{}")["executionArn"]
    deadline = time.monotonic() + 10
    while True:
        files = list((directory / "consumed").glob("*.json"))
        assert len(files) <= 1 and all(f.stat().st_size <= 1024 for f in files)
        if files:
            marker = json.loads(files[0].read_bytes())
            break
        assert time.monotonic() < deadline, "nested Native handler did not consume"
        time.sleep(0.005)
    submitted = [e for e in history(sfn, arn) if e["type"] == "TaskSubmitted"]
    assert len(submitted) == 1
    child_arn = json.loads(submitted[0]["taskSubmittedEventDetails"]["output"])["ExecutionArn"]
    if stop:
        call(sfn, "stop_execution", executionArn=arn, error="NestedStopped")
        assert terminal(sfn, arn)["status"] == "ABORTED"
        assert terminal(sfn, child_arn)["status"] == "ABORTED"
        absent(marker["pid"])
    else:
        (directory / "release").write_bytes(b"release")
        result = terminal(sfn, arn)
        assert result["status"] == "SUCCEEDED", result
        output = json.loads(result["output"])
        assert output["ExecutionArn"] == child_arn
        assert output["Output"]["computed"] == 42
        assert output["Output"]["pid"] == marker["pid"] and output["Output"]["request_id"] == marker["request_id"]
        check_history(sfn, arn, ".sync:2", child_arn, history, omitted)
