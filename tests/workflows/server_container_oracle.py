"""Independent literal computations and the container's internal SDK origin."""
import re

ORIGINS = frozenset(("server_container_sdk.py", "server_container_oracle.py",
                     "handler_sdk_image.py", "lambda_fault_support.py", "stepfunctions_history.py",
                     "stepfunctions_nested.py"))


def failure_origin(error):
    """Return public source coordinates only; never format the exception."""
    trace, origin = error.__traceback__, "unknown"
    for _ in range(32):
        if trace is None:
            return origin
        name = trace.tb_frame.f_code.co_filename.rsplit("/", 1)[-1]
        if name in ORIGINS and 1 <= trace.tb_lineno <= 10000:
            origin = name + ":" + str(trace.tb_lineno)
        trace = trace.tb_next
    return "unknown"


def internal_origin(value):
    assert isinstance(value, str) and re.fullmatch(r"http://rustack-server-[0-9a-f]{32}:4566/", value)
    return value


def container_id(value):
    assert isinstance(value, str) and re.fullmatch(r"[0-9a-f]{64}", value)
    return value


def result(value, computed, counter, architecture):
    assert isinstance(value, dict) and set(value) == {
        "computed", "counter", "versions", "selectorsCleared", "nodeVersion", "architecture"}
    assert type(value["computed"]) is int and value["computed"] == computed
    assert type(value["counter"]) is int and value["counter"] == counter
    assert value["versions"] == {"s3": "3.1105.0", "dynamodb": "3.1105.0"}
    assert value["selectorsCleared"] is True and value["nodeVersion"] == "24.22.1"
    assert value["architecture"] == {"arm64": "arm64", "x86_64": "x64"}[architecture]


def warm(previous, current):
    assert isinstance(current, dict) and set(current) == {"function", "relay"}
    assert container_id(current["function"]) != container_id(current["relay"])
    if previous is not None:
        assert previous == current, "server-container warm environment changed"
    return current
