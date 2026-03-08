"""Shared utilities for Lambda compatibility tests."""

import uuid
import zipfile
import io


def unique_name(prefix: str) -> str:
    """Generate a unique function name with a random suffix."""
    return f"test-{prefix}-{uuid.uuid4().hex[:8]}"


def make_zip(handler_code: str = "", filename: str = "index.py") -> bytes:
    """Create a minimal zip file containing a single Python handler file.

    If handler_code is empty, a default echo handler is used.
    """
    if not handler_code:
        handler_code = (
            "def handler(event, context):\n"
            "    return {'statusCode': 200, 'body': 'ok'}\n"
        )
    buf = io.BytesIO()
    with zipfile.ZipFile(buf, "w", zipfile.ZIP_DEFLATED) as zf:
        zf.writestr(filename, handler_code)
    return buf.getvalue()
