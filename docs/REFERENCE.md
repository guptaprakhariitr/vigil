# VigilAI reference

Complete reference for the `vigil` CLI: every command and flag, every config/env option with its
meaning, deploy guides, and a glossary. For the concepts behind it, see
[HOW-IT-WORKS.md](HOW-IT-WORKS.md).

Common flags: `--db <file>` (the SQLite state store, default `vigil.db`) and `--project <name>`
(default `default`) appear on most commands and are omitted from each table below for brevity.

---

## Commands

### `vigil investigate <path>` — one-shot root-cause
Ingest a log file/dir, correlate, run one engine hop, print a cited RCA report. No daemon, no state.

| Flag | Meaning |
|---|---|
| `--repo <dir>` | source repo to ground the analysis (stack→file, recent diff). Read-only. |
| `--engine <e>` | `claude-cli` (default) · `cursor-cli` · `anthropic-api` · `local` · `none`. |
| `--no-engine` | skip the LLM; deterministic report only. |
| `--out <file>` | write the report to a file instead of stdout. |
| `--show-bundle` | also print the evidence bundle sent to the engine (for debugging/trust). |

### `vigil up [path]` — always-on daemon (one project)
Watch a project's sources, detect & group incidents, investigate novel ones, act per autonomy. Omit
`[path]` to use the registered project's saved sources; pass a `[path]` for an ad-hoc single source.

| Flag | Meaning |
|---|---|
| `--repo <dir>` | source repo for grounding/validation. |
| `--engine <e>` / `--no-engine` | engine selection (as above). |
| `--interval <secs>` | poll interval between passes (default 5). |
| `--once` | run a single pass and exit (testing/cron). |
| `--max-iterations <n>` | stop after N passes (0 = forever). |
| `--autonomy <level>` | `notify` (default) · `report` · `propose` · `merge` · `release` (see below). |
| `--min-confidence <0..1>` | minimum finding confidence before a code action is allowed (default 0.7). |

### `vigil run` — portfolio scheduler (all projects)
Watch **every registered project** round-robin under one global budget. Picks up newly-added projects
on the fly (it re-reads the registry each round); skips paused projects.

| Flag | Meaning |
|---|---|
| `--interval <secs>` | round interval (default 15). |
| `--once` / `--max-iterations <n>` | single round / stop after N rounds. |
| `--token-budget <n>` | global engine-token budget for the run (0 = unlimited). When exhausted, detection continues but no new engine calls are spent. |
| `--max-rss-mb <n>` | resource budget: while VigilAI's own RSS exceeds this, it runs detection-only (sheds its own load). 0 = off. |
| `--no-engine` | deterministic-only across the portfolio. |

### `vigil sweep [path]` — batch investigate
Investigate **every** open escalate-routed incident in one pass (not just the dominant one). Idempotent
(skips resolved/already-investigated). Same engine/autonomy flags as `up`.

### `vigil project …` — the registry
| Subcommand | Meaning |
|---|---|
| `add <name> <path>` | register a project (its first source). Flags: `--repo`, `--engine`, `--autonomy`, `--min-confidence`. |
| `add-source <name> <path>` | attach another source (container/service) to an existing project. **A project = one system, many sources.** |
| `list` | show all projects: autonomy, threshold, source count, open incidents, top signature. |

### Tier-1 policy & learning
| Command | Meaning |
|---|---|
| `vigil warm <path>` | one engine call drafts the mute/watch/escalate policy from observed templates. `--context "<text>"` gives the author system hints (frameworks, deploy cadence). |
| `vigil policy` | print the current policy with each rule's provenance (`warm-setup`/`feedback`/`calibration`/`manual`). |
| `vigil route <mute\|watch\|escalate> <template-id-prefix>` | manually set a template's route (feedback). |
| `vigil feedback <accept\|reject> <incident-id-prefix>` | judge a finding. `accept` → resolve + reinforce escalate (recurrences then suppressed). `reject` → demote one rung; `--noise` mutes outright. `--reason "<text>"` is recorded. |
| `vigil calibrate` | engine proposes policy deltas from labeled outcomes, applied only if escalate-recall ≥ `--gate` (default 0.98). Dry-run unless `--apply`. |

### Inspect
| Command | Meaning |
|---|---|
| `vigil incidents` | list grouped incidents (severity, status, count, blast, RCA ✓, signature). |
| `vigil status` | health: events, open incidents, on-disk footprint, self-metered RSS, paused state. |
| `vigil usage` | token ledger — engine calls + estimated tokens for the project. |
| `vigil audit [--limit n]` | append-only trail of every decision (investigate/validate/act/feedback/calibrate). |
| `vigil ask "<question>"` | natural-language answer over the project's stored incidents/findings, cited; refuses to speculate. |
| `vigil metrics` | host metrics: current load average + memory + recent peaks + a resource-**pressure** verdict (OOM/saturation). Sampled read-only from `/proc` (Linux/container); the daemon records them so the engine can correlate "load/OOM caused the crash." |
| `vigil tui` | live terminal dashboard (refreshes in place). `--interval`, `--once`. |
| `vigil serve` | read-only web dashboard on localhost. `--port` (default 8787). `GET /` HTML, `GET /api/incidents` JSON. |

### `vigil validate <patch>` — patch gate (standalone)
Check a unified-diff patch in an isolated worktree. `--repo <dir>` (required), `--sha <deployed-sha>`
(defaults to HEAD), `--test "<cmd>"` (run in the worktree). Exit 0 = applies + clean + tests pass.

### Operations
| Command | Meaning |
|---|---|
| `vigil pause [project\|*]` / `vigil resume [project\|*]` | pause/resume the scheduler for a project (or all). |
| `vigil telemetry [status\|on\|off\|never]` | opt-in anonymous telemetry consent. **Off by default; nothing is sent unless consent is `on` AND `VIGIL_TELEMETRY_ENDPOINT` is set.** |
| `vigil self-update [--apply]` | check for a newer release (`--repo owner/name`); `--apply` to download + replace. |

---

## Autonomy levels

The dial bounds what VigilAI may do with a *validated* finding. Higher rungs subsume lower ones; a code
action always also requires confidence ≥ `--min-confidence`. **Credential ceiling = a scoped git token;
it never holds deploy creds.**

| Level | Behavior |
|---|---|
| `notify` | tell a human only (default, safest). |
| `report` | write a cited RCA report; no code action. |
| `propose` | open a PR with the validated fix for review. |
| `merge` | open the PR and mark it auto-merge — **your CI is the real gate**. |
| `release` | hand off to **your CD**; VigilAI never deploys directly (stops at the PR/merge). |

## Severity & routing

- **Severity** (deterministic): `SEV2` if blast ≥ 3 or count ≥ 50; `SEV3` if count ≥ 10; else `SEV4`.
- **Route** (Tier-1): `mute` (known noise, never alert) · `watch` (track, alert on change) · `escalate`
  (send to the engine). Unknown signature → `escalate` (safe default).

## Configuration & environment

| Variable | Used by | Meaning |
|---|---|---|
| `ANTHROPIC_API_KEY` | `--engine anthropic-api` | API key (required for that engine). |
| `VIGIL_MODEL` | `anthropic-api` | model id (default `claude-sonnet-4-6`). |
| `VIGIL_CURSOR_BIN` | `--engine cursor-cli` | Cursor CLI binary name (default `cursor-agent`). |
| `OLLAMA_HOST` | `--engine local` | Ollama base URL (default `http://localhost:11434`). |
| `VIGIL_LOCAL_MODEL` | `--engine local` | local model name (default `llama3.1`). |
| `VIGIL_TELEMETRY_ENDPOINT` | telemetry | destination URL; **unset = no egress is ever possible**. |
| `GH_TOKEN` | `self-update`, PR creation | scoped git/GitHub token. Never logged or written anywhere. |

Per-project config (engine, autonomy, threshold, repo, sources) lives in the store and is set via
`vigil project add` / `add-source`. State is a single SQLite file (`--db`), safe to back up or delete.

---

## Deploy

VigilAI runs as one lightweight, read-only, resource-capped agent — VM, Docker, Kubernetes, or on-prem.
The deterministic pipeline runs out of the box; mount your engine to enable RCA.

### Docker
```bash
# build from the workspace root (it holds both crates)
docker build -f vigil/Dockerfile -t vigil:latest .
docker run --rm -v "$PWD/logs:/logs:ro" vigil:latest sweep /logs --project demo --no-engine
```
The runtime image carries `git` (needed for worktree validation) and runs non-root. To enable RCA,
provide an engine — e.g. mount your logged-in Claude (`-v "$HOME/.claude:/home/vigil/.claude:ro"`) or
pass `ANTHROPIC_API_KEY`.

### Docker Compose (single host)
```bash
LOGS_DIR=./logs REPO_DIR=./repo docker compose -f deploy/docker-compose.yml up -d
```
Read-only root fs, `no-new-privileges`, state on a named volume. See `deploy/docker-compose.yml`.

### Kubernetes (Helm)
```bash
helm install vigil deploy/helm/vigil \
  --set logs.hostPath=/var/log/app \
  --set autonomy=notify \
  --set engine.mode=none           # or wire an engine
```
One Deployment + PVC, non-root, read-only rootfs, dropped capabilities, resource limits (do-no-harm).
Tune `values.yaml`: `project`, `logs.existingClaim|hostPath`, `engine.mode`, `autonomy`,
`minConfidence`, `resources`, `persistence`.

---

## Glossary

| Term | Meaning |
|---|---|
| **Event** | one normalized log line (multi-line tracebacks coalesced), with level, service, body. |
| **Template** | a log line with its variable parts masked; the unit of grouping. |
| **Fingerprint** | the stable hash of a template — the dedup/recurrence key. |
| **Cluster** | all events sharing a template, with a count and an exemplar. |
| **Incident** | a correlated group with a dominant signature, severity, status, and blast radius. |
| **Signature** | the human-readable template of an incident's dominant error. |
| **Blast radius** | how wide it reaches — distinct services and tenants affected. |
| **Severity** | SEV2/3/4 from how loud (count) and how wide (blast) it is. |
| **Route** | the Tier-1 decision: mute / watch / escalate. |
| **Policy** | the per-project table of routes — the deterministic hot path. |
| **Finding** | the engine's result for an incident: cause, confidence, citations, optional patch. |
| **Confidence** | the engine's 0–1 self-rating; gates code actions. |
| **Citation** | a reference from the finding back to the exact evidence (cluster/frame/diff) it used. |
| **Evidence bundle** | the compact JSON of clusters + stack→source + recent change sent to the engine. |
| **Source** | one log stream (a container/service). A project has many. |
| **Project** | one logical system (compose/repo) — the isolation boundary. |
| **Cursor** | the persisted per-source read offset, so a restart doesn't re-ingest. |
| **Validation** | applying a patch in an isolated worktree off the deployed SHA + secret-scan + tests. |
| **Calibration** | an eval-gated engine sweep that proposes policy deltas without weakening recall. |
| **Verified-recurring** | once accepted, recurrences of an incident are suppressed (no new engine call). |
| **Token ledger** | per-project record of engine calls and estimated tokens (`vigil usage`). |
| **Audit** | append-only log of every decision VigilAI took. |
