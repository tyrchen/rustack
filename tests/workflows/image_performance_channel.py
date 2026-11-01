"""Bounded initialization-owned sockets for the flood fixture control protocol."""
import time

from image_performance_support import canonical, integer, require


class FloodChannel:
    def __init__(self, connection, function, serial):
        require(isinstance(function, str) and 1 <= len(function) <= 64 and function.isascii()
                and all(value.isalnum() or value in "_-" for value in function), "flood-channel-function")
        require(integer(serial, 1, 100) and connection.fileno() >= 0, "flood-channel-origin")
        self.connection, self.function, self.serial = connection, function, serial
        self.phase, self.request, self.ready_ns, self.submitted_ns = "new", None, None, None
        self.finished = False

    def accept(self, connection, path, event):
        require(connection is self.connection and connection.fileno() >= 0, "flood-channel-replaced")
        require(isinstance(event, dict) and event.get("functionName") == self.function, "flood-channel-identity")
        if path == "/flood-ready":
            require(self.phase == "new" and set(event) == {"functionName"}, "flood-channel-ready-order")
            self.phase = "readyReply"
            return True
        request = canonical(event.get("requestId"))
        if path == "/flood-begin":
            require(self.phase == "ready" and self.submitted_ns is not None, "flood-channel-begin-order")
            self.request, self.phase = request, "beginReply"
            return True
        require(path == "/flood-end" and self.phase == "begin" and request == self.request,
                "flood-channel-end-order")
        self.phase = "endReply"
        return False

    def written(self, path, clock=time.monotonic_ns):
        expected = {"/flood-ready": ("readyReply", "ready"),
                    "/flood-begin": ("beginReply", "begin"), "/flood-end": ("endReply", "complete")}
        require(path in expected and self.phase == expected[path][0], "flood-channel-write-order")
        self.phase = expected[path][1]
        if path == "/flood-ready":
            self.ready_ns = clock()

    def submit(self, started):
        require(self.phase == "ready" and self.submitted_ns is None and self.connection.fileno() >= 0
                and type(started) is int and type(self.ready_ns) is int and 0 < self.ready_ns < started,
                "flood-channel-not-prepared")
        self.submitted_ns = started

    def finish(self, request):
        require(self.phase == "complete" and not self.finished and self.connection.fileno() < 0
                and self.request == canonical(request), "flood-channel-not-closed")
        self.finished = True


class FloodChannels:
    """One current strong socket owner; its collector serializes every mutation."""
    def __init__(self):
        self.current = None
        self.prepared = self.ready_before_invoke = self.same_connection = self.closed = 0

    def accept(self, connection, path, event):
        if path == "/flood-ready":
            require(self.prepared < 100 and (self.current is None
                    or (self.current.finished and self.current.connection.fileno() < 0
                        and self.current.connection is not connection)), "flood-channel-premature-replacement")
            function = event.get("functionName") if isinstance(event, dict) else None
            require(self.current is None or function == self.current.function, "flood-channel-function-changed")
            self.current = FloodChannel(connection, function, self.prepared + 1)
        channel = self.current
        require(channel is not None, "flood-channel-missing")
        keep_alive = channel.accept(connection, path, event)

        def written():
            channel.written(path)
            if path == "/flood-ready":
                self.prepared += 1

        return keep_alive, written

    def ready(self, cycle):
        channel = self.current
        return (channel is not None and channel.serial == self.prepared == cycle
                and channel.phase == "ready" and channel.ready_ns is not None
                and channel.connection.fileno() >= 0)

    def submit(self, cycle, started):
        require(self.current is not None and self.current.serial == self.prepared == cycle,
                "flood-channel-cycle")
        self.current.submit(started)
        self.ready_before_invoke += 1
        return self.current

    def finish(self, channel, request):
        require(channel is self.current, "flood-channel-cycle-changed")
        channel.finish(request)
        self.same_connection += 1
        self.closed += 1

    def summary(self):
        values = {"prepared": self.prepared, "readyBeforeInvoke": self.ready_before_invoke,
                  "sameConnection": self.same_connection, "closed": self.closed}
        require(all(value == 100 for value in values.values()) and self.current is not None
                and self.current.finished, "flood-channel-count")
        return values
