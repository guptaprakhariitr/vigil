//! `vigil` — the open wrapper CLI.
//!   investigate  — agentless one-shot RCA (Phase 1)
//!   up           — always-on daemon: watch logs, detect & group incidents,
//!                  investigate novel ones, notify (Phase 2)
//!   status       — health + footprint
//!   incidents    — list grouped incidents from the store

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
    Up {
        path: PathBuf,
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

fn make_engine(no_engine: bool, engine: &str) -> Box<dyn EngineAdapter> {
    if no_engine || engine == "none" {
        Box::new(engine::NullEngine)
    } else {
        Box::new(engine::ClaudeCli::default())
    }
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
            let seed = bundle::build(&incident, &project, repo_str.as_deref());
            if show_bundle {
                eprintln!("--- evidence bundle ---\n{}", serde_json::to_string_pretty(&seed)?);
            }
            let adapter = make_engine(no_engine, &engine);
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

        Cmd::Up { path, repo, project, db, engine, no_engine, interval, once, max_iterations } => {
            let mut store = Store::open(&db)?;
            let adapter = make_engine(no_engine, &engine);
            let policy = store.load_policy(&project)?;
            eprintln!(
                "▶ vigil up · project={} · watching {} · engine={} · db={} · policy={} rules\n  (read-only · Ctrl-C to stop)",
                project, path.display(), adapter.name(), db, policy.len()
            );
            let mut processed = 0usize;
            let mut iter = 0u64;
            loop {
                iter += 1;
                let events = ingest::ingest_path(&path, &project)?;
                if events.len() > processed {
                    let fresh = &events[processed..];
                    store.insert_events(&project, fresh)?;
                    let n = fresh.len();
                    processed = events.len();

                    // detect: dominant error signature over everything seen
                    let incident = correlate::correlate(&events, repo.as_deref());
                    if let Some(top) = incident.top.clone() {
                        let count = top.count as i64;
                        let blast = detect::blast_radius(&events, &top.template_id);
                        let sev = detect::severity(top.count, blast);
                        // Tier-1: decide route before spending anything (sub-ms, 0 tokens).
                        let cand = Candidate { template_id: &top.template_id, signature: &top.template, count: top.count, blast };
                        let (route, why) = triage::route(&policy, &cand);
                        let (is_new, id) =
                            store.upsert_incident(&project, &top.template_id, &top.template, sev, blast as i64, count)?;
                        if route == Route::Mute {
                            eprintln!("· +{} events · muted by policy ({}) — {}", n, why, top.template);
                        } else if is_new && route == Route::Escalate {
                            println!(
                                "🔔 {} {} · NEW incident ×{} · blast {} — {}",
                                sev, project, count, blast, top.template
                            );
                            // novel + escalate → spend one engine call
                            let repo_str = repo.as_ref().map(|p| p.display().to_string());
                            let seed = bundle::build(&incident, &project, repo_str.as_deref());
                            match adapter.investigate(&seed) {
                                Ok(f) if !f.abstain => {
                                    let short: String = f.cause.chars().take(160).collect();
                                    println!("   ↳ cause: {} (conf {:.2})", short, f.confidence);
                                    store.record_finding(id, &f.cause, f.confidence, &f.citations.join(","))?;
                                    // do-no-harm: validate any proposed patch before trusting it.
                                    if let (Some(patch), Some(r)) = (f.patch.as_ref(), repo.as_ref()) {
                                        match validate::validate_patch(r, incident.recent_change.as_deref().and_then(sha_of), patch, None) {
                                            Ok(v) => println!("   ↳ patch: {} [{}]", v.summary(), v.files.join(", ")),
                                            Err(e) => eprintln!("   ! validation error: {e}"),
                                        }
                                    }
                                }
                                Ok(f) => println!("   ↳ engine abstained: {}", f.reason.unwrap_or_default()),
                                Err(e) => eprintln!("   ! engine error: {e}"),
                            }
                        } else if is_new {
                            println!("👁  {} {} · watching ×{} blast {} ({}) — {}", sev, project, count, blast, why, top.template);
                        } else {
                            eprintln!("· +{} events · recurring incident ×{} ({}, {})", n, count, sev, route.as_str());
                        }
                    } else {
                        eprintln!("· +{} events · no incident (healthy)", n);
                    }
                } else {
                    eprintln!("· no new logs");
                }

                if once || (max_iterations != 0 && iter >= max_iterations) {
                    break;
                }
                std::thread::sleep(Duration::from_secs(interval));
            }
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
            if let Some(t) = incs.iter().find(|i| i.severity == "SEV2" || i.severity == "SEV1") {
                println!("  ! top     : {} {}", t.severity, t.signature);
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
                    i.severity, i.status, i.count, i.blast_radius, fix, i.signature
                );
            }
        }

        Cmd::Warm { path, project, db, engine, context } => {
            let store = Store::open(&db)?;
            let adapter = make_engine(false, &engine);
            let events = ingest::ingest_path(&path, &project)?;
            let incident = correlate::correlate(&events, None);
            let templates: Vec<(String, String, usize)> = incident
                .clusters
                .iter()
                .map(|c| (c.template_id.clone(), c.template.clone(), c.count))
                .collect();
            eprintln!("· warm-setup: {} templates → 1 engine call ({})", templates.len(), adapter.name());
            let rules = triage::warm_setup(adapter.as_ref(), &project, &context, &templates)?;
            store.save_policy(&project, &rules)?;
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
                println!("  {:<9} [{:<10}] {}", r.route.as_str(), r.source, r.signature);
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
                    store.set_route(&project, id, sig, &Route::parse(&route).as_str(), "manual")?;
                    println!("✓ {} → {}", &id[..id.len().min(12)], Route::parse(&route).as_str());
                }
                None => println!("no incident matches template id prefix '{template}'"),
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
