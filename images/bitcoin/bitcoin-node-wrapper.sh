#!/bin/bash
# Entrypoint that swarm-fuzzes startup-only bitcoind options. Each node persists
# a personality in its datadir -- one inclusion weight per tunable plus the
# structural flags -- and re-rolls the values on every boot. /dev/urandom is
# Antithesis-controlled entropy.
set -uo pipefail

DATADIR=/data
STABLE="$DATADIR/swarm_stable.conf"
WEIGHTS="$DATADIR/swarm_weights.conf"
NODE=bitcoin-node

rand() { od -An -tu2 -N2 /dev/urandom | tr -d ' '; }
weight() { echo $(( $(rand) % 101 )); }
coin() { [ $(( $(rand) % 100 )) -lt "$1" ]; }
pick() { local n=$(( $(rand) % $# )); shift "$n"; echo "$1"; }

# Boundary-heavy value sets. Core saturates and clamps most out-of-range values,
# but blockmaxweight outside [-blockreservedweight, MAX_BLOCK_WEIGHT], prune
# below 550 (other than 0/1), limitclustercount above 64 and maxconnections
# below 0 are init errors. maxsigcachesize allocates v/2 MiB per cache up front.
declare -A TUNABLES=(
    [dbcache]="1 4 50 300 1000"
    [par]="0 1 2 4 15"
    [prevoutfetchthreads]="0 1 2 4 8 16"
    [maxmempool]="5 50 300"
    [mempoolexpiry]="1 24 336"
    [maxconnections]="8 40 125"
    [persistmempool]="0 1"
    [rpcthreads]="1 2 4 16"
    [rpcworkqueue]="2 16 64 1000"
    [blockmaxweight]="8000 400000 3985000 4000000"
    [bytespersigop]="0 1 20 125000 3435973837"
    [maxsigcachesize]="0 1 8 32 128"
    [checkmempool]="0 10 1000 1000000"
    [dbbatchsize]="1000 1048576 16777216 134217728"
    [datacarriersize]="0 1 83 100000"
    [maxreceivebuffer]="-1 1 5000 100000"
    [limitclustercount]="1 2 10 64"
)
# checkmempool=1 is deliberately absent: a full consistency check per mempool
# operation is quadratic against the bulkdata filler.

# Flags the swarm owns, stripped from the base command so we never double-set.
MANAGED="${!TUNABLES[*]} prune fastprune txindex txospenderindex coinstatsindex blockfilterindex"

# Branches differ in which options exist, and an unknown option is a startup
# failure, so offer only what this binary documents.
HELP=$("$NODE" -regtest -help -help-debug 2>/dev/null)
known() { grep -Eq "^[[:space:]]+-$1(=|[[:space:]]|$)" <<< "$HELP"; }

BASE=()
for arg in "$@"; do
    managed=0
    for m in $MANAGED; do case "$arg" in -$m|-$m=*) managed=1 ;; esac; done
    [ "$managed" = 0 ] && BASE+=("$arg")
done

FLAGS=()
mkdir -p "$DATADIR"

# Structural flags: drawn once, reused across restarts.
if [ ! -f "$STABLE" ]; then
    stable=()
    # node1 never prunes: a pruned node advertises NODE_NETWORK_LIMITED and won't
    # serve blocks older than tip-288, so keep one archival node to recover from.
    if [ "${HOSTNAME:-}" != "node1" ] && known prune && coin "$(weight)"; then
        stable+=("-prune=$(pick 1 550 1000)")
        known fastprune && coin "$(weight)" && stable+=("-fastprune")
    else
        known txindex && coin "$(weight)" && stable+=("-txindex=1")
        known txospenderindex && coin "$(weight)" && stable+=("-txospenderindex=1")
        known coinstatsindex && coin "$(weight)" && stable+=("-coinstatsindex=1")
        known blockfilterindex && coin "$(weight)" && stable+=("-blockfilterindex=1")
    fi
    printf '%s\n' "${stable[@]}" > "$STABLE"
fi
while IFS= read -r line; do
    [ -n "$line" ] && FLAGS+=("$line")
done < "$STABLE"

# One inclusion weight per tunable, drawn once; entries missing from the file are
# filled in, so a larger table keeps the weights already drawn.
declare -A WEIGHT=()
if [ -f "$WEIGHTS" ]; then
    while read -r flag w; do [ -n "$flag" ] && WEIGHT[$flag]=$w; done < "$WEIGHTS"
fi
for flag in "${!TUNABLES[@]}"; do
    [ -n "${WEIGHT[$flag]:-}" ] && continue
    WEIGHT[$flag]=$(weight)
    printf '%s %s\n' "$flag" "${WEIGHT[$flag]}" >> "$WEIGHTS"
done

for flag in "${!TUNABLES[@]}"; do
    known "$flag" || continue
    coin "${WEIGHT[$flag]}" || continue
    FLAGS+=("-$flag=$(pick ${TUNABLES[$flag]})")
done

echo "[bitcoin-node-wrapper] ${HOSTNAME:-node} swarm config: ${FLAGS[*]}" >&2

exec "$NODE" "${BASE[@]}" "${FLAGS[@]}"
