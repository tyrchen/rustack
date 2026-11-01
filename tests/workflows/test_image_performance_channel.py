"""Pure socket-identity and phase controls for initialization-owned flood channels."""
import unittest
import uuid

from image_performance_channel import FloodChannel, FloodChannels


class Socket:
    def __init__(self, descriptor=7):
        self.descriptor = descriptor

    def fileno(self):
        return self.descriptor


class ChannelControls(unittest.TestCase):
    def ready(self, channels, connection):
        keep, written = channels.accept(connection, "/flood-ready", {"functionName":"Flood"})
        self.assertTrue(keep)
        self.assertEqual(channels.prepared, 0)
        written()
        self.assertEqual(channels.prepared, 1)
        return channels.submit(1, channels.current.ready_ns + 1)

    def test_should_require_written_ready_then_same_socket_begin_and_end_then_physical_close(self):
        channels, connection, request = FloodChannels(), Socket(), str(uuid.uuid4())
        channel = self.ready(channels, connection)
        body = {"functionName":"Flood", "requestId":request}
        keep, written = channels.accept(connection, "/flood-begin", body)
        self.assertTrue(keep)
        with self.assertRaises(ValueError): channels.accept(connection, "/flood-end", body)
        written()
        keep, written = channels.accept(connection, "/flood-end", body)
        self.assertFalse(keep)
        with self.assertRaises(ValueError): channels.finish(channel, request)
        written()
        with self.assertRaises(ValueError): channels.finish(channel, request)
        connection.descriptor = -1
        channels.finish(channel, request)
        self.assertEqual((channels.prepared, channels.ready_before_invoke, channels.same_connection, channels.closed), (1,1,1,1))
        with self.assertRaises(ValueError): channels.finish(channel, request)

    def test_should_reject_reconnected_objects_even_when_the_descriptor_is_reused(self):
        channels, connection = FloodChannels(), Socket()
        self.ready(channels, connection)
        event = {"functionName":"Flood", "requestId":str(uuid.uuid4())}
        with self.assertRaisesRegex(ValueError, "replaced"):
            channels.accept(Socket(connection.fileno()), "/flood-begin", event)
        with self.assertRaises(ValueError):
            channels.accept(Socket(), "/flood-ready", {"functionName":"Flood"})
        connection.descriptor = -1
        with self.assertRaisesRegex(ValueError, "replaced"):
            channels.accept(connection, "/flood-begin", event)

    def test_should_reject_unwritten_ready_replayed_submission_and_wrong_identity(self):
        channels, connection = FloodChannels(), Socket()
        _, written = channels.accept(connection, "/flood-ready", {"functionName":"Flood"})
        with self.assertRaises(ValueError): channels.submit(1, 100)
        written()
        when = channels.current.ready_ns + 1
        channel = channels.submit(1, when)
        with self.assertRaises(ValueError): channels.submit(1, when + 1)
        with self.assertRaises(ValueError):
            channels.accept(connection, "/flood-begin", {"functionName":"Other", "requestId":str(uuid.uuid4())})
        request = str(uuid.uuid4())
        _, written = channels.accept(connection, "/flood-begin", {"functionName":"Flood", "requestId":request})
        written()
        with self.assertRaises(ValueError):
            channels.accept(connection, "/flood-end", {"functionName":"Flood", "requestId":str(uuid.uuid4())})
        self.assertEqual(channel.phase, "begin")

    def test_should_bound_identifiers_phase_replies_and_incomplete_summary(self):
        for name in ("", "x" * 65, "a/b", "雪"):
            with self.assertRaises(ValueError): FloodChannel(Socket(), name, 1)
        channel = FloodChannel(Socket(), "Flood", 1)
        with self.assertRaises(ValueError): channel.written("/flood-ready")
        with self.assertRaises(ValueError):
            channel.accept(channel.connection, "/flood-ready", {"functionName":"Flood", "extra":True})
        channels = FloodChannels()
        with self.assertRaises(ValueError): channels.summary()

    def test_should_count_exactly_one_hundred_distinct_closed_channels_without_replay(self):
        channels = FloodChannels()
        for cycle in range(1, 101):
            connection, request = Socket(), str(uuid.uuid4())
            _, written = channels.accept(connection, "/flood-ready", {"functionName":"Flood"})
            written()
            channel = channels.submit(cycle, channels.current.ready_ns + 1)
            for phase in ("begin", "end"):
                _, written = channels.accept(connection, "/flood-" + phase, {"functionName":"Flood", "requestId":request})
                written()
            connection.descriptor = -1
            channels.finish(channel, request)
        self.assertEqual(channels.summary(), {"prepared":100,"readyBeforeInvoke":100,"sameConnection":100,"closed":100})
        with self.assertRaises(ValueError): channels.accept(Socket(), "/flood-ready", {"functionName":"Flood"})


if __name__ == "__main__":
    unittest.main()
