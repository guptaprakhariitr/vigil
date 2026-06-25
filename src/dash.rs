//! Read-only dashboards over the store (Phase 5 UX): a refreshing terminal view
//! (`tui`) and a multi-page web UI (`serve`) — portfolio → incidents → incident
//! detail → sources → explore → patches → policy → settings. Server-rendered
//! from the one SQLite store, self-contained, no deps. Everything is real data.

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

/// One plain-text dashboard frame for the terminal (`vigil tui`).
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

// ---------------------------------------------------------------------------
// Web UI
// ---------------------------------------------------------------------------

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}
/// One-line preview of a long (multi-KB) signature; full text goes in title="".
fn preview(s: &str, n: usize) -> String {
    let one = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > n { format!("{}…", one.chars().take(n).collect::<String>()) } else { one }
}
fn sev_pill(sev: &str) -> &'static str {
    match sev { "SEV1" | "SEV2" => "red", "SEV3" => "amber", _ => "grey" }
}
fn route_pill(r: &str) -> &'static str {
    match r { "escalate" => "red", "watch" => "amber", _ => "grey" }
}

const CSS: &str = r#"
:root{--bg:#FBF8F2;--panel:#fff;--line:#ECE6DA;--text:#1E1A14;--muted:#7A715F;--accent:#D9821A;--soft:#FBEFD8;--teal:#1f7d6b;--tealbg:#e4f4f0}
*{box-sizing:border-box}html,body{margin:0;max-width:100%;overflow-x:hidden}
body{background:var(--bg);color:var(--text);font:14px/1.55 -apple-system,BlinkMacSystemFont,"Segoe UI",Roboto,sans-serif}
a{color:inherit;text-decoration:none}
.mono{font-family:ui-monospace,"SF Mono",Menlo,Consolas,monospace}
.top{display:flex;align-items:center;gap:12px;padding:10px 16px;background:#F1ECE2;border-bottom:1px solid var(--line)}
.top .brand{font-weight:800;font-size:15px}.top .brand b{color:var(--accent)}
.top .crumb{font:11px ui-monospace,monospace;color:var(--muted)}
.top .sp{flex:1}.top .pill{font:10.5px ui-monospace,monospace;border:1px solid var(--line);border-radius:999px;padding:2px 9px;color:var(--muted);background:#fff}
.shell{display:flex;min-height:calc(100vh - 41px)}
.rail{width:84px;background:#F7F2E9;border-right:1px solid var(--line);display:flex;flex-direction:column;gap:3px;padding:10px 8px;flex:none}
.rico{display:flex;flex-direction:column;align-items:center;gap:4px;padding:9px 0;border-radius:9px;color:var(--muted);font:9px ui-monospace,monospace;text-align:center}
.rico:hover{background:#efe7d6}.rico.on{background:var(--soft);color:var(--accent)}
.rico svg{width:20px;height:20px;stroke:currentColor;fill:none;stroke-width:1.7;stroke-linecap:round;stroke-linejoin:round}
.main{flex:1;min-width:0;padding:22px 26px 60px}
h1{font-size:21px;margin:0 0 3px}.sub{color:var(--muted);font-size:13px;margin:0 0 20px}
.sub a{color:var(--accent)}
.stats{display:flex;flex-wrap:wrap;gap:11px;margin-bottom:22px}
.stat{background:var(--panel);border:1px solid var(--line);border-radius:11px;padding:12px 16px;flex:1 1 120px}
.stat b{display:block;font-size:23px;font-family:ui-monospace,monospace;color:var(--accent)}
.stat span{font-size:11px;color:var(--muted);text-transform:uppercase;letter-spacing:.05em}
h2{font:11px ui-monospace,monospace;letter-spacing:.13em;text-transform:uppercase;color:var(--muted);margin:26px 0 10px}
.tablewrap{overflow-x:auto;border:1px solid var(--line);border-radius:11px;background:var(--panel)}
table{width:100%;border-collapse:collapse;table-layout:fixed}
th,td{text-align:left;padding:10px 13px;border-bottom:1px solid var(--line);font-size:13px;vertical-align:top}
th{background:#FAF6EE;font:10px ui-monospace,monospace;letter-spacing:.06em;text-transform:uppercase;color:var(--muted)}
tr:last-child td{border-bottom:none}
tr.clk:hover td{background:#FCF7EC;cursor:pointer}
td.n{font-family:ui-monospace,monospace;text-align:right}td.c{text-align:center}
td.sig{font-family:ui-monospace,monospace;font-size:12px;color:var(--muted);white-space:nowrap;overflow:hidden;text-overflow:ellipsis;max-width:0}
col.sigcol{width:54%}col.sm{width:62px}col.md{width:96px}
.pill{display:inline-block;font:10.5px ui-monospace,monospace;padding:2px 8px;border-radius:999px;border:1px solid var(--line);color:var(--muted);background:#fff;white-space:nowrap}
.pill.red{background:#fde6e0;border-color:#f4c9bd;color:#c2492e}.pill.amber{background:#fbeed4;border-color:#f0ddb5;color:#a8700c}
.pill.green{background:var(--tealbg);border-color:#bfe6dd;color:var(--teal)}.pill.grey{background:#f1ece2;border-color:#e0d8c8;color:#8a7f6b}
.pill.blue{background:#e9eefb;border-color:#c7d4f3;color:#3b5fd1}
.empty{color:var(--muted);text-align:center;padding:20px}
code{font-family:ui-monospace,monospace;background:var(--soft);padding:1px 5px;border-radius:4px;font-size:12px}
.pgrid{display:grid;gap:13px;grid-template-columns:repeat(auto-fill,minmax(260px,1fr))}
.pcard{border:1px solid var(--line);border-radius:12px;background:var(--panel);padding:14px 15px;display:block}
.pcard:hover{border-color:var(--accent)}.pcard.alert{border-color:#f4c9bd}
.pcard .ph{display:flex;align-items:center;gap:9px;margin-bottom:9px}
.pcard .hd{width:9px;height:9px;border-radius:50%;flex:none}.hd.ok{background:#2bbd63}.hd.err{background:#e8674b}
.pcard .pn{font-weight:700;font-size:14px}.pcard .env{font:9.5px ui-monospace,monospace;color:var(--muted);margin-left:auto}
.pcard .meta{display:flex;flex-wrap:wrap;gap:6px;margin-top:4px}
.card{background:var(--panel);border:1px solid var(--line);border-radius:12px;padding:16px 18px;margin-bottom:14px}
.cause{background:var(--soft);border:1px solid #f0ddb5;border-radius:10px;padding:13px 15px;line-height:1.6}
.cause .cite{color:var(--accent);font-family:ui-monospace,monospace;font-size:11px}
.confbar{height:8px;border-radius:5px;background:#eee4d2;overflow:hidden;margin:6px 0}
.confbar i{display:block;height:100%;background:var(--accent)}
.kv{display:flex;gap:8px;flex-wrap:wrap;margin:8px 0}
pre.diff{background:#FAF6EE;border:1px solid var(--line);border-radius:9px;padding:11px 12px;overflow-x:auto;font-family:ui-monospace,monospace;font-size:11.5px;line-height:1.5;margin:0}
pre.diff .add{color:var(--teal);background:rgba(63,184,160,.12);display:block}
pre.diff .del{color:#c2492e;background:rgba(232,103,75,.10);display:block}
pre.diff .ctx{color:var(--muted);display:block}
.tl{list-style:none;margin:0;padding:0}
.tl li{position:relative;padding:0 0 14px 22px;border-left:2px solid var(--line);margin-left:6px}
.tl li:last-child{border-left-color:transparent}
.tl li::before{content:"";position:absolute;left:-6px;top:2px;width:10px;height:10px;border-radius:50%;background:var(--accent)}
.tl .ts{font:10px ui-monospace,monospace;color:var(--muted)}
.tl .st{font-weight:600;font-size:12.5px}.tl .dt{font:11px ui-monospace,monospace;color:var(--muted)}
.clus{font-family:ui-monospace,monospace;font-size:11.5px}
.clus .cr{display:flex;gap:9px;padding:3px 0;border-bottom:1px solid #f4efe6}
.clus .cn{color:#c2492e;font-weight:700;flex:none;width:46px;text-align:right}
.clus .ct{color:var(--muted);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.srcrow{display:flex;align-items:center;gap:12px;padding:11px 13px;border:1px solid var(--line);border-radius:10px;margin-bottom:9px;background:var(--panel)}
.srcrow .nm{font-family:ui-monospace,monospace;font-size:12.5px}
.setrow{display:flex;justify-content:space-between;gap:14px;padding:12px 2px;border-bottom:1px solid var(--line)}
.setrow b{font-size:13px}.setrow .v{font-family:ui-monospace,monospace;font-size:12px;color:var(--muted)}
footer{margin-top:26px;color:#a89c86;font:11px ui-monospace,monospace}
"#;

const SPRITE: &str = r##"<svg width="0" height="0" style="position:absolute" aria-hidden="true">
<symbol id="i-proj" viewBox="0 0 24 24"><rect x="4" y="4" width="7" height="7" rx="1.5"/><rect x="13" y="4" width="7" height="7" rx="1.5"/><rect x="4" y="13" width="7" height="7" rx="1.5"/><rect x="13" y="13" width="7" height="7" rx="1.5"/></symbol>
<symbol id="i-inc" viewBox="0 0 24 24"><path d="M12 4 L21 19 H3 Z"/><path d="M12 10v4"/><path d="M12 17h.01"/></symbol>
<symbol id="i-src" viewBox="0 0 24 24"><path d="M12 3v10"/><path d="M8 9l4 4 4-4"/><path d="M4 15v3a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-3"/></symbol>
<symbol id="i-exp" viewBox="0 0 24 24"><circle cx="11" cy="11" r="6"/><path d="M20 20l-4-4"/></symbol>
<symbol id="i-pat" viewBox="0 0 24 24"><circle cx="7" cy="6" r="2"/><circle cx="7" cy="18" r="2"/><circle cx="17" cy="8" r="2"/><path d="M7 8v8"/><path d="M17 10c0 4-10 1-10 6"/></symbol>
<symbol id="i-pol" viewBox="0 0 24 24"><path d="M12 3l7 3v5c0 4-3 7-7 8-4-1-7-4-7-8V6z"/></symbol>
<symbol id="i-set" viewBox="0 0 24 24"><path d="M4 8h9"/><circle cx="15" cy="8" r="2"/><path d="M17 8h3"/><path d="M4 16h3"/><circle cx="9" cy="16" r="2"/><path d="M11 16h9"/></symbol>
</svg>"##;

fn rail(active: &str, project: &str) -> String {
    let p = urlenc(project);
    let item = |ic: &str, label: &str, href: String, key: &str| {
        let on = if key == active { " on" } else { "" };
        format!("<a class=\"rico{on}\" href=\"{href}\"><svg><use href=\"#{ic}\"/></svg>{label}</a>")
    };
    format!(
        "<nav class=rail>{}{}{}{}{}{}{}</nav>",
        item("i-proj", "Projects", "/".into(), "proj"),
        item("i-inc", "Incidents", format!("/p/{p}"), "inc"),
        item("i-src", "Sources", format!("/p/{p}/sources"), "src"),
        item("i-exp", "Explore", format!("/p/{p}/explore"), "exp"),
        item("i-pat", "Patches", format!("/p/{p}/patches"), "pat"),
        item("i-pol", "Policy", format!("/p/{p}/policy"), "pol"),
        item("i-set", "Settings", format!("/p/{p}/settings"), "set"),
    )
}

fn urlenc(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c.to_string() } else { format!("%{:02X}", c as u32) }).collect()
}

fn shell(active: &str, project: &str, title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\">\
<meta http-equiv=refresh content=8><title>VigilAI · {}</title><style>{CSS}</style></head><body>{SPRITE}\
<div class=top><span class=brand>Vigil<b>AI</b></span><span class=crumb>{}</span><span class=sp></span>\
<span class=pill>read-only</span><span class=pill>auto-refresh 8s</span></div>\
<div class=shell>{}<main class=main>{}</main></div></body></html>",
        esc(title), esc(title), rail(active, project), body
    )
}

/// Render the dashboard for a path. Returns (content_type, body).
pub fn render(store: &Store, default_project: &str, path: &str) -> Result<(String, String)> {
    let parts: Vec<String> = path.split('?').next().unwrap_or("/").trim_matches('/')
        .split('/').filter(|s| !s.is_empty()).map(urldec).collect();
    let p: Vec<&str> = parts.iter().map(|s| s.as_str()).collect();
    match p.as_slice() {
        [] => Ok(("text/html; charset=utf-8".into(), page_portfolio(store, default_project)?)),
        ["api", "incidents"] => {
            let incs = store.list_incidents()?;
            let arr: Vec<_> = incs.iter().map(|i| serde_json::json!({
                "id": i.id, "severity": i.severity, "status": i.status, "count": i.count,
                "blast": i.blast_radius, "has_finding": i.has_finding, "signature": i.signature})).collect();
            Ok(("application/json".into(), serde_json::to_string(&arr)?))
        }
        ["p", proj] => Ok(("text/html; charset=utf-8".into(), page_incidents(store, proj)?)),
        ["p", proj, "incident", id] => Ok(("text/html; charset=utf-8".into(), page_incident(store, proj, id.parse().unwrap_or(0))?)),
        ["p", proj, "sources"] => Ok(("text/html; charset=utf-8".into(), page_sources(store, proj)?)),
        ["p", proj, "explore"] => Ok(("text/html; charset=utf-8".into(), page_explore(store, proj)?)),
        ["p", proj, "patches"] => Ok(("text/html; charset=utf-8".into(), page_patches(store, proj)?)),
        ["p", proj, "policy"] => Ok(("text/html; charset=utf-8".into(), page_policy(store, proj)?)),
        ["p", proj, "settings"] => Ok(("text/html; charset=utf-8".into(), page_settings(store, proj)?)),
        _ => Ok(("text/html; charset=utf-8".into(), shell("", default_project, "not found", "<h1>Not found</h1><p class=sub><a href=/>← portfolio</a></p>"))),
    }
}

fn page_portfolio(store: &Store, default_project: &str) -> Result<String> {
    let projects = store.list_projects()?;
    let (events, open) = store.counts()?;
    let mut cards = String::new();
    for pr in &projects {
        let (o, top) = store.open_incident_count(&pr.name)?;
        let srcs = store.list_sources(&pr.name)?.len();
        let cls = if o > 0 { "pcard alert" } else { "pcard" };
        let hd = if o > 0 { "err" } else { "ok" };
        cards.push_str(&format!(
            "<a class=\"{cls}\" href=\"/p/{}\"><div class=ph><span class=\"hd {hd}\"></span><span class=pn>{}</span><span class=env>{}</span></div>\
<div class=meta><span class=pill>{srcs} src</span><span class=pill>{} open</span><span class=pill>conf≥{:.2}</span></div>\
<div class=sub style=\"margin:8px 0 0\">{}</div></a>",
            urlenc(&pr.name), esc(&pr.name), esc(&pr.autonomy), o, pr.min_confidence,
            esc(&preview(top.as_deref().unwrap_or("healthy — no open incidents"), 70))
        ));
    }
    if projects.is_empty() { cards.push_str("<div class=empty>No projects yet — <code>vigil project add</code>.</div>"); }
    let body = format!(
        "<h1>Portfolio</h1><p class=sub>every system VigilAI watches · one control plane, isolated projects</p>\
<div class=stats><div class=stat><b>{}</b><span>projects</span></div><div class=stat><b>{events}</b><span>events</span></div>\
<div class=stat><b>{open}</b><span>open incidents</span></div></div><div class=pgrid>{cards}</div>",
        projects.len()
    );
    Ok(shell("proj", default_project, "Portfolio", &body))
}

fn page_incidents(store: &Store, project: &str) -> Result<String> {
    let incs = store.list_incidents_for(project)?;
    let (calls, toks) = store.usage(project)?;
    let policy = store.load_policy(project)?;
    let open = incs.iter().filter(|i| i.status == "open").count();
    let mut rows = String::new();
    for i in &incs {
        let fix = if i.has_finding { "✓" } else { "—" };
        rows.push_str(&format!(
            "<tr class=clk onclick=\"location='/p/{}/incident/{}'\"><td><span class=\"pill {}\">{}</span></td>\
<td>{}</td><td class=n>{}</td><td class=n>{}</td><td class=c>{fix}</td><td class=sig title=\"{}\">{}</td></tr>",
            urlenc(project), i.id, sev_pill(&i.severity), i.severity, esc(&i.status), i.count, i.blast_radius,
            esc(&i.signature), esc(&preview(&i.signature, 110))
        ));
    }
    if incs.is_empty() { rows.push_str("<tr><td colspan=6 class=empty>No incidents — healthy.</td></tr>"); }
    let body = format!(
        "<h1>{}</h1><p class=sub>incidents · click a row for the full investigation</p>\
<div class=stats><div class=stat><b>{open}</b><span>open</span></div><div class=stat><b>{}</b><span>total</span></div>\
<div class=stat><b>{calls}</b><span>engine calls</span></div><div class=stat><b>~{toks}</b><span>est tokens</span></div>\
<div class=stat><b>{}</b><span>policy rules</span></div></div>\
<div class=tablewrap><table><colgroup><col class=sm><col class=md><col class=sm><col class=sm><col class=sm><col class=sigcol></colgroup>\
<tr><th>Sev</th><th>Status</th><th>Count</th><th>Blast</th><th>RCA</th><th>Signature</th></tr>{rows}</table></div>",
        esc(project), incs.len(), policy.len()
    );
    Ok(shell("inc", project, &format!("{project} · incidents"), &body))
}

fn page_incident(store: &Store, project: &str, id: i64) -> Result<String> {
    let Some((i, evidence)) = store.get_incident(project, id)? else {
        return Ok(shell("inc", project, "not found", "<h1>Incident not found</h1>"));
    };
    let finding = store.get_finding(id)?;
    let audit = store.audit_for_incident(id)?;

    // header
    let mut body = format!(
        "<p class=sub><a href=\"/p/{}\">← incidents</a></p><h1><span class=\"pill {}\">{}</span> &nbsp;{}</h1>\
<p class=sub>×{} occurrences · blast {} · {}</p>",
        urlenc(project), sev_pill(&i.severity), i.severity, esc(&preview(&i.signature, 90)),
        i.count, i.blast_radius, esc(&i.status)
    );

    // root cause / finding
    body.push_str("<h2>Root cause</h2>");
    match &finding {
        Some(f) if !f.cause.is_empty() => {
            let cites = f.citations.split(',').filter(|c| !c.is_empty())
                .map(|c| format!("<span class=pill>{}</span>", esc(c))).collect::<Vec<_>>().join(" ");
            body.push_str(&format!(
                "<div class=card><div class=cause>{}</div><div style=\"margin-top:10px\">confidence {:.2}\
<div class=confbar><i style=\"width:{}%\"></i></div></div><div class=kv>{}</div></div>",
                esc(&f.cause), f.confidence, (f.confidence * 100.0) as i64, cites
            ));
            // proposed fix / diff
            if !f.patch.is_empty() {
                let diff = render_diff(&f.patch);
                let where_ = if f.branch.is_empty() { "not proposed (autonomy=notify)".to_string() } else { esc(&f.branch) };
                body.push_str(&format!(
                    "<h2>Proposed fix</h2><div class=card><div class=kv><span class=pill {}>{}</span><span class=pill>{}</span></div>{}</div>",
                    if f.validation.contains('✓') { "green" } else { "amber" },
                    esc(if f.validation.is_empty() { "not validated" } else { &f.validation }),
                    where_, diff
                ));
            }
        }
        _ => body.push_str("<div class=card><div class=empty>No engine finding yet (deterministic detection only, or muted/watched).</div></div>"),
    }

    // evidence (from the stored bundle)
    if let Some(ev) = evidence.as_ref().and_then(|e| serde_json::from_str::<serde_json::Value>(e).ok()) {
        body.push_str("<h2>Evidence</h2><div class=card>");
        if let Some(rc) = ev.get("recent_change").and_then(|v| v.as_str()) {
            body.push_str(&format!("<div class=kv><span class=pill blue>recent change</span> <span class=mono>{}</span></div>", esc(&preview(rc, 100))));
        }
        if let Some(hm) = ev.get("host_metrics").and_then(|v| if v.is_null() { None } else { Some(v) }) {
            if let Some(pr) = hm.get("pressure").and_then(|v| v.as_str()) {
                body.push_str(&format!("<div class=kv><span class=pill amber>host pressure</span> <span class=mono>{}</span></div>", esc(pr)));
            }
        }
        if let Some(clusters) = ev.get("clusters").and_then(|v| v.as_array()) {
            body.push_str("<div class=clus style=\"margin-top:8px\">");
            for c in clusters.iter().take(8) {
                let cnt = c.get("count").and_then(|v| v.as_i64()).unwrap_or(0);
                let tpl = c.get("template").and_then(|v| v.as_str()).unwrap_or("");
                let svc = c.get("service").and_then(|v| v.as_str()).unwrap_or("");
                body.push_str(&format!("<div class=cr><span class=cn>×{cnt}</span><span class=ct title=\"{}\">[{}] {}</span></div>",
                    esc(tpl), esc(svc), esc(&preview(tpl, 90))));
            }
            body.push_str("</div>");
        }
        if let Some(stack) = ev.get("stack").and_then(|v| v.as_array()) {
            if !stack.is_empty() {
                body.push_str("<div style=\"margin-top:10px\" class=mono>");
                for fr in stack.iter().take(6) {
                    let file = fr.get("file").and_then(|v| v.as_str()).unwrap_or("");
                    let line = fr.get("line").and_then(|v| v.as_i64()).unwrap_or(0);
                    body.push_str(&format!("<div style=\"color:#c2492e;font-size:11.5px\">↳ {}:{}</div>", esc(file), line));
                }
                body.push_str("</div>");
            }
        }
        body.push_str("</div>");
    }

    // decision timeline (real audit)
    body.push_str("<h2>Decision timeline</h2><div class=card><ul class=tl>");
    if audit.is_empty() { body.push_str("<li><span class=dt>no decisions recorded</span></li>"); }
    for (ts, stage, action, detail) in &audit {
        body.push_str(&format!(
            "<li><div class=ts>{}</div><div class=st>{} · {}</div><div class=dt>{}</div></li>",
            esc(ts), esc(stage), esc(action), esc(&preview(detail, 120))
        ));
    }
    body.push_str("</ul></div>");

    Ok(shell("inc", project, &format!("{project} · incident {id}"), &body))
}

fn render_diff(patch: &str) -> String {
    let mut out = String::from("<pre class=diff>");
    for l in patch.lines() {
        let cls = if l.starts_with("+++") || l.starts_with("---") { "ctx" }
            else if l.starts_with('+') { "add" } else if l.starts_with('-') { "del" } else { "ctx" };
        out.push_str(&format!("<span class={cls}>{}</span>", esc(l)));
    }
    out.push_str("</pre>");
    out
}

fn page_sources(store: &Store, project: &str) -> Result<String> {
    let srcs = store.list_sources(project)?;
    let mut rows = String::new();
    for s in &srcs {
        rows.push_str(&format!("<div class=srcrow><span class=pill green>watching</span><span class=nm>{}</span></div>", esc(s)));
    }
    if srcs.is_empty() { rows.push_str("<div class=empty>No sources.</div>"); }
    let body = format!(
        "<h1>{} · sources</h1><p class=sub>this system's containers/services — one project, many sources</p>\
<div class=stats><div class=stat><b>{}</b><span>sources</span></div></div>{rows}\
<p class=sub style=\"margin-top:14px\">Add more with <code>vigil project add-source {} &lt;path&gt;</code></p>",
        esc(project), srcs.len(), esc(project)
    );
    Ok(shell("src", project, &format!("{project} · sources"), &body))
}

fn page_explore(store: &Store, project: &str) -> Result<String> {
    let policy = store.load_policy(project)?;
    let mut tpl = String::new();
    for r in policy.iter().take(40) {
        tpl.push_str(&format!("<tr><td><span class=\"pill {}\">{}</span></td><td class=sig title=\"{}\">{}</td></tr>",
            route_pill(r.route.as_str()), r.route.as_str(), esc(&r.signature), esc(&preview(&r.signature, 110))));
    }
    if policy.is_empty() { tpl.push_str("<tr><td colspan=2 class=empty>No templates mined yet.</td></tr>"); }
    let body = format!(
        "<h1>{} · explore</h1><p class=sub>mined templates &amp; their routes · ask in natural language with <code>vigil ask</code></p>\
<div class=tablewrap><table><colgroup><col class=sm><col class=sigcol></colgroup>\
<tr><th>Route</th><th>Template</th></tr>{tpl}</table></div>",
        esc(project)
    );
    Ok(shell("exp", project, &format!("{project} · explore"), &body))
}

fn page_patches(store: &Store, project: &str) -> Result<String> {
    let incs = store.list_incidents_for(project)?;
    let mut out = String::new();
    let mut n = 0;
    for i in &incs {
        if let Some(f) = store.get_finding(i.id)? {
            if !f.patch.is_empty() {
                n += 1;
                let where_ = if f.branch.is_empty() { "—".to_string() } else { esc(&f.branch) };
                out.push_str(&format!(
                    "<div class=card><div class=kv><span class=\"pill {}\">{}</span>\
<span class=pill {}>{}</span><span class=pill>{}</span>\
<a class=pill blue href=\"/p/{}/incident/{}\">incident {}</a></div>\
<div class=sub style=\"margin:6px 0\">{}</div>{}</div>",
                    sev_pill(&i.severity), i.severity,
                    if f.validation.contains('✓') { "green" } else { "amber" },
                    esc(if f.validation.is_empty() { "not validated" } else { &f.validation }),
                    where_, urlenc(project), i.id, i.id,
                    esc(&preview(&i.signature, 90)), render_diff(&f.patch)
                ));
            }
        }
    }
    if n == 0 { out.push_str("<div class=empty>No proposed patches yet. Raise <code>--autonomy propose</code> to have VigilAI open fixes.</div>"); }
    let body = format!("<h1>{} · patches</h1><p class=sub>validated fixes VigilAI proposed — review before merge</p>{out}", esc(project));
    Ok(shell("pat", project, &format!("{project} · patches"), &body))
}

fn page_policy(store: &Store, project: &str) -> Result<String> {
    let policy = store.load_policy(project)?;
    let mut rows = String::new();
    for r in &policy {
        rows.push_str(&format!(
            "<tr><td><span class=\"pill {}\">{}</span></td><td>{}</td><td class=sig title=\"{}\">{}</td></tr>",
            route_pill(r.route.as_str()), r.route.as_str(), esc(&r.source), esc(&r.signature), esc(&preview(&r.signature, 110))));
    }
    if policy.is_empty() { rows.push_str("<tr><td colspan=3 class=empty>No policy — run <code>vigil warm</code>.</td></tr>"); }
    let body = format!(
        "<h1>{} · Tier-1 policy</h1><p class=sub>deterministic mute/watch/escalate rules · engine-authored, runs hot at 0 tokens</p>\
<div class=tablewrap><table><colgroup><col class=sm><col class=md><col class=sigcol></colgroup>\
<tr><th>Route</th><th>Source</th><th>Template</th></tr>{rows}</table></div>",
        esc(project)
    );
    Ok(shell("pol", project, &format!("{project} · policy"), &body))
}

fn page_settings(store: &Store, project: &str) -> Result<String> {
    let pr = store.get_project(project)?;
    let (calls, toks) = store.usage(project)?;
    let tel = store.get_setting("telemetry")?.unwrap_or_else(|| "unset".into());
    let row = |k: &str, v: String| format!("<div class=setrow><b>{}</b><span class=v>{}</span></div>", k, v);
    let body = match pr {
        Some(p) => format!(
            "<h1>{} · settings</h1><p class=sub>per-project config · overrides the portfolio defaults</p><div class=card>{}{}{}{}{}{}{}</div>",
            esc(project),
            row("Engine", esc(&p.engine)),
            row("Autonomy", format!("{} <span class=sub>(notify→report→propose→merge→release)</span>", esc(&p.autonomy))),
            row("Min confidence", format!("{:.2}", p.min_confidence)),
            row("Repo", esc(p.repo.as_deref().unwrap_or("—"))),
            row("Sources", format!("{}", store.list_sources(project)?.len())),
            row("Engine usage", format!("{calls} calls · ~{toks} est tokens")),
            row("Telemetry", format!("{} <span class=sub>(off by default; no egress unless an endpoint is set)</span>", esc(&tel))),
        ),
        None => format!("<h1>{} · settings</h1><div class=empty>Project not registered.</div>", esc(project)),
    };
    Ok(shell("set", project, &format!("{project} · settings"), &body))
}

fn urldec(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'%' && i + 2 < b.len() {
            if let Ok(n) = u8::from_str_radix(&s[i + 1..i + 3], 16) {
                out.push(n as char);
                i += 3;
                continue;
            }
        }
        out.push(b[i] as char);
        i += 1;
    }
    out
}

/// Live terminal dashboard — clears + redraws each tick. `once` renders one frame.
pub fn tui(db: &str, project: &str, interval: u64, once: bool) -> Result<()> {
    loop {
        let store = Store::open(db)?;
        print!("\x1b[2J\x1b[H{}", text_frame(&store, project)?);
        std::io::stdout().flush().ok();
        if once {
            break;
        }
        std::thread::sleep(std::time::Duration::from_secs(interval));
    }
    Ok(())
}

/// Multi-page read-only web UI on localhost. `/` portfolio → projects → incidents → detail.
pub fn serve(db: &str, project: &str, port: u16) -> Result<()> {
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    eprintln!("▶ vigil serve · http://127.0.0.1:{port}  (default project={project}, read-only · Ctrl-C to stop)");
    for stream in listener.incoming() {
        let mut s = match stream {
            Ok(s) => s,
            Err(_) => continue,
        };
        let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(5)));
        let mut buf = [0u8; 4096];
        let nread = s.read(&mut buf).unwrap_or(0);
        let req = String::from_utf8_lossy(&buf[..nread]);
        let path = req.lines().next().and_then(|l| l.split_whitespace().nth(1)).unwrap_or("/");
        let store = Store::open(db)?;
        let (ctype, body) = render(&store, project, path).unwrap_or_else(|e| {
            ("text/html; charset=utf-8".into(), format!("<h1>error</h1><pre>{}</pre>", esc(&e.to_string())))
        });
        let resp = format!(
            "HTTP/1.1 200 OK\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        );
        let _ = s.write_all(resp.as_bytes());
    }
    Ok(())
}
