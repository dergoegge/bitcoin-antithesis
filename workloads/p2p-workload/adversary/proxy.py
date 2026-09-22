"""Redirect Core's SOCKS5 requests to local P2P peers and assert on progress."""

import asyncio
from collections import OrderedDict, deque
import logging
import queue
import threading

from antithesis.assertions import sometimes

from peer import Peer
from test_framework.messages import NODE_P2P_V2
from test_framework.p2p import NetworkThread, P2P_SERVICES, p2p_lock
from test_framework.socks5 import Socks5Configuration, Socks5Server

logger = logging.getLogger("adversary.proxy")

# Bound attribution history independently of how many driver batches run.
MAX_ANNOUNCED_ADDRESSES = 65536


class ProxyPeer(Peer):
    def __init__(self, address, port, announced_via=()):
        super().__init__(0, transport="auto", send_version=True, support_addrv2=True)
        self.proxy_destination = {"address": address, "port": port}
        self.announced_via = frozenset(announced_via)
        self.listener = None
        self.listen_timeout = None
        self.transport_prefix = b""

    def data_received(self, data):
        if self.transport == "auto":
            # Preserve the v1 header: the framework's v2 responder fallback
            # consumes it without feeding it back into the v1 message parser.
            self.transport_prefix += data
            v1_prefix = self.magic_bytes + b"version\x00\x00\x00\x00\x00"
            if len(self.transport_prefix) < len(v1_prefix) and v1_prefix.startswith(self.transport_prefix):
                return
            data = self.transport_prefix
            self.transport_prefix = b""
            if data.startswith(v1_prefix):
                self.v2_state = None
                self.transport = "v1"
            else:
                self.transport = "v2"
        super().data_received(data)

    def close_listener(self):
        """Release a one-shot listener on the network thread."""
        if self.listener is not None:
            self.listener.close()
            self.listener = None
        if self.listen_timeout is not None:
            self.listen_timeout.cancel()
            self.listen_timeout = None

    def _close(self):
        self.close_listener()
        super()._close()

    def on_open(self):
        self.close_listener()
        super().on_open()
        self.assert_progress()

    def on_verack(self, message):
        super().on_verack(message)
        self.assert_progress()

    def on_close(self):
        super().on_close()
        self.assert_progress()

    def describe(self):
        return {**super().describe(), "proxy_destination": self.proxy_destination,
                "announced_via": sorted(self.announced_via)}

    def assert_progress(self):
        # Called at request creation (before networking), then by network
        # callbacks. These observations survive the announcing driver's exit.
        details = self.describe()
        sometimes(
            self.connected_at is not None,
            "The SOCKS5 proxy forwards node1's requested connections to the adversary",
            details,
        )
        sometimes(
            # Re-announcing the same target in both encodings makes the
            # encoding that caused the connection ambiguous.
            self.handshake_complete and self.announced_via == {"addr"},
            "node1 completes an outbound P2P handshake through the proxy after receiving addr",
            details,
        )
        sometimes(
            self.handshake_complete and self.announced_via == {"addrv2"},
            "node1 completes an outbound P2P handshake through the proxy after receiving addrv2",
            details,
        )


class Proxy(Socks5Server):
    def __init__(self, register_peer, port=9050):
        self.register_peer = register_peer
        self.lock = threading.Lock()
        self.peers = deque(maxlen=256)
        self.announcements = OrderedDict()
        conf = Socks5Configuration()
        conf.addr = ("0.0.0.0", port)
        conf.unauth = True
        conf.auth = True  # Core's default -proxyrandomize supplies credentials.
        conf.destinations_factory = self._destination
        super().__init__(conf)
        self.start()

    def announce(self, peer, message, encoding):
        # Hold the attribution lock across the send so a fast SOCKS request
        # cannot overtake this record. A failed send records no announcement.
        with self.lock:
            peer.send_without_ping(message)
            for address in message.addrs:
                target = (address.ip, address.port)
                self.announcements.setdefault(target, set()).add(encoding)
                self.announcements.move_to_end(target)
            while len(self.announcements) > MAX_ANNOUNCED_ADDRESSES:
                self.announcements.popitem(last=False)

    async def _listen(self, peer):
        if peer.disconnect_requested:
            return None
        # Prepare the framework's inbound handshake without using its finite
        # TestNode listener pool. One listener per request preserves attribution
        # even when several SOCKS connections arrive concurrently.
        peer.peer_accept_connection(
            0, net="regtest", timeout_factor=1.0, supports_v2_p2p=True,
            reconnect=False, services=P2P_SERVICES | NODE_P2P_V2,
        )
        loop = NetworkThread.network_event_loop
        peer.listener = await loop.create_server(lambda: peer, "127.0.0.1", 0)
        if peer.disconnect_requested:
            peer.close_listener()
            return None
        port = peer.listener.sockets[0].getsockname()[1]

        def expired():
            peer.connect_error = "proxy did not reach the local P2P listener"
            peer.peer_disconnect()

        peer.listen_timeout = loop.call_later(10, expired)
        return {"actual_to_addr": "127.0.0.1", "actual_to_port": port}

    def _drain_queue(self):
        # The framework also logs these commands/errors. Keep its queue from
        # growing indefinitely alongside our bounded connection history.
        while True:
            try:
                self.queue.get_nowait()
            except queue.Empty:
                break

    def _destination(self, address, port, proxy_client=None):
        # Recent framework versions also pass the SOCKS client's endpoint.
        self._drain_queue()
        with self.lock:
            peer = ProxyPeer(address, port, self.announcements.get((address, port), ()))
            self.peers.append(peer)
        self.register_peer(peer)
        peer.assert_progress()
        future = asyncio.run_coroutine_threadsafe(self._listen(peer), NetworkThread.network_event_loop)
        try:
            destination = future.result(timeout=10)
        except (OSError, TimeoutError) as e:
            future.cancel()
            peer.connect_error = str(e)
            peer.peer_disconnect()
            logger.warning("proxy listener failed for %s:%d: %s", address, port, e)
            return None
        if destination is None:
            return None
        logger.info("proxy connection %d: %s:%d -> %s:%d", peer.conn_id,
                    address, port, destination["actual_to_addr"], destination["actual_to_port"])
        return destination

    def connections(self):
        self._drain_queue()
        with self.lock:
            peers = list(self.peers)
        with p2p_lock:
            return {"connections": [peer.describe() for peer in peers]}
