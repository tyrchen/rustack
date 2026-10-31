"""Clock fences for the ordinary archive observer; no cloud or Docker calls."""
import unittest
from unittest.mock import Mock, patch

import provided_zip_archive_sdk as archive


def completed():
    return dict(executionArn="execution", stateMachineArn="machine", status="SUCCEEDED",
                input="{}", output="{}", startDate=0, stopDate=1)


class ArchiveClockControls(unittest.TestCase):
    def test_should_accept_terminal_response_observed_before_original_cutoff(self):
        client = Mock()
        client.describe_execution.return_value = completed()
        with patch.object(archive.time, "monotonic", side_effect=[0, 89]):
            self.assertEqual(archive.execution(client, "execution"), completed())
        client.describe_execution.assert_called_once_with(executionArn="execution")

    def test_should_reject_terminal_response_at_original_cutoff(self):
        client = Mock()
        client.describe_execution.return_value = completed()
        with patch.object(archive.time, "monotonic", side_effect=[0, 90]):
            with self.assertRaisesRegex(AssertionError, "observation expired"):
                archive.execution(client, "execution")

    def test_should_reject_terminal_response_after_original_cutoff(self):
        client = Mock()
        client.describe_execution.return_value = completed()
        with patch.object(archive.time, "monotonic", side_effect=[0, 91]):
            with self.assertRaisesRegex(AssertionError, "observation expired"):
                archive.execution(client, "execution")

    def test_should_preserve_original_cutoff_across_running_then_terminal_observation(self):
        client = Mock()
        client.describe_execution.side_effect = [dict(status="RUNNING"), completed()]
        with patch.object(archive.time, "monotonic", side_effect=[0, 1, 91]), patch.object(archive.time, "sleep") as sleep:
            with self.assertRaisesRegex(AssertionError, "observation expired"):
                archive.execution(client, "execution")
        self.assertEqual(client.describe_execution.call_count, 2)
        sleep.assert_called_once_with(0.025)
