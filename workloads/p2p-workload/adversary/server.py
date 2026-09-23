"""The adversary: a server that owns P2P connections to node1.

The drivers don't speak P2P themselves. They call `Adversary`'s methods
through a multiprocessing manager (see `client.py`) and get the outcome back as
plain data to assert on. Connections outlive the drivers that open them.

All randomness is the caller's business: the server does exactly what a
request says, so that a driver drawing from ``antithesis.random`` decides what
node1 gets to see.

The P2P side is Bitcoin Core's functional test framework, so every connection
is a ``python-p2p-tester`` peer: it answers node1's pings, requests announced
inventory and otherwise stays quiet.
"""

import logging
import socket
import sys
import threading
import time

from test_framework.messages import NODE_P2P_V2, msg_addr, msg_addrv2, msg_ping
from test_framework.p2p import P2P_SERVICES, NetworkThread, p2p_lock
from client import AUTHKEY, PORT, AdversaryManager
from peer import Peer
from proxy import Proxy

logger = logging.getLogger("adversary")

NODE_HOST = "node1"
NODE_PORT = 18444
PROXY_PORT = 9050
# When a new connection doesn't fit, the oldest one is closed to make room.
MAX_CONNECTIONS = 16

CONNECT_TIMEOUT = 10

# How often a blocked request re-checks the state the event loop updates.
POLL_INTERVAL = 0.05


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


class Adversary:
    """The connection registry and the methods the drivers call on it."""

    def __init__(self):
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
        self.proxy = Proxy(self._register_peer, PROXY_PORT)

    def _register_peer(self, peer):
        with self.lock:
            self._prune_settled()
            self._make_room()
            peer.conn_id = self.next_id
            self.next_id += 1
            self.peers[peer.conn_id] = peer

    @staticmethod
    def _loop_exception(loop, context):
        logger.warning("network thread: %s: %r", context.get("message"), context.get("exception"))

    def _peer(self, conn_id):
        """Raises `KeyError` for connections that were closed and forgotten."""
        with self.lock:
            return self.peers[conn_id]

    def list_connections(self):
        with self.lock:
            peers = list(self.peers.values())
        with p2p_lock:
            return [p.describe() for p in peers]

    def proxy_connections(self):
        return self.proxy.connections()

    def send_addresses(self, conn_id, encoding, addresses):
        """Announce `CAddress`es in an ``addr`` or ``addrv2`` message; returns
        whether it was sent."""
        peer = self._peer(conn_id)
        message = msg_addr() if encoding == "addr" else msg_addrv2()
        message.addrs = addresses
        sent = False
        with p2p_lock:
            # Core announces sendaddrv2 during the version handshake.
            negotiated = encoding == "addr" or peer.message_count["sendaddrv2"] > 0
            if peer.is_connected and peer.handshake_complete and negotiated:
                try:
                    self.proxy.announce(peer, message, encoding)
                    sent = True
                except IOError:
                    pass
        return sent

    def new_connection(self, transport="v1", send_version=True, services=P2P_SERVICES,
                       support_addrv2=False, wtxidrelay=True, handshake_timeout=30):
        """Open a connection to node1 and wait for the version handshake (or a
        disconnect) for `handshake_timeout` seconds."""
        supports_v2 = transport == "v2"
        if supports_v2:
            # What the test framework advertises when it speaks v2 itself.
            services |= NODE_P2P_V2

        with self.lock:
            self._prune_settled()
            self._make_room()
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
            node_ip = socket.gethostbyname(NODE_HOST)
        except OSError as e:
            return finish(f"resolving {NODE_HOST} failed: {e}")

        # Same call as `TestNode.add_p2p_connection`: `peer_connect` prepares the
        # connection (v2 handshake state, the version message to send once
        # connected) and returns a thunk that schedules the connect on the
        # network thread. The connect runs as a fire-and-forget task, so a
        # refused connection only shows up as `is_connected` staying false; the
        # underlying error is logged by asyncio when the task is collected.
        peer.peer_connect(
            dstaddr=node_ip,
            dstport=NODE_PORT,
            net="regtest",
            timeout_factor=1.0,
            supports_v2_p2p=supports_v2,
            services=services,
            send_version=send_version,
        )()
        if not wait_for(lambda: peer.is_connected, CONNECT_TIMEOUT):
            return finish(f"not connected after {CONNECT_TIMEOUT}s")

        # Handshake done, or the connection gone, or out of patience: all three
        # are results, and the driver knows which one it asked for.
        wait_for(lambda: peer.handshake_complete or not peer.is_connected, handshake_timeout)
        return finish()

    def ping(self, conn_id, nonce, timeout):
        """Ping and wait for the matching pong, a disconnect, or `timeout` seconds."""
        peer = self._peer(conn_id)

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
            result = {**peer.describe(), "pong_received": nonce in peer.pongs}
        logger.info(
            "connection %d: ping nonce=%d sent=%s pong_received=%s",
            peer.conn_id,
            nonce,
            sent,
            result["pong_received"],
        )
        return result

    # --- registry housekeeping (call with `self.lock` held) ---------------

    def _prune_settled(self):
        """Forget connections that are over, so that the registry stays bounded."""
        for conn_id in [conn_id for conn_id, peer in self.peers.items() if peer.settled]:
            logger.debug("connection %d: pruned", conn_id)
            del self.peers[conn_id]

    def _make_room(self):
        """Close the oldest connections until one more fits."""
        while len(self.peers) >= MAX_CONNECTIONS:
            oldest = min(self.peers.values(), key=lambda peer: peer.created_at)
            oldest.peer_disconnect()
            del self.peers[oldest.conn_id]
            logger.info("connection %d: closed to make room", oldest.conn_id)


def main():
    logging.basicConfig(
        level=logging.DEBUG,
        format="%(asctime)s %(levelname)s %(name)s: %(message)s",
        stream=sys.stdout,
    )
    adversary = Adversary()
    AdversaryManager.register("adversary", callable=lambda: adversary)
    server = AdversaryManager(address=("0.0.0.0", PORT), authkey=AUTHKEY).get_server()
    logger.info("listening on 0.0.0.0:%d", PORT)
    server.serve_forever()


if __name__ == "__main__":
    main()
