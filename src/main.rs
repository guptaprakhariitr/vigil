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
use vigil_engine::{bundle, correlate, detect, engine, ingest, report, store::Store};

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
            eprintln!(
                "▶ vigil up · project={} · watching {} · engine={} · db={}\n  (read-only · Ctrl-C to stop)",
                project, path.display(), adapter.name(), db
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
                        let (is_new, id) =
                            store.upsert_incident(&project, &top.template_id, &top.template, sev, blast as i64, count)?;
                        if is_new {
                            println!(
                                "🔔 {} {} · NEW incident ×{} · blast {} — {}",
                                sev, project, count, blast, top.template
                            );
                            // novel → escalate to the engine
                            let repo_str = repo.as_ref().map(|p| p.display().to_string());
                            let seed = bundle::build(&incident, &project, repo_str.as_deref());
                            match adapter.investigate(&seed) {
                                Ok(f) if !f.abstain => {
                                    let short: String = f.cause.chars().take(160).collect();
                                    println!("   ↳ cause: {} (conf {:.2})", short, f.confidence);
                                    store.record_finding(id, &f.cause, f.confidence, &f.citations.join(","))?;
                                }
                                Ok(f) => println!("   ↳ engine abstained: {}", f.reason.unwrap_or_default()),
                                Err(e) => eprintln!("   ! engine error: {e}"),
                            }
                        } else {
                            eprintln!("· +{} events · recurring incident ×{} ({})", n, count, sev);
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
    }
    Ok(())
}
