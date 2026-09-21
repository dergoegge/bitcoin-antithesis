# The adversary image: Bitcoin Core's functional test framework as the P2P
# implementation, the Antithesis Python SDK, the adversary server, the health
# checker and the test drivers. Which of these a container runs is decided by
# its entrypoint in docker-compose.yaml.

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
COPY p2p_workload /adversary/p2p_workload

# `test_framework` and `p2p_workload` are importable from anywhere; logs reach
# Antithesis as they are written.
ENV PYTHONPATH=/adversary \
    PYTHONUNBUFFERED=1

# Test commands, as symlinks to the driver modules, and the assertion catalog
# so that Antithesis knows every property up front, including those a run never
# reaches.
RUN chmod +x /adversary/p2p_workload/drivers/*.py \
    && mkdir -p /opt/antithesis/test/v1/main /opt/antithesis/catalog \
    && ln -s /adversary/p2p_workload/drivers/parallel_driver_new_connection.py \
        /opt/antithesis/test/v1/main/parallel_driver_new_connection \
    && ln -s /adversary/p2p_workload/drivers/eventually_ping_pong.py \
        /opt/antithesis/test/v1/main/eventually_ping_pong \
    && ln -s /adversary/p2p_workload /opt/antithesis/catalog/p2p_workload

EXPOSE 9000

# Default: run the adversary server. The health checker overrides this.
CMD ["python3", "-m", "p2p_workload.server"]
