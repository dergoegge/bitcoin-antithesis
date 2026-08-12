#!/bin/sh
# Explore the running GUI with Bombadil, checking the properties in the
# specification against every state it reaches.
#
# This is a parallel driver so that the chain keeps moving while the interface is
# being explored: blocks arrive and transactions confirm underneath the clicks,
# which is the state a scripted GUI test never sees. It attaches to the
# application the container is already running instead of starting its own, so
# the interface it explores is the one whose wallet the other drivers fund.
set -eu

TIME_LIMIT="${BOMBADIL_TIME_LIMIT:-5m}"
SOCKET="${BRIDGE_SOCKET:-/run/bitcoin-core-app/bridge.sock}"

# Deliberately not under ANTITHESIS_OUTPUT_DIR. Bombadil writes a trace entry
# per state, each carrying the whole item tree, which is tens of megabytes for a
# five minute run -- worth having while debugging, far too much to carry into
# every report. It stays in the container, where the debugger can reach it, and
# a local run can read it straight from the filesystem.
OUTPUT_DIR="${BOMBADIL_OUTPUT_DIR:-/tmp/bombadil}"

# How far into the interface a run got, as sometimes-assertions.
#
# A property cannot say this. Bombadil reports every one of them as an `always`
# that must hold in each state, so "we reached the review page" would be a
# failure in each run that did not -- and most will not. A sometimes-assertion
# is the right shape: Antithesis wants it true in at least one run across the
# test, and says so in the report when nothing ever reached it. The SDK's file
# format is the whole interface, so the driver writes them itself.
#
# Each is keyed on an action the run took, rather than on an item existing: a
# popup can sit in the tree unbuilt, but a click is something that happened.
coverage() {
    "$1" gui_send_form_reviewed \
        "The GUI reached the send review page, so a recipient and amount were accepted" \
        "Left-clicking .*sendReviewButton"
    "$1" gui_payment_broadcast \
        "A payment was broadcast through the GUI" \
        "Left-clicking .*sendReviewBroadcastButton"
    "$1" gui_receive_address_generated \
        "The GUI generated a receive address" \
        "Left-clicking .*requestPayment(Generate|Create)Button"
    "$1" gui_typed_a_valid_address \
        "A valid address was typed into the GUI" \
        "Typing \"bcrt1q[a-z0-9]+\" into .*[Aa]ddress"
}

# condition, hit, message. The message is the id too: every worked example --
# the documented one and every assertion Bombadil's SDK emits -- uses the same
# string for both, and a run whose ids differed from their messages produced no
# properties at all.
#
# must_hit is true even though the assertion is a sometimes: that is what the
# documented declaration uses, and it is what says the assertion has to be
# satisfied somewhere across the test rather than being optional. A first
# attempt with must_hit false wrote well-formed events that the report ignored.
emit_assert() {
    [ -n "${ANTITHESIS_OUTPUT_DIR:-}" ] || return 0
    printf '{"antithesis_assert":{"assert_type":"sometimes","display_type":"Sometimes","condition":%s,"hit":%s,"must_hit":true,"id":"%s","message":"%s","details":null,"location":{"begin_column":0,"begin_line":0,"class":"qml_gui","file":"qml_gui/parallel_driver_explore.sh","function":"reached"}}}\n' \
        "$1" "$2" "$3" "$3" >> "${ANTITHESIS_OUTPUT_DIR}/sdk.jsonl"
}

# The catalog entry the SDK would write at startup. Without it an assertion that
# is never true is missing from the report rather than reported as never true.
declare_reached() { emit_assert false false "$2"; }

coverage declare_reached

# Antithesis may start several instances of a parallel driver at once, and two
# drivers clicking the same interface is not a test of anything. The first one
# to take the lock does the exploring; the rest step aside.
exec 9>/run/bombadil.lock
if ! flock -n 9; then
    echo "[explore] another exploration run holds the bridge, nothing to do"
    exit 0
fi

# The specification is bundled relative to the working directory.
cd /opt/spec

# --ignore blockClock leaves the block clock out of the observed state: it
# animates continuously, so with it in the interface never looks settled and
# every state differs from the last for reasons unrelated to the test.
# Bombadil prints the whole item tree for every state it visits, which at five
# states a second is hundreds of lines a second and swamps the run's logging
# budget -- Antithesis reports "logging output was limited" and the useful lines
# are lost among the trees. It goes to a file next to the trace instead, which
# is collected either way, and only the summary and any violations are echoed
# below. Properties reach the report through the SDK regardless of this.
EXPLORE_LOG="${OUTPUT_DIR}/exploration.log"
mkdir -p "${OUTPUT_DIR}"

status=0
bombadil qml test \
    --attach "${SOCKET}" \
    --specification ./gui.ts \
    --time-limit "${TIME_LIMIT}" \
    --ignore blockClock \
    --settle-timeout-ms 4000 \
    --output-path "${OUTPUT_DIR}" \
    --output-path-overwrite > "${EXPLORE_LOG}" 2>&1 || status=$?

# What a reader of the report needs: how the run ended, how much it did, and
# every property that failed. The trees stay in the file.
grep -E "^(Exited|Throughput|Trace written|Qt/QML messages)" "${EXPLORE_LOG}" || true
grep -E "was violated|qml test failed" "${EXPLORE_LOG}" || true

# On a failure the last few actions are what a reader wants, and the file they
# are in may not outlive the container, so they go to the report as well.
if [ "${status}" -ne 0 ]; then
    echo "[explore] last actions before the run ended:"
    grep -E "^[0-9]{2}:[0-9]{2}\.[0-9]+ " "${EXPLORE_LOG}" | tail -20
fi

reached() {
    if grep -qE "$3" "${EXPLORE_LOG}"; then condition=true; else condition=false; fi
    echo "[explore] $1: ${condition}"
    emit_assert "${condition}" true "$2"
}

coverage reached

# Driving an application it did not start, Bombadil cannot tell an application
# that exited from a bridge that stopped answering: both end the run as an
# error. Say which one it was, so that a crash and a killed container are not
# read as a property violation, and are told apart from each other by what the
# container's own log says next.
#
# Matched against the full command line: the process name itself is truncated to
# 15 characters, which "bitcoin-core-app" does not fit into.
if [ "${status}" -ne 0 ] && ! pgrep -f bitcoin-core-app >/dev/null 2>&1; then
    echo "[explore] the application is no longer running: the run ended with the process, not with a property"
fi

exit "${status}"
