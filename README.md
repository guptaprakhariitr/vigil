# vigil — VigilAI CLI (open wrapper)

The on-call engineer that runs on your own box. `vigil` is the open wrapper: CLI, adapters, schemas.
The deterministic engine ships as the private `vigil-engine` crate (bundled binary in releases).

> Plan: see `docs/coding_plan.md`. Specs/diagrams in `vc_utilities/idea-artifact/`.

## Phase 1 (now): agentless investigate
```
vigil investigate <log-path> --repo <dir> --project <name> [--engine claude-cli|none] [--no-engine] [--out report.md]
```
Pipeline: ingest → template-mine → correlate → evidence bundle → engine (one hop) → **cited RCA report**.
`--no-engine` runs the deterministic pipeline with zero tokens.

## Dev build
Checkout `vigil` and `vigil-engine` side by side (path dependency), then:
```
cargo run -p vigil -- investigate fixtures/payments-logs --repo /tmp/payments-repo --project acme-payments
```

License: Apache-2.0.
