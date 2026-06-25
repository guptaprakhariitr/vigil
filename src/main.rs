//! `vigil` — the open wrapper CLI. Phase-1: agentless `investigate`.
//! Ingest → template-mine → correlate → evidence bundle → engine → cited RCA.

use anyhow::Result;
use clap::{Parser, Subcommand};
use std::path::PathBuf;
use vigil_engine::{bundle, correlate, engine, ingest, report};
use vigil_engine::engine::EngineAdapter;

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
        /// Log file or directory to read (read-only).
        path: PathBuf,
        /// Repo to map stack frames → source and detect the recent change.
        #[arg(long)]
        repo: Option<PathBuf>,
        /// Project name (label).
        #[arg(long, default_value = "default")]
        project: String,
        /// Engine: claude-cli | none.
        #[arg(long, default_value = "claude-cli")]
        engine: String,
        /// Skip the LLM hop (deterministic report only).
        #[arg(long)]
        no_engine: bool,
        /// Write the report here instead of stdout.
        #[arg(long)]
        out: Option<PathBuf>,
        /// Also print the evidence bundle JSON (what would be sent to the engine).
        #[arg(long)]
        show_bundle: bool,
    },
}

fn main() -> Result<()> {
    match Cli::parse().cmd {
        Cmd::Investigate { path, repo, project, engine, no_engine, out, show_bundle } => {
            let repo_ref = repo.as_deref();

            // 1) ingest  2) template-mine (in ingest)  3) correlate
            let events = ingest::ingest_path(&path, &project)?;
            eprintln!("· ingested {} events from {}", events.len(), path.display());
            let incident = correlate::correlate(&events, repo_ref);
            eprintln!(
                "· {} templates · {} errors · top: {}",
                incident.clusters.len(),
                incident.error_count,
                incident.top.as_ref().map(|t| t.template.as_str()).unwrap_or("(none)")
            );

            // 4) evidence bundle
            let repo_str = repo.as_ref().map(|p| p.display().to_string());
            let seed = bundle::build(&incident, &project, repo_str.as_deref());
            if show_bundle {
                eprintln!("--- evidence bundle ---\n{}", serde_json::to_string_pretty(&seed)?);
            }

            // 5) engine (one hop) — or deterministic-only
            let adapter: Box<dyn EngineAdapter> = if no_engine || engine == "none" {
                Box::new(engine::NullEngine)
            } else {
                Box::new(engine::ClaudeCli::default())
            };
            eprintln!("· engine: {}", adapter.name());
            let finding = match adapter.investigate(&seed) {
                Ok(f) => f,
                Err(e) => {
                    eprintln!("! engine failed ({e}); falling back to deterministic report");
                    engine::Finding { abstain: true, reason: Some(format!("engine error: {e}")), ..Default::default() }
                }
            };

            // 6) report
            let md = report::render(&incident, &finding, &project, adapter.name());
            match out {
                Some(p) => {
                    std::fs::write(&p, &md)?;
                    eprintln!("· report written → {}", p.display());
                }
                None => println!("{}", md),
            }
        }
    }
    Ok(())
}
