"""Bounded secret-free hash observations for real image callback consumers."""

import hashlib
import re


class PrivacyScanError(ValueError):
    """A scan failed without including candidate or credential data."""


class ScanBudget:
    """One aggregate history candidate budget, continued by the Rust archive audit."""

    MAX_CANDIDATES = 65536
    MAX_SOURCE_BYTES = 1024 * 1024
    MAX_HISTORY_BYTES = 16 * 1024 * 1024
    MAX_CHUNKS = 4096

    def __init__(self, hashes=(), *, limit=MAX_CANDIDATES, candidates=0):
        if (type(limit) is not int or not 1 <= limit <= self.MAX_CANDIDATES
                or type(candidates) is not int or not 0 <= candidates <= limit):
            raise PrivacyScanError("invalid callback scan budget")
        self.limit = limit
        self.candidates = candidates
        self.history_bytes = 0
        self.failed = False
        self.hashes = set()
        self.add_hashes(hashes)

    def reject(self, message):
        self.failed = True
        raise PrivacyScanError(message)

    def add_hashes(self, hashes):
        if self.failed:
            raise PrivacyScanError("callback scan already failed")
        if not isinstance(hashes, (list, tuple, set)) or len(hashes) > 64:
            self.reject("invalid callback hash collection")
        for value in hashes:
            if not isinstance(value, str) or re.fullmatch(r"[0-9a-f]{64}", value) is None:
                self.reject("invalid callback hash")
            self.hashes.add(value)
            if len(self.hashes) > 64:
                self.reject("callback hash collection cap")

    def scan(self, data):
        if self.failed:
            raise PrivacyScanError("callback scan already failed")
        if not self.hashes:
            self.reject("callback scan requires observed hashes")
        if not isinstance(data, (bytes, bytearray)) or len(data) > self.MAX_SOURCE_BYTES:
            self.reject("callback history source cap")
        if self.history_bytes + len(data) > self.MAX_HISTORY_BYTES:
            self.reject("callback history aggregate byte cap")
        self.history_bytes += len(data)
        run = 0
        for offset, value in enumerate(data):
            run = run + 1 if 48 <= value <= 57 or 97 <= value <= 102 else 0
            if run < 64:
                continue
            if self.candidates == self.limit:
                self.reject("callback aggregate candidate cap")
            self.candidates += 1
            candidate = data[offset - 63:offset + 1]
            if hashlib.sha256(candidate).hexdigest() in self.hashes:
                self.reject("callback credential found in observed data")

    def scan_chunks(self, chunks):
        """Collect one capped source before scanning so chunk boundaries cannot hide data."""
        if self.failed:
            raise PrivacyScanError("callback scan already failed")
        data = bytearray()
        for count, chunk in enumerate(chunks, start=1):
            if count > self.MAX_CHUNKS or not isinstance(chunk, (bytes, bytearray)):
                self.reject("callback source chunk cap")
            if len(data) + len(chunk) > self.MAX_SOURCE_BYTES:
                self.reject("callback history source cap")
            data.extend(chunk)
        self.scan(data)
