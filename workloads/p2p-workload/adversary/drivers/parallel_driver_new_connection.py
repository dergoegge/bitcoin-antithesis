#!/usr/bin/env python3
"""Open one more connection to node1 and see how far the version handshake gets.

Antithesis picks the shape of the connection: v1 or v2 (BIP324) transport, the
service flags we advertise, whether we ask for addrv2 and wtxid relay, and
whether we send a version message at all. With faults running, the handshake
only needs to complete sometimes; a peer that stays silent must never get a
verack out of node1.
"""

import json

from antithesis.assertions import always, sometimes
from antithesis.random import random_choice

from client import AdversaryClient, AdversaryError
from test_framework.messages import (
    NODE_NETWORK,
    NODE_NETWORK_LIMITED,
    NODE_NONE,
    NODE_WITNESS,
)

# How long the adversary waits for node1's verack before reporting back.
HANDSHAKE_TIMEOUT = 30.0
# A peer that never sends its version has nothing to wait for; this only
# leaves node1 a moment to (wrongly) speak first.
SILENT_TIMEOUT = 5.0

TRANSPORTS = ["v1", "v2"]
# `random_choice` is uniform, so duplicates are weights: mostly do the
# handshake properly, sometimes connect and say nothing.
SEND_VERSION = [True, True, True, False]
SERVICES = [
    NODE_NETWORK | NODE_WITNESS,
    NODE_NETWORK_LIMITED | NODE_WITNESS,
    NODE_WITNESS,
    NODE_NONE,
]


def main():
    params = {
        "transport": random_choice(TRANSPORTS),
        "send_version": random_choice(SEND_VERSION),
        "services": random_choice(SERVICES),
        "support_addrv2": random_choice([True, False]),
        "wtxidrelay": random_choice([True, False]),
    }
    params["handshake_timeout"] = HANDSHAKE_TIMEOUT if params["send_version"] else SILENT_TIMEOUT

    client = AdversaryClient.from_env()
    try:
        result = client.call(
            "new_connection", params, timeout=params["handshake_timeout"] + 30
        )
    except (OSError, AdversaryError) as e:
        # Without the adversary there is nothing to observe about node1.
        print(f"new_connection: adversary unavailable: {e}")
        return
    print(f"new_connection: {json.dumps(result)}")

    completed = result["handshake_complete"]
    sent_version = params["send_version"]
    details = {"params": params, "result": result}

    sometimes(
        completed,
        "A new P2P connection to node1 completes the version handshake",
        details,
    )
    sometimes(
        completed and params["transport"] == "v2",
        "A new v2 transport (BIP324) connection to node1 completes the version handshake",
        details,
    )
    # Faults reaching node1 show up here: a proper handshake attempt that got nowhere.
    sometimes(
        sent_version and not completed,
        "A new P2P connection to node1 that sent its version does not complete the handshake",
        details,
    )
    # node1 must wait for our version before it sends its own and its verack.
    always(
        sent_version or not completed,
        "node1 does not complete a version handshake with a peer that never sent its version",
        details,
    )


if __name__ == "__main__":
    main()
