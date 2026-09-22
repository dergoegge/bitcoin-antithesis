#!/usr/bin/env python3
"""Once faults stop, a connection node1 just handshaked must answer a ping
with a pong carrying the same nonce.

Connections that completed their handshake before the faults are pinged too,
but only as a sometimes check. A killed or partitioned node1 leaves the TCP
socket open on this side: the ping sits in the send buffer and the pong times
out, which looks the same as node1 ignoring it. There is no way to tell those
apart, so an old connection that stays silent is not a failure.

The fresh connection is. It is opened now, retried until node1 is back, and
node1 has just completed a handshake on it, so it must answer the ping. It
also covers timelines where the faults took every older connection down.
"""

import json
import os
import time

from antithesis.assertions import always, sometimes
from antithesis.random import get_random, random_choice

from client import AdversaryClient, AdversaryError

# Time for node1's closes to arrive before we start asking questions.
SETTLE_SECS = 5.0
# How long a connection gets to answer a ping.
PONG_TIMEOUT = 60.0
HANDSHAKE_TIMEOUT = 30.0
# Default of `retry_budget`.
DEFAULT_RETRY_BUDGET_SECS = 60 * 60
RETRY_INTERVAL = 1.0

TRANSPORTS = ["v1", "v2"]


def retry_budget():
    """Total time node1 gets to come back and complete a handshake.

    A driver that outlives the test run is stopped before it asserts anything,
    leaving the property unchecked rather than failed, so the budget has to
    stay below the run's duration (``EVENTUALLY_RETRY_BUDGET_SECS``).
    """
    value = os.environ.get("EVENTUALLY_RETRY_BUDGET_SECS")
    if value is None:
        return DEFAULT_RETRY_BUDGET_SECS
    try:
        return float(value)
    except ValueError:
        print(f"ping_pong: ignoring EVENTUALLY_RETRY_BUDGET_SECS={value!r}")
        return DEFAULT_RETRY_BUDGET_SECS


def ping(client, connection):
    """Ping one connection with a fresh nonce and return the adversary's report,
    or None if the adversary couldn't carry out the request at all."""
    nonce = get_random()
    try:
        result = client.call(
            "ping",
            {"id": connection["id"], "nonce": nonce, "timeout": PONG_TIMEOUT},
            timeout=PONG_TIMEOUT + 30,
        )
    except (OSError, AdversaryError) as e:
        print(f"ping_pong: connection {connection['id']}: ping request failed: {e}")
        return None
    print(f"ping_pong: connection {connection['id']}: {json.dumps(result)}")
    return result


def main():
    client = AdversaryClient.from_env()
    budget = retry_budget()
    start = time.monotonic()

    time.sleep(SETTLE_SECS)

    # The adversary itself may have been hit, so give it the same patience.
    while True:
        try:
            connections = client.call("list_connections")["connections"]
            break
        except (OSError, AdversaryError) as e:
            print(f"ping_pong: adversary unavailable: {e}")
            if time.monotonic() - start > budget:
                print("ping_pong: giving up on the adversary")
                return
            time.sleep(RETRY_INTERVAL)

    survivors = [c for c in connections if c["connected"] and c["handshake_complete"]]
    print(
        f"ping_pong: {len(connections)} known connection(s), "
        f"{len(survivors)} handshaked and still open"
    )

    survivor_pongs = 0
    for connection in survivors:
        result = ping(client, connection)
        if result is None:
            continue
        survivor_pongs += result["pong_received"]
    sometimes(
        survivor_pongs > 0,
        "A P2P connection to node1 survives fault injection and still answers pings",
        {"survivors": len(survivors), "pongs": survivor_pongs},
    )

    # Now a fresh connection, for as long as it takes node1 to come back.
    fresh = None
    last = None
    attempts = 0
    while fresh is None:
        attempts += 1
        params = {
            "transport": random_choice(TRANSPORTS),
            "send_version": True,
            "handshake_timeout": HANDSHAKE_TIMEOUT,
        }
        try:
            last = client.call("new_connection", params, timeout=HANDSHAKE_TIMEOUT + 30)
            if last["handshake_complete"]:
                fresh = last
                break
            print(f"ping_pong: attempt {attempts}: {json.dumps(last)}")
        except (OSError, AdversaryError) as e:
            last = {"error": str(e)}
            print(f"ping_pong: attempt {attempts}: adversary unavailable: {e}")
        if time.monotonic() - start > budget:
            break
        time.sleep(RETRY_INTERVAL)

    elapsed = round(time.monotonic() - start, 1)
    always(
        fresh is not None,
        "node1 completes a version handshake with a new peer once faults stop",
        {"attempts": attempts, "seconds": elapsed, "last_result": last},
    )
    if fresh is None:
        return

    result = ping(client, fresh)
    if result is None:
        return
    always(
        result["pong_received"],
        "A freshly handshaked P2P connection to node1 answers a ping with a pong carrying the same nonce",
        {"connection": fresh, "ping": result},
    )


if __name__ == "__main__":
    main()
