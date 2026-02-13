#!/bin/sh
set -e

# Run tip block generator first
/opt/antithesis/test/v1/main/serial_driver_tip_block_generator

# Then run compile
/opt/antithesis/test/v1/main/serial_driver_compile
