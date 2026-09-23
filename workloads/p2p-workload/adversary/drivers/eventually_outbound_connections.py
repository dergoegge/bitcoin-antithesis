#!/usr/bin/env python3
"""Once faults stop, node1 fills its automatic outbound slots: 8 full-relay and
2 block-relay-only connections.

node1 picks them from addrman, and only knows the addresses the address driver
announced, so this is only a failure if ``getaddrmaninfo`` shows enough new
addresses to try. Every destination goes through the proxy to the adversary,
which completes the handshake, so none of them fail.
"""

import time

from antithesis.assertions import always, sometimes

import client
from test_framework.authproxy import AuthServiceProxy

FULL_RELAY = 8
BLOCK_RELAY = 2
# node1 makes at most one outbound connection per network group, which
# getaddrmaninfo doesn't report. The address driver's addresses mostly have
# distinct groups, so ask for twice as many addresses as there are slots.
MIN_NEW_ADDRESSES = 2 * (FULL_RELAY + BLOCK_RELAY)
POLL_INTERVAL = 1.0


def query(method):
    """One RPC to node1, or None if it didn't answer."""
    try:
        return getattr(AuthServiceProxy(client.NODE_RPC_URL), method)()
    except Exception as e:  # node1 restarting, RPC in warmup, ...
        return None


def main():
    start = time.monotonic()
    deadline = start + client.EVENTUALLY_BUDGET_SECS

    while (addrman := query("getaddrmaninfo")) is None:
        if time.monotonic() > deadline:
            return
        time.sleep(POLL_INTERVAL)

    new = addrman["all_networks"]["new"]
    sometimes(
        new >= MIN_NEW_ADDRESSES,
        "node1's addrman has enough new addresses to fill its outbound connections",
        {"addrman": addrman},
    )
    if new < MIN_NEW_ADDRESSES:
        return

    counts = None
    while True:
        peers = query("getpeerinfo")
        if peers is not None:
            types = [peer["connection_type"] for peer in peers]
            counts = {
                "full_relay": types.count("outbound-full-relay"),
                "block_relay": types.count("block-relay-only"),
            }
            if counts["full_relay"] >= FULL_RELAY and counts["block_relay"] >= BLOCK_RELAY:
                break
        if time.monotonic() > deadline:
            break
        time.sleep(POLL_INTERVAL)

    always(
        counts is not None
        and counts["full_relay"] >= FULL_RELAY
        and counts["block_relay"] >= BLOCK_RELAY,
        "node1 eventually has 8 full-relay and 2 block-relay-only outbound connections",
        {"outbound": counts, "addrman": addrman, "seconds": round(time.monotonic() - start, 1)},
    )


if __name__ == "__main__":
    main()
