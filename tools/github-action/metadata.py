"""Bounded public daemon metadata and exact Action-container affiliation."""
import json
import math
import re
import uuid

from settings import ActionError

NONCE_LABEL = "rustack.github-action.nonce"
ENGINE_LABEL = "rustack.github-action.engine"
IMAGE_FORMAT = '{"id":{{json .Id}},"os":{{json .Os}},"architecture":{{json .Architecture}}}'
CONTAINER_FORMAT = ('{"id":{{json .Id}},"name":{{json .Name}},"image":{{json .Image}},'
                    '"reference":{{json .Config.Image}},"labels":{{json .Config.Labels}},'
                    '"running":{{json .State.Running}},"dead":{{json .State.Dead}},'
                    '"oom":{{json .State.OOMKilled}},"exit":{{json .State.ExitCode}},'
                    '"startedAt":{{json .State.StartedAt}},"health":{{json .State.Health.Status}},'
                    '"ports":{{json .NetworkSettings.Ports}},'
                    '"bindings":{{json .HostConfig.PortBindings}},'
                    '"restart":{{json .HostConfig.RestartPolicy}}}')


def document(raw, maximum=32768, string_limit=256, member_limit=512):
    if type(member_limit) is not int or not 1 <= member_limit <= 4096:
        raise ActionError("metadata-member-policy")
    if not isinstance(raw, bytes) or len(raw) > maximum:
        raise ActionError("metadata-byte-budget")
    try:
        text = raw.decode("utf-8")
    except UnicodeDecodeError:
        raise ActionError("metadata-encoding") from None
    depth, quoted, escaped, size = 0, False, False, 0
    for char in text:
        if quoted:
            if char == '"' and not escaped:
                quoted = False
            else:
                size += len(char.encode("utf-8"))
                if size > string_limit:
                    raise ActionError("metadata-string-budget")
                escaped = char == "\\" and not escaped
        elif char == '"':
            quoted, size = True, 0
        elif char in "[{":
            depth += 1
            if depth > 8:
                raise ActionError("metadata-depth-budget")
        elif char in "]}":
            depth -= 1
            if depth < 0:
                raise ActionError("metadata-shape")
    if quoted or depth:
        raise ActionError("metadata-shape")
    def unique(pairs):
        result = {}
        for key, value in pairs:
            if key in result or len(key.encode("utf-8")) > 256:
                raise ActionError("metadata-duplicate-or-key")
            result[key] = value
        return result
    def constant(_value):
        raise ActionError("metadata-number")
    def finite(value):
        number = float(value)
        if not math.isfinite(number):
            raise ActionError("metadata-number")
        return number
    try:
        value = json.loads(text, object_pairs_hook=unique, parse_constant=constant, parse_float=finite)
    except (ValueError, RecursionError):
        raise ActionError("metadata-json") from None
    pending, members = [value], 0
    while pending:
        current = pending.pop()
        if isinstance(current, str):
            try:
                if len(current.encode("utf-8")) > string_limit:
                    raise ActionError("metadata-string-budget")
            except UnicodeEncodeError:
                raise ActionError("metadata-encoding") from None
        if isinstance(current, (dict, list)):
            members += len(current)
            if members > member_limit:
                raise ActionError("metadata-member-budget")
            pending.extend(current.values() if isinstance(current, dict) else current)
    return value


def identifier(value):
    if not isinstance(value, str) or re.fullmatch(r"[0-9a-f]{64}", value) is None:
        raise ActionError("container-identity")
    return value


def source_id(value):
    if not isinstance(value, str) or re.fullmatch(r"sha256:[0-9a-f]{64}", value) is None:
        raise ActionError("image-identity")
    return value


def engine_id(value):
    try:
        parsed = uuid.UUID(value)
    except (ValueError, AttributeError, TypeError):
        raise ActionError("daemon-identity") from None
    if parsed.int == 0 or str(parsed) != value:
        raise ActionError("daemon-identity")
    return value


def endpoint(value):
    if not isinstance(value, str) or not value.startswith("unix://"):
        raise ActionError("local-unix-daemon-required")
    path = value[7:]
    if len(path.encode("utf-8")) > 4096 or re.fullmatch(r"/[A-Za-z0-9_.-]+(?:/[A-Za-z0-9_.-]+)*", path) is None:
        raise ActionError("local-unix-daemon-required")
    if any(part in (".", "..") for part in path.split("/")[1:]):
        raise ActionError("local-unix-daemon-required")
    return value


def owned(view, cid, source, engine, nonce):
    if not isinstance(view, dict):
        raise ActionError("container-metadata")
    labels = view.get("labels")
    if (view.get("id") != cid or view.get("image") != source or view.get("reference") != source
            or view.get("name") != "/rustack" or not isinstance(labels, dict)
            or labels.get(NONCE_LABEL) != nonce or labels.get(ENGINE_LABEL) != engine):
        raise ActionError("container-affiliation")
    if (type(view.get("running")) is not bool or type(view.get("exit")) is not int
            or not 0 <= view["exit"] <= 255 or view.get("dead") is not False or view.get("oom") is not False):
        raise ActionError("container-state")
    return view


def publication(view, port):
    expected = {"4566/tcp": [{"HostIp": "127.0.0.1", "HostPort": str(port)}]}
    if (view.get("ports") != expected or view.get("bindings") != expected
            or view.get("restart") != {"Name": "no", "MaximumRetryCount": 0}):
        raise ActionError("container-publication")
    started = view.get("startedAt")
    if (not isinstance(started, str) or not 1 <= len(started) <= 64
            or not started.isascii() or any(ord(c) < 32 or ord(c) > 126 for c in started)):
        raise ActionError("container-start-identity")
    if view.get("running") is not True:
        raise ActionError("container-exited-before-handoff")
    if view.get("health") not in ("healthy", "starting", "unhealthy"):
        raise ActionError("container-health-metadata")
    return started, view["health"] == "healthy"


def readiness(raw):
    value = document(raw, maximum=65536)
    if not isinstance(value, dict) or value.get("ready") is not True:
        raise ActionError("http-not-ready")
    services = value.get("services")
    if not isinstance(services, dict) or not 1 <= len(services) <= 32:
        raise ActionError("http-services")
    for name, state in services.items():
        if (not isinstance(name, str) or re.fullmatch(r"[a-z0-9][a-z0-9_-]{0,63}", name) is None
                or state not in ("running", "disabled")):
            raise ActionError("http-services")
