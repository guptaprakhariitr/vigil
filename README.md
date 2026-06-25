# VigilAI — `vigil`

The on-call engineer that runs on your **own** box. Point it at your logs; it finds the root cause,
cites the evidence, and (if you let it) opens a fix PR. Your infra, your engine (Claude / Cursor /
API / local), your autonomy dial.

## Get started with your coding agent

The fastest way to set this up is to let **Claude Code** (or **Cursor**) do it. Paste this prompt:

```
Run npx skills add guptaprakhariitr/vigil --all and use the setup-vigil skill to set up VigilAI in this project
```

<details>
<summary><b>What will the agent do?</b></summary>

> *via guptaprakhariitr/vigil — the `setup-vigil` skill*
>
> 1. **Map your system** — find the app's services/containers and where each writes logs
> 2. **Build/get `vigil`** and point it at those logs (read-only)
> 3. **Register them as ONE project** with all sources (a system, not one-project-per-container)
> 4. **Connect your engine** — your logged-in Claude/Cursor, an API key, or a local model
> 5. **Warm the Tier-1 policy** (one call drafts mute/watch/escalate) and start watching
> 6. **Verify** — on a real error you get a **cited** root cause; raise autonomy to open a fix PR
>
> **No application code changes. No OpenTelemetry/SDKs to add. No signup, no ingest token.** It's
> self-hosted and read-only — nothing leaves your box except to the engine you choose.

</details>

Prefer to do it by hand? Continue below. (Onboarding pattern inspired by
[clicky](https://github.com/farzaa/clicky) and [superlog](https://superlog.sh) — but VigilAI needs
**zero code changes** and **no account**: it reads the logs you already have.)

## Install

**Download the binary** for your platform from [Releases](https://github.com/guptaprakhariitr/vigil/releases)
(the deterministic engine is bundled in — nothing else to fetch):
```bash
# macOS (Apple silicon) example — see Releases for your platform/asset name
curl -fsSL -o vigil https://github.com/guptaprakhariitr/vigil/releases/latest/download/vigil-aarch64-apple-darwin
chmod +x vigil

# put it on your PATH so `vigil` works from anywhere
sudo mv vigil /usr/local/bin/vigil      # or: mkdir -p ~/.local/bin && mv vigil ~/.local/bin/
vigil --version
```
Linux x86_64: swap the asset for `vigil-x86_64-unknown-linux-gnu`. If `~/.local/bin` isn't already on
your PATH, add `export PATH="$HOME/.local/bin:$PATH"` to your shell profile (`~/.zshrc` / `~/.bashrc`).
**Or run anywhere** with Docker / Compose / Helm — see [docs/REFERENCE.md](docs/REFERENCE.md#deploy).

The macOS binary is **Developer ID–signed and notarized by Apple** (Team `86F7TVY8RD`, hardened
runtime). Linux x86_64 binary + the container image are also published. (If a download is ever
quarantined, `xattr -dr com.apple.quarantine ./vigil` clears it.)

> Building from source needs the private `vigil-engine` crate beside this repo, so it's for
> maintainers; everyone else uses the released binary or container above.

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
