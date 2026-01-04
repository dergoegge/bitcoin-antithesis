# Antithesis Test Template for Bitcoin

This repository contains an Antithesis test template for Bitcoin, designed to
facilitate testing of Bitcoin full nodes using the Antithesis platform.

## Justfile Commands

### Build Commands
- `just build-images` - Build all Docker images via docker compose
- `just tag` - Tag images for the Antithesis registry
- `just build-and-tag` - Build and tag all images
- `just push` - Push images to registry
- `just build-and-push` - Build, tag, and push all images

**IMPORTANT: Never run `just push` or `just build-and-push` automatically. Only
the user should decide when to push images to the registry.**

### Test Run Commands (DO NOT invoke automatically)
- `just test-run-basic <workload>` - Launch a basic test run on Antithesis
- `just test-run-custom <workload> <container_faults_excluded> <network_faults_excluded>` - Launch a custom test run with fault exclusions

### Debugger Commands (DO NOT invoke automatically)
- `just launch-debugger <moment> <debug_description>` - Launch debugger with a moment string and description

**IMPORTANT: The test run and debugger commands should NEVER be invoked by the
LLM. These commands interact with external Antithesis services and should only
be run manually by the user.**

### Configuration Options

These variables can be overridden when running commands, e.g., `just duration=60 test-run-basic`:

| Variable | Default | Description |
|----------|---------|-------------|
| `registry` | `us-central1-docker.pkg.dev/molten-verve-216720/brink-repository` | Docker registry for pushing images |
| `duration` | `30` | Test run duration in minutes |
| `fault_profile` | `full` | Fault injection profile for custom test runs |
| `report_recipients` | `niklas@brink.dev` | Email recipients for test reports |

## Antithesis Resources

- [Composer](https://antithesis.com/docs/test_templates/)
- [Commands](https://antithesis.com/docs/test_templates/test_composer_reference/)
- [Rust SDK](https://github.com/antithesishq/antithesis-sdk-rust/)
