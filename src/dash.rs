//! Read-only dashboards over the store (Phase 5): a refreshing terminal view
//! (`tui`) and a tiny self-contained web UI (`serve`). Both render the same
//! data — incidents, policy, health — with no extra dependencies (ANSI +
//! std::net). CLI ↔ TUI ↔ Web parity, all backed by the one SQLite store.

use anyhow::Result;
use std::io::{Read, Write};
use std::net::TcpListener;
use vigil_engine::store::Store;

fn sev_emoji(s: &str) -> &'static str {
    match s {
        "SEV1" | "SEV2" => "🔴",
        "SEV3" => "🟠",
        _ => "🟡",
    }
}

/// One plain-text dashboard frame for the terminal.
pub fn text_frame(store: &Store, project: &str) -> Result<String> {
    let (events, open) = store.counts()?;
    let incs = store.list_incidents()?;
    let policy = store.load_policy(project)?;
    let (calls, toks) = store.usage(project)?;
    let mut out = String::new();
    out.push_str("\x1b[1m VigilAI · live\x1b[0m   (Ctrl-C to exit)\n");
    out.push_str(&format!(
        " events {events} · {open} open incident(s) · engine calls {calls} (~{toks} tok) · policy {} rules\n",
        policy.len()
    ));
    out.push_str(" ───────────────────────────────────────────────────────────────\n");
    if incs.is_empty() {
        out.push_str("   (no incidents — healthy)\n");
    }
    for i in incs.iter().take(12) {
        let fix = if i.has_finding { "✓ cause" } else { "·" };
        out.push_str(&format!(
            " {} {:<4} {:<9} ×{:<4} blast {:<2} {:<7} {}\n",
            sev_emoji(&i.severity), i.severity, i.status, i.count, i.blast_radius, fix,
            i.signature.chars().take(60).collect::<String>()
        ));
    }
    out.push_str(" ───────────────────────────────────────────────────────────────\n");
    for (ts, stage, action, detail) in store.list_audit(project, 5)?.iter() {
        out.push_str(&format!("   {ts}  {stage:<10} {action:<8} {}\n", detail.chars().take(48).collect::<String>()));
    }
    Ok(out)
}

/// Render the self-contained HTML dashboard (server-side, meta-refresh).
pub fn html_page(store: &Store, project: &str) -> Result<String> {
    let (events, open) = store.counts()?;
    let incs = store.list_incidents()?;
    let policy = store.load_policy(project)?;
    let (calls, toks) = store.usage(project)?;
    let esc = |s: &str| s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");

    let mut rows = String::new();
    for i in &incs {
        let fix = if i.has_finding { "✓ cause" } else { "—" };
        let sc = match i.severity.as_str() { "SEV1" | "SEV2" => "s2", "SEV3" => "s3", _ => "s4" };
        rows.push_str(&format!(
            "<tr><td><span class=sev {sc}>{}</span></td><td>{}</td><td class=n>{}</td><td class=n>{}</td><td>{}</td><td class=sig>{}</td></tr>",
            i.severity, i.status, i.count, i.blast_radius, fix, esc(&i.signature)
        ));
    }
    if incs.is_empty() {
        rows.push_str("<tr><td colspan=6 class=empty>No incidents — healthy.</td></tr>");
    }
    let mut prows = String::new();
    for r in &policy {
        prows.push_str(&format!(
            "<tr><td><span class=route r-{0}>{0}</span></td><td>{1}</td><td class=sig>{2}</td></tr>",
            r.route.as_str(), esc(&r.source), esc(&r.signature)
        ));
    }
    if policy.is_empty() {
        prows.push_str("<tr><td colspan=3 class=empty>No policy yet — run <code>vigil warm</code>.</td></tr>");
    }

    Ok(format!(r#"<!doctype html><html><head><meta charset=utf-8>
<meta name=viewport content="width=device-width,initial-scale=1"><meta http-equiv=refresh content=5>
<title>VigilAI · {project}</title><style>
:root{{--bg:#0d1117;--pan:#141b24;--pan2:#1b2430;--line:#27313f;--tx:#dce3ec;--dim:#8a98a8;--ac:#2dd4bf}}
*{{box-sizing:border-box}}body{{margin:0;background:var(--bg);color:var(--tx);font:14px/1.5 ui-sans-serif,system-ui,-apple-system,Segoe UI,Roboto}}
.wrap{{max-width:980px;margin:0 auto;padding:32px 20px}}
h1{{font-size:22px;margin:0 0 4px}}h1 span{{color:var(--ac)}}.sub{{color:var(--dim);font-size:13px;margin:0 0 22px}}
.stats{{display:flex;gap:12px;flex-wrap:wrap;margin-bottom:24px}}
.stat{{background:var(--pan);border:1px solid var(--line);border-radius:10px;padding:12px 16px;flex:1 1 120px}}
.stat b{{display:block;font-size:24px;color:var(--ac);font-family:ui-monospace,monospace}}
.stat span{{font-size:11px;color:var(--dim);text-transform:uppercase;letter-spacing:.05em}}
h2{{font-size:12px;letter-spacing:.12em;text-transform:uppercase;color:var(--dim);margin:26px 0 10px;font-family:ui-monospace,monospace}}
table{{width:100%;border-collapse:collapse;background:var(--pan);border:1px solid var(--line);border-radius:10px;overflow:hidden}}
th,td{{text-align:left;padding:9px 13px;border-bottom:1px solid var(--line);font-size:13px;vertical-align:top}}
th{{background:var(--pan2);font-size:10px;letter-spacing:.08em;text-transform:uppercase;color:var(--dim)}}
tr:last-child td{{border:none}}td.n{{font-family:ui-monospace,monospace}}td.sig{{color:var(--dim);font-family:ui-monospace,monospace;font-size:12px}}
.sev{{font-family:ui-monospace,monospace;font-size:11px;padding:2px 7px;border-radius:6px}}.s2{{color:#ff8a8a;background:#2a1416}}.s3{{color:#f0b429;background:#2c2310}}.s4{{color:#9aa6b4;background:#1c232d}}
.route{{font-family:ui-monospace,monospace;font-size:11px;padding:2px 7px;border-radius:6px}}.r-escalate{{color:#ff8a8a;background:#2a1416}}.r-watch{{color:#f0b429;background:#2c2310}}.r-mute{{color:#8a98a8;background:#1c232d}}
.empty{{color:var(--dim);text-align:center;padding:18px}}code{{font-family:ui-monospace,monospace;background:var(--pan2);padding:1px 5px;border-radius:4px}}
footer{{margin-top:28px;color:#5d6b7c;font-size:11px;font-family:ui-monospace,monospace}}
</style></head><body><div class=wrap>
<h1><span>VigilAI</span> · {project}</h1><p class=sub>read-only live dashboard · auto-refresh 5s</p>
<div class=stats>
<div class=stat><b>{events}</b><span>events</span></div>
<div class=stat><b>{open}</b><span>open incidents</span></div>
<div class=stat><b>{calls}</b><span>engine calls</span></div>
<div class=stat><b>~{toks}</b><span>est tokens</span></div>
<div class=stat><b>{}</b><span>policy rules</span></div>
</div>
<h2>Incidents</h2><table><tr><th>Sev</th><th>Status</th><th>Count</th><th>Blast</th><th>RCA</th><th>Signature</th></tr>{rows}</table>
<h2>Tier-1 policy</h2><table><tr><th>Route</th><th>Source</th><th>Signature</th></tr>{prows}</table>
<footer>vigil serve · {} incidents · CLI ↔ TUI ↔ Web parity, one store</footer>
</div></body></html>"#,
        policy.len(), incs.len()
    ))
}

/// Live terminal dashboard — clears + redraws each tick. `once` renders one frame.
pub fn tui(db: &str, project: &str, interval: u64, once: bool) -> Result<()> {
    loop {
        let store = Store::open(db)?;
        let frame = text_frame(&store, project)?;
        print!("\x1b[2J\x1b[H{frame}"); // clear + home + frame
        std::io::stdout().flush().ok();
        if once {
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(interval));
    }
    Ok(())
}

/// Minimal read-only web UI. `GET /` → HTML dashboard; `GET /api/incidents` → JSON.
pub fn serve(db: &str, project: &str, port: u16) -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    eprintln!("▶ vigil serve · http://127.0.0.1:{port}  (project={project}, read-only · Ctrl-C to stop)");
    for stream in listener.incoming() {
        let mut s = match stream {
            Ok(s) => s,
            Err(_) => continue,
        };
        let mut buf = [0u8; 1024];
        let nread = s.read(&mut buf).unwrap_or(0);
        let req = String::from_utf8_lossy(&buf[..nread]);
        let path = req.lines().next().and_then(|l| l.split_whitespace().nth(1)).unwrap_or("/");
        let store = Store::open(db)?;
        let (ctype, body) = if path.starts_with("/api/incidents") {
            let incs = store.list_incidents()?;
            let arr: Vec<_> = incs
                .iter()
                .map(|i| serde_json::json!({"severity":i.severity,"status":i.status,"count":i.count,"blast":i.blast_radius,"has_finding":i.has_finding,"signature":i.signature}))
                .collect();
            ("application/json", serde_json::to_string(&arr)?)
        } else {
            ("text/html; charset=utf-8", html_page(&store, project)?)
        };
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = s.write_all(resp.as_bytes());
    }
    Ok(())
}
