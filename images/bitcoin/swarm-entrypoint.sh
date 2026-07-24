#!/bin/bash
# bitcoind swarm-config entrypoint wrapper (originally prototyped in brink_swarm.nb2).
# Replaces bitcoin-node-wrapper: fuzzes a broad set of startup options per node,
# persists a per-node "personality", and execs bitcoin-node directly.
set -uo pipefail
DATADIR=/data
STABLE="$DATADIR/swarm_stable.conf"
WEIGHTS="$DATADIR/swarm_weights.conf"
INNER="$(command -v bitcoin-node || echo bitcoin-node)"      # real node binary we exec
rand(){ od -An -tu2 -N2 /dev/urandom | tr -d " "; }         # 0..65535, Antithesis-controlled
rw(){ echo $(( $(rand) % 101 )); }                          # random weight 0..100 (a %)
coin(){ [ "$(( $(rand) % 100 ))" -lt "$1" ]; }              # true with prob $1%
pick(){ local n=$(( $(rand) % $# )); shift $n; echo "$1"; } # random element of args
# flags the swarm owns; stripped from the base command so we never double-set them
MANAGED="dbcache par maxmempool maxconnections persistmempool prune txindex blockfilterindex coinstatsindex fastprune mempoolexpiry prevoutfetchthreads"
declare -A TUNABLES=( ["-dbcache="]="4 50 300 1000" ["-par="]="0 1 2 4" ["-maxmempool="]="5 50 300" ["-maxconnections="]="8 40 125" ["-persistmempool="]="0 1" ["-mempoolexpiry="]="1 24 336" ["-prevoutfetchthreads="]="0 1 2 4 8 16" )
mkdir -p "$DATADIR"
# 1) drop any managed flag from the base command line
BASE=()
for a in "$@"; do
  keep=1
  for m in $MANAGED; do case "$a" in -$m|-$m=*) keep=0 ;; esac; done
  [ "$keep" = 1 ] && BASE+=("$a")
done
F=()
# 2) structural flags: draw ONCE, persist (prune XOR indexes), reuse on restart
if [ ! -f "$STABLE" ]; then
  s=()
  if coin "$(rw)"; then
    s+=("-prune=$(pick 1 550 1000)")
    coin "$(rw)" && s+=("-fastprune")   # only meaningful alongside -prune
  else
    coin "$(rw)" && s+=("-txindex=1"); coin "$(rw)" && s+=("-blockfilterindex=1")
  fi
  if [ "${#s[@]}" -gt 0 ]; then printf "%s\n" "${s[@]}" > "$STABLE"; else : > "$STABLE"; fi
fi
mapfile -t st < "$STABLE"
[ "${#st[@]}" -gt 0 ] && F+=("${st[@]}")
# 3) one random weight PER tunable, drawn once & persisted = this node personality
if [ ! -f "$WEIGHTS" ]; then
  for flag in "${!TUNABLES[@]}"; do printf "%s %s\n" "$flag" "$(rw)" >> "$WEIGHTS"; done
fi
# 4) tunables: re-rolled every boot, included at this node persisted weight
while read -r flag w; do
  [ -z "$flag" ] && continue
  coin "$w" || continue
  F+=("$flag$(pick ${TUNABLES[$flag]})")
done < "$WEIGHTS"
# 5) report the chosen config (stdout -> node log; file -> Antithesis output dir)
echo "SWARM_CONFIG node=${HOSTNAME:-?} added=[${F[*]}]"
printf "node=%s\nweights=%s\nflags=%s\n" "${HOSTNAME:-?}" "$(tr "\n" ";" < "$WEIGHTS" 2>/dev/null)" "${F[*]}" > "${ANTITHESIS_OUTPUT_DIR:-/tmp}/swarm_${HOSTNAME:-node}.txt" 2>/dev/null || true
exec "$INNER" "${BASE[@]}" "${F[@]}"
