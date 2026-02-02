FROM debian:bookworm AS builder

ARG BITCOIN_BRANCH=master
ARG LLVM_V=21
ARG BITCOIN_REPO=https://github.com/bitcoin/bitcoin.git

# Install dependencies
RUN apt-get update && apt-get install -y \
    curl \
    gnupg \
    lsb-release \
    software-properties-common \
    wget \
    git \
    cmake \
    ninja-build \
    python3 \
    pkg-config \
    build-essential \
    && rm -rf /var/lib/apt/lists/*

# Install LLVM
RUN curl -sSL https://apt.llvm.org/llvm.sh | bash -s -- ${LLVM_V} all
RUN ln -s /usr/bin/clang-${LLVM_V} /usr/bin/clang && \
    ln -s /usr/bin/clang++-${LLVM_V} /usr/bin/clang++ && \
    ln -s /usr/bin/llvm-ar-${LLVM_V} /usr/bin/llvm-ar && \
    ln -s /usr/bin/llvm-nm-${LLVM_V} /usr/bin/llvm-nm && \
    ln -s /usr/bin/llvm-ranlib-${LLVM_V} /usr/bin/llvm-ranlib && \
    ln -s /usr/bin/lld-${LLVM_V} /usr/bin/lld

RUN clang-${LLVM_V} --version

WORKDIR /src

# Copy instrumentation header
COPY antithesis_instrumentation.h /src/antithesis_instrumentation.h

# Clone Bitcoin Core
RUN echo "re-clone again"
RUN git clone --depth 1 --branch ${BITCOIN_BRANCH} ${BITCOIN_REPO} bitcoin

# Copy instrumentation header to Bitcoin source
RUN cp /src/antithesis_instrumentation.h /src/bitcoin/src/antithesis_instrumentation.h

ENV CC=clang-${LLVM_V}
ENV CXX=clang++-${LLVM_V}
ENV LD=lld-${LLVM_V}

ENV SOURCES_PATH=/tmp/bitcoin-depends
RUN make -C bitcoin/depends NO_QT=1 NO_ZMQ=1 NO_USDT=1 download-linux SOURCES_PATH=$SOURCES_PATH
# Keep extracted source
RUN sed -i --regexp-extended '/.*rm -rf .*extract_dir.*/d' ./bitcoin/depends/funcs.mk && \
    make -C ./bitcoin/depends DEBUG=1 NO_QT=1 NO_ZMQ=1 NO_USDT=1 \
      SOURCES_PATH=$SOURCES_PATH \
      AR=llvm-ar-${LLVM_V} NM=llvm-nm-${LLVM_V} RANLIB=llvm-ranlib-${LLVM_V} STRIP=llvm-strip-${LLVM_V} \
      -j$(nproc)

ARG SANITIZERS=""
RUN cmake -S bitcoin/ -B bitcoin/build \
     --toolchain depends/$(./bitcoin/depends/config.guess)/toolchain.cmake \
    -DAPPEND_CPPFLAGS="-DFUZZING_BUILD_MODE_UNSAFE_FOR_PRODUCTION -DABORT_ON_FAILED_ASSUME" \
    -DAPPEND_CXXFLAGS="-fsanitize-coverage=trace-pc-guard -g -include /src/bitcoin/src/antithesis_instrumentation.h" \
    -DAPPEND_CFLAGS="-fsanitize-coverage=trace-pc-guard -g -include /src/bitcoin/src/antithesis_instrumentation.h" \
    -DAPPEND_LDFLAGS="-fuse-ld=lld-${LLVM_V} -Wl,-z,muldefs,--build-id" \
    -DSANITIZERS="$SANITIZERS"

RUN cmake --build /src/bitcoin/build -j$(nproc) --target bitcoind
RUN cmake --build /src/bitcoin/build -j$(nproc) --target bitcoin-cli

# Collect llvm-symbolizer's shared library dependencies
RUN mkdir -p /symbolizer-libs && \
    ldd /usr/bin/llvm-symbolizer-${LLVM_V} | \
    awk '/=>/ {print $3}' | \
    xargs -I{} cp -L {} /symbolizer-libs/

# Runtime image
FROM debian:bookworm-slim

# Install minimal runtime dependencies
RUN apt-get update && apt-get install -y \
    wget \
    gdb \
    && rm -rf /var/lib/apt/lists/*

# Copy bitcoind binary
COPY --from=builder /src/bitcoin/build/bin/bitcoind /usr/local/bin/bitcoind
COPY --from=builder /src/bitcoin/build/bin/bitcoin-cli /usr/local/bin/bitcoin-cli
COPY --from=builder /src/bitcoin/test/sanitizer_suppressions/tsan ./tsan.supp
RUN echo "race:/src/bitcoin/src/antithesis_instrumentation.h" >> ./tsan.supp
# Copy llvm-symbolizer and its dependencies
COPY --from=builder /usr/bin/llvm-symbolizer-21 /usr/bin/llvm-symbolizer
COPY --from=builder /symbolizer-libs/ /usr/lib/

# Create symbols directory for Antithesis
RUN mkdir -p /symbols
RUN ln -s /usr/local/bin/bitcoind /symbols/bitcoind

# Download libvoidstar for Antithesis instrumentation
RUN wget https://antithesis.com/assets/instrumentation/libvoidstar.so -O /usr/lib/libvoidstar.so

# Create data directory
RUN mkdir -p /data

# Expose ports (P2P and RPC)
EXPOSE 18444 18443

ENV TSAN_OPTIONS="suppressions=/tsan.supp external_symbolizer_path=/usr/bin/llvm-symbolizer"
# Default entrypoint
ENTRYPOINT ["bitcoind"]
CMD ["-regtest", "-datadir=/data", "-rpcallowip=0.0.0.0/0", "-rpcbind=0.0.0.0", "-rpcuser=user", "-rpcpassword=password", "-server=1", "-listen=1", "-listenonion=0", "-fallbackfee=0.0001"]
