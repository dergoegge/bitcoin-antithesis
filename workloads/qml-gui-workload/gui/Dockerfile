# The system under test: `bitcoin-core-app` with the QML test automation bridge
# enabled, plus the Bombadil CLI that drives it through that bridge.
#
# Both live in one image because the bridge is a Unix domain socket, so the
# driver and the application have to share a filesystem. The application runs as
# the container's process; Bombadil attaches to it from a test command, which
# means the interface being explored is the same long-lived one the chain
# drivers are moving underneath.
#
# Build context is this directory (it carries the entrypoint and the test
# commands); the sources are cloned, matching how images/bitcoin builds a node.

ARG LLVM_V=20

# Bombadil, with only the QML driver enabled: the terminal driver needs a Zig
# toolchain to build libghostty, which this image has no use for.
FROM docker.io/library/rust:1-bookworm AS bombadil

ARG BOMBADIL_REPO=https://github.com/dergoegge/bombadil.git
ARG BOMBADIL_BRANCH=bombadil-bitcoin-qml

RUN apt-get update && apt-get install -y --no-install-recommends \
        pkg-config libssl-dev ca-certificates git \
    && rm -rf /var/lib/apt/lists/*

# bombadil-cli embeds the Inspect frontend, whose build script shells out to
# trunk, so it has to be present even when Inspect is not used.
RUN rustup target add wasm32-unknown-unknown \
    && cargo install --locked trunk

WORKDIR /src

# The commit the branch is expected to be at. It is only echoed, but naming it
# is what makes the clone re-run: without it the layer stays cached and a
# rebuild silently keeps whichever commit was current the first time. It is
# declared here rather than with the other args because an ARG invalidates
# everything below it, and only the clone should depend on this one.
ARG BOMBADIL_REV=unpinned
RUN echo "bombadil ${BOMBADIL_BRANCH} at ${BOMBADIL_REV}" \
    && git clone --depth 1 --branch ${BOMBADIL_BRANCH} ${BOMBADIL_REPO} . \
    && git -C . rev-parse HEAD > /src/bombadil.rev \
    && cat /src/bombadil.rev

# bombadil-cli's build script drives trunk with `--offline`, which will not
# fetch anything: not the wasm-bindgen and wasm-opt binaries it needs, and not
# the frontend's crates. Building the frontend once with the network is what
# puts all of them in the caches the offline build then reads, and it keeps the
# versions the frontend asks for rather than pinning them here.
RUN cd lib/bombadil-inspect && trunk build --release

# Offline, though, trunk takes its tools only from PATH -- it reaches for its
# cache on the download path it is not allowed to take -- so the binaries the
# build above cached are put there. Their directories go on PATH rather than the
# files being linked into one, because binaryen's wasm-opt loads a library from
# beside itself; and they are discovered rather than named, so the versions stay
# the frontend's to choose.
RUN tools="$(find /root/.cache/trunk -type f \
        \( -name wasm-bindgen -o -name wasm-opt \) -executable \
        -printf '%h\n' | sort -u | paste -sd:)" \
    && echo "trunk tools on PATH: ${tools}" \
    && PATH="${tools}:${PATH}" cargo build --release --locked \
        -p bombadil-cli \
        --no-default-features --features qml


# The application, instrumented for Antithesis the same way the node image is.
FROM docker.io/library/debian:bookworm AS app

ARG GUI_QML_REPO=https://github.com/dergoegge/gui-qml.git
ARG GUI_QML_BRANCH=antithesis
ARG LLVM_V

RUN apt-get update && apt-get install -y --no-install-recommends \
        build-essential cmake ninja-build curl git ca-certificates gnupg \
        lsb-release software-properties-common python3 pkgconf \
        libboost-dev libsqlite3-dev libgl-dev libqrencode-dev \
        qt6-base-dev qt6-tools-dev qt6-l10n-tools qt6-tools-dev-tools \
        qt6-declarative-dev \
    && rm -rf /var/lib/apt/lists/*

RUN curl -sSL https://apt.llvm.org/llvm.sh | bash -s -- ${LLVM_V} \
    && apt-get update \
    && apt-get install -y --no-install-recommends clang-${LLVM_V} lld-${LLVM_V} \
    && rm -rf /var/lib/apt/lists/*

# The sanitizer the application is built with, as the node image takes one.
# AddressSanitizer earns its slowdown here: the faults this workload has turned
# up are lifetime bugs -- a socket used after the client disconnected, a crash
# inside Qt's dialog implementation -- and those are what it names, at the
# access rather than at the eventual segfault.
ARG SANITIZERS=address
#
# Coverage instrumentation, compiled as a single object file: the header has to
# be included in exactly one translation unit, not force-included everywhere.
COPY --from=instrumentation antithesis_instrumentation.h /src/antithesis_instrumentation.h
RUN clang++-${LLVM_V} -c -g ${SANITIZERS:+-fsanitize=${SANITIZERS}} \
    -o /src/antithesis_instrumentation.o \
    -x c++ /dev/null \
    -include /src/antithesis_instrumentation.h

# The Bitcoin Core tree the app builds against is a submodule of this
# repository, pinned to the commit the branch was developed against.
# See BOMBADIL_REV: declared next to the clone it invalidates, for the same
# reason.
ARG GUI_QML_REV=unpinned
RUN echo "gui-qml ${GUI_QML_BRANCH} at ${GUI_QML_REV}" \
    && git clone --depth 1 --branch ${GUI_QML_BRANCH} ${GUI_QML_REPO} /src/gui-qml \
    && git -C /src/gui-qml rev-parse HEAD > /src/gui-qml.rev \
    && cat /src/gui-qml.rev \
    && { git -C /src/gui-qml submodule update --init --recursive --depth 1 \
         || git -C /src/gui-qml submodule update --init --recursive; }

WORKDIR /src/gui-qml

ENV CC=clang-${LLVM_V}
ENV CXX=clang++-${LLVM_V}

# The flags go through CMAKE_CXX_FLAGS rather than Core's APPEND_CXXFLAGS
# because this is a superproject: the app's own sources are built by the
# top-level project, which does not read Core's flag options.
#
# BUILD_APP_TESTS is off because the run exercises the application itself rather
# than the unit test binaries, and ENABLE_TEST_AUTOMATION is what compiles the
# bridge Bombadil talks to.
RUN cmake -B build -GNinja \
        -DENABLE_TEST_AUTOMATION=ON \
        -DBUILD_APP_TESTS=OFF \
        -DBUILD_DAEMON=OFF \
        -DENABLE_IPC=OFF \
        -DCMAKE_CXX_FLAGS="-fsanitize-coverage=trace-pc-guard ${SANITIZERS:+-fsanitize=${SANITIZERS}} -g -DABORT_ON_FAILED_ASSUME" \
        -DCMAKE_EXE_LINKER_FLAGS="-fuse-ld=lld-${LLVM_V} ${SANITIZERS:+-fsanitize=${SANITIZERS}} -Wl,--build-id /src/antithesis_instrumentation.o" \
    && cmake --build build -j"$(nproc)" --target bitcoin-core-app

# llvm-symbolizer's shared libraries, for the runtime image below.
RUN mkdir -p /symbolizer-libs \
    && ldd /usr/bin/llvm-symbolizer-${LLVM_V} \
    | awk '/=>/ {print $3}' \
    | xargs -I{} cp -L {} /symbolizer-libs/


FROM docker.io/library/debian:bookworm

# Runtime Qt plugins only: the container runs headless with software rendering,
# so no GPU driver stack is needed. libqt6test6 is here because the test bridge
# synthesises input through QTest, so the application links Qt6::Test and will
# not start without it.
#
# qt-labs-folderlistmodel is imported by Qt's non-native FileDialog rather than
# by this application; without it, opening the export dialog segfaults inside
# Qt.
RUN apt-get update && apt-get install -y --no-install-recommends \
        ca-certificates wget util-linux procps libsqlite3-0 libqrencode4 libgl1 \
        qt6-qpa-plugins \
        libqt6widgets6 libqt6network6 libqt6test6 \
        qml6-module-qtqml qml6-module-qtqml-models \
        qml6-module-qtqml-workerscript qml6-module-qt-labs-settings \
        qml6-module-qt-labs-folderlistmodel \
        qml6-module-qtquick qml6-module-qtquick-window \
        qml6-module-qtquick-layouts qml6-module-qtquick-controls \
        qml6-module-qtquick-dialogs qml6-module-qtquick-templates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=app /src/gui-qml/build/bin/bitcoin-core-app /usr/local/bin/bitcoin-core-app
COPY --from=bombadil /src/target/release/bombadil /usr/local/bin/bombadil

# The commits the two binaries above were built from, so that what is in an
# image can be checked rather than inferred from when it was built:
#   docker run --rm --entrypoint cat <image> /opt/build-info
COPY --from=app /src/gui-qml.rev /tmp/gui-qml.rev
COPY --from=bombadil /src/bombadil.rev /tmp/bombadil.rev
RUN printf 'gui-qml %s\nbombadil %s\n' \
        "$(cat /tmp/gui-qml.rev)" "$(cat /tmp/bombadil.rev)" > /opt/build-info \
    && rm /tmp/gui-qml.rev /tmp/bombadil.rev \
    && cat /opt/build-info

# The specification is this workload's own: the properties that hold for the
# GUI depend on the state the workload puts it in, and this one funds the
# wallet and keeps spending from it.
COPY spec /opt/spec

# Antithesis instrumentation: the object file linked into the application looks
# for this library at runtime, and reports coverage to it when it is present.
RUN wget https://antithesis.com/assets/instrumentation/libvoidstar.so -O /usr/lib/libvoidstar.so
RUN mkdir -p /symbols && ln -s /usr/local/bin/bitcoin-core-app /symbols/bitcoin-core-app

# Without these a crash report is a list of addresses. The application is built
# with -fsanitize-coverage, so its sanitizer runtime prints the stack trace, and
# it shells out to an external symbolizer to name the frames -- "invalid path to
# external symbolizer" in every trace this workload produced until now. The node
# image does the same thing for bitcoind.
COPY --from=app /usr/bin/llvm-symbolizer-20 /usr/bin/llvm-symbolizer
COPY --from=app /symbolizer-libs/ /usr/lib/
ENV ASAN_OPTIONS="external_symbolizer_path=/usr/bin/llvm-symbolizer:detect_leaks=0:abort_on_error=1" \
    TSAN_OPTIONS="external_symbolizer_path=/usr/bin/llvm-symbolizer" \
    UBSAN_OPTIONS="external_symbolizer_path=/usr/bin/llvm-symbolizer"

COPY entrypoint.sh /usr/local/bin/bitcoin-core-app-wrapper
COPY test /opt/antithesis/test
RUN chmod +x /usr/local/bin/bitcoin-core-app-wrapper /opt/antithesis/test/v1/qml_gui/*

ENV QT_QPA_PLATFORM=offscreen \
    QT_QUICK_BACKEND=software \
    LIBGL_ALWAYS_SOFTWARE=1 \
    HOME=/root \
    BRIDGE_SOCKET=/run/bitcoin-core-app/bridge.sock

# P2P and RPC
EXPOSE 18444 18443

ENTRYPOINT ["bitcoin-core-app-wrapper"]
