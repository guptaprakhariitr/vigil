---
name: setup-vigil
description: Set up VigilAI to watch this project — discover the app's services and their log sources, build/get the `vigil` binary, register them as ONE project with all sources, connect an engine (logged-in Claude/Cursor, an API key, or a local model), warm the Tier-1 policy, and start watching for cited root-cause findings. Self-hosted, read-only, no code changes, no signup.
---

# Set up VigilAI in this project

You are setting up **VigilAI** — a self-hosted, read-only on-call engineer that watches a system's
logs, finds the **root cause** of incidents (cited to the evidence), and can open a fix PR. It makes
**no changes to the user's application code**, requires **no signup or ingest token**, and sends
nothing off the box except to the engine the user chooses.

Work through these steps with the user, confirming before anything that creates processes or branches.

## 1. Understand the system
- Identify the app and **all its services/containers** — e.g. from `docker compose ps`, a
  `docker-compose.yml`, Kubernetes manifests, or how it's run. A web app with a worker, scheduler,
  broker, and DB is **one system**.
- Find **where each service writes logs**. Prefer one file per service: for Docker, stream each
  container with `docker logs -f <name> >> ./logs/<name>.log 2>&1 &`; for files, note their paths.
- Note the app's **source repo** (for grounding stack traces and validating fixes) if available.

## 2. Get `vigil`
- If the repo is checked out: `cargo build --release` (needs `vigil` + `vigil-engine` side by side);
  binary at `target/release/vigil`. Otherwise build the container per `docs/REFERENCE.md` (Deploy).
- Sanity check: `vigil --version`.

## 3. Register the system as ONE project (many sources)
A **project = one logical system; its containers are sources of that one project** — never one
project per container.
```
vigil project add <app> ./logs/<first>.log --repo <repo-dir> --autonomy notify
vigil project add-source <app> ./logs/<each-other-service>.log   # repeat per service
vigil project list                                               # confirm sources
```

## 4. Connect an engine (BYO — pick with the user)
The deterministic pipeline runs with `--no-engine`; root-cause analysis needs an engine:
- `--engine claude-cli` — the user's logged-in Claude Code on the box (no key). Default if present.
- `--engine cursor-cli` — logged-in Cursor CLI.
- `--engine anthropic-api` — set `ANTHROPIC_API_KEY` (off-box reasoning; good for prod).
- `--engine local` — an on-box Ollama model (no egress).
Keep **`--autonomy notify`** to start (it never holds deploy creds; PRs only at `propose`+).

## 5. Warm the Tier-1 policy (one engine call)
Draft the deterministic mute/watch/escalate rules from the real logs, then review them:
```
vigil warm ./logs/<service>.log --project <app> --context "<frameworks, what's normal>"
vigil policy --project <app>
```

## 6. Start watching & verify
```
vigil run                       # watch the whole portfolio (all sources, on the fly)
vigil status / vigil incidents  # health + grouped incidents
vigil ask "what broke and is it the root cause?" --project <app>
```
Optionally a live view: `vigil tui` (terminal) or `vigil serve` (web). When a real error occurs you
should see a **cited** root cause; raise `--autonomy propose` only when the user wants PRs.

## Guardrails (do not cross)
- **Read-only**: read logs and (read-only) the repo. Never modify app code, never query its prod DB.
- **No instrumentation, no signup, no token.** Don't add OpenTelemetry/SDKs or edit the app.
- **No egress** beyond the chosen engine. Don't enable telemetry unless the user asks.
- Confirm before starting long-running processes or raising autonomy above `notify`.

See `README.md` and `docs/REFERENCE.md` for every command and option.
