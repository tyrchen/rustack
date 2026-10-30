"""Causal barrier failure controls, including real quiet/header sockets."""
import concurrent.futures
import http.client
import json
import socket
import time
import unittest
import uuid

from custom_ric_sdk import Collector, collecting


class CollectorTests(unittest.TestCase):
    def test_should_join_quiet_and_partial_header_connections_on_shutdown(self):
        for payload in (b"", b"POST /consume HTTP/1.1\r\nX-Held: "):
            started = time.monotonic()
            with collecting() as collector:
                peer = socket.create_connection(("127.0.0.1", collector.server_port), timeout=2)
                try:
                    peer.sendall(payload)
                    deadline = time.monotonic() + 2
                    while True:
                        with collector.condition:
                            if collector.connections:
                                break
                        self.assertLess(time.monotonic(), deadline)
                        time.sleep(0.005)
                except BaseException:
                    peer.close()
                    raise
            with peer:
                try:
                    self.assertEqual(peer.recv(1), b"")
                except ConnectionResetError:
                    # Closing an unread partial request may retire TCP with RST.
                    pass
            self.assertLess(time.monotonic() - started, 3)
            self.assertFalse(collector.connections)

    def test_should_reject_expired_barrier_even_with_both_lanes_ready(self):
        collector = Collector()
        try:
            collector.consumed = {"A": str(uuid.uuid4()), "B": str(uuid.uuid4())}
            collector.checked = {"B": collector.consumed["B"]}
            collector.deadline = time.monotonic() - 1
            with self.assertRaises(AssertionError):
                collector.exchange("/checked", {"lane":"A", "requestId":collector.consumed["A"], "statuses":[400]*3})
            self.assertNotIn("A", collector.checked)
        finally:
            collector.server_close()

    def test_should_exchange_both_active_ids_and_release_only_after_both_checks(self):
        with collecting() as collector:
            ids = {lane: str(uuid.uuid4()) for lane in ("A", "B")}
            def post(path, value):
                connection = http.client.HTTPConnection("127.0.0.1", collector.server_port, timeout=3)
                try:
                    connection.request("POST", path, json.dumps(value), {"x-fixture-token":collector.token})
                    response = connection.getresponse()
                    self.assertEqual(response.status, 200)
                    return json.loads(response.read(2049))
                finally:
                    connection.close()
            with concurrent.futures.ThreadPoolExecutor(max_workers=2) as pool:
                first = pool.submit(post, "/consume", {"lane":"A", "requestId":ids["A"]})
                deadline = time.monotonic() + 2
                while "A" not in collector.consumed:
                    self.assertLess(time.monotonic(), deadline)
                    time.sleep(0.005)
                self.assertFalse(first.done())
                second = pool.submit(post, "/consume", {"lane":"B", "requestId":ids["B"]})
                self.assertEqual(first.result(3), {"foreignRequestId":ids["B"]})
                self.assertEqual(second.result(3), {"foreignRequestId":ids["A"]})
                first = pool.submit(post, "/checked", {"lane":"A", "requestId":ids["A"], "statuses":[400]*3})
                deadline = time.monotonic() + 2
                while "A" not in collector.checked:
                    self.assertLess(time.monotonic(), deadline)
                    time.sleep(0.005)
                self.assertFalse(first.done())
                second = pool.submit(post, "/checked", {"lane":"B", "requestId":ids["B"], "statuses":[400]*3})
                self.assertEqual(first.result(3), {"release":True})
                self.assertEqual(second.result(3), {"release":True})

    def test_should_abort_on_bad_token_and_oversized_headers(self):
        for extra in (b"x-fixture-token: wrong\r\nContent-Length: 2\r\n\r\n{}", b"X-Oversized: " + b"x"*4096 + b"\r\n\r\n"):
            with collecting() as collector:
                with socket.create_connection(("127.0.0.1", collector.server_port), timeout=2) as peer:
                    peer.sendall(b"POST /consume HTTP/1.1\r\nHost: localhost\r\n" + extra)
                    peer.recv(1024)
                self.assertTrue(collector.aborted)


if __name__ == "__main__":
    unittest.main()
