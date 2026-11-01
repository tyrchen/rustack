"""Causal per-target release and finite, unique asynchronous completion evidence."""
import concurrent.futures
import json
import time
from types import SimpleNamespace
import unittest
import uuid
from unittest.mock import Mock, patch

from custom_ric_sdk import collecting
from lambda_admission_sdk import QuotaHold, wait_work


def held(name):
    return {"stage":"busy", "functionName":name, "pid":1, "requestId":str(uuid.uuid4())}


def completed(ticket):
    return {"ticket":ticket, "functionName":"A", "requestId":str(uuid.uuid4()), "computed":15, "counter":1}


class AdmissionControls(unittest.TestCase):
    def test_should_reject_lower_actual_counts_even_when_reported_limits_match(self):
        for accepted, active, expected in [(8,1,8), (32,8,32), (7,8,8), (0,24,0)]:
            with self.subTest(accepted=accepted, active=active, expected=expected):
                response = Mock(status=200)
                response.read.return_value = json.dumps({"workers":{"lambdaAdmission":{
                    "acceptedInvocations":accepted,"activeExecutions":active,"accepting":True,
                    "limits":{"acceptedInvocations":128,"activeExecutions":32,"defaultFunctionExecutions":8}}}}).encode()
                connection = Mock()
                connection.getresponse.return_value = response
                with patch("lambda_admission_sdk.http.client.HTTPConnection", return_value=connection), \
                     patch("lambda_admission_sdk.time.monotonic", side_effect=[0,0,2]), \
                     patch("lambda_admission_sdk.time.sleep"), \
                     self.assertRaisesRegex(AssertionError, "ownership never reached"):
                    wait_work(SimpleNamespace(endpoint="http://127.0.0.1:9999"), expected, expected, 1)
                connection.close.assert_called_once()

    def test_should_release_only_the_observed_target_and_join_the_aborted_peer(self):
        with collecting(QuotaHold) as hold:
            hold.targets(["A", "B"])
            pool = concurrent.futures.ThreadPoolExecutor(max_workers=2)
            try:
                a = pool.submit(hold.exchange, "/hold", held("A"))
                b = pool.submit(hold.exchange, "/hold", held("B"))
                hold.wait_seen(2, time.monotonic() + 2)
                with self.assertRaises(AssertionError):
                    hold.release_target("Unknown")
                self.assertEqual(hold.released_targets, set())
                hold.release_target("A")
                self.assertEqual(a.result(2), {"release":True})
                self.assertFalse(b.done())
                with self.assertRaises(AssertionError):
                    hold.release_target("A")
                hold.abort()
                with self.assertRaises(AssertionError):
                    b.result(2)
            finally:
                hold.abort()
                pool.shutdown(wait=True)

    def test_should_refuse_expired_release_without_mutation(self):
        with collecting(QuotaHold) as hold:
            hold.targets(["A"])
            hold.observed["A"] = held("A")
            hold.deadline = time.monotonic() - 1
            with self.assertRaises(AssertionError):
                hold.release_target("A")
            self.assertEqual(hold.released_targets, set())

    def test_should_require_unique_expected_actual_computations_before_ready(self):
        with collecting(QuotaHold) as hold:
            hold.targets(["A"])
            hold.expect_tickets(2)
            first = completed(0)
            self.assertEqual(hold.exchange("/completed", first), {"recorded":True})
            invalid = [dict(first), dict(completed(1), requestId=first["requestId"]),
                       dict(completed(1), computed=0), dict(completed(1), counter=0),
                       dict(completed(1), functionName="Unknown"), dict(completed(1), ticket=True),
                       dict(completed(1), ticket=2)]
            for row in invalid:
                with self.subTest(row=row), self.assertRaises(AssertionError):
                    hold.exchange("/completed", row)
                self.assertEqual(hold.completed, {0:first})
            hold.exchange("/completed", completed(1))
            self.assertEqual(set(hold.wait_completed(time.monotonic() + 2)), {0,1})
            with self.assertRaises(AssertionError):
                hold.wait_completed(time.monotonic() - 1)


if __name__ == "__main__":
    unittest.main()
