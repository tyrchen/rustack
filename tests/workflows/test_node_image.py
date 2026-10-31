"""The ordinary Node observer rejects responses at or beyond its original cutoff."""
import unittest

from node_image_sdk import poll_response


class NodeObserverControls(unittest.TestCase):
    def observe(self, ticks, responses):
        times = iter(ticks)
        values = iter(responses)
        return poll_response(lambda: next(values), lambda value: value == "done", 90,
                             clock=lambda: next(times), sleep=lambda _: None)

    def test_should_accept_terminal_before_original_cutoff(self):
        self.assertEqual(self.observe([0, 89], ["done"]), "done")

    def test_should_reject_terminal_at_original_cutoff(self):
        with self.assertRaises(TimeoutError):
            self.observe([0, 90], ["done"])

    def test_should_reject_late_terminal_after_running(self):
        with self.assertRaises(TimeoutError):
            self.observe([0, 1, 91], ["running", "done"])


if __name__ == "__main__":
    unittest.main()
