"""P2P workload: an adversarial peer for node1, driven by Antithesis test commands.

Three containers make up the workload:

- ``node1``: the Bitcoin Core node under test.
- ``adversary``: a long-lived server (`p2p_workload.server`) that owns P2P
  connections to node1, speaking the protocol through Bitcoin Core's own
  functional test framework (``test_framework.p2p``). The test drivers in
  `p2p_workload.drivers` run inside this container and tell the server what to
  do -- open a connection with a given handshake configuration, send a ping
  with a given nonce -- and assert on the results it reports back.
- ``health-checker``: waits for node1 and the adversary, then signals
  ``setup_complete`` (`p2p_workload.health_checker`).
"""
