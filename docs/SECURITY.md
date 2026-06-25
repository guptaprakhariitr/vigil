# Security

VigilAI runs inside your infrastructure and can read logs, read your repo, talk to an LLM, and (if you
raise the autonomy dial) open pull requests. This page documents its threat model, the guarantees it
enforces, and the findings + fixes from an internal code review.

## Threat model & guarantees

- **Read-only data plane.** VigilAI reads log files and (read-only) your repo. It never queries your
  production database and never writes to your running system.
- **Credential ceiling = a scoped git token.** The most it ever holds is a git token to open a PR. It
  **never holds deploy credentials and never deploys** — merge goes through *your* CI, release through
  *your* CD.
- **Patches are sandboxed.** Proposed fixes are applied only in a throwaway `git worktree` branched off
  your deployed SHA, never your working copy; tests run capped.
- **The LLM is constrained.** It receives a compact evidence bundle and must return a schema-checked
  finding grounded in that bundle, or abstain. The engine's output is treated as untrusted (see below).
- **No egress you didn't choose.** The only outbound calls are to the engine you select. Telemetry is
  off by default and sends nothing unless you both consent (`vigil telemetry on`) **and** set
  `VIGIL_TELEMETRY_ENDPOINT`.
- **Local by default.** `vigil serve` binds to `127.0.0.1` only.
- **The web UI is token-protected** (Jupyter-style): a token/password gates every page and action,
  set via `--token` or auto-generated, persisted, and printed at startup. The token is carried in an
  `HttpOnly; SameSite=Strict` cookie; an unauthenticated request gets a login page, never data. The
  mutating actions (accept/reject a finding, re-route a rule, pause) require the same token.

## Internal review — findings & fixes

| Area | Finding | Resolution |
|---|---|---|
| Web UI (`serve`) | **Stored XSS** — log-derived signatures/causes were placed in `title="…"` attributes but the HTML escaper handled only `& < >`, so a log line containing `"` could break out and inject script. | Escaper now also encodes `"`→`&quot;` and `'`→`&#39;` (text **and** attribute safe). |
| Ingest | **Unbounded file read** — `read_to_string` on a multi-GB/maliciously-large log could exhaust memory before the resource budget engaged. | Reads are capped (256 MB); an oversized file is tailed to its most-recent bytes — memory is bounded. |
| Autonomous patching | A proposed patch could touch **CI / deploy / secret** files (`.github/workflows`, `Dockerfile`, Helm, `*.tf`, `.env`, `*.pem`/`*.key`). | Validation hard-refuses patches touching sensitive paths — those changes always require a human, regardless of autonomy. |
| Web server | A slow/hung client could block the single-threaded loop. | 5-second read timeout per connection. |

## Verified clean in review

- **No SQL injection** — every query is parameterized (`rusqlite` `params!`); no `format!`-built SQL.
- **No shell/command injection** — `git`/`gh` are invoked via `Command` with explicit args (no shell).
  The only `sh -c` is the **operator-supplied** `--test` command run inside the validation worktree; the
  engine's suggested `test` string is **never executed**.
- **No path traversal on apply** — `git apply` is used without `--unsafe-paths`, so it refuses paths
  that escape the repo.
- **No secret leakage** — `GH_TOKEN` / `ANTHROPIC_API_KEY` are read from the environment and used only as
  request auth; they are never logged, printed, or written to disk. `vigil usage` reports token *counts*,
  not secrets.
- **Secret-introduction guard** — patches that *add* a token/key (e.g. `ghp_…`, `AKIA…`, `BEGIN … PRIVATE KEY`)
  are refused before apply.

## Residual / by-design risks

- **Prompt injection → malicious patch.** A crafted log line could try to steer the engine toward a
  harmful patch. Mitigations: schema-constrained + grounded output, worktree isolation, secret +
  sensitive-path refusal, and — crucially — a patch is only ever a **PR for human review** until you
  choose `merge`/`release`, where **your CI** is the gate. Keep autonomy at `notify`/`propose` for
  untrusted log sources.
- **Operator-supplied test command** executes a shell in the worktree by design; only configure it from
  trusted input.

## Reporting

Found something? Open a private security advisory on the repo (or email the maintainer). Please don't
file public issues for vulnerabilities.
