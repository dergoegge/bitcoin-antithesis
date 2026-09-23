"""Wait for node1 to answer an RPC and for the adversary to answer a request,
then tell Antithesis that setup is complete."""

import time

from antithesis.lifecycle import setup_complete

import client
from test_framework.authproxy import AuthServiceProxy

POLL_INTERVAL = 1.0


def wait_for_node(rpc_url):
    while True:
        try:
            # A fresh proxy per attempt: a refused connection leaves the old one
            # in a state that is not worth reasoning about.
            info = AuthServiceProxy(rpc_url).getblockchaininfo()
            print(f"node1: ready (chain: {info['chain']}, blocks: {info['blocks']})")
            return info
        except Exception as e:  # connection refused, RPC in warmup, ...
            print(f"node1: not ready ({e})")
            time.sleep(POLL_INTERVAL)


def wait_for_adversary():
    while True:
        try:
            client.connect("adversary")
            print("adversary: ready")
            return
        except client.UNAVAILABLE as e:
            print(f"adversary: not ready ({e})")
            time.sleep(POLL_INTERVAL)


def main():
    print("Health checker: waiting for node1 and the adversary...")
    info = wait_for_node(client.NODE_RPC_URL)
    wait_for_adversary()

    setup_complete(
        {
            "message": "node1 answers RPCs and the adversary is listening",
            "chain": info["chain"],
            "chain_height": info["blocks"],
        }
    )
    print("Health checker: setup_complete signaled, exiting")


if __name__ == "__main__":
    main()
