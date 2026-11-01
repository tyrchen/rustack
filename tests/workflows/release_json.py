"""Bounded release-evidence JSON with duplicate, depth and number rejection."""
import json
import math
import os

LIMIT = 1024 * 1024


def document(raw):
    if not isinstance(raw, bytes) or not 1 <= len(raw) <= LIMIT:
        raise ValueError("release-json-byte-bound")
    text = raw.decode("utf-8")
    depth, quoted, escaped = 0, False, False
    for character in text:
        if quoted:
            if character == '"' and not escaped:
                quoted = False
            escaped = character == "\\" and not escaped
        elif character == '"':
            quoted, escaped = True, False
        elif character in "[{":
            depth += 1
            if depth > 16:
                raise ValueError("release-json-depth")
        elif character in "]}":
            depth -= 1
            if depth < 0:
                raise ValueError("release-json-depth")
    if quoted or depth:
        raise ValueError("release-json-shape")
    def pairs(rows):
        result = {}
        for key, value in rows:
            if key in result or len(key.encode()) > 256:
                raise ValueError("release-json-duplicate-key")
            result[key] = value
        return result
    def integer(value):
        if len(value) > 20:
            raise ValueError("release-json-integer")
        return int(value)
    def floating(value):
        number = float(value)
        if not math.isfinite(number):
            raise ValueError("release-json-float")
        return number
    def constant(_value):
        raise ValueError("release-json-constant")
    result = json.loads(text, object_pairs_hook=pairs, parse_int=integer, parse_float=floating, parse_constant=constant)
    pending, count = [result], 0
    while pending:
        item = pending.pop()
        count += 1
        if count > 32768:
            raise ValueError("release-json-member-bound")
        if isinstance(item, str) and len(item.encode()) > 16384:
            raise ValueError("release-json-string-bound")
        if isinstance(item, dict):
            pending.extend(item.values())
        elif isinstance(item, list):
            pending.extend(item)
    return result


def publish(path, value):
    raw = json.dumps(value, separators=(",", ":"), allow_nan=False).encode()
    if len(raw) > LIMIT:
        raise ValueError("release-json-output-bound")
    with path.open("xb") as output:
        os.chmod(path, 0o600)
        output.write(raw)
        output.flush()
        os.fsync(output.fileno())
    descriptor = os.open(path.parent, os.O_RDONLY | os.O_DIRECTORY)
    try:
        os.fsync(descriptor)
    finally:
        os.close(descriptor)
