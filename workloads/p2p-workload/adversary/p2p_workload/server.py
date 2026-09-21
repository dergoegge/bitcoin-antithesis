"""The adversary: a server that owns P2P connections to node1.

The drivers don't speak P2P themselves. They send this server a request over a
newline-delimited JSON protocol, one object per line, and get the outcome back
as structured data to assert on:

    -> {"method": "new_connection", "params": {"transport": "v1", ...}}
    <- {"success": true, "result": {"id": 3, "handshake_complete": true, ...}}
    <- {"success": false, "error": "unknown connection id 42"}

Methods:

- ``status``: counts of connections, for the health checker.
- ``new_connection``: open a connection to node1 and wait for the version
  handshake (or a disconnect) for ``handshake_timeout`` seconds.
- ``list_connections``: every connection the adversary still knows about.
- ``ping``: send a ping with the given nonce on a connection and wait for the
  matching pong, a disconnect, or ``timeout`` seconds.
- ``disconnect``: close a connection and forget about it.

All randomness is the caller's business: the server does exactly what a
request says, so that a driver drawing from ``antithesis.random`` decides what
node1 gets to see.

The P2P side is Bitcoin Core's functional test framework, so every connection
is a ``python-p2p-tester`` peer: it answers node1's pings, requests announced
inventory and otherwise stays quiet.

Configuration (environment):

- ``NODE1_P2P_ADDR``: ``host:port`` of node1's P2P listener (``node1:18444``).
- ``ADVERSARY_PORT``: port this server listens on (``9000``).
- ``ADVERSARY_MAX_CONNECTIONS``: connections kept at once (``16``). When a new
  one doesn't fit, the oldest one is closed to make room.
- ``ADVERSARY_LOG_LEVEL``: ``DEBUG`` logs every P2P message sent or received.
"""

import json
import logging
import os
import socket
import socketserver
import sys
import threading
import time

from test_framework.messages import NODE_P2P_V2, msg_ping
from test_framework.p2p import P2P_SERVICES, NetworkThread, P2PInterface, p2p_lock

logger = logging.getLogger("adversary")

DEFAULT_NODE_P2P_ADDR = "node1:18444"
DEFAULT_PORT = 9000
DEFAULT_MAX_CONNECTIONS = 16

# How often a blocked request re-checks the state the event loop updates.
POLL_INTERVAL = 0.05


class RequestError(Exception):
    """A request that can't be carried out as asked (bad params, unknown id)."""


def wait_for(predicate, timeout):
    """Poll ``predicate`` under ``p2p_lock`` until it holds or ``timeout`` passes.

    Unlike ``P2PInterface.wait_until`` this neither raises nor logs on timeout:
    a handshake or pong not showing up is a result to report, not an error.
    """
    deadline = time.monotonic() + timeout
    while True:
        with p2p_lock:
            if predicate():
                return True
        if time.monotonic() >= deadline:
            return False
        time.sleep(POLL_INTERVAL)


class Peer(P2PInterface):
    """One connection to node1, remembering what the handshake and pings did.

    The ``on_*`` callbacks run on the network thread; everything they record is
    read by request threads under ``p2p_lock`` (``connected_at``/``closed_at``
    are single assignments and read without it, like ``is_connected``).
    """

    def __init__(self, conn_id, *, transport, send_version, **kwargs):
        super().__init__(**kwargs)
        self.conn_id = conn_id
        self.transport = transport
        self.sent_version = send_version
        self.created_at = time.time()
        self.connected_at = None
        self.closed_at = None
        # Set by the request that opened the connection if the connect failed.
        self.connect_error = None
        # When node1's verack arrived, i.e. the handshake completed.
        self.verack_at = None
        # node1's side of the handshake.
        self.node_version = None
        # Pongs by nonce, so that concurrent pings on one connection don't
        # confuse each other.
        self.pongs = {}

    def on_open(self):
        self.connected_at = time.time()

    def on_close(self):
        self.closed_at = time.time()

    def on_version(self, message):
        self.node_version = {
            "version": message.nVersion,
            "subver": message.strSubVer,
            "services": message.nServices,
            "starting_height": message.nStartingHeight,
            "relay": message.relay,
        }
        super().on_version(message)

    def on_verack(self, message):
        self.verack_at = time.time()

    def on_pong(self, message):
        self.pongs[message.nonce] = time.time()

    @property
    def handshake_complete(self):
        return self.verack_at is not None

    @property
    def settled(self):
        """Whether the connection is over: it closed, or it never opened."""
        return not self.is_connected and (
            self.closed_at is not None or self.connect_error is not None
        )

    def describe(self):
        """A JSON-friendly snapshot; call with ``p2p_lock`` held."""
        return {
            "id": self.conn_id,
            "transport": self.transport,
            "sent_version": self.sent_version,
            "connected": self.is_connected,
            "handshake_complete": self.handshake_complete,
            "created_at": self.created_at,
            "connected_at": self.connected_at,
            "verack_at": self.verack_at,
            "closed_at": self.closed_at,
            "connect_error": self.connect_error,
            "node_version": self.node_version,
            "message_count": dict(self.message_count),
        }


class Adversary:
    """The connection registry and the request handlers that operate on it."""

    def __init__(self, node_host, node_port, max_connections):
        self.node_host = node_host
        self.node_port = node_port
        self.max_connections = max_connections
        self.started_at = time.time()
        # Guards `peers` and `next_id`. Never held while waiting on the network.
        self.lock = threading.Lock()
        self.peers = {}
        self.next_id = 1
        self.network_thread = NetworkThread()
        self.network_thread.start()
        # The thread creates the event loop once it runs; requests need it.
        while NetworkThread.network_event_loop is None or not NetworkThread.network_event_loop.is_running():
            time.sleep(POLL_INTERVAL)
        # A connect that fails does so inside a fire-and-forget task, which
        # asyncio would otherwise report with a full traceback once the task is
        # collected. One line says it all; the request already reported the
        # failure to its driver.
        NetworkThread.network_event_loop.call_soon_threadsafe(
            NetworkThread.network_event_loop.set_exception_handler, self._loop_exception
        )

    @staticmethod
    def _loop_exception(loop, context):
        logger.warning("network thread: %s: %r", context.get("message"), context.get("exception"))

    # --- request dispatch --------------------------------------------------

    def dispatch(self, method, params):
        handler = {
            "status": self.status,
            "new_connection": self.new_connection,
            "list_connections": self.list_connections,
            "ping": self.ping,
            "disconnect": self.disconnect,
        }.get(method)
        if handler is None:
            raise RequestError(f"unknown method {method!r}")
        if not isinstance(params, dict):
            raise RequestError("params must be an object")
        return handler(params)

    def _peer(self, params):
        try:
            conn_id = int(params["id"])
        except (KeyError, TypeError, ValueError):
            raise RequestError("missing or invalid connection id") from None
        with self.lock:
            peer = self.peers.get(conn_id)
        if peer is None:
            raise RequestError(f"unknown connection id {conn_id}")
        return peer

    # --- methods -----------------------------------------------------------

    def status(self, params):
        with self.lock:
            peers = list(self.peers.values())
        with p2p_lock:
            connected = [p for p in peers if p.is_connected]
            handshaked = [p for p in connected if p.handshake_complete]
        return {
            "node": f"{self.node_host}:{self.node_port}",
            "max_connections": self.max_connections,
            "connections": len(peers),
            "connected": len(connected),
            "handshaked": len(handshaked),
            "uptime": round(time.time() - self.started_at, 3),
        }

    def list_connections(self, params):
        with self.lock:
            peers = list(self.peers.values())
        with p2p_lock:
            return {"connections": [p.describe() for p in peers]}

    def new_connection(self, params):
        transport = params.get("transport", "v1")
        if transport not in ("v1", "v2"):
            raise RequestError(f"transport must be 'v1' or 'v2', not {transport!r}")
        send_version = bool(params.get("send_version", True))
        services = int(params.get("services", P2P_SERVICES))
        support_addrv2 = bool(params.get("support_addrv2", False))
        wtxidrelay = bool(params.get("wtxidrelay", True))
        connect_timeout = float(params.get("connect_timeout", 10))
        handshake_timeout = float(params.get("handshake_timeout", 30))
        supports_v2 = transport == "v2"
        if supports_v2:
            # What the test framework advertises when it speaks v2 itself.
            services |= NODE_P2P_V2
        requested = {
            "transport": transport,
            "send_version": send_version,
            "services": services,
            "support_addrv2": support_addrv2,
            "wtxidrelay": wtxidrelay,
            "connect_timeout": connect_timeout,
            "handshake_timeout": handshake_timeout,
        }
        started = time.monotonic()

        with self.lock:
            self._prune_settled()
            evicted = self._make_room()
            conn_id = self.next_id
            self.next_id += 1
            peer = Peer(
                conn_id,
                transport=transport,
                send_version=send_version,
                support_addrv2=support_addrv2,
                wtxidrelay=wtxidrelay,
            )
            self.peers[conn_id] = peer

        def finish(error=None):
            if error is not None:
                peer.connect_error = error
            with p2p_lock:
                result = peer.describe()
            result.update(
                {
                    "requested": requested,
                    "evicted": evicted,
                    "elapsed": round(time.monotonic() - started, 3),
                }
            )
            logger.info(
                "connection %d: %s transport=%s sent_version=%s connected=%s handshake_complete=%s error=%s",
                conn_id,
                "opened" if result["connected"] else "failed",
                transport,
                send_version,
                result["connected"],
                result["handshake_complete"],
                error,
            )
            return result

        # The version message carries node1's address as a raw IPv4, so the
        # hostname has to be resolved here even though the event loop would
        # happily connect to it by name.
        try:
            node_ip = socket.gethostbyname(self.node_host)
        except OSError as e:
            return finish(f"resolving {self.node_host} failed: {e}")

        # Same call as `TestNode.add_p2p_connection`: `peer_connect` prepares the
        # connection (v2 handshake state, the version message to send once
        # connected) and returns a thunk that schedules the connect on the
        # network thread. The connect runs as a fire-and-forget task, so a
        # refused connection only shows up as `is_connected` staying false; the
        # underlying error is logged by asyncio when the task is collected.
        peer.peer_connect(
            dstaddr=node_ip,
            dstport=self.node_port,
            net="regtest",
            timeout_factor=1.0,
            supports_v2_p2p=supports_v2,
            services=services,
            send_version=send_version,
        )()
        if not wait_for(lambda: peer.is_connected, connect_timeout):
            return finish(f"not connected after {connect_timeout}s")

        # Handshake done, or the connection gone, or out of patience: all three
        # are results, and the driver knows which one it asked for.
        wait_for(lambda: peer.handshake_complete or not peer.is_connected, handshake_timeout)
        return finish()

    def ping(self, params):
        peer = self._peer(params)
        try:
            nonce = int(params["nonce"])
        except (KeyError, TypeError, ValueError):
            raise RequestError("missing or invalid nonce") from None
        if not 0 <= nonce < 2**64:
            raise RequestError("nonce must fit in 64 bits")
        timeout = float(params.get("timeout", 60))
        started = time.monotonic()

        sent = False
        if peer.is_connected:
            try:
                peer.send_without_ping(msg_ping(nonce=nonce))
                sent = True
            except IOError:
                # Lost the race with a disconnect.
                pass
        if sent:
            wait_for(lambda: nonce in peer.pongs or not peer.is_connected, timeout)

        with p2p_lock:
            result = peer.describe()
            pong_at = peer.pongs.get(nonce)
        result.update(
            {
                "nonce": nonce,
                "sent": sent,
                "pong_received": pong_at is not None,
                "pong_at": pong_at,
                # Whether the connection is gone now; a pong may still have made
                # it through first.
                "disconnected": not peer.is_connected,
                "timed_out": sent and pong_at is None and peer.is_connected,
                "elapsed": round(time.monotonic() - started, 3),
            }
        )
        logger.info(
            "connection %d: ping nonce=%d sent=%s pong_received=%s disconnected=%s",
            peer.conn_id,
            nonce,
            sent,
            result["pong_received"],
            result["disconnected"],
        )
        return result

    def disconnect(self, params):
        peer = self._peer(params)
        peer.peer_disconnect()
        with self.lock:
            self.peers.pop(peer.conn_id, None)
        with p2p_lock:
            return peer.describe()

    # --- registry housekeeping (call with `self.lock` held) ---------------

    def _prune_settled(self):
        """Forget connections that are over, so that the registry stays bounded."""
        for conn_id in [conn_id for conn_id, peer in self.peers.items() if peer.settled]:
            logger.debug("connection %d: pruned", conn_id)
            del self.peers[conn_id]

    def _make_room(self):
        """Close the oldest connections until one more fits; returns their ids."""
        evicted = []
        while len(self.peers) >= self.max_connections:
            oldest = min(self.peers.values(), key=lambda peer: peer.created_at)
            oldest.peer_disconnect()
            del self.peers[oldest.conn_id]
            evicted.append(oldest.conn_id)
            logger.info("connection %d: closed to make room", oldest.conn_id)
        return evicted


class RequestHandler(socketserver.StreamRequestHandler):
    """One line in, one line out, for as long as the client keeps the socket."""

    def handle(self):
        for line in self.rfile:
            line = line.strip()
            if not line:
                continue
            try:
                request = json.loads(line)
                if not isinstance(request, dict) or "method" not in request:
                    raise RequestError("request must be an object with a 'method'")
                result = self.server.adversary.dispatch(
                    request["method"], request.get("params") or {}
                )
                response = {"success": True, "result": result}
            except RequestError as e:
                response = {"success": False, "error": str(e)}
            except json.JSONDecodeError as e:
                response = {"success": False, "error": f"invalid JSON: {e}"}
            except Exception as e:
                logger.exception("request failed: %s", line[:200])
                response = {"success": False, "error": f"{type(e).__name__}: {e}"}
            try:
                self.wfile.write((json.dumps(response) + "\n").encode("utf-8"))
                self.wfile.flush()
            except OSError as e:
                logger.warning("failed to send response: %s", e)
                return


class AdversaryServer(socketserver.ThreadingTCPServer):
    allow_reuse_address = True
    daemon_threads = True

    def __init__(self, address, adversary):
        super().__init__(address, RequestHandler)
        self.adversary = adversary


def parse_host_port(value, default_port):
    host, sep, port = value.rpartition(":")
    if not sep:
        return value, default_port
    return host, int(port)


def main():
    logging.basicConfig(
        level=os.environ.get("ADVERSARY_LOG_LEVEL", "DEBUG").upper(),
        format="%(asctime)s %(levelname)s %(name)s: %(message)s",
        stream=sys.stdout,
    )
    node_host, node_port = parse_host_port(
        os.environ.get("NODE1_P2P_ADDR", DEFAULT_NODE_P2P_ADDR), 18444
    )
    port = int(os.environ.get("ADVERSARY_PORT", DEFAULT_PORT))
    max_connections = int(os.environ.get("ADVERSARY_MAX_CONNECTIONS", DEFAULT_MAX_CONNECTIONS))

    adversary = Adversary(node_host, node_port, max_connections)
    server = AdversaryServer(("0.0.0.0", port), adversary)
    logger.info(
        "listening on 0.0.0.0:%d, node1 at %s:%d, keeping up to %d connections",
        port,
        node_host,
        node_port,
        max_connections,
    )
    server.serve_forever()


if __name__ == "__main__":
    main()
