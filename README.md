# VigilAI — `vigil`

The on-call engineer that runs on your **own** box. `vigil` is the open wrapper (CLI, adapters,
schemas, packaging); the deterministic engine ships as the private `vigil-engine` crate.

**Your infra, your engine, your autonomy.** It runs where your code and logs already are, you pick
the engine (your logged-in Claude / Cursor / an API key / deterministic-only), and you set how far it
may act — from *notify* all the way to *open a PR*. The hot path is deterministic and spends **zero
tokens**; the engine is one constrained, cited hop, only when a real incident is novel.

## The loop
```
detect → triage (Tier-1 policy) → investigate (engine) → validate (git worktree) → gate (autonomy) → propose
```
- **Deterministic core:** ingest → template-mine → correlate → detect. The model only *proposes*; the
  engine *validates*. Abstains when evidence is thin; every claim is cited to the evidence.
- **Tier-1 routing policy:** a per-project mute/watch/escalate table the engine *authors* once at
  warm-setup and refines from your feedback. Runs hot, 0 tokens, fully auditable. Unknown → escalate (safe).
- **Do-no-harm:** read-only data plane; patches are only ever applied in an isolated worktree off the
  **deployed SHA**; secret-bearing patches are refused before apply; a PR needs both a clean validation
  **and** confidence ≥ threshold. Credential ceiling is a scoped git token — it never deploys.

## Commands
```
vigil investigate <logs> --repo <dir> [--engine claude-cli|none] [--out report.md]   # one-shot cited RCA
vigil project add <name> <logs> [--repo ..] [--autonomy ..] [--min-confidence ..]    # register a project
vigil up [--project <name>]            # always-on daemon (watch → detect → triage → investigate → act)
vigil sweep [--project <name>]         # batch: investigate every open escalate-routed incident once
vigil warm <logs> --project <name>     # one engine call drafts the Tier-1 policy
vigil policy / route <mute|watch|escalate> <id>    # view / give feedback on the policy
vigil feedback <accept|reject> <id> [--noise]      # learning loop → deterministic rule delta
vigil incidents / status / usage / audit           # store views, token ledger, audit trail
vigil validate <patch> --repo <dir> [--sha ..] [--test ..]   # check a patch in an isolated worktree
```
`--autonomy` = notify | report | propose | merge | release. `--no-engine` runs deterministic-only.

## Deploy (anywhere)
Runs as one lightweight, read-only, resource-capped agent — VM, Docker, Kubernetes, or on-prem.
```
# Docker (built from the workspace root, which holds both crates)
docker build -f vigil/Dockerfile -t vigil:latest .
docker run --rm -v "$PWD/logs:/logs:ro" vigil:latest sweep /logs --project demo --no-engine

# Compose (single host)
LOGS_DIR=./logs docker compose -f deploy/docker-compose.yml up -d

# Kubernetes (Helm)
helm install vigil deploy/helm/vigil --set logs.hostPath=/var/log/app --set autonomy=notify
```
BYO engine: the deterministic pipeline runs out of the box; mount your logged-in Claude CLI (or wire an
API key) to enable RCA. See `deploy/` for the chart and compose file.

## Dev build
Checkout `vigil` and `vigil-engine` side by side (path dependency), then:
```
cargo run -p vigil -- investigate fixtures/payments-logs --repo /tmp/payments-repo --project acme-payments
cargo test   # deterministic cross-check tests
```

> Plan: `docs/coding_plan.md`. Validated findings & learn-loop log: `docs/FINDINGS.md`.

License: Apache-2.0 (wrapper). The engine is distributed as a bundled binary.
