"""Actual provided runtime using ordinary signed boto3 callback requests."""

import json
import os
import pathlib
import sys
import time
import urllib.parse
import urllib.request
import uuid

import boto3
from botocore.config import Config
from botocore.exceptions import ClientError


def run():
    endpoint = os.environ["CALLBACK_ENDPOINT"]
    parsed = urllib.parse.urlsplit(endpoint)
    assert len(endpoint) <= 128 and parsed.scheme == "http"
    assert parsed.hostname == "127.0.0.1" and parsed.port
    assert parsed.path in ("", "/") and not parsed.query and not parsed.fragment
    assert not parsed.username and not parsed.password
    client = boto3.client(
        "stepfunctions", endpoint_url=endpoint, region_name="us-east-1",
        aws_access_key_id=os.environ["CALLBACK_ACCESS_KEY"],
        aws_secret_access_key=os.environ["CALLBACK_SECRET_KEY"],
        config=Config(connect_timeout=2, read_timeout=4,
                      retries={"total_max_attempts": 1}, proxies={}),
    )
    api = os.environ["AWS_LAMBDA_RUNTIME_API"]
    assert len(api) <= 64 and api.startswith("127.0.0.1:")
    markers = pathlib.Path(os.environ["CALLBACK_MARKER_DIR"])
    gates = pathlib.Path(os.environ["CALLBACK_GATE_DIR"])
    processes = pathlib.Path(os.environ["PROCESS_MARKER_DIR"])
    assert markers.is_dir() and gates.is_dir() and processes.is_dir()
    pid = os.getpid()
    (processes / f"pid-{pid}").write_text(str(pid))
    opener = urllib.request.build_opener(urllib.request.ProxyHandler({}))
    base = f"http://{api}/2018-06-01/runtime/invocation/"

    def post(request_id, failure=False):
        route = "error" if failure else "response"
        payload = ({"errorType": "CallbackInvocationFailure", "errorMessage": "actual fixture error"}
                   if failure else {"ignored": "synchronous body"})
        with opener.open(urllib.request.Request(
            f"{base}{request_id}/{route}", data=json.dumps(payload).encode(), method="POST"
        ), timeout=4) as response:
            assert response.status == 202

    def record(request_id, stage, retry, heartbeats=0, safe=False, closed=False):
        evidence = dict(requestId=request_id, pid=pid, sdk="boto3", stage=stage,
                        retry=retry, heartbeats=heartbeats, historySafe=safe, closedToken=closed)
        pending = markers / f"{request_id}.pending"
        body = json.dumps(evidence)
        assert len(body.encode()) <= 1024
        pending.write_text(body)
        pending.replace(markers / f"{request_id}.json")

    def expect_closed(operation):
        try:
            operation()
        except ClientError as error:
            assert error.response["Error"]["Code"] == "TaskTimedOut"
        else:
            raise AssertionError("closed token accepted")

    previous = None
    while True:
        with opener.open(f"{base}next", timeout=60) as response:
            request_id = response.headers["Lambda-Runtime-Aws-Request-Id"]
            assert str(uuid.UUID(request_id)) == request_id and len(request_id) == 36
            body = response.read(524289)
            assert len(body) <= 524288
        event = json.loads(body)
        assert set(event) == {"token", "executionArn", "mode", "n", "retry"}
        token = event["token"]
        assert isinstance(token, str) and len(token) == 64
        assert all(value in "0123456789abcdef" for value in token)
        mode, retry = event["mode"], event["retry"]
        assert mode in {"early", "late", "externalFailure", "invocationFailure", "retry",
                        "heartbeat", "absoluteTimeout", "heartbeatTimeout", "stop"}
        assert type(retry) is int and 0 <= retry <= 1
        assert type(event["n"]) is int and 0 <= event["n"] <= 1000
        assert len(event["executionArn"]) <= 256
        assert event["executionArn"].startswith("arn:aws:states:us-east-1:000000000000:execution:")
        if retry == 1:
            assert previous is not None and previous != token
            expect_closed(lambda: client.send_task_heartbeat(taskToken=previous))
        record(request_id, "consumed", retry)
        late = mode in {"late", "heartbeat", "absoluteTimeout", "heartbeatTimeout"}
        if late:
            post(request_id)
            record(request_id, "returned", retry)
        if late or mode == "stop":
            until = time.monotonic() + 45
            count = 0
            while not (gates / f"{request_id}.release").exists() and not (gates / "all.release").exists():
                assert time.monotonic() < until
                if mode in {"heartbeat", "absoluteTimeout"}:
                    try:
                        response = client.send_task_heartbeat(taskToken=token)
                    except ClientError as error:
                        assert mode == "absoluteTimeout"
                        assert error.response["Error"]["Code"] == "TaskTimedOut"
                    else:
                        assert response["ResponseMetadata"]["HTTPStatusCode"] == 200
                        count += 1
                        record(request_id, "heartbeat", retry, heartbeats=count)
                time.sleep(0.1)
        if mode == "externalFailure" or mode == "retry" and retry == 0:
            response = client.send_task_failure(
                taskToken=token, error="CallbackRetry" if mode == "retry" else "CallbackExternalFailure",
                cause="actual handler SDK failure",
            )
            assert response["ResponseMetadata"]["HTTPStatusCode"] == 200
        else:
            complete = lambda: client.send_task_success(taskToken=token, output=json.dumps(
                dict(value=event["n"] * 2, sdk="boto3", requestId=request_id, retry=retry)))
            if mode in {"absoluteTimeout", "heartbeatTimeout"}:
                expect_closed(complete)
            else:
                assert complete()["ResponseMetadata"]["HTTPStatusCode"] == 200
        expect_closed(lambda: client.send_task_heartbeat(taskToken=token))
        history = client.get_execution_history(executionArn=event["executionArn"],
                                               includeExecutionData=True, maxResults=1000)
        safe = token not in json.dumps(history, default=str)
        assert safe and any(item["type"] == "TaskScheduled" for item in history["events"])
        record(request_id, "callback", retry, safe=safe, closed=True)
        if not late:
            until = time.monotonic() + 45
            while not (gates / f"{request_id}.release").exists() and not (gates / "all.release").exists():
                assert time.monotonic() < until
                time.sleep(0.1)
            post(request_id, failure=mode == "invocationFailure")
        previous = token


if __name__ == "__main__":
    try:
        run()
    except Exception:
        # No token-bearing SDK exception or input is printed by the fixture.
        sys.exit(1)
