# How VigilAI works

VigilAI turns a stream of logs into **cited, validated root-cause findings** while spending as little
of your LLM as possible. The principle: **a deterministic core does the heavy lifting; the model is one
constrained, cited hop, only when something is genuinely novel.**

## The loop

```
ingest → template-mine → correlate → detect → triage (Tier-1) → investigate (engine) → validate → gate (autonomy) → act
```

1. **Ingest** — read a log file/dir, auto-detect format (JSON / logfmt / plain), normalize to events.
   Multi-line tracebacks are coalesced into one event; the `service` is parsed from the line content.
2. **Template-mine** — mask the variable bits (numbers, ids, ips, paths, `key=value`) so `…undefined at
   charge.ts:9 tenant=acme` and `…tenant=globex` collapse to one **template** with a stable fingerprint.
3. **Correlate** — group events into an **incident**, find the dominant error signature, map its stack
   frame back to a source file, and align it with the most recent git change (the likely trigger).
4. **Detect** — compute **severity** (how loud) and **blast radius** (how many services/tenants).
5. **Triage (Tier-1)** — a deterministic per-project policy routes the candidate: **mute / watch /
   escalate**. This runs on the hot path: sub-millisecond, **zero tokens**, fully auditable.
6. **Investigate (engine)** — only an *escalated, novel* incident gets a single LLM call. It receives a
   compact **evidence bundle** (clusters, stack→source snippet, recent diff) and must reply with a
   schema-constrained finding: cause, confidence, citations, optional patch — or **abstain**.
7. **Validate** — any proposed patch is applied in a throwaway **git worktree branched off your deployed
   SHA**, never your working copy. Secret-bearing patches are refused before apply; tests run if configured.
8. **Gate + act** — the **autonomy dial** decides what's allowed: notify → report → propose (PR) →
   merge (auto-merge, your CI is the gate) → release (your CD). A code action needs **both** a clean
   validation **and** confidence ≥ threshold, else it falls back to notify.

## The Tier-1 routing policy (why it's cheap)

The expensive thing is the LLM, so VigilAI keeps it off the hot path. Routing — *send to the engine or
not, how severe, which evidence* — is **classification**, best done as deterministic rules:

- A per-project table maps each template → **mute / watch / escalate**.
- The **engine authors** that table **once** at warm-setup (`vigil warm`), then refines it from your
  feedback (`vigil route`, `vigil feedback`) and periodic, eval-gated **calibration** (`vigil calibrate`).
- The table runs hot at 0 tokens; the model touches it occasionally, never per event.
- **Unknown signature → escalate** (safe default: bias to recall — never silently drop a real one).

This is the "LLM writes the rules, deterministic hot path runs them" design — and it's what delivers
the token economy (tens of thousands of log lines → a couple of engine calls; healthy services → zero).

## Learning & self-correction

- **Verified-recurring:** once you `accept` a finding, recurrences of that incident are suppressed —
  no repeat engine calls. Near-duplicate signatures are recognized too (lexical similarity).
- **Feedback → rule delta:** `accept` reinforces escalate; `reject` demotes (escalate→watch→mute), so
  the same noise never burns another call.
- **Calibration sweep:** the engine periodically reviews labeled outcomes and proposes policy deltas,
  applied **only if** they pass an escalate-recall gate (≥ 0.98) — it can sharpen the policy but never
  silently weaken detection.

## Do no harm

VigilAI must never degrade the system it watches:

- **Read-only data plane** — it reads logs and (read-only) your repo; it never queries your prod DB.
- **Resource-capped** — CPU/mem budgets; it sheds *its own* load (detection-only) before it ever
  affects the app, and self-meters in `vigil status`. `vigil pause` is a kill switch.
- **Patches are sandboxed** — applied only in an isolated worktree off the deployed SHA, with tests
  capped; your working tree is never touched.
- **Credential ceiling = a scoped git token.** It can open a PR; it **never** holds deploy creds and
  never deploys. Merge happens through *your* CI, release through *your* CD.

## A project = one system, with many sources

A **project** is **one logical system** — a docker-compose, a repo, an app boundary — with **many
sources** (its containers/services: web, worker, beat, broker, db). VigilAI ingests all of a project's
sources and **correlates them together**, so a shared-dependency failure (e.g. the broker dies) is *one*
incident across the affected services, and `ask` sees the whole system.

- **Add every container of a system as a source of one project** — never one project per container
  (that fragments a single root cause into N incidents and blinds correlation).
- A **separate system** (another app, a customer's stack) is a **separate project** — the isolation
  boundary. Nothing correlates, leaks, or competes for budget across projects.

```bash
vigil project add superset ./logs/web.log --repo .   # the system
vigil project add-source superset ./logs/worker.log  # + its containers
vigil project add-source superset ./logs/beat.log
vigil run                                             # watch the whole portfolio
```

## Choose your engine (BYO)

One adapter contract, four backends — pick per project, swap anytime:

| Engine | What it is |
|---|---|
| `claude-cli` | your logged-in Claude Code on the box (no API key) |
| `cursor-cli` | your logged-in Cursor CLI |
| `anthropic-api` | an API key (`ANTHROPIC_API_KEY`) — off-box reasoning, prod-friendly |
| `local` | an on-box Ollama model — no egress |
| `none` | deterministic-only: detect/group/triage, zero LLM |

See **[REFERENCE.md](REFERENCE.md)** for every command, flag, and config option.
