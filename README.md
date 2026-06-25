# VigilAI — `vigil`

The on-call engineer that runs on your **own** box. Point it at your logs; it finds the root cause,
cites the evidence, and (if you let it) opens a fix PR. Your infra, your engine (Claude / Cursor /
API / local), your autonomy dial.

## Setup

**Build** (checkout `vigil` and `vigil-engine` side by side):
```bash
cargo build --release        # binary at target/release/vigil
```
**Or run anywhere** with Docker / Compose / Helm — see [docs/REFERENCE.md](docs/REFERENCE.md#deploy).

## Try it in 60 seconds

```bash
# 1) one-shot root-cause on a log directory, grounded in your repo
vigil investigate ./logs --repo .

# 2) or watch a system continuously (a project = one app, all its containers)
vigil project add myapp ./logs/web.log --repo .
vigil project add-source myapp ./logs/worker.log
vigil up --project myapp
```
No LLM configured? Add `--no-engine` — the deterministic pipeline (detect, group, triage) still runs.
To enable root-cause analysis, BYO engine: `--engine claude-cli` (your logged-in Claude) or
`--engine anthropic-api` (set `ANTHROPIC_API_KEY`).

## What you get

```
🔔 SEV2 myapp · NEW incident ×216 · blast 1 — ERROR payments cannot read 'id' of undefined …
   ↳ cause: a refactor (ce92608) made charge() read session.customer.id; on Stripe timeout
            `session` is undefined → null-deref at src/charge.ts:9  (conf 0.94)
   ↳ patch: applies ✓ · tests skipped · secret-scan clean  →  branch vigil/fix-…
```
- A **cited** root cause (every claim tied to a log cluster, stack frame, or diff).
- Noise filtered: thousands of lines and hundreds of recurrences → **one incident, a couple of engine calls**.
- A **validated** fix (applied in a throwaway git worktree off your deployed SHA — never your working copy), opened as a PR only if you raised the autonomy dial.

Inspect anytime: `vigil incidents`, `vigil status`, `vigil ask "what broke and is it fixed?"`,
or a live dashboard — `vigil tui` (terminal) / `vigil serve` (web).

## Learn more

- **[docs/HOW-IT-WORKS.md](docs/HOW-IT-WORKS.md)** — the pipeline, the Tier-1 policy, the do-no-harm guarantees, and the project/sources model.
- **[docs/REFERENCE.md](docs/REFERENCE.md)** — every command and flag, every config/env option with its meaning, deploy guides, and a glossary.

License: Apache-2.0 (wrapper). The deterministic engine ships as a bundled binary.
