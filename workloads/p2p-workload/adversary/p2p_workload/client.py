"""Client for the adversary's request protocol, used by the drivers and the
health checker. See `p2p_workload.server` for the methods."""

import json
import os
import socket

DEFAULT_HOST = "127.0.0.1"
DEFAULT_PORT = 9000


class AdversaryError(Exception):
    """The adversary answered, but with an error."""


class AdversaryClient:
    def __init__(self, host, port):
        self.host = host
        self.port = port

    @classmethod
    def from_env(cls):
        """``ADVERSARY_HOST``/``ADVERSARY_PORT``; the drivers run inside the
        adversary container, so the default is localhost."""
        return cls(
            os.environ.get("ADVERSARY_HOST", DEFAULT_HOST),
            int(os.environ.get("ADVERSARY_PORT", DEFAULT_PORT)),
        )

    def call(self, method, params=None, *, timeout=60.0):
        """Send one request and return its ``result``.

        Raises `AdversaryError` if the adversary rejected the request, and
        ``OSError`` (including ``TimeoutError``) if it couldn't be reached or
        didn't answer within ``timeout`` seconds. Requests that wait on node1
        (``new_connection``, ``ping``) take their own timeouts as params;
        ``timeout`` here should leave room for those.
        """
        request = json.dumps({"method": method, "params": params or {}}) + "\n"
        with socket.create_connection((self.host, self.port), timeout=timeout) as sock:
            sock.sendall(request.encode("utf-8"))
            with sock.makefile("r", encoding="utf-8") as reader:
                line = reader.readline()
        if not line:
            raise AdversaryError("adversary closed the connection without answering")
        response = json.loads(line)
        if not response.get("success"):
            raise AdversaryError(response.get("error", "unknown error"))
        return response["result"]
