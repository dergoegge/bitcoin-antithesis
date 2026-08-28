set dotenv-load
set positional-arguments

registry := "us-central1-docker.pkg.dev/molten-verve-216720/brink-repository"
api := "https://brink.antithesis.com/api"
duration := "30"
report_recipients := env("ANTITHESIS_REPORT_RECIPIENTS", "niklas@brink.dev")
fault_profile := "full"
no_cache := ""

# Every workload with a config/docker-compose.yaml. Add new ones here only.
workloads := "initial-rpc-workload ir-workload"

_default:
    @just --list --unsorted

# --- helpers ----------------------------------------------------------------

# Refuse to run without a sudo prompt, so an LLM can't fire these off.
[private]
_guard:
    @sudo true # llm guard

# All images belonging to one workload (compose-derived + its config image)
[private]
images w:
    @docker compose -f workloads/$1/config/docker-compose.yaml config --images | sort -u
    @echo "$1-config:antithesis"

# All images across every workload, deduplicated
[private]
all-images:
    @for w in {{workloads}}; do just images $w; done | sort -u

# Run a per-workload recipe once per workload, e.g. `just _each build-workload`
[private]
_each recipe:
    @for w in {{workloads}}; do just $1 $w; done

# POST a params object to an Antithesis endpoint
[private]
_launch endpoint params: _guard
    #!/usr/bin/env bash
    set -euo pipefail
    date
    curl --fail -u "brink:$ANTITHESIS_BRINK_PW" \
      -X POST "{{api}}/$1" \
      -H 'content-type: application/json' \
      -d "$(jq -n --argjson p "$2" '{params: $p}')"

# Parse a Moment.from({...}) string into {session_id, input_hash, vtime}
[private]
moment-json moment:
    #!/usr/bin/env bash
    set -euo pipefail
    m="$1"
    qp() { sed -n "s/.*$1: \"\([^\"]*\)\".*/\1/p" <<<"$m"; }
    session_id=$(qp session_id)
    input_hash=$(qp input_hash)
    vtime=$(sed -n 's/.*vtime: \([0-9.]*\).*/\1/p' <<<"$m")
    [[ -z "$session_id" ]] && echo "Failed to parse session_id" >&2 && exit 1
    [[ -z "$input_hash" ]] && echo "Failed to parse input_hash" >&2 && exit 1
    [[ -z "$vtime" ]] && echo "Failed to parse vtime" >&2 && exit 1
    jq -n --arg s "$session_id" --arg h "$input_hash" --arg v "$vtime" \
      '{session_id: $s, input_hash: $h, vtime: $v}'

# --- images -----------------------------------------------------------------

# Build every image of a single workload
[group('images')]
build-workload w:
    docker compose -f workloads/$1/config/docker-compose.yaml build {{no_cache}}
    docker build {{no_cache}} -t $1-config:antithesis workloads/$1/config/

# Tag every image of a single workload for the Antithesis registry
[group('images')]
tag-workload w:
    @just images $1 | xargs -I{} docker tag {} {{registry}}/{}

# Push every image of a single workload
[group('images')]
push-workload w: _guard
    @just images $1 | xargs -I{} sh -c 'docker tag "$0" "{{registry}}/$0" && docker push "{{registry}}/$0"'

# Build all images
[group('images')]
build-images: (_each "build-workload")

# Tag all images for the Antithesis registry
[group('images')]
tag:
    @just all-images | xargs -I{} docker tag {} {{registry}}/{}

# Push all images to the registry
[group('images')]
push: _guard
    @just all-images | xargs -I{} docker push {{registry}}/{}

# Build and tag all images
[group('images')]
build-and-tag: build-images tag

# Build, tag, and push all images
[group('images')]
build-and-push: build-and-tag push

# --- antithesis -------------------------------------------------------------

# Launch a test run on Antithesis
[group('antithesis')]
test-run-basic workload:
    #!/usr/bin/env bash
    set -euo pipefail
    params=$(jq -n \
      --arg desc "Antithesis test template for Bitcoin on master" \
      --arg dur "{{duration}}" \
      --arg cfg "$1-config:antithesis" \
      --arg rcpt "{{report_recipients}}" \
      '{"antithesis.description": $desc,
        "antithesis.duration": $dur,
        "antithesis.config_image": $cfg,
        "antithesis.images": "",
        "antithesis.report.recipients": $rcpt}')
    just _launch v1/launch/basic_test "$params"

# Launch a test run with fault exclusions
[group('antithesis')]
test-run-custom workload container_faults_excluded network_faults_excluded node_to_disk_fault description is_ephemeral="false":
    #!/usr/bin/env bash
    set -euo pipefail
    fill_disk="false"
    if [[ -n "$4" ]]; then
        fill_disk="true"
    fi
    params=$(jq -n \
      --arg desc "$5" \
      --arg dur "{{duration}}" \
      --arg cfg "$1-config:antithesis" \
      --arg name "$1" \
      --arg eph "$6" \
      --arg fill "$fill_disk" \
      --arg disknode "$4" \
      --arg profile "{{fault_profile}}" \
      --arg cfaults "$2" \
      --arg nfaults "$3" \
      --arg rcpt "{{report_recipients}}" \
      '{"antithesis.description": $desc,
        "antithesis.duration": $dur,
        "antithesis.config_image": $cfg,
        "antithesis.test_name": $name,
        "antithesis.images": "",
        "antithesis.is_ephemeral": $eph,
        "custom.fill_disk": $fill,
        "custom.node_to_disk_fault": $disknode,
        "custom.fault_profile": $profile,
        "custom.exclusion_container_fault": $cfaults,
        "custom.exclusion_network_fault": $nfaults,
        "antithesis.report.recipients": $rcpt}')
    just _launch v1/launch/brink "$params"

# Launch debugger with moment string, e.g.:
# just launch-debugger 'Moment.from({ session_id: "abc-123", input_hash: "456", vtime: 123.45 })' 'debugging issue X'
[group('antithesis')]
launch-debugger moment debug_description:
    #!/usr/bin/env bash
    set -euo pipefail
    m=$(just moment-json "$1")
    echo "Parsed values: $(jq -c . <<<"$m")"
    params=$(jq -n --argjson m "$m" \
      --arg rcpt "{{report_recipients}}" \
      --arg desc "$2" \
      '{"antithesis.debugging.session_id": $m.session_id,
        "antithesis.debugging.input_hash": $m.input_hash,
        "antithesis.debugging.vtime": $m.vtime,
        "antithesis.report.recipients": $rcpt,
        "antithesis.event_description": $desc}')
    just _launch interactivity/v1/launch/debugging "$params"

# Fetch log archives for a moment
[group('antithesis')]
get_archives moment:
    #!/usr/bin/env bash
    set -euo pipefail
    m=$(just moment-json "$1")
    echo "Parsed values: $(jq -c . <<<"$m")"
    params=$(jq -n --argjson m "$m" --arg rcpt "{{report_recipients}}" \
      '{"custom.session_id": $m.session_id,
        "custom.input_hash": $m.input_hash,
        "custom.vtime": $m.vtime,
        "antithesis.report.recipients": $rcpt}')
    just _launch v1/launch/get_log_artifact "$params"
