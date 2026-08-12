#!/bin/sh
# Runs the application for the lifetime of the container.
#
# The arguments that make it drivable are set here rather than in the compose
# file, because they have to agree with where the exploration command looks for
# the bridge. Everything else about the node -- the chain, the RPC interface,
# the peer -- comes from the compose file, like it does for a bitcoind
# container.
set -eu

mkdir -p "$(dirname "${BRIDGE_SOCKET}")" /data /run/guisettings

# A stale socket from a previous process would be inherited by a container
# restart, and the driver would connect to nothing.
rm -f "${BRIDGE_SOCKET}"

# -qml_onboarded skips the onboarding flow, so the node starts on its own and
# the application comes up in its runtime interface. -test-settings-dir keeps
# QSettings inside the container rather than in the user's config directory.
exec bitcoin-core-app \
    -test-automation="${BRIDGE_SOCKET}" \
    -test-settings-dir=/run/guisettings \
    -qml_onboarded=1 \
    "$@"
