"""Regression checks for proxy lifecycle, assertions, and address generation.

Run with the adversary image's PYTHONPATH and dependencies, using unittest.
"""

import asyncio
import unittest
from unittest.mock import Mock, patch

from drivers import parallel_driver_addresses as driver
from proxy import Proxy, ProxyPeer
from server import Adversary
from test_framework.messages import MAGIC_BYTES, msg_version
from test_framework.p2p import NetworkThread, P2PInterface, P2P_VERSION


class ProxyLifecycleTest(unittest.IsolatedAsyncioTestCase):
    async def asyncSetUp(self):
        self.loop = asyncio.get_running_loop()
        self.loop_patch = patch.object(NetworkThread, "network_event_loop", self.loop)
        self.loop_patch.start()
        self.addCleanup(self.loop_patch.stop)
        self.peer = ProxyPeer("11.23.45.67", 8333)
        self.peer.conn_id = 1
        self.proxy = object.__new__(Proxy)
        self.adversary = object.__new__(Adversary)
        self.adversary.peers = {1: self.peer}
        self.adversary.max_connections = 1

    async def test_eviction_while_listener_is_being_created(self):
        started, release = asyncio.Event(), asyncio.Event()
        create_server = self.loop.create_server

        async def delayed_create(*args, **kwargs):
            started.set()
            await release.wait()
            return await create_server(*args, **kwargs)

        with patch.object(self.loop, "create_server", delayed_create):
            task = asyncio.create_task(self.proxy._listen(self.peer))
            await started.wait()
            self.assertEqual(self.adversary._make_room(), [1])
            release.set()
            self.assertIsNone(await task)
        self.assertIsNone(self.peer.listener)
        self.assertFalse(self.peer.is_connected)
        self.assertEqual(self.adversary.peers, {})

    async def test_eviction_closes_ready_listener(self):
        destination = await self.proxy._listen(self.peer)
        self.adversary._make_room()
        await asyncio.sleep(0)
        self.assertIsNone(self.peer.listener)
        self.assertIsNone(self.peer.listen_timeout)
        with self.assertRaises(OSError):
            await asyncio.open_connection(destination["actual_to_addr"], destination["actual_to_port"])

    async def test_listener_timeout_rejects_late_acceptance(self):
        with patch.object(self.loop, "call_later", wraps=self.loop.call_later) as timer:
            await self.proxy._listen(self.peer)
        timer.call_args.args[1]()
        self.assertTrue(self.peer.disconnect_requested)
        await asyncio.sleep(0)
        self.assertIsNone(self.peer.listener)
        # A socket accepted just before the timeout may still have on_open
        # queued. It must be rejected even after registry pruning.
        self.peer._transport = Mock()
        self.peer.on_open()
        self.peer._transport.abort.assert_called_once()
        self.assertIsNone(self.peer.connected_at)


class TransportDetectionTest(unittest.TestCase):
    def test_v1_version_survives_transport_detection_and_fragmentation(self):
        sender = P2PInterface()
        sender.magic_bytes = MAGIC_BYTES["regtest"]
        version = msg_version()
        version.nVersion = P2P_VERSION
        wire = sender.build_message(version)
        for chunk_size in (1, 7, 16, len(wire)):
            with self.subTest(chunk_size=chunk_size):
                peer = ProxyPeer("11.23.45.67", 8333)
                peer.peer_accept_connection(
                    0, net="regtest", timeout_factor=1.0,
                    supports_v2_p2p=True, reconnect=False,
                )
                peer.send_without_ping = Mock()
                for offset in range(0, len(wire), chunk_size):
                    peer.data_received(wire[offset:offset + chunk_size])
                self.assertEqual(peer.transport, "v1")
                self.assertEqual(peer.message_count["version"], 1)
                self.assertIsNotNone(peer.node_version)


class ProxyAssertionTest(unittest.TestCase):
    def test_ambiguous_announcements_only_count_generic_forwarding(self):
        with patch("proxy.sometimes") as check:
            peer = ProxyPeer("11.23.45.67", 8333, ["addr", "addrv2"])
            peer.on_open()
            check.reset_mock()
            peer.on_verack(None)
            self.assertEqual([c.args[0] for c in check.call_args_list], [True, False, False])

    def test_progress_is_reported_without_driver_polling(self):
        for encoding in ("addr", "addrv2"):
            with self.subTest(encoding=encoding), patch("proxy.sometimes") as check:
                peer = ProxyPeer("11.23.45.67", 8333, [encoding])
                peer.assert_progress()
                self.assertEqual([c.args[0] for c in check.call_args_list], [False, False, False])
                check.reset_mock()
                peer.on_open()
                self.assertEqual([c.args[0] for c in check.call_args_list], [True, False, False])
                check.reset_mock()
                peer.on_verack(None)
                self.assertEqual([c.args[0] for c in check.call_args_list],
                                 [True, encoding == "addr", encoding == "addrv2"])


class AddressDriverTest(unittest.TestCase):
    def test_unrestricted_port_bits(self):
        for encoding in ("addr", "addrv2"):
            for port in (0, 1, 12345, 65535):
                choices = ([False] if encoding == "addrv2" else []) + [True, None, 0]
                with self.subTest(encoding=encoding, port=port), \
                     patch.object(driver, "random_choice", side_effect=choices), \
                     patch.object(driver, "get_random", side_effect=[0x0b172d43, port]):
                    self.assertEqual(driver.make_addresses(encoding, 1)[0]["port"], port)

    def test_unrestricted_ipv4_bits(self):
        for bits, expected in ((0, "0.0.0.0"), (0x7f000001, "127.0.0.1"),
                               (0xffffffff, "255.255.255.255")):
            with self.subTest(bits=bits), \
                 patch.object(driver, "random_choice", side_effect=[True, 8333, 0]), \
                 patch.object(driver, "get_random", return_value=bits):
                self.assertEqual(driver.make_addresses("addr", 1)[0]["address"], expected)

    def test_unrestricted_ipv6_bits(self):
        for words, expected in (((0, 0), "::"),
                                ((0xfd00000000000000, 1), "fd00::1"),
                                ((2**64 - 1, 2**64 - 1), "ffff:ffff:ffff:ffff:ffff:ffff:ffff:ffff")):
            with self.subTest(words=words), \
                 patch.object(driver, "random_choice", side_effect=[True, True, 8333, 0]), \
                 patch.object(driver, "get_random", side_effect=words):
                self.assertEqual(driver.make_addresses("addrv2", 1)[0]["address"], expected)

    def test_driver_returns_after_sending(self):
        client = Mock()
        client.call.side_effect = [{"id": 1, "handshake_complete": True}, {"sent": True}]
        with patch.object(driver.AdversaryClient, "from_env", return_value=client), \
             patch.object(driver, "make_addresses", return_value=[]), \
             patch.object(driver, "random_choice", side_effect=["addr", 1, False, "v1"]), \
             patch.object(driver.time, "sleep", side_effect=AssertionError("driver waited")):
            driver.main()
        self.assertEqual([c.args[0] for c in client.call.call_args_list],
                         ["new_connection", "send_addresses"])

    def test_driver_reuses_an_eligible_connection(self):
        closed = {"id": 1, "connected": False, "handshake_complete": True}
        incomplete = {"id": 2, "connected": True, "handshake_complete": False}
        legacy = {"id": 3, "connected": True, "handshake_complete": True, "message_count": {}}
        modern = {"id": 4, "connected": True, "handshake_complete": True,
                  "message_count": {"sendaddrv2": 1}}
        for encoding, chosen, eligible in (("addr", legacy, [legacy, modern]),
                                           ("addrv2", modern, [modern])):
            with self.subTest(encoding=encoding):
                client = Mock()
                client.call.side_effect = [
                    {"connections": [closed, incomplete, legacy, modern]}, {"sent": True},
                ]
                with patch.object(driver.AdversaryClient, "from_env", return_value=client), \
                     patch.object(driver, "make_addresses", return_value=[]), \
                     patch.object(driver, "random_choice", side_effect=[encoding, 1, True, chosen]) as choice:
                    driver.main()
                self.assertEqual(choice.call_args.args[0], eligible)
                self.assertEqual([c.args[0] for c in client.call.call_args_list],
                                 ["list_connections", "send_addresses"])
                self.assertEqual(client.call.call_args.args[1]["id"], chosen["id"])

    def test_driver_opens_connection_when_none_can_be_reused(self):
        for connections in ([], [{"id": 1, "connected": True, "handshake_complete": True,
                                  "message_count": {}}]):
            with self.subTest(connections=connections):
                client = Mock()
                client.call.side_effect = [
                    {"connections": connections}, {"id": 2, "handshake_complete": True}, {"sent": True},
                ]
                with patch.object(driver.AdversaryClient, "from_env", return_value=client), \
                     patch.object(driver, "make_addresses", return_value=[]), \
                     patch.object(driver, "random_choice", side_effect=["addrv2", 1, True, "v2"]):
                    driver.main()
                self.assertEqual([c.args[0] for c in client.call.call_args_list],
                                 ["list_connections", "new_connection", "send_addresses"])


if __name__ == "__main__":
    unittest.main()
