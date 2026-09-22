"""Run inside the adversary image against a fresh workload Compose deployment.

    docker cp workloads/p2p-workload/tests/addr_proxy_smoke.py adversary:/tmp/
    docker exec adversary python3 /tmp/addr_proxy_smoke.py

Requires an empty node1 datadir. Only read-only RPCs are used: all addresses
must reach addrman through P2P, and all outbound connections must be automatic.
"""

import time

from client import AdversaryClient
from test_framework.authproxy import AuthServiceProxy
from test_framework.messages import NODE_NETWORK, NODE_P2P_V2, NODE_WITNESS


def rpc():
    return AuthServiceProxy("http://user:password@node1:18443")


def observe(client, addresses, timeout=60):
    """Only the smoke test waits for outbound results; the driver returns."""
    targets = {(a["address"], a["port"]) for a in addresses}
    deadline = time.monotonic() + timeout
    while True:
        matches = [c for c in client.call("proxy_connections")["connections"]
                   if (c["proxy_destination"]["address"], c["proxy_destination"]["port"]) in targets]
        if any(c["handshake_complete"] for c in matches) or time.monotonic() >= deadline:
            return matches
        time.sleep(0.25)


def main():
    client = AdversaryClient.from_env()
    deadline = time.monotonic() + 60
    while True:
        try:
            client.call("status")
            info = rpc().getnetworkinfo()
            break
        except Exception:
            if time.monotonic() >= deadline:
                raise
            time.sleep(0.25)
    print(f"Testing Bitcoin {info['subversion']}", flush=True)
    assert rpc().getnodeaddresses(0) == [], "test requires a fresh datadir"

    # Distinct network groups, so Core can keep all four outbound connections.
    cases = [
        ("addr", "v1", "11.23.45.67", 8333),
        ("addr", "v2", "23.45.67.89", 18444),
        ("addrv2", "v1", "45.67.89.101", 65535),
        ("addrv2", "v2", "2606:4700:1234::abcd", 18444),
    ]
    for encoding, transport, address, port in cases:
        services = NODE_NETWORK | NODE_WITNESS
        if transport == "v2":
            services |= NODE_P2P_V2
        peer = client.call("new_connection", {
            "transport": transport, "send_version": True, "support_addrv2": True,
        })
        assert peer["handshake_complete"], peer
        entries = [{"address": address, "port": port, "services": services}]
        sent = client.call("send_addresses", {
            "id": peer["id"], "encoding": encoding, "addresses": entries,
        })
        assert sent["sent"], sent
        connections = observe(client, entries, timeout=60)
        completed = [c for c in connections if c["handshake_complete"]]
        assert completed, (encoding, transport, connections)
        assert all(encoding in c["announced_via"] for c in completed), completed
        assert all(c["connected_at"] is not None for c in completed), completed
        assert any(a["address"] == address and a["port"] == port
                   for a in rpc().getnodeaddresses(0)), "P2P address never reached addrman"
        endpoint = f"[{address}]:{port}" if ":" in address else f"{address}:{port}"
        outbound = [p for p in rpc().getpeerinfo() if p["addr"] == endpoint]
        assert outbound and all(not p["inbound"] for p in outbound), outbound
        assert all(p["connection_type"] == "outbound-full-relay" for p in outbound), outbound
        assert all(p["transport_protocol_type"] == transport for p in outbound), outbound
        pong = client.call("ping", {"id": completed[0]["id"], "nonce": 12345, "timeout": 5})
        assert pong["pong_received"], pong
        print(f"PASS {encoding}: {endpoint}, {transport} automatic outbound, handshake and pong", flush=True)

    # An unrelated request must not count as the result of this announcement.
    assert observe(client, [{"address": "79.1.2.3", "port": 8333}], timeout=0) == []
    print("PASS unrelated proxy destinations do not satisfy the observation", flush=True)


if __name__ == "__main__":
    main()
