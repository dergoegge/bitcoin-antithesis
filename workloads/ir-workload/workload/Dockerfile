FROM rustlang/rust:nightly-bookworm AS builder

WORKDIR /build

# Copy the entire workspace
COPY Cargo.toml Cargo.toml
COPY fuzzamoto fuzzamoto
COPY fuzzamoto-ir fuzzamoto-ir
COPY ir-builder ir-builder
COPY workload workload

RUN cargo build --release -p ir-workload

FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates \
    && rm -rf /var/lib/apt/lists/*

# Create Antithesis test directory
RUN mkdir -p /opt/antithesis/test/v1/main

# Copy serial driver binaries to Antithesis test directory
COPY --from=builder /build/target/release/serial_driver_single_tx_generator /opt/antithesis/test/v1/main/
COPY --from=builder /build/target/release/serial_driver_get_data_generator /opt/antithesis/test/v1/main/
COPY --from=builder /build/target/release/serial_driver_input_mutator /opt/antithesis/test/v1/main/
COPY --from=builder /build/target/release/serial_driver_operation_mutator /opt/antithesis/test/v1/main/
COPY --from=builder /build/target/release/serial_driver_compile /opt/antithesis/test/v1/main/
COPY --from=builder /build/target/release/serial_driver_txo_generator /opt/antithesis/test/v1/main/
COPY --from=builder /build/target/release/serial_driver_header_generator /opt/antithesis/test/v1/main/
COPY --from=builder /build/target/release/serial_driver_one_parent_one_child_generator /opt/antithesis/test/v1/main/
COPY --from=builder /build/target/release/serial_driver_long_chain_generator /opt/antithesis/test/v1/main/
COPY --from=builder /build/target/release/serial_driver_large_tx_generator /opt/antithesis/test/v1/main/
COPY --from=builder /build/target/release/serial_driver_send_block_generator /opt/antithesis/test/v1/main/
COPY --from=builder /build/target/release/serial_driver_inventory_generator /opt/antithesis/test/v1/main/
COPY --from=builder /build/target/release/serial_driver_compact_block_generator /opt/antithesis/test/v1/main/
COPY --from=builder /build/target/release/serial_driver_compact_filter_query_generator /opt/antithesis/test/v1/main/
COPY --from=builder /build/target/release/serial_driver_bloom_filter_load_generator /opt/antithesis/test/v1/main/
COPY --from=builder /build/target/release/serial_driver_bloom_filter_add_generator /opt/antithesis/test/v1/main/
COPY --from=builder /build/target/release/serial_driver_bloom_filter_clear_generator /opt/antithesis/test/v1/main/
COPY --from=builder /build/target/release/serial_driver_addr_relay_generator /opt/antithesis/test/v1/main/
COPY --from=builder /build/target/release/serial_driver_addr_relay_v2_generator /opt/antithesis/test/v1/main/
COPY --from=builder /build/target/release/serial_driver_get_addr_generator /opt/antithesis/test/v1/main/
COPY --from=builder /build/target/release/serial_driver_send_message_generator /opt/antithesis/test/v1/main/
COPY --from=builder /build/target/release/serial_driver_block_generator /opt/antithesis/test/v1/main/
COPY --from=builder /build/target/release/serial_driver_tip_block_generator /opt/antithesis/test/v1/main/
COPY --from=builder /build/target/release/eventually_consistent /opt/antithesis/test/v1/main/
COPY workload/serial_driver_tip_and_compile.sh /opt/antithesis/test/v1/main/serial_driver_tip_and_compile

# Copy health checker to root (not a test driver)
COPY --from=builder /build/target/release/health_checker /health_checker

# Make binaries executable
RUN chmod +x /opt/antithesis/test/v1/main/* /health_checker

# Default command - do nothing, let Antithesis orchestrate
CMD ["sleep", "infinity"]
