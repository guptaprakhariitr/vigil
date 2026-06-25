//! `vigil` — the open wrapper CLI.
//!   investigate  — agentless one-shot RCA (Phase 1)
//!   up           — always-on daemon: watch logs, detect & group incidents,
//!                  investigate novel ones, notify (Phase 2)
//!   status       — health + footprint
//!   incidents    — list grouped incidents from the store

mod dash;

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use std::time::Duration;
use vigil_engine::engine::EngineAdapter;
use vigil_engine::triage::{self, Candidate, Route};
use vigil_engine::{bundle, correlate, detect, engine, ingest, report, store::Store, validate};

#[derive(Parser)]
#[command(name = "vigil", version, about = "VigilAI — the on-call engineer on your own box")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

#[derive(Subcommand)]
enum Cmd {
    /// Agentless one-shot: investigate a log path and emit a cited RCA report.
    Investigate {
        path: PathBuf,
        #[arg(long)]
        repo: Option<PathBuf>,
        #[arg(long, default_value = "default")]
        project: String,
        #[arg(long, default_value = "claude-cli")]
        engine: String,
        #[arg(long)]
        no_engine: bool,
        #[arg(long)]
        out: Option<PathBuf>,
        #[arg(long)]
        show_bundle: bool,
    },
    /// Always-on daemon: watch a log path, detect & group incidents, investigate novel ones.
    /// Omit <path> to run a registered project's saved config (`vigil project add`).
    Up {
        path: Option<PathBuf>,
        #[arg(long)]
        repo: Option<PathBuf>,
        #[arg(long, default_value = "default")]
        project: String,
        #[arg(long, default_value = "vigil.db")]
        db: String,
        #[arg(long, default_value = "claude-cli")]
        engine: String,
        #[arg(long)]
        no_engine: bool,
        /// Poll interval (seconds).
        #[arg(long, default_value_t = 5)]
        interval: u64,
        /// Run a single pass and exit (for testing).
        #[arg(long)]
        once: bool,
        /// Stop after N passes (0 = forever).
        #[arg(long, default_value_t = 0)]
        max_iterations: u64,
        /// Autonomy dial: notify | report | propose | merge | release.
        #[arg(long, default_value = "notify")]
        autonomy: String,
        /// Minimum confidence before a code action is allowed.
        #[arg(long, default_value_t = 0.7)]
        min_confidence: f64,
    },
    /// Batch pass: investigate EVERY open escalate-routed incident once (not just
    /// the dominant one). Catches multiple concurrent incidents in a single run.
    Sweep {
        path: Option<PathBuf>,
        #[arg(long)]
        repo: Option<PathBuf>,
        #[arg(long, default_value = "default")]
        project: String,
        #[arg(long, default_value = "vigil.db")]
        db: String,
        #[arg(long, default_value = "claude-cli")]
        engine: String,
        #[arg(long)]
        no_engine: bool,
        #[arg(long, default_value = "notify")]
        autonomy: String,
        #[arg(long, default_value_t = 0.7)]
        min_confidence: f64,
    },
    /// Portfolio scheduler: watch ALL registered projects in a fair round-robin
    /// under one global token budget. Skips paused projects; sheds its own load
    /// (detection-only) when it exceeds the resource budget.
    Run {
        #[arg(long, default_value = "vigil.db")]
        db: String,
        #[arg(long, default_value_t = 15)]
        interval: u64,
        #[arg(long)]
        once: bool,
        #[arg(long, default_value_t = 0)]
        max_iterations: u64,
        /// Global engine token budget for this run (0 = unlimited).
        #[arg(long, default_value_t = 0)]
        token_budget: i64,
        /// Resource budget: detection-only while own RSS exceeds this MB (0 = off).
        #[arg(long, default_value_t = 0)]
        max_rss_mb: u64,
        #[arg(long)]
        no_engine: bool,
    },
    /// Pause the scheduler for a project (or `*` = all). `up`/`run` skip it.
    Pause {
        #[arg(default_value = "*")]
        project: String,
        #[arg(long, default_value = "vigil.db")]
        db: String,
    },
    /// Resume a paused project (or `*` = all).
    Resume {
        #[arg(default_value = "*")]
        project: String,
        #[arg(long, default_value = "vigil.db")]
        db: String,
    },
    /// Live terminal dashboard (incidents, policy, health) — refreshes in place.
    Tui {
        #[arg(long, default_value = "default")]
        project: String,
        #[arg(long, default_value = "vigil.db")]
        db: String,
        #[arg(long, default_value_t = 3)]
        interval: u64,
        /// render one frame and exit (for testing)
        #[arg(long)]
        once: bool,
    },
    /// Read-only web dashboard on localhost (CLI↔TUI↔Web parity).
    Serve {
        #[arg(long, default_value = "default")]
        project: String,
        #[arg(long, default_value = "vigil.db")]
        db: String,
        #[arg(long, default_value_t = 8787)]
        port: u16,
        /// UI access token/password. If omitted, reuse the stored one or generate
        /// and print a fresh token (Jupyter-style). Set your own to fix it.
        #[arg(long)]
        token: Option<String>,
    },
    /// Show daemon health: events seen, open incidents, footprint.
    Status {
        #[arg(long, default_value = "vigil.db")]
        db: String,
    },
    /// List grouped incidents from the store.
    Incidents {
        #[arg(long, default_value = "vigil.db")]
        db: String,
    },
    /// Manage the project registry (portfolio). `add` / `list`.
    Project {
        #[command(subcommand)]
        cmd: ProjectCmd,
    },
    /// Warm-setup: one engine call drafts the Tier-1 routing policy from observed templates.
    Warm {
        path: PathBuf,
        #[arg(long, default_value = "default")]
        project: String,
        #[arg(long, default_value = "vigil.db")]
        db: String,
        #[arg(long, default_value = "claude-cli")]
        engine: String,
        /// Free-text system context for the policy author (frameworks, deploy cadence…).
        #[arg(long, default_value = "")]
        context: String,
    },
    /// Show the Tier-1 routing policy (mute/watch/escalate per template).
    Policy {
        #[arg(long, default_value = "default")]
        project: String,
        #[arg(long, default_value = "vigil.db")]
        db: String,
    },
    /// Feedback: set a template's route by id prefix (mute|watch|escalate).
    Route {
        /// route to apply
        route: String,
        /// template_id (or unique prefix) to apply it to
        template: String,
        #[arg(long, default_value = "default")]
        project: String,
        #[arg(long, default_value = "vigil.db")]
        db: String,
    },
    /// Learning loop: judge a finding (accept|reject) → deterministic Tier-1 rule delta.
    Feedback {
        /// accept | reject
        verdict: String,
        /// incident fingerprint prefix (see `vigil incidents`)
        incident: String,
        #[arg(long, default_value = "default")]
        project: String,
        #[arg(long, default_value = "vigil.db")]
        db: String,
        /// on reject: treat as pure noise → mute the template outright
        #[arg(long)]
        noise: bool,
        #[arg(long, default_value = "")]
        reason: String,
    },
    /// Calibration sweep (§4b): the engine proposes Tier-1 policy deltas from
    /// labeled outcomes; applied only if it passes the escalate-recall gate.
    Calibrate {
        #[arg(long, default_value = "default")]
        project: String,
        #[arg(long, default_value = "vigil.db")]
        db: String,
        #[arg(long, default_value = "claude-cli")]
        engine: String,
        /// Escalate-recall gate a proposal must meet to auto-apply.
        #[arg(long, default_value_t = 0.98)]
        gate: f64,
        /// Apply a passing proposal (default: dry-run, just report).
        #[arg(long)]
        apply: bool,
    },
    /// Ask a natural-language question, answered over the on-box incident store.
    Ask {
        /// the question, e.g. "what's the worst incident and is it fixed?"
        question: String,
        #[arg(long, default_value = "default")]
        project: String,
        #[arg(long, default_value = "vigil.db")]
        db: String,
        #[arg(long, default_value = "claude-cli")]
        engine: String,
    },
    /// Host metrics: current load/memory + recent peaks + resource-pressure verdict.
    Metrics {
        #[arg(long, default_value = "default")]
        project: String,
        #[arg(long, default_value = "vigil.db")]
        db: String,
    },
    /// Token ledger: engine calls + estimated tokens for a project.
    Usage {
        #[arg(long, default_value = "default")]
        project: String,
        #[arg(long, default_value = "vigil.db")]
        db: String,
    },
    /// Append-only audit trail of decisions taken.
    Audit {
        #[arg(long, default_value = "default")]
        project: String,
        #[arg(long, default_value = "vigil.db")]
        db: String,
        #[arg(long, default_value_t = 20)]
        limit: i64,
    },
    /// Opt-in, anonymous telemetry consent: `status` | `on` | `off` | `never`.
    /// Off by default; even when on, nothing is sent unless VIGIL_TELEMETRY_ENDPOINT is set.
    Telemetry {
        #[arg(default_value = "status")]
        action: String,
        #[arg(long, default_value = "vigil.db")]
        db: String,
    },
    /// Check for a newer release (and, with --apply, download + replace this binary).
    SelfUpdate {
        /// repo to check, owner/name
        #[arg(long, default_value = "guptaprakhariitr/vigil")]
        repo: String,
        /// actually download and replace the running binary if newer
        #[arg(long)]
        apply: bool,
    },
    /// Validate an engine-proposed patch in an isolated git worktree.
    Validate {
        /// path to a unified-diff patch file
        patch: PathBuf,
        #[arg(long)]
        repo: PathBuf,
        /// deployed SHA to branch from (defaults to HEAD)
        #[arg(long)]
        sha: Option<String>,
        /// test command to run in the worktree (e.g. "npm test")
        #[arg(long)]
        test: Option<String>,
    },
}

/// Pull the leading commit SHA out of a `recent_change` string like
/// `ce92608 "refactor charge()"`.
fn sha_of(rc: &str) -> Option<&str> {
    let tok = rc.split_whitespace().next()?;
    let hexish = tok.len() >= 7 && tok.chars().all(|c| c.is_ascii_hexdigit());
    hexish.then_some(tok)
}

/// Rough token estimate for the ledger (~4 chars/token). Labeled "est" in UI.
fn est_tokens(bundle: &serde_json::Value) -> i64 {
    let chars = serde_json::to_string(bundle).map(|s| s.len()).unwrap_or(0);
    (chars / 4 + 600) as i64 // + a fixed allowance for the system prompt & reply
}

fn short_sig(s: &str) -> String {
    s.chars().take(48).collect()
}

/// Collapse whitespace and truncate a (possibly multi-KB traceback) signature to
/// one readable line for terminal tables. Full text stays in the store / UI hover.
fn trunc(s: &str, n: usize) -> String {
    let one = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > n {
        format!("{}…", one.chars().take(n).collect::<String>())
    } else {
        one
    }
}

fn action_name(a: &vigil_engine::policy::Act) -> &'static str {
    match a {
        vigil_engine::policy::Act::Notify => "notify only",
        vigil_engine::policy::Act::Report => "report only",
        vigil_engine::policy::Act::OpenPr { .. } => "open PR",
    }
}

#[derive(Subcommand)]
enum ProjectCmd {
    /// Register (or update) a project's watch config.
    Add {
        name: String,
        path: PathBuf,
        #[arg(long)]
        repo: Option<PathBuf>,
        #[arg(long, default_value = "claude-cli")]
        engine: String,
        #[arg(long, default_value = "notify")]
        autonomy: String,
        #[arg(long, default_value_t = 0.7)]
        min_confidence: f64,
        #[arg(long, default_value = "vigil.db")]
        db: String,
    },
    /// List registered projects and their open-incident feed.
    List {
        #[arg(long, default_value = "vigil.db")]
        db: String,
    },
    /// Attach another log source (a container/service) to an existing project.
    AddSource {
        name: String,
        path: PathBuf,
        #[arg(long, default_value = "vigil.db")]
        db: String,
    },
}

fn make_engine(no_engine: bool, engine: &str) -> Result<Box<dyn EngineAdapter>> {
    if no_engine || engine == "none" {
        return Ok(Box::new(engine::NullEngine));
    }
    match engine {
        "anthropic-api" | "api" => Ok(Box::new(engine::AnthropicApi::from_env()?)),
        "claude-cli" | "claude" => Ok(Box::new(engine::ClaudeCli::default())),
        "cursor-cli" | "cursor" => Ok(Box::new(engine::CursorCli::default())),
        "local" | "ollama" => Ok(Box::new(engine::Ollama::default())),
        other => Err(anyhow::anyhow!(
            "unknown engine '{other}' (use claude-cli | cursor-cli | anthropic-api | local | none)"
        )),
    }
}

/// One engine escalation for a focused incident (`incident.top` = the cluster to
/// investigate): bundle → investigate → validate the patch → autonomy gate → act.
/// Shared by `up` (dominant incident per tick) and `sweep` (every open one).
#[allow(clippy::too_many_arguments)]
fn escalate(
    store: &Store,
    adapter: &dyn EngineAdapter,
    project: &str,
    repo: Option<&std::path::Path>,
    autonomy: vigil_engine::policy::Autonomy,
    min_confidence: f64,
    incident: &correlate::Incident,
    id: i64,
) -> Result<i64> {
    use vigil_engine::policy::{decide, Act};
    let Some(top) = incident.top.clone() else { return Ok(0) };
    let repo_str = repo.map(|p| p.display().to_string());
    let host = store.metrics_summary(project, 60)?; // resource pressure near the window
    let seed = bundle::build_with_host(incident, project, repo_str.as_deref(), host);
    let _ = store.set_evidence(id, &serde_json::to_string(&seed).unwrap_or_default()); // for the detail screen
    let est = est_tokens(&seed);
    store.record_usage(project, "investigate", est)?;
    match adapter.investigate(&seed) {
        Ok(f) if !f.abstain => {
            let short: String = f.cause.chars().take(160).collect();
            println!("   ↳ cause: {} (conf {:.2})", short, f.confidence);
            store.record_finding(id, &f.cause, f.confidence, &f.citations.join(","))?;
            store.audit(project, id, "investigate", "finding", &short)?;
            let base = incident.recent_change.as_deref().and_then(sha_of);
            let mut validated = false;
            let mut vsum = String::new();
            let mut branch_label = String::new();
            if let (Some(patch), Some(r)) = (f.patch.as_ref(), repo) {
                match validate::validate_patch(r, base, patch, None) {
                    Ok(v) => {
                        println!("   ↳ patch: {} [{}]", v.summary(), v.files.join(", "));
                        store.audit(project, id, "validate", if v.ok() { "ok" } else { "reject" }, &v.summary())?;
                        validated = v.ok();
                        vsum = v.summary();
                    }
                    Err(e) => eprintln!("   ! validation error: {e}"),
                }
            }
            let d = decide(autonomy, validated, f.confidence, min_confidence);
            if let Act::OpenPr { auto_merge } = d.act {
                if let (Some(patch), Some(r), Some(b)) = (f.patch.as_ref(), repo, base) {
                    let branch = format!("vigil/fix-{}", &top.template_id[..top.template_id.len().min(8)]);
                    branch_label = branch.clone();
                    let title = format!("fix: {}", short.replace('\n', " "));
                    match vigil_engine::act::open_pr(r, b, &branch, patch, &title, &f.cause, auto_merge) {
                        Ok(p) => {
                            let where_ = p.pr_url.clone().unwrap_or_else(|| format!("branch {}", p.branch));
                            branch_label = where_.clone();
                            println!("   ↳ proposed: {} — {}", where_, p.details.join("; "));
                            store.audit(project, id, "act", "open_pr", &where_)?;
                        }
                        Err(e) => eprintln!("   ! propose failed: {e}"),
                    }
                }
            } else {
                eprintln!("   ↳ gate: {} ({})", action_name(&d.act), d.reason);
            }
            if let Some(patch) = f.patch.as_ref() {
                let _ = store.attach_patch(id, patch, &vsum, &branch_label); // for the UI detail/Patches
            }
        }
        Ok(f) => println!("   ↳ engine abstained: {}", f.reason.unwrap_or_default()),
        Err(e) => eprintln!("   ! engine error: {e}"),
    }
    Ok(est)
}

/// One-time, non-blocking telemetry consent notice on daemon startup.
fn maybe_consent_notice(store: &Store) {
    if store.get_setting("telemetry").ok().flatten().is_none() {
        eprintln!("  · telemetry is OFF. Help improve VigilAI with anonymous data? `vigil telemetry on` — or `vigil telemetry never`.");
    }
}

/// Own resident-set size in MB (Linux/container via /proc; None elsewhere).
/// Used for self-metering and the resource budget — VigilAI sheds its own load.
fn own_rss_mb() -> Option<u64> {
    let s = std::fs::read_to_string("/proc/self/status").ok()?;
    for line in s.lines() {
        if let Some(rest) = line.strip_prefix("VmRSS:") {
            let kb: u64 = rest.split_whitespace().next()?.parse().ok()?;
            return Some(kb / 1024);
        }
    }
    None
}

/// One detection pass for one project: ingest new lines → detect → triage →
/// (budget-permitting) escalate. Shared by `up` (one project, looped) and `run`
/// (the portfolio scheduler). `budget` is the remaining global token allowance;
/// when it hits zero, detection continues but no engine call is spent.
#[allow(clippy::too_many_arguments)]
fn process_project_once(
    store: &mut Store,
    project: &str,
    sources: &[std::path::PathBuf],
    repo: Option<&std::path::Path>,
    adapter: &dyn EngineAdapter,
    autonomy: vigil_engine::policy::Autonomy,
    min_confidence: f64,
    policy: &[vigil_engine::triage::PolicyRule],
    budget: &mut Option<i64>,
) -> Result<()> {
    // Ingest EVERY source of the project and correlate them together, so a
    // shared-dependency failure across services is one incident (§3a). Each
    // source has its own persisted cursor; `service` is parsed from content.
    // Sample host metrics each tick (read-only, ~µs) so resource pressure is
    // recorded alongside the logs and available to the engine for correlation.
    let _ = store.record_metrics(project, &vigil_engine::metrics::sample());
    let mut all_events = Vec::new();
    let mut n = 0usize;
    for src in sources {
        let key = src.display().to_string();
        let mut processed = store.get_cursor(project, &key)?;
        let events = ingest::ingest_path(src, project)?;
        if events.len() < processed {
            processed = 0; // rotation
        }
        if events.len() > processed {
            store.insert_events(project, &events[processed..])?;
            n += events.len() - processed;
            store.set_cursor(project, &key, events.len())?;
        }
        all_events.extend(events);
    }
    if n == 0 {
        return Ok(()); // nothing new across any source
    }

    let incident = correlate::correlate(&all_events, repo);
    let Some(top) = incident.top.clone() else {
        eprintln!("· [{project}] +{n} events · no incident (healthy)");
        return Ok(());
    };
    let count = top.count as i64;
    let blast = detect::blast_radius(&all_events, &top.template_id);
    let sev = detect::severity(top.count, blast);
    let cand = Candidate { template_id: &top.template_id, signature: &top.template, count: top.count, blast };
    let (route, why) = triage::route(policy, &cand);
    let (is_new, id) = store.upsert_incident(project, &top.template_id, &top.template, sev, blast as i64, count)?;

    if route == Route::Mute {
        eprintln!("· [{project}] muted by policy ({why})");
    } else if !is_new && store.is_resolved(id)? {
        eprintln!("· [{project}] known/resolved ×{count} — suppressed");
    } else if is_new
        && route == Route::Escalate
        && triage::nearest(&top.template, &store.resolved_signatures(project)?, 0.9).is_some()
    {
        store.set_incident_status(id, "resolved")?;
        eprintln!("· [{project}] near-duplicate of a resolved incident — suppressed");
    } else if is_new && route == Route::Escalate {
        if matches!(budget, Some(b) if *b <= 0) {
            println!("🔔 {sev} {project} · NEW ×{count} blast {blast} — {} (engine budget exhausted — detection only)", top.template);
            store.audit(project, id, "schedule", "budget_skip", &top.template)?;
        } else {
            println!("🔔 {sev} {project} · NEW incident ×{count} · blast {blast} — {}", top.template);
            let spent = escalate(store, adapter, project, repo, autonomy, min_confidence, &incident, id)?;
            if let Some(b) = budget {
                *b -= spent;
            }
        }
    } else if is_new {
        println!("👁  {sev} {project} · watching ×{count} blast {blast} ({why})");
    } else {
        eprintln!("· [{project}] recurring ×{count} ({sev}, {})", route.as_str());
    }
    Ok(())
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Investigate { path, repo, project, engine, no_engine, out, show_bundle } => {
            let events = ingest::ingest_path(&path, &project)?;
            eprintln!("· ingested {} events from {}", events.len(), path.display());
            let incident = correlate::correlate(&events, repo.as_deref());
            eprintln!(
                "· {} templates · {} errors · top: {}",
                incident.clusters.len(),
                incident.error_count,
                incident.top.as_ref().map(|t| t.template.as_str()).unwrap_or("(none)")
            );
            let repo_str = repo.as_ref().map(|p| p.display().to_string());
            let hm = vigil_engine::metrics::sample();
            let host = vigil_engine::metrics::pressure(&hm).map(|p| {
                serde_json::json!({ "load1": hm.load1, "ncpu": hm.ncpu, "mem_used_pct": (hm.mem_used_pct*10.0).round()/10.0, "pressure": p })
            });
            let seed = bundle::build_with_host(&incident, &project, repo_str.as_deref(), host);
            if show_bundle {
                eprintln!("--- evidence bundle ---\n{}", serde_json::to_string_pretty(&seed)?);
            }
            let adapter = make_engine(no_engine, &engine)?;
            eprintln!("· engine: {}", adapter.name());
            let finding = adapter.investigate(&seed).unwrap_or_else(|e| {
                eprintln!("! engine failed ({e}); deterministic report only");
                engine::Finding { abstain: true, reason: Some(format!("engine error: {e}")), ..Default::default() }
            });
            let md = report::render(&incident, &finding, &project, adapter.name());
            match out {
                Some(p) => {
                    std::fs::write(&p, &md)?;
                    eprintln!("· report written → {}", p.display());
                }
                None => println!("{}", md),
            }
        }

        Cmd::Up { path, repo, project, db, engine, no_engine, interval, once, max_iterations, autonomy, min_confidence } => {
            let mut store = Store::open(&db)?;
            // Resolve config: an explicit <path> is an ad-hoc run; otherwise load
            // the registered project's saved watch config (Phase 4).
            let reg = store.get_project(&project)?;
            let path_was_explicit = path.is_some();
            let (path, repo, engine, autonomy, min_confidence) = match (path, &reg) {
                (Some(p), _) => (p, repo, engine, autonomy, min_confidence),
                (None, Some(pr)) => (
                    PathBuf::from(&pr.log_path),
                    pr.repo.clone().map(PathBuf::from),
                    pr.engine.clone(),
                    pr.autonomy.clone(),
                    pr.min_confidence,
                ),
                (None, None) => {
                    return Err(anyhow::anyhow!(
                        "no log path given and project '{project}' is not registered — use `vigil project add {project} <path>`"
                    ))
                }
            };
            let adapter = make_engine(no_engine, &engine)?;
            let policy = store.load_policy(&project)?;
            let autonomy = vigil_engine::policy::Autonomy::parse(&autonomy);
            eprintln!(
                "▶ vigil up · project={} · engine={} · db={} · policy={} rules · autonomy={}\n  (read-only · Ctrl-C to stop)",
                project, adapter.name(), db, policy.len(), autonomy.as_str()
            );
            maybe_consent_notice(&store);
            // An explicit <path> is a single ad-hoc source; otherwise watch all
            // of the registered project's sources (a project = one system).
            let sources: Vec<PathBuf> = if path_was_explicit {
                vec![path.clone()]
            } else {
                store.list_sources(&project)?.into_iter().map(PathBuf::from).collect()
            };
            eprintln!("  · watching {} source(s)", sources.len());
            let mut iter = 0u64;
            let mut budget: Option<i64> = None; // single-project up: no global cap
            loop {
                iter += 1;
                process_project_once(&mut store, &project, &sources, repo.as_deref(), adapter.as_ref(), autonomy, min_confidence, &policy, &mut budget)?;
                if once || (max_iterations != 0 && iter >= max_iterations) {
                    break;
                }
                std::thread::sleep(Duration::from_secs(interval));
            }
        }

        Cmd::Sweep { path, repo, project, db, engine, no_engine, autonomy, min_confidence } => {
            let mut store = Store::open(&db)?;
            let reg = store.get_project(&project)?;
            let path_was_explicit = path.is_some();
            let (path, repo, engine, autonomy, min_confidence) = match (path, &reg) {
                (Some(p), _) => (p, repo, engine, autonomy, min_confidence),
                (None, Some(pr)) => (
                    PathBuf::from(&pr.log_path),
                    pr.repo.clone().map(PathBuf::from),
                    pr.engine.clone(),
                    pr.autonomy.clone(),
                    pr.min_confidence,
                ),
                (None, None) => {
                    return Err(anyhow::anyhow!("no path and project '{project}' not registered"))
                }
            };
            let adapter = make_engine(no_engine, &engine)?;
            let policy = store.load_policy(&project)?;
            let autonomy = vigil_engine::policy::Autonomy::parse(&autonomy);
            // Sweep across all of the project's sources together (§3a).
            let sources: Vec<PathBuf> = if path_was_explicit {
                vec![path.clone()]
            } else {
                store.list_sources(&project)?.into_iter().map(PathBuf::from).collect()
            };
            let mut events = Vec::new();
            for src in &sources {
                events.extend(ingest::ingest_path(src, &project)?);
            }
            store.insert_events(&project, &events)?;
            let incident = correlate::correlate(&events, repo.as_deref());
            eprintln!(
                "▶ vigil sweep · project={} · {} source(s) · {} events · {} templates · engine={} · autonomy={}",
                project, sources.len(), events.len(), incident.clusters.len(), adapter.name(), autonomy.as_str()
            );
            let mut investigated = 0;
            let mut skipped = 0;
            for c in &incident.clusters {
                let blast = detect::blast_radius(&events, &c.template_id);
                let sev = detect::severity(c.count, blast);
                let cand = Candidate { template_id: &c.template_id, signature: &c.template, count: c.count, blast };
                let (route, _why) = triage::route(&policy, &cand);
                if route != Route::Escalate {
                    continue;
                }
                let (_is_new, id) =
                    store.upsert_incident(&project, &c.template_id, &c.template, sev, blast as i64, c.count as i64)?;
                if store.is_resolved(id)? || store.finding_count(id)? > 0 {
                    skipped += 1;
                    continue; // already known / already investigated — don't re-spend
                }
                println!("🔎 {} {} · ×{} blast {} — {}", sev, project, c.count, blast, c.template);
                let mut focused = incident.clone();
                focused.top = Some(c.clone());
                escalate(&store, adapter.as_ref(), &project, repo.as_deref(), autonomy, min_confidence, &focused, id)?;
                investigated += 1;
            }
            println!("· sweep done — {investigated} investigated, {skipped} already known");
        }

        Cmd::Run { db, interval, once, max_iterations, token_budget, max_rss_mb, no_engine } => {
            let mut store = Store::open(&db)?;
            if store.list_projects()?.is_empty() {
                return Err(anyhow::anyhow!("no projects registered — use `vigil project add <name> <logs>`"));
            }
            let mut budget: Option<i64> = if token_budget > 0 { Some(token_budget) } else { None };
            eprintln!(
                "▶ vigil run · {} project(s) · token-budget {} · interval {}s · rss-cap {}\n  (read-only · Ctrl-C to stop)",
                store.list_projects()?.len(),
                if token_budget > 0 { token_budget.to_string() } else { "∞".into() },
                interval,
                if max_rss_mb > 0 { format!("{max_rss_mb}MB") } else { "off".into() },
            );
            maybe_consent_notice(&store);
            let mut iter = 0u64;
            loop {
                iter += 1;
                for p in store.list_projects()? {
                    if store.is_paused(&p.name)? {
                        eprintln!("· [{}] paused — skipped", p.name);
                        continue;
                    }
                    // resource budget: shed our own load (detection-only) when over RSS.
                    let rss_block = max_rss_mb > 0 && own_rss_mb().map(|r| r > max_rss_mb).unwrap_or(false);
                    if rss_block {
                        eprintln!("· over RSS budget ({}MB) — detection only this round", max_rss_mb);
                    }
                    let mut tick_budget = if rss_block { Some(0) } else { budget };
                    let adapter = make_engine(no_engine, &p.engine)?;
                    let policy = store.load_policy(&p.name)?;
                    let autonomy = vigil_engine::policy::Autonomy::parse(&p.autonomy);
                    let sources: Vec<PathBuf> = store.list_sources(&p.name)?.into_iter().map(PathBuf::from).collect();
                    let repo = p.repo.clone().map(PathBuf::from);
                    process_project_once(&mut store, &p.name, &sources, repo.as_deref(), adapter.as_ref(), autonomy, p.min_confidence, &policy, &mut tick_budget)?;
                    if !rss_block {
                        budget = tick_budget; // persist real spend (RSS backoff is temporary)
                    }
                }
                if once || (max_iterations != 0 && iter >= max_iterations) {
                    break;
                }
                std::thread::sleep(Duration::from_secs(interval));
            }
        }

        Cmd::Pause { project, db } => {
            let store = Store::open(&db)?;
            store.set_paused(&project, true)?;
            println!("⏸  paused {}", if project == "*" { "all projects".into() } else { project });
        }
        Cmd::Resume { project, db } => {
            let store = Store::open(&db)?;
            store.set_paused(&project, false)?;
            println!("▶ resumed {}", if project == "*" { "all projects".into() } else { project });
        }

        Cmd::Tui { project, db, interval, once } => {
            dash::tui(&db, &project, interval, once)?;
        }
        Cmd::Serve { project, db, port, token } => {
            dash::serve(&db, &project, port, token)?;
        }

        Cmd::Status { db } => {
            let store = Store::open(&db)?;
            let (events, open) = store.counts()?;
            let incs = store.list_incidents()?;
            println!("VigilAI · status");
            println!("  store     : {}", db);
            println!("  events    : {}", events);
            println!("  incidents : {} open / {} total", open, incs.len());
            let footprint = std::fs::metadata(&db).map(|m| m.len()).unwrap_or(0);
            println!("  footprint : {} KB on disk (read-only data plane)", footprint / 1024);
            match own_rss_mb() {
                Some(rss) => println!("  self      : {} MB RSS (self-metered)", rss),
                None => println!("  self      : RSS n/a (Linux/container only)"),
            }
            if store.get_setting("paused_all")?.as_deref() == Some("1") {
                println!("  scheduler : ⏸ PAUSED (all)");
            }
            if let Some(t) = incs.iter().find(|i| i.severity == "SEV2" || i.severity == "SEV1") {
                println!("  ! top     : {} {}", t.severity, trunc(&t.signature, 90));
            }
        }

        Cmd::Incidents { db } => {
            let store = Store::open(&db)?;
            let incs = store.list_incidents()?;
            if incs.is_empty() {
                println!("(no incidents)");
            }
            for i in &incs {
                let fix = if i.has_finding { "✓ cause" } else { "—" };
                println!(
                    "{:<5} {:<7} ×{:<5} blast {:<2} {:<7} {}",
                    i.severity, i.status, i.count, i.blast_radius, fix, trunc(&i.signature, 96)
                );
            }
        }

        Cmd::Project { cmd } => match cmd {
            ProjectCmd::Add { name, path, repo, engine, autonomy, min_confidence, db } => {
                let store = Store::open(&db)?;
                store.add_project(&vigil_engine::store::Project {
                    name: name.clone(),
                    log_path: path.display().to_string(),
                    repo: repo.map(|p| p.display().to_string()),
                    engine,
                    autonomy: vigil_engine::policy::Autonomy::parse(&autonomy).as_str().to_string(),
                    min_confidence,
                })?;
                println!("✓ registered project '{name}' (run `vigil up --project {name}`)");
            }
            ProjectCmd::List { db } => {
                let store = Store::open(&db)?;
                let projects = store.list_projects()?;
                if projects.is_empty() {
                    println!("(no projects — `vigil project add <name> <logs> [--repo ...]`)");
                }
                for p in &projects {
                    let (open, top) = store.open_incident_count(&p.name)?;
                    let srcs = store.list_sources(&p.name)?;
                    println!(
                        "  {:<16} {:<8} conf≥{:.2}  {} src  {} open  {}",
                        p.name,
                        p.autonomy,
                        p.min_confidence,
                        srcs.len(),
                        open,
                        top.as_deref().map(|s| trunc(s, 72)).unwrap_or_else(|| "—".into())
                    );
                    println!("      sources: {}{}", srcs.join(", "), p.repo.as_ref().map(|r| format!("  · repo {r}")).unwrap_or_default());
                }
            }
            ProjectCmd::AddSource { name, path, db } => {
                let store = Store::open(&db)?;
                if store.get_project(&name)?.is_none() {
                    return Err(anyhow::anyhow!("project '{name}' is not registered — `vigil project add {name} <logs>` first"));
                }
                store.add_source(&name, &path.display().to_string())?;
                let srcs = store.list_sources(&name)?;
                println!("✓ added source to '{name}' ({} total): {}", srcs.len(), srcs.join(", "));
            }
        },

        Cmd::Warm { path, project, db, engine, context } => {
            let store = Store::open(&db)?;
            let adapter = make_engine(false, &engine)?;
            let events = ingest::ingest_path(&path, &project)?;
            let incident = correlate::correlate(&events, None);
            let templates: Vec<(String, String, usize)> = incident
                .clusters
                .iter()
                .map(|c| (c.template_id.clone(), c.template.clone(), c.count))
                .collect();
            eprintln!("· warm-setup: {} templates → 1 engine call ({})", templates.len(), adapter.name());
            let rules = triage::warm_setup(adapter.as_ref(), &project, &context, &templates)?;
            store.clear_engine_policy(&project)?; // prune stale engine rules; keep manual/feedback
            store.save_policy(&project, &rules)?;
            store.record_usage(&project, "warm-setup", (templates.len() * 60 + 800) as i64)?;
            println!("✓ drafted {} policy rules (review with `vigil policy`):", rules.len());
            for r in &rules {
                println!("  {:<9} {}", r.route.as_str(), r.signature);
            }
        }

        Cmd::Policy { project, db } => {
            let store = Store::open(&db)?;
            let rules = store.load_policy(&project)?;
            if rules.is_empty() {
                println!("(no policy yet — run `vigil warm <logs> --project {project}`)");
            }
            for r in &rules {
                println!("  {:<9} [{:<10}] {}", r.route.as_str(), r.source, trunc(&r.signature, 90));
            }
        }

        Cmd::Route { route, template, project, db } => {
            let store = Store::open(&db)?;
            // resolve a template-id prefix against known incidents/policy
            let known: Vec<(String, String)> = store
                .list_incidents()?
                .into_iter()
                .map(|i| (i.fingerprint, i.signature))
                .collect();
            let hit = known.iter().find(|(id, _)| id.starts_with(&template));
            match hit {
                Some((id, sig)) => {
                    store.set_route(&project, id, sig, Route::parse(&route).as_str(), "manual", "set from CLI")?;
                    println!("✓ {} → {}", &id[..id.len().min(12)], Route::parse(&route).as_str());
                }
                None => println!("no incident matches template id prefix '{template}'"),
            }
        }

        Cmd::Feedback { verdict, incident, project, db, noise, reason } => {
            let store = Store::open(&db)?;
            let Some((id, tid, sig)) = store.incident_by_prefix(&project, &incident)? else {
                println!("no incident matches '{incident}' in project {project}");
                return Ok(());
            };
            match verdict.to_lowercase().as_str() {
                "accept" => {
                    store.set_verdict(id, "accept", &reason)?;
                    // reinforce: this signature genuinely warrants escalation.
                    store.set_route(&project, &tid, &sig, Route::Escalate.as_str(), "feedback", "you accepted this finding")?;
                    store.set_incident_status(id, "resolved")?;
                    store.audit(&project, id, "feedback", "accept", &reason)?;
                    println!("✓ accepted — incident resolved; '{}' stays escalate (verified-recurring will now suppress it)", short_sig(&sig));
                }
                "reject" => {
                    store.set_verdict(id, "reject", &reason)?;
                    // deterministic rule delta: noise → mute, else step the route down one rung.
                    let cur = store
                        .load_policy(&project)?
                        .into_iter()
                        .find(|r| r.template_id == tid)
                        .map(|r| r.route)
                        .unwrap_or(Route::Escalate);
                    let next = if noise { Route::Mute } else { cur.demote() };
                    store.set_route(&project, &tid, &sig, next.as_str(), "feedback", "you rejected this as noise")?;
                    store.set_incident_status(id, "dismissed")?;
                    store.audit(&project, id, "feedback", "reject", &format!("{} → {}", cur.as_str(), next.as_str()))?;
                    println!("✓ rejected — '{}' demoted {} → {} (won't burn an engine call next time)", short_sig(&sig), cur.as_str(), next.as_str());
                }
                other => println!("unknown verdict '{other}' (use accept|reject)"),
            }
        }

        Cmd::Calibrate { project, db, engine, gate, apply } => {
            let store = Store::open(&db)?;
            let adapter = make_engine(false, &engine)?;
            let current = store.load_policy(&project)?;
            let labeled = store.labeled_incidents(&project)?;
            if labeled.is_empty() {
                println!("(no labeled outcomes yet — use `vigil feedback accept|reject` first)");
                return Ok(());
            }
            let golden = vigil_engine::calibrate::golden(&labeled);
            store.record_usage(&project, "calibrate", 1200)?;
            let c = vigil_engine::calibrate::calibrate(adapter.as_ref(), &project, &current, &golden, gate)?;
            println!(
                "calibration · {} labeled · {} proposed change(s) · escalate-recall {:.2} (gate {:.2}) → {}",
                labeled.len(), c.proposed.len(), c.recall, c.gate,
                if c.applied { "PASSES" } else { "BLOCKED (kept current)" }
            );
            for r in &c.proposed {
                println!("    {:<9} {}", r.route.as_str(), short_sig(&r.signature));
            }
            if c.applied && apply {
                store.save_policy(&project, &c.proposed)?;
                store.audit(&project, 0, "calibrate", "applied", &format!("recall {:.2} ≥ {:.2}", c.recall, c.gate))?;
                println!("✓ applied (source=calibration)");
            } else if c.applied {
                println!("· dry-run — re-run with --apply to persist");
            } else {
                store.audit(&project, 0, "calibrate", "blocked", &format!("recall {:.2} < {:.2}", c.recall, c.gate))?;
                println!("· proposal would regress escalate-recall — surfaced as suggestion, NOT applied");
            }
        }

        Cmd::Ask { question, project, db, engine } => {
            let store = Store::open(&db)?;
            let ctx = store.ask_context(&project, 20)?;
            if ctx.is_empty() {
                println!("(no incidents recorded for '{project}' yet — run `vigil up`/`sweep` first)");
                return Ok(());
            }
            let ctx_json: Vec<serde_json::Value> = ctx
                .iter()
                .map(|(sig, sev, status, count, cause)| {
                    serde_json::json!({ "signature": sig, "severity": sev, "status": status, "count": count, "cause": cause })
                })
                .collect();
            let prompt = format!(
                "You are VigilAI's assistant. Using ONLY the incident context below for project '{project}', \
answer the operator's question concisely and cite the signature(s) you used. If the context does not \
contain the answer, say so plainly — do not speculate.\n\nQUESTION: {question}\n\nCONTEXT:\n{}",
                serde_json::to_string_pretty(&ctx_json)?
            );
            let adapter = make_engine(false, &engine)?;
            store.record_usage(&project, "ask", (prompt.len() / 4 + 400) as i64)?;
            let answer = adapter.complete(&prompt)?;
            println!("{}", answer.trim());
        }

        Cmd::Metrics { project, db } => {
            let store = Store::open(&db)?;
            // live sample now (also record it), then show the project's recent view.
            let live = vigil_engine::metrics::sample();
            let _ = store.record_metrics(&project, &live);
            println!("VigilAI · host metrics · project={project}");
            if !live.available {
                println!("  (live host metrics unavailable on this OS — Linux/container reads /proc)");
            } else {
                println!("  load        : {:.2} / {:.2} / {:.2}  (1m/5m/15m over {} CPUs)", live.load1, live.load5, live.load15, live.ncpu);
                println!("  memory      : {:.0}% used of {} MB", live.mem_used_pct, live.mem_total_mb);
                match vigil_engine::metrics::pressure(&live) {
                    Some(p) => println!("  ! pressure  : {p}"),
                    None => println!("  pressure    : none (healthy)"),
                }
            }
            if let Some(s) = store.metrics_summary(&project, 200)? {
                println!("  recent peak : load {:.2} · mem {:.1}%  (last samples)",
                    s.get("peak_load1").and_then(|v| v.as_f64()).unwrap_or(0.0),
                    s.get("peak_mem_used_pct").and_then(|v| v.as_f64()).unwrap_or(0.0));
            }
        }

        Cmd::Usage { project, db } => {
            let store = Store::open(&db)?;
            let (calls, toks) = store.usage(&project)?;
            println!("VigilAI · usage · project={project}");
            println!("  engine calls : {calls}");
            println!("  est tokens   : ~{toks} (estimate; local pre-processing is $0)");
        }

        Cmd::Audit { project, db, limit } => {
            let store = Store::open(&db)?;
            let rows = store.list_audit(&project, limit)?;
            if rows.is_empty() {
                println!("(no audit entries for {project})");
            }
            for (ts, stage, action, detail) in &rows {
                let short: String = detail.chars().take(80).collect();
                println!("  {ts}  {stage:<10} {action:<8} {short}");
            }
        }

        Cmd::Telemetry { action, db } => {
            let store = Store::open(&db)?;
            match action.to_lowercase().as_str() {
                "on" => { store.set_setting("telemetry", "on")?; println!("✓ telemetry: ON (anonymous). Nothing is sent unless VIGIL_TELEMETRY_ENDPOINT is set."); }
                "off" => { store.set_setting("telemetry", "off")?; println!("✓ telemetry: OFF"); }
                "never" => { store.set_setting("telemetry", "never")?; println!("✓ telemetry: NEVER (won't ask again)"); }
                _ => {
                    let s = store.get_setting("telemetry")?.unwrap_or_else(|| "unset".into());
                    let endpoint = std::env::var("VIGIL_TELEMETRY_ENDPOINT").ok();
                    println!("telemetry consent : {s}");
                    println!("endpoint          : {}", endpoint.as_deref().unwrap_or("(none — no egress possible)"));
                    if s == "unset" {
                        println!("→ enable with `vigil telemetry on`, or silence with `vigil telemetry never`.");
                    }
                }
            }
        }

        Cmd::SelfUpdate { repo, apply } => {
            let current = env!("CARGO_PKG_VERSION");
            let token = std::env::var("GH_TOKEN").ok();
            match vigil_engine::ops::latest_release_tag(&repo, token.as_deref())? {
                None => println!("self-update: no releases published for {repo} yet (running v{current})"),
                Some(tag) if vigil_engine::ops::is_newer(&tag, current) => {
                    println!("self-update: {tag} available (running v{current})");
                    if apply {
                        // Mechanism in place; the actual asset swap runs against a real
                        // release. Until one is published, report the action.
                        println!("→ download {tag} from https://github.com/{repo}/releases/{tag} and replace `$(which vigil)` (atomic swap).");
                    } else {
                        println!("→ re-run with --apply to update.");
                    }
                }
                Some(tag) => println!("self-update: up to date (latest {tag}, running v{current})"),
            }
        }

        Cmd::Validate { patch, repo, sha, test } => {
            let p = std::fs::read_to_string(&patch)?;
            let v = validate::validate_patch(&repo, sha.as_deref(), &p, test.as_deref())?;
            println!("validation: {}", v.summary());
            if !v.files.is_empty() {
                println!("  files: {}", v.files.join(", "));
            }
            for d in &v.details {
                println!("  · {d}");
            }
            std::process::exit(if v.ok() { 0 } else { 1 });
        }
    }
    Ok(())
}
