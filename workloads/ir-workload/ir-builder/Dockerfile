FROM rustlang/rust:nightly-bookworm AS builder

WORKDIR /build

# Copy workspace Cargo.toml
COPY Cargo.toml Cargo.toml

# Copy all workspace members
COPY fuzzamoto fuzzamoto
COPY fuzzamoto-ir fuzzamoto-ir
COPY ir-builder ir-builder

# Create stub for workload to satisfy workspace
RUN mkdir -p workload/src/bin && \
    echo '[package]\nname = "workload"\nversion.workspace = true\nedition.workspace = true\n\n[dependencies]' > workload/Cargo.toml && \
    echo 'fn main() {}' > workload/src/bin/health_checker.rs && \
    echo 'fn main() {}' > workload/src/bin/workload.rs && \
    touch workload/src/lib.rs

RUN cargo build --release -p ir-builder

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

COPY --from=builder /build/target/release/ir-builder /ir-builder

RUN chmod +x /ir-builder

ENV IR_BUILDER_PORT=9000

EXPOSE 9000

CMD ["/ir-builder"]
