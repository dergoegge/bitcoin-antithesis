# QML GUI workload

Explores `bitcoin-core-app` — the [QML GUI](https://github.com/bitcoin-core/gui-qml)
— with [Bombadil](https://github.com/antithesishq/bombadil) while the chain
underneath it moves, and checks the GUI's properties on every state it reaches.

The other workloads in this repository drive Bitcoin Core through its RPC
interface. This one drives it through its user interface, which is a different
kind of test: the application is not asked to do something specific, it is
clicked through autonomously, and a property has to hold no matter where the
exploration ends up.

## Environment

| Container | What it is |
|---|---|
| `gui` | The system under test: `bitcoin-core-app`, running a node in-process, with the test automation bridge enabled. Bombadil lives in this image too. |
| `node1` | A plain instrumented bitcoind, peered with `gui`. |
| `health-checker` | Funds the GUI's wallet before the run starts, then signals `setup_complete`. |
| `workload` | The chain-side test commands. |

Bombadil and the application share a container because the bridge is a Unix
domain socket, so the driver has to share a filesystem with the application.

The application is started by the container, not by Bombadil, and the
exploration command attaches to it with `--attach`. That is what lets the
interface being explored be a long-lived one whose chain and wallet are being
moved by other test commands, rather than a fresh regtest datadir that never
receives anything.

## Setup

`health_checker` performs the same setup as the initial RPC workload — a wallet
on every node, coinbase rewards spread across them, and 100 blocks on top so
those rewards are spendable — with the difference that one of the nodes here is
the GUI's own. Mining happens on `node1`, so the GUI's coins arrive over the
wire the way a real wallet's do.

By the time `setup_complete` is signalled, the application is showing a synced
chain of 120 blocks, a connected peer, and a loaded wallet with a spendable
balance and a transaction history — so the screens that render wallet state have
something to render, which is where the interesting properties live.

## Test commands

All three belong to the `qml_gui` template:

| Command | Container | What it does |
|---|---|---|
| `parallel_driver_explore.sh` | `gui` | Runs Bombadil against the interface for `BOMBADIL_TIME_LIMIT` (5m by default), then finishes so Antithesis can start another run. Only its summary and any violations go to the report; the per-state trees and `trace.jsonl` stay in the container under `/tmp/bombadil`, since a trace carries the whole item tree per state and runs to tens of megabytes. |
| `parallel_driver_blocks` | `workload` | Mines 1, 2, 16, 32 or 128 blocks on a randomly chosen node. |
| `parallel_driver_tx_simple` | `workload` | Sends a payment between the two nodes, in a randomly chosen direction. |

The chain drivers pick their node at random, so the GUI ends up on both sides of
everything: it mines and it follows, it sends and it receives. Mining on both
nodes at once is also what produces the competing tips that make the GUI reorg.

Exploration is a parallel driver so the chain keeps moving while the interface is
being clicked through. Antithesis may start several instances of a parallel
driver at once, so the command takes a lock and the instances that lose it exit
immediately — two drivers clicking the same interface is not a test of anything.

## Properties

[`gui/spec/gui.ts`](gui/spec/gui.ts) is the specification. Bombadil reports each
property to the Antithesis SDK on every state, so they appear in the triage
report without further wiring.

From the default QML specification: no QML binding, type or reference errors;
nothing logged at critical or fatal level; the application does not exit while
being explored; every state offers something to click; the page stack always
knows which page it is showing.

Specific to this GUI: translated strings are always substituted, no amount on
screen exceeds the total supply, a wallet balance is never negative, and no page
renders blank.

The amount and text properties read only non-editable items. Exploration types
random strings into fields, and a field shows what was typed, so reading those
back tests the string generator rather than the interface -- a run reported
`noUntranslatedPlaceholders` violated eight times over a `%6` it had typed
itself.

The specification is this workload's rather than a copy of the one in the GUI's
repository, because which properties hold depends on the state the GUI is put
in. That one explores an untouched regtest datadir, where every amount on
screen is a zero balance and "no negative amounts anywhere" holds. Here the
wallet spends, and an outgoing payment is rendered with a leading minus, so the
same assertion would fail on correct behaviour. This one checks the magnitude
of every amount and the sign of balances only.

## Building

```bash
just build-images     # or: docker compose -f config/docker-compose.yaml build
```

The `gui` image builds the application from
`dergoegge/gui-qml@antithesis` and Bombadil from
`dergoegge/bombadil@bombadil-bitcoin-qml`. Both are build args
(`GUI_QML_REPO`/`GUI_QML_BRANCH`, `BOMBADIL_REPO`/`BOMBADIL_BRANCH`), so a
different branch can be tested without touching the harness:

```bash
GUI_QML_BRANCH=my-branch docker compose -f config/docker-compose.yaml build gui
```

The build compiles Bitcoin Core, the GUI and Bombadil from source, so it takes a
while. The application is instrumented for Antithesis the same way the node
image is: `-fsanitize-coverage=trace-pc-guard` with the SDK's instrumentation
object linked in, and the binary published under `/symbols`.

## Running locally

The whole environment comes up under Docker Compose, which is the quickest way
to check the harness before pushing images:

```bash
cd config
docker compose up -d node1 gui
docker compose run --rm health-checker
docker exec -e BOMBADIL_TIME_LIMIT=2m gui \
    /opt/antithesis/test/v1/qml_gui/parallel_driver_explore.sh
docker compose run --rm --no-deps workload \
    /opt/antithesis/test/v1/qml_gui/parallel_driver_blocks
docker compose down
```

Run the exploration in the background and the chain drivers alongside it to get
what Antithesis will do: an interface being clicked through while its chain moves.

The exploration command prints only its summary and any violations. Each state
and the action taken go to `/tmp/bombadil/exploration.log` inside the `gui`
container, alongside `trace.jsonl`, which records every state and which
`bombadil qml test --reproduce <trace>` replays:

```bash
docker exec gui tail -40 /tmp/bombadil/exploration.log
docker cp gui:/tmp/bombadil/trace.jsonl .
```

Set `BOMBADIL_OUTPUT_DIR` to put them somewhere else.

### Watching it happen

The application renders offscreen in a normal run, because nothing is looking.
`docker-compose.watch.yaml` puts its window on your desktop through the X
socket instead, so you can watch Bombadil drive it and click it yourself.
Rendering stays in software either way, so nothing about the application's
behaviour changes.

```bash
cd config
docker compose -f docker-compose.yaml -f docker-compose.watch.yaml up -d node1 gui
docker compose -f docker-compose.yaml -f docker-compose.watch.yaml run --rm health-checker
docker exec -e BOMBADIL_TIME_LIMIT=10m gui \
    /opt/antithesis/test/v1/qml_gui/parallel_driver_explore.sh
```

It takes `DISPLAY` and `XAUTHORITY` from your environment, so it needs a local X
server; there is nothing to watch over SSH. Give the driver a longer
`BOMBADIL_TIME_LIMIT` than you would otherwise -- five minutes of clicking goes
past faster than it reads.

The overlay is for local use. Antithesis runs the environment from
`docker-compose.yaml` alone.

## Launching in Antithesis

Push the images, then launch with `qml-gui-workload-config:antithesis` as the
config image. Exclude container faults for `gui`:

```bash
just test-run-custom qml-gui-workload gui "" "" "QML GUI exploration"
```

Bombadil is driving an application it did not start, so it cannot tell an
application that was killed from a bridge that stopped answering: either ends
the exploration run as an error rather than as the `applicationKeepsRunning`
property it would report if it had launched the application itself. Killing the
container therefore turns every exploration command into a failure that reads
like a bug and is not one. Network faults are worth keeping: a partitioned GUI
still has an interface, and how it renders a node that has fallen behind is
exactly what this workload should be looking at.

The cost of that exclusion is the application's own restart path — coming up
against an existing datadir, reloading a wallet, catching up — which is worth
testing too. A run with container faults left on covers it, at the price of one
failed exploration command per kill; the command says which of the two happened
in its last line, so those are quick to tell apart.

## What the first runs turned up

- **The application dies opening a Qt Quick dialog.** `SEGV on unknown address
  0x8` inside `libQt6QuickDialogs2QuickImpl`, reached through
  `libQt6QuickDialogs2`. Seen from two call sites that both open a `FileDialog`:
  a control on the wallet settings tab, and `activityExportButton`. Every QML
  dialog module and Controls style is present in the image, so it is not a
  missing runtime dependency, and it reproduces under `xcb` on a real display as
  well as offscreen, so it is not the headless platform either.

  It was tempting to blame the test bridge, since the stack passes through the
  bridge's socket handler on its way in. It is not that: the crash still happens
  with the bridge's own use-after-free fixed (`gui-qml` commit
  `af9b58a7b2`). Whether a human clicking the same button hits it is still
  unanswered — that is one click to find out.

  It ends the exploration run that hits it, and the application stays down until
  its container comes back.
- **A use-after-free in the test bridge**, found by the same runs and fixed in
  `gui-qml`: `processClientCommands` held a `QLocalSocket*` and a reference into
  its read buffer across a command, and commands that pump the event loop let
  the client disconnect and the socket be deleted underneath them. `SEGV` at
  address `0x1c8` in `QLocalSocket`, from `testbridge.cpp:271`.
- **A balance can stay stale.** With a wallet loaded over RPC and funded only by
  coinbase outputs maturing, the wallet badge read `0.00000000 ₿` for a whole
  60-second run while the node's wallet held 250 BTC. A later payment refreshed
  it. Not asserted here — a balance the interface never claims to have updated
  is hard to call wrong from the interface alone — but worth a property that can
  compare the two, which needs an RPC the specification does not have.

## Notes

- The GUI's node is configured like any other node in this repository: RPC on
  `18443` with `user:password`, P2P on `18444`. That is what lets the chain
  drivers treat it as an ordinary node, and it is how the health checker funds
  the wallet the GUI displays.
- The application is started with `-qml_onboarded=1`, which skips onboarding so
  the node comes up on its own. Exploring onboarding needs a run that starts
  before it is completed; Bombadil's `--onboarding` flag does that when it
  launches the application itself.
- Controls whose name or label matches `quit`, `reset`, `delete` and similar are
  left alone by Bombadil's default action set, so a run does not end by shutting
  the application down a few steps in.
- Amounts are read together with their unit. The interface renders them as
  `₿`, `mBTC`, `bits` or `sat`, and which one is a display setting that
  exploration can change, so a supply bound in BTC would fail on correct
  behaviour the moment the unit became satoshis.
- The block clock is excluded from the observed state (`--ignore blockClock`).
  It animates continuously, so leaving it in means the interface never looks
  settled and every state differs from the last for reasons unrelated to the
  test.
