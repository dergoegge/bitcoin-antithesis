set dotenv-load

registry := "us-central1-docker.pkg.dev/molten-verve-216720/brink-repository"
duration := "30"
report_recipients := "niklas@brink.dev"
fault_profile := "full"
no_cache := ""

# Build all images
build-images:
    docker compose -f workloads/initial-rpc-workload/config/docker-compose.yaml build {{no_cache}}
    docker build {{no_cache}} -t initial-rpc-workload-config:antithesis workloads/initial-rpc-workload/config/
    docker compose -f workloads/ir-workload/config/docker-compose.yaml build {{no_cache}}
    docker build {{no_cache}} -t ir-workload-config:antithesis workloads/ir-workload/config/

# Tag all images for the Antithesis registry
tag:
    docker tag bitcoin-node1:antithesis {{registry}}/bitcoin-node1:antithesis
    docker tag bitcoin-node2:antithesis {{registry}}/bitcoin-node2:antithesis
    docker tag bitcoin-node3:antithesis {{registry}}/bitcoin-node3:antithesis
    docker tag initial-rpc-workload-workload:antithesis {{registry}}/initial-rpc-workload-workload:antithesis
    docker tag initial-rpc-workload-config:antithesis {{registry}}/initial-rpc-workload-config:antithesis
    docker tag ir-workload-workload:antithesis {{registry}}/ir-workload-workload:antithesis
    docker tag ir-workload-ir-builder:antithesis {{registry}}/ir-workload-ir-builder:antithesis
    docker tag ir-workload-config:antithesis {{registry}}/ir-workload-config:antithesis

# Build and tag all images
build-and-tag: build-images tag

# Push all images to the registry
push:
    sudo true # llm guard
    docker push {{registry}}/bitcoin-node1:antithesis
    docker push {{registry}}/bitcoin-node2:antithesis
    docker push {{registry}}/bitcoin-node3:antithesis
    docker push {{registry}}/initial-rpc-workload-workload:antithesis
    docker push {{registry}}/initial-rpc-workload-config:antithesis
    docker push {{registry}}/ir-workload-workload:antithesis
    docker push {{registry}}/ir-workload-ir-builder:antithesis
    docker push {{registry}}/ir-workload-config:antithesis

# Launch a test run on Antithesis
test-run-basic workload:
    #!/usr/bin/env bash
    set -euo pipefail
    sudo true # llm guard
    date
    curl --fail -u "brink:$ANTITHESIS_BRINK_PW" \
      -X POST https://brink.antithesis.com/api/v1/launch/basic_test \
      -d '{"params": { "antithesis.description":"Antithesis test template for Bitcoin on master",
          "antithesis.duration":"{{duration}}",
          "antithesis.config_image":"{{workload}}-config:antithesis",
          "antithesis.images":"",
          "antithesis.report.recipients":"{{report_recipients}}"
          } }'

test-run-custom workload container_faults_excluded network_faults_excluded node_to_disk_fault description:
    #!/usr/bin/env bash
    set -euo pipefail
    sudo true # llm guard
    date
    fill_disk="false"
    if [[ -n "{{node_to_disk_fault}}" ]]; then
        fill_disk="true"
    fi
    curl --fail -u "brink:$ANTITHESIS_BRINK_PW" \
      -X POST https://brink.antithesis.com/api/v1/launch/brink \
      -d '{"params": { "antithesis.description":"{{description}}",
          "antithesis.duration":"{{duration}}",
          "antithesis.config_image":"{{workload}}-config:antithesis",
          "antithesis.test_name":"{{workload}}",
          "antithesis.images":"",
          "custom.fill_disk":"'"$fill_disk"'",
          "custom.node_to_disk_fault":"{{node_to_disk_fault}}",
          "custom.fault_profile": "{{fault_profile}}",
          "custom.exclusion_container_fault": "{{container_faults_excluded}}",
          "custom.exclusion_network_fault": "{{network_faults_excluded}}",
          "antithesis.report.recipients":"{{report_recipients}}"
          } }'

# Launch debugger with moment string, e.g.:
# just launch-debugger 'Moment.from({ session_id: "abc-123", input_hash: "456", vtime: 123.45 })' 'debugging issue X'
launch-debugger moment debug_description:
    #!/usr/bin/env bash
    set -euo pipefail
    sudo true # llm guard
    session_id=$(echo '{{moment}}' | sed -n 's/.*session_id: "\([^"]*\)".*/\1/p')
    input_hash=$(echo '{{moment}}' | sed -n 's/.*input_hash: "\([^"]*\)".*/\1/p')
    vtime=$(echo '{{moment}}' | sed -n 's/.*vtime: \([0-9.]*\).*/\1/p')
    echo "Parsed values: session_id=$session_id, input_hash=$input_hash, vtime=$vtime"
    [[ -z "$session_id" ]] && echo "Failed to parse session_id" && exit 1
    [[ -z "$input_hash" ]] && echo "Failed to parse input_hash" && exit 1
    [[ -z "$vtime" ]] && echo "Failed to parse vtime" && exit 1
    date
    curl --fail -u "brink:$ANTITHESIS_BRINK_PW" \
      -X POST https://brink.antithesis.com/api/interactivity/v1/launch/debugging \
      -d '{"params": {
      "antithesis.debugging.session_id":"'"$session_id"'",
      "antithesis.debugging.input_hash":"'"$input_hash"'",
      "antithesis.debugging.vtime":"'"$vtime"'",
      "antithesis.report.recipients":"{{report_recipients}}",
      "antithesis.event_description":"{{debug_description}}"
      }}'

get_archives moment:
    #!/usr/bin/env bash
    set -euo pipefail
    sudo true # llm guard
    session_id=$(echo '{{moment}}' | sed -n 's/.*session_id: "\([^"]*\)".*/\1/p')
    input_hash=$(echo '{{moment}}' | sed -n 's/.*input_hash: "\([^"]*\)".*/\1/p')
    vtime=$(echo '{{moment}}' | sed -n 's/.*vtime: \([0-9.]*\).*/\1/p')
    echo "Parsed values: session_id=$session_id, input_hash=$input_hash, vtime=$vtime"
    [[ -z "$session_id" ]] && echo "Failed to parse session_id" && exit 1
    [[ -z "$input_hash" ]] && echo "Failed to parse input_hash" && exit 1
    [[ -z "$vtime" ]] && echo "Failed to parse vtime" && exit 1
    date
    curl --fail -u "brink:$ANTITHESIS_BRINK_PW" \
      -X POST https://brink.antithesis.com/api/v1/launch/get_log_artifact \
      -d '{"params": { "custom.session_id":"'"$session_id"'",
          "custom.input_hash":"'"$input_hash"'",
          "custom.vtime":"'"$vtime"'",
          "antithesis.report.recipients":"{{report_recipients}}"
          } }'

# Build, tag, and push all images
build-and-push: build-and-tag push
