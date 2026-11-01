"""One direct child group, bounded pipes and physically joined observations."""
from dataclasses import dataclass, field
import os
import selectors
import signal
import subprocess
import time

from settings import ActionError


@dataclass(frozen=True)
class Reply:
    code: int
    stdout: bytes = field(repr=False)
    stderr: bytes = field(repr=False)


class Runner:
    def __init__(self, environment, clock=time.monotonic):
        self.clock = clock
        allowed = ("PATH", "HOME", "DOCKER_CONFIG", "DOCKER_HOST", "DOCKER_CONTEXT",
                   "DOCKER_API_VERSION", "XDG_RUNTIME_DIR")
        self.environment = {key: environment[key] for key in allowed if key in environment}
        self.environment.update(LC_ALL="C", PYTHONDONTWRITEBYTECODE="1")
        self.unsettled = False
        self.retiring = False

    def pin_endpoint(self):
        self.environment.pop("DOCKER_HOST", None)
        self.environment.pop("DOCKER_CONTEXT", None)

    @staticmethod
    def _alive(child):
        try:
            os.killpg(child.pid, 0)
            return True
        except ProcessLookupError:
            return False
        except PermissionError:
            # Exited, unreaped Darwin groups can report EPERM. Reap our
            # direct child and retry in the bounded observation loop. A
            # denial remains present/unproven; it never means absence.
            child.poll()
            return True

    @staticmethod
    def _signal(child, value):
        try:
            os.killpg(child.pid, value)
        except ProcessLookupError:
            return
        except PermissionError:
            child.poll()
            try:
                os.killpg(child.pid, value)
            except (ProcessLookupError, PermissionError):
                # Only a later ESRCH observation may discharge the group.
                # Persistent denial expires as unsettled retirement.
                return

    def _retire(self, child, selector):
        self.retiring = True
        try:
            self._retire_group(child, selector)
        finally:
            self.retiring = False

    def _retire_group(self, child, selector):
        cutoff = time.monotonic() + 3
        kill_at = cutoff - 2
        self._signal(child, signal.SIGTERM)
        while time.monotonic() < cutoff:
            child.poll()
            if time.monotonic() >= kill_at:
                self._signal(child, signal.SIGKILL)
            try:
                self._discard(selector, min(0.025, max(0, cutoff - time.monotonic())))
            except OSError:
                # IO failure cannot bypass process retirement. These are our
                # exact failed pipe handles, never unrelated descriptors.
                for key in list(selector.get_map().values()):
                    selector.unregister(key.fileobj)
                    key.fileobj.close()
            if child.returncode is not None and not self._alive(child) and not selector.get_map():
                child.wait(timeout=0)
                return
        self.unsettled = True
        child.poll()
        raise ActionError("child-retirement-unsettled")

    @staticmethod
    def _discard(selector, seconds):
        for key, _ in selector.select(seconds):
            try:
                value = os.read(key.fd, 8192)
            except BlockingIOError:
                continue
            if not value:
                selector.unregister(key.fileobj)
                key.fileobj.close()

    def run(self, arguments, deadline, stdout_limit=32768, working_directory=None):
        if stdout_limit not in (32768, 65536):
            raise ActionError("child-output-policy")
        if self.unsettled or self.clock() >= deadline:
            raise ActionError("child-deadline-before-dispatch")
        if not arguments or len(arguments) > 128 or sum(len(os.fsencode(a)) for a in arguments) > 32768:
            raise ActionError("child-argument-budget")
        child = None
        selector = selectors.DefaultSelector()
        try:
            child = subprocess.Popen(arguments, stdin=subprocess.DEVNULL, stdout=subprocess.PIPE,
                                     stderr=subprocess.PIPE, env=self.environment, start_new_session=True, cwd=working_directory)
            for stream, label in ((child.stdout, "stdout"), (child.stderr, "stderr")):
                os.set_blocking(stream.fileno(), False)
                selector.register(stream, selectors.EVENT_READ, label)
            output = {"stdout": bytearray(), "stderr": bytearray()}
            limits = {"stdout": stdout_limit, "stderr": 32768}
            while selector.get_map() or child.poll() is None:
                remaining = deadline - self.clock()
                if remaining <= 0:
                    raise ActionError("child-deadline")
                for key, _ in selector.select(min(remaining, 0.05)):
                    try:
                        block = os.read(key.fd, min(8192, limits[key.data] - len(output[key.data]) + 1))
                    except BlockingIOError:
                        continue
                    if not block:
                        selector.unregister(key.fileobj)
                        key.fileobj.close()
                    else:
                        output[key.data].extend(block)
                        if len(output[key.data]) > limits[key.data]:
                            raise ActionError("child-output-budget")
            child.wait(timeout=0)
            if self._alive(child):
                raise ActionError("child-group-outlived-command")
            if self.clock() >= deadline:
                raise ActionError("child-late-result")
            return Reply(child.returncode, bytes(output["stdout"]), bytes(output["stderr"]))
        except BaseException:
            if child is not None:
                self._retire(child, selector)
            raise
        finally:
            for key in list(selector.get_map().values()):
                key.fileobj.close()
            selector.close()
            if child is not None:
                for stream in (child.stdout, child.stderr):
                    if stream is not None:
                        stream.close()
