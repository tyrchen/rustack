"""Pinned local daemon operations, fenced by its immutable engine identity."""
import time

import metadata
from settings import ActionError


class Client:
    def __init__(self, runner, program, environment, deadline):
        self.runner, self.program = runner, program
        context = environment.get("DOCKER_CONTEXT")
        selected = None if context else environment.get("DOCKER_HOST")
        if not selected:
            arguments = [program, "context", "inspect", "--format", "{{json .Endpoints.docker.Host}}"]
            if context:
                if not isinstance(context, str) or not 1 <= len(context) <= 128 or not context.isascii() or any(ord(c) < 32 or ord(c) > 126 for c in context):
                    raise ActionError("daemon-context-name")
                arguments.extend(["--", context])
            reply = runner.run(arguments,
                               min(deadline, time.monotonic() + 5))
            if reply.code != 0:
                raise ActionError("daemon-context-lookup")
            selected = metadata.document(reply.stdout, string_limit=4096)
        self.endpoint = metadata.endpoint(selected)
        runner.pin_endpoint()
        self.engine = self._identity(deadline)

    def _raw(self, parts, deadline, seconds):
        reply = self.runner.run([self.program, "--host", self.endpoint, *parts],
                                min(deadline, time.monotonic() + seconds))
        if reply.code != 0:
            raise ActionError("docker-command-failed")
        if time.monotonic() >= deadline:
            raise ActionError("docker-late-observation")
        return reply.stdout

    def _identity(self, deadline):
        body = self._raw(["info", "--format", "{{json .ID}}"], deadline, 5)
        return metadata.engine_id(metadata.document(body))

    def call(self, parts, deadline, seconds=10):
        if self._identity(deadline) != self.engine:
            raise ActionError("daemon-identity-changed")
        result = self._raw(parts, deadline, seconds)
        if self._identity(deadline) != self.engine:
            raise ActionError("daemon-identity-changed")
        return result

    def image(self, reference, deadline):
        listing = self.call(["image", "ls", "--no-trunc", "--filter", "reference=" + reference,
                             "--format", "{{json .ID}}"], deadline)
        lines = listing.splitlines()
        if len(lines) > 1:
            raise ActionError("ambiguous-local-image")
        if lines:
            metadata.source_id(metadata.document(lines[0]))
        else:
            self.call(["pull", "--quiet", reference], deadline, 300)
        view = metadata.document(self.call(["image", "inspect", "--format", metadata.IMAGE_FORMAT, reference], deadline))
        if (not isinstance(view, dict) or view.get("os") != "linux"
                or view.get("architecture") not in ("arm64", "amd64")):
            raise ActionError("image-platform")
        return metadata.source_id(view.get("id"))

    def inspect(self, cid, source, nonce, deadline):
        view = metadata.document(self.call(["container", "inspect", "--format", metadata.CONTAINER_FORMAT, cid], deadline))
        return metadata.owned(view, cid, source, self.engine, nonce)
