#!/bin/sh
# Entrypoint that fuzzes startup-only bitcoind options.
set -e

rand_byte() {
    od -An -N1 -tu1 /dev/urandom | tr -d ' '
}

# 0..=16 (17 values, mod 17).
INPUTFETCH=$(( $(rand_byte) % 17 ))

# Pick from {1, 4} -- single byte mod 2 selects which one.
if [ $(( $(rand_byte) % 2 )) -eq 0 ]; then PAR=1; else PAR=4; fi

echo "[bitcoin-node-wrapper] prevoutfetchthreads=${INPUTFETCH} par=${PAR}" >&2

exec bitcoin-node "$@" "-prevoutfetchthreads=${INPUTFETCH}" "-par=${PAR}"
