# The adversary image: Bitcoin Core's functional test framework as the P2P
# implementation, the Antithesis Python SDK, the adversary server, the test
# drivers and the health checker (whose container overrides the entrypoint).

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
COPY *.py /adversary/
COPY drivers /adversary/drivers

# `test_framework` and the adversary modules are importable from anywhere; logs
# reach Antithesis as they are written.
ENV PYTHONPATH=/adversary \
    PYTHONUNBUFFERED=1

# Catalog both driver and proxy assertions, including properties a run never
# reaches. The proxy reports progress independently of test command lifetimes.
RUN chmod +x /adversary/drivers/*.py \
    && mkdir -p /opt/antithesis/test/v1/main /opt/antithesis/catalog \
    && for driver in /adversary/drivers/*.py; do \
        ln -s "$driver" "/opt/antithesis/test/v1/main/$(basename "$driver" .py)"; done \
    && ln -s /adversary/drivers /opt/antithesis/catalog/p2p-workload \
    && ln -s /adversary/proxy.py /opt/antithesis/catalog/p2p-proxy.py

EXPOSE 9000 9050

CMD ["python3", "/adversary/server.py"]
