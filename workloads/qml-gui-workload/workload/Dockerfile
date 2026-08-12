FROM rust:1.85-bookworm AS builder

WORKDIR /build

COPY Cargo.toml Cargo.toml
COPY Cargo.lock Cargo.lock
COPY src src

RUN cargo build --release --locked

# Final image
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Test commands. The name of this directory is the test template these commands
# belong to; the GUI image uses the same one so that exploring the interface and
# driving the chain are part of a single template.
RUN mkdir -p /opt/antithesis/test/v1/qml_gui

COPY --from=builder /build/target/release/parallel_driver_blocks /opt/antithesis/test/v1/qml_gui/
COPY --from=builder /build/target/release/parallel_driver_tx_simple /opt/antithesis/test/v1/qml_gui/

# The health checker runs as its own container rather than as a test command.
COPY --from=builder /build/target/release/health_checker /health_checker

RUN chmod +x /opt/antithesis/test/v1/qml_gui/* /health_checker

CMD ["sleep", "infinity"]
