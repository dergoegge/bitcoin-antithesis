"""P2P connection state shared by the adversary server and SOCKS5 proxy."""

import time

from test_framework.p2p import NetworkThread, P2PInterface


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
        self.disconnect_requested = False

    def on_open(self):
        if self.disconnect_requested:
            self._transport.abort()
            return
        self.connected_at = time.time()

    def peer_disconnect(self):
        # Remember eviction even if the asynchronous connect/listen has not
        # completed yet. Aborting only the current transport would miss it.
        self.disconnect_requested = True

        NetworkThread.network_event_loop.call_soon_threadsafe(self._close)

    def _close(self):
        if self._transport is not None:
            self._transport.abort()
        self.closed_at = time.time()

    def on_close(self):
        self.closed_at = time.time()

    def on_version(self, message):
        self.transport = "v2" if self.supports_v2_p2p else "v1"
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

