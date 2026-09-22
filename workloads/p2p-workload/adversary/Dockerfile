# The adversary image: Bitcoin Core's functional test framework as the P2P
# implementation, the Antithesis Python SDK, the adversary server and the test
# drivers. The health checker is a separate program (../health_checker.py); it
# ships in this image only because it needs the same dependencies, and its
# container overrides the entrypoint.
#
# Build context is the workload directory, one level up, so the health checker
# can be copied in from outside this folder.

# The test framework is pure standard-library Python and lives in the Bitcoin
# Core repository; take it from the same branch node1 is built from so that the
# message definitions match.
FROM python:3.12-slim AS bitcoin-src

ARG BITCOIN_REPO=https://github.com/bitcoin/bitcoin.git
ARG BITCOIN_BRANCH=master

RUN apt-get update && apt-get install -y --no-install-recommends git ca-certificates \
    && rm -rf /var/lib/apt/lists/*

RUN git clone --depth 1 --branch ${BITCOIN_BRANCH} --filter=blob:none --sparse ${BITCOIN_REPO} /bitcoin \
    && git -C /bitcoin sparse-checkout set test/functional/test_framework

# Runtime image
FROM python:3.12-slim

RUN pip install --no-cache-dir antithesis==0.3.1

WORKDIR /adversary
COPY --from=bitcoin-src /bitcoin/test/functional/test_framework /adversary/test_framework
COPY adversary/client.py adversary/server.py adversary/peer.py adversary/proxy.py /adversary/
COPY adversary/drivers /adversary/drivers
COPY health_checker.py /health_checker.py

# `test_framework` and the adversary modules are importable from anywhere; logs
# reach Antithesis as they are written.
ENV PYTHONPATH=/adversary \
    PYTHONUNBUFFERED=1

# Catalog both driver and proxy assertions, including properties a run never
# reaches. The proxy reports progress independently of test command lifetimes.
RUN chmod +x /adversary/drivers/*.py \
    && mkdir -p /opt/antithesis/test/v1/main /opt/antithesis/catalog \
    && ln -s /adversary/drivers/parallel_driver_new_connection.py \
        /opt/antithesis/test/v1/main/parallel_driver_new_connection \
    && ln -s /adversary/drivers/parallel_driver_addresses.py \
        /opt/antithesis/test/v1/main/parallel_driver_addresses \
    && ln -s /adversary/drivers/eventually_ping_pong.py \
        /opt/antithesis/test/v1/main/eventually_ping_pong \
    && ln -s /adversary/drivers /opt/antithesis/catalog/p2p-workload \
    && ln -s /adversary/proxy.py /opt/antithesis/catalog/p2p-proxy.py

EXPOSE 9000 9050

CMD ["python3", "/adversary/server.py"]
