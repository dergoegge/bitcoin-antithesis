#!/usr/bin/env python3
"""Announce address batches and return; the proxy observes outbound progress.

No RPC adds addresses or forces connections: node1 must learn the destinations
from the P2P message and select them from addrman itself. The proxy redirects
every destination to a local P2P peer, including public-looking IP addresses.
"""

import ipaddress
import time

from antithesis.random import get_random, random_choice

import client
from test_framework.messages import CAddress, NODE_NETWORK, NODE_P2P_V2, NODE_WITNESS

HANDSHAKE_TIMEOUT = 30
# Single entries, the small-batch relay boundary, and MAX_ADDR_TO_SEND=1000.
# Fresh inbound peers only have one address-processing token initially, so
# large batches also exercise rate limiting while still seeding addrman.
BATCH_SIZES = [1, 2, 10, 11, 999, 1000]


def make_addresses(encoding, count):
    addresses = []
    for _ in range(count):
        ipv6 = encoding == "addrv2" and random_choice([False, True])
        if random_choice([False, True]):
            # Unrestricted address bits, including private/reserved ranges.
            if ipv6:
                ip = str(ipaddress.IPv6Address((get_random() << 64) | get_random()))
            else:
                ip = str(ipaddress.IPv4Address(get_random() & 0xffffffff))
        elif ipv6:
            # Global IPv6 prefixes in distinct /32 network groups.
            prefix = random_choice(["2001:470", "2606:4700", "2a00:1450", "2404:6800"])
            suffix = get_random()
            ip = f"{prefix}:{suffix & 0xffff:x}::{(suffix >> 16) & 0xffff:x}:{(suffix >> 32) & 0xffff:x}"
        else:
            # Keep routable candidates in the mix to drive outbound traffic.
            first = random_choice([11, 23, 31, 45, 57, 63, 79, 89, 101, 123])
            bits = get_random()
            ip = f"{first}.{bits & 255}.{(bits >> 8) & 255}.{1 + (bits >> 16) % 254}"
        port = random_choice([8333, 18444, 65535, None])
        if port is None:
            port = get_random() & 0xffff
        address = CAddress()
        address.net = CAddress.NET_IPV6 if ipv6 else CAddress.NET_IPV4
        # Canonical form, as node1 will name it in its SOCKS5 request.
        address.ip = str(ipaddress.ip_address(ip))
        address.port = port
        address.nServices = NODE_NETWORK | NODE_WITNESS | random_choice([0, NODE_P2P_V2])
        address.time = int(time.time())
        addresses.append(address)
    return addresses


def main():
    encoding = random_choice(["addr", "addrv2"])
    addresses = make_addresses(encoding, random_choice(BATCH_SIZES))
    try:
        adversary = client.connect()
    except client.UNAVAILABLE:
        return
    peer = None
    if random_choice([True, False]):
        eligible = [c for c in adversary.list_connections() if c["connected"] and c["handshake_complete"]
                    and (encoding == "addr" or c["message_count"].get("sendaddrv2", 0) > 0)]
        if eligible:
            peer = random_choice(eligible)
    if peer is None:
        peer = adversary.new_connection(
            transport=random_choice(client.TRANSPORTS),
            support_addrv2=True,
            handshake_timeout=HANDSHAKE_TIMEOUT,
        )
    if peer["handshake_complete"]:
        try:
            adversary.send_addresses(peer["id"], encoding, addresses)
        except KeyError:
            # Evicted since it was selected.
            pass


if __name__ == "__main__":
    main()
