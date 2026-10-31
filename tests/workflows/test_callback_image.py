"""Privacy-oracle controls independent of the callback image and ASL evaluator."""

import hashlib
import unittest

from callback_image_support import PrivacyScanError, ScanBudget
from callback_image_sdk import Driver, catalog


TOKEN = b"0123456789abcdef" * 4
HASH = hashlib.sha256(TOKEN).hexdigest()
OTHER_HASH = hashlib.sha256(b"b" * 64).hexdigest()


class CallbackPrivacyControls(unittest.TestCase):
    def test_should_detect_exact_and_every_shifted_embedded_token(self):
        for prefix in range(64):
            with self.subTest(prefix=prefix):
                scanner = ScanBudget([HASH])
                with self.assertRaisesRegex(PrivacyScanError, "credential found"):
                    scanner.scan(b"a" * prefix + TOKEN + b"b" * 63)
                self.assertTrue(scanner.failed)

    def test_should_detect_token_split_at_every_decoder_boundary(self):
        for boundary in range(1, len(TOKEN)):
            with self.subTest(boundary=boundary):
                scanner = ScanBudget([HASH])
                with self.assertRaisesRegex(PrivacyScanError, "credential found"):
                    scanner.scan_chunks([b'{"value":"' + TOKEN[:boundary], TOKEN[boundary:] + b'"}'])

    def test_should_charge_all_overlapping_windows_across_sources(self):
        scanner = ScanBudget([OTHER_HASH], limit=3)
        scanner.scan(b"a" * 65)
        self.assertEqual(scanner.candidates, 2)
        scanner.scan(b"c" * 64)
        self.assertEqual(scanner.candidates, 3)
        with self.assertRaisesRegex(PrivacyScanError, "aggregate candidate cap"):
            scanner.scan(b"d" * 64)
        with self.assertRaisesRegex(PrivacyScanError, "already failed"):
            scanner.scan(b"safe")

    def test_should_accept_exact_global_cap_and_reject_next_source(self):
        scanner = ScanBudget([OTHER_HASH])
        scanner.scan(b"a" * (ScanBudget.MAX_CANDIDATES + 63))
        self.assertEqual(scanner.candidates, ScanBudget.MAX_CANDIDATES)
        with self.assertRaisesRegex(PrivacyScanError, "aggregate candidate cap"):
            scanner.scan_chunks([b"c" * 32, b"c" * 32])

    def test_should_reject_invalid_hashes_and_budget_before_scanning(self):
        for hashes in (["invalid"], ["A" * 64], [HASH] * 65, HASH, [1]):
            with self.subTest(kind=type(hashes).__name__), self.assertRaises(PrivacyScanError):
                ScanBudget(hashes)
        for values in ({"limit": 0}, {"limit": 65537}, {"candidates": -1}, {"candidates": 65537}):
            with self.subTest(values=values), self.assertRaises(PrivacyScanError):
                ScanBudget([HASH], **values)
        with self.assertRaisesRegex(PrivacyScanError, "requires observed hashes"):
            ScanBudget().scan(b"safe")

    def test_should_bound_chunks_sources_and_aggregate_history_bytes(self):
        with self.assertRaisesRegex(PrivacyScanError, "source cap"):
            ScanBudget([HASH]).scan(b"z" * (ScanBudget.MAX_SOURCE_BYTES + 1))
        with self.assertRaisesRegex(PrivacyScanError, "chunk cap"):
            ScanBudget([HASH]).scan_chunks([b""] * (ScanBudget.MAX_CHUNKS + 1))
        scanner = ScanBudget([HASH])
        for _ in range(16):
            scanner.scan(b"z" * ScanBudget.MAX_SOURCE_BYTES)
        with self.assertRaisesRegex(PrivacyScanError, "aggregate byte cap"):
            scanner.scan(b"z")


class CallbackGateControls(unittest.TestCase):
    def test_should_keep_retry_return_grants_distinct_and_reject_invalid_keys(self):
        class Table:
            def __init__(self):
                self.keys = []

            def put_item(self, *, TableName, Item):
                self.keys.append(Item["id"]["S"])

        table = Table()
        driver = Driver(None, None, table, None, "arm64", "timing")
        nonce = "1" * 32
        driver.gate(nonce, "return")
        driver.gate(nonce, "return", retry=1)
        self.assertEqual(table.keys, ["gate:" + nonce + ":0:return", "gate:" + nonce + ":1:return"])
        for selected, kind, retry in (("invalid", "return", 0), (nonce, "unknown", 0),
                                      (nonce, "return", 2), (nonce, "return", True)):
            with self.subTest(kind=kind, retry=retry), self.assertRaises(AssertionError):
                driver.gate(selected, kind, retry)
        self.assertEqual(len(table.keys), 2)

    def test_should_bind_both_groups_to_all_nine_independent_catalog_cases(self):
        first, second = catalog("causal"), catalog("timing")
        self.assertEqual(len(first), 4)
        self.assertEqual(len(second), 5)
        self.assertEqual(len({case["mode"] for case in first + second}), 9)
        self.assertEqual([(case["mode"], case["status"], case.get("error")) for case in second], [
            ("retry", "SUCCEEDED", None), ("heartbeat", "SUCCEEDED", None),
            ("absoluteTimeout", "FAILED", "States.Timeout"),
            ("heartbeatTimeout", "FAILED", "States.HeartbeatTimeout"), ("stop", "ABORTED", "Stopped")])


if __name__ == "__main__":
    unittest.main()
