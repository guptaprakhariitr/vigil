//! Read-only-by-default web UI + terminal TUI over the store (Phase 5 UX).
//! Mirrors the UI navigation tree: a system rail (All-projects: Projects,
//! Incidents, Sources, Explore, Patches, Ask, Settings) and, inside a project,
//! a sub-nav (Overview · Incidents · Sources · Explore · Rules · Config) plus a
//! tabbed Investigation (Cause · Evidence · Fix). GET renders; a few safe POSTs
//! (feedback, route, pause) mutate the store. Everything is real data.

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

// ===========================================================================
// Web UI
// ===========================================================================

fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;").replace('\'', "&#39;")
}
fn preview(s: &str, n: usize) -> String {
    let one = s.split_whitespace().collect::<Vec<_>>().join(" ");
    if one.chars().count() > n { format!("{}…", one.chars().take(n).collect::<String>()) } else { one }
}
fn sev_pill(s: &str) -> &'static str { match s { "SEV1" | "SEV2" => "red", "SEV3" => "amber", _ => "grey" } }
fn route_pill(r: &str) -> &'static str { match r { "escalate" => "red", "watch" => "amber", _ => "grey" } }
fn urlenc(s: &str) -> String {
    s.chars().map(|c| if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') { c.to_string() } else { format!("%{:02X}", c as u32) }).collect()
}
fn urldec(s: &str) -> String {
    let b = s.as_bytes();
    let mut out = String::new();
    let mut i = 0;
    while i < b.len() {
        match b[i] {
            b'%' if i + 2 < b.len() => match u8::from_str_radix(&s[i + 1..i + 3], 16) {
                Ok(n) => { out.push(n as char); i += 3; }
                Err(_) => { out.push('%'); i += 1; }
            },
            b'+' => { out.push(' '); i += 1; }
            c => { out.push(c as char); i += 1; }
        }
    }
    out
}
fn form_get<'a>(form: &'a str, key: &str) -> Option<String> {
    form.split('&').find_map(|kv| {
        let mut it = kv.splitn(2, '=');
        (it.next()? == key).then(|| urldec(it.next().unwrap_or("")))
    })
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
.main{flex:1;min-width:0;padding:20px 26px 60px}
h1{font-size:21px;margin:0 0 3px}.sub{color:var(--muted);font-size:13px;margin:0 0 18px}.sub a{color:var(--accent)}
.subnav{display:flex;gap:3px;flex-wrap:wrap;border-bottom:1px solid var(--line);margin:0 0 18px}
.subnav a{font-size:13px;font-weight:600;color:var(--muted);padding:9px 13px;border-bottom:2px solid transparent}
.subnav a:hover{color:var(--text)}.subnav a.on{color:var(--accent);border-bottom-color:var(--accent)}
.scope{display:inline-flex;margin-left:auto}
.scope a{font:11px ui-monospace,monospace;color:var(--muted);border:1px solid var(--line);padding:4px 11px;background:#fff}
.scope a:first-child{border-radius:7px 0 0 7px}.scope a:last-child{border-radius:0 7px 7px 0;border-left:none}
.scope a.on{background:var(--soft);color:var(--accent);border-color:#f0ddb5;font-weight:700}
.stats{display:flex;flex-wrap:wrap;gap:11px;margin-bottom:20px}
.stat{background:var(--panel);border:1px solid var(--line);border-radius:11px;padding:12px 16px;flex:1 1 120px}
.stat b{display:block;font-size:23px;font-family:ui-monospace,monospace;color:var(--accent)}
.stat span{font-size:11px;color:var(--muted);text-transform:uppercase;letter-spacing:.05em}
h2{font:11px ui-monospace,monospace;letter-spacing:.13em;text-transform:uppercase;color:var(--muted);margin:24px 0 10px}
.tablewrap{overflow-x:auto;border:1px solid var(--line);border-radius:11px;background:var(--panel)}
table{width:100%;border-collapse:collapse;table-layout:fixed}
th,td{text-align:left;padding:10px 13px;border-bottom:1px solid var(--line);font-size:13px;vertical-align:top}
th{background:#FAF6EE;font:10px ui-monospace,monospace;letter-spacing:.06em;text-transform:uppercase;color:var(--muted)}
tr:last-child td{border-bottom:none}tr.clk:hover td{background:#FCF7EC;cursor:pointer}
td.n{font-family:ui-monospace,monospace;text-align:right}td.c{text-align:center}
td.sig{font-family:ui-monospace,monospace;font-size:12px;color:var(--muted);white-space:nowrap;overflow:hidden;text-overflow:ellipsis;max-width:0}
col.sigcol{width:46%}col.sm{width:60px}col.md{width:92px}col.lg{width:130px}
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
.grid2{display:grid;gap:14px;grid-template-columns:1fr 1fr}@media(max-width:680px){.grid2{grid-template-columns:1fr}}
.cause{background:var(--soft);border:1px solid #f0ddb5;border-radius:10px;padding:13px 15px;line-height:1.6}
.confbar{height:8px;border-radius:5px;background:#eee4d2;overflow:hidden;margin:6px 0}.confbar i{display:block;height:100%;background:var(--accent)}
.kv{display:flex;gap:8px;flex-wrap:wrap;margin:8px 0;align-items:center}
pre.diff{background:#FAF6EE;border:1px solid var(--line);border-radius:9px;padding:11px 12px;overflow-x:auto;font-family:ui-monospace,monospace;font-size:11.5px;line-height:1.5;margin:0}
pre.diff .add{color:var(--teal);background:rgba(63,184,160,.12);display:block}pre.diff .del{color:#c2492e;background:rgba(232,103,75,.10);display:block}pre.diff .ctx{color:var(--muted);display:block}
.tl{list-style:none;margin:0;padding:0}.tl li{position:relative;padding:0 0 14px 22px;border-left:2px solid var(--line);margin-left:6px}
.tl li:last-child{border-left-color:transparent}.tl li::before{content:"";position:absolute;left:-6px;top:2px;width:10px;height:10px;border-radius:50%;background:var(--accent)}
.tl .ts{font:10px ui-monospace,monospace;color:var(--muted)}.tl .st{font-weight:600;font-size:12.5px}.tl .dt{font:11px ui-monospace,monospace;color:var(--muted)}
.clus{font-family:ui-monospace,monospace;font-size:11.5px}.clus .cr{display:flex;gap:9px;padding:3px 0;border-bottom:1px solid #f4efe6}
.clus .cn{color:#c2492e;font-weight:700;flex:none;width:46px;text-align:right}.clus .ct{color:var(--muted);overflow:hidden;text-overflow:ellipsis;white-space:nowrap}
.srcrow{display:flex;align-items:center;gap:12px;padding:11px 13px;border:1px solid var(--line);border-radius:10px;margin-bottom:9px;background:var(--panel)}
.srcrow .nm{font-family:ui-monospace,monospace;font-size:12.5px}.srcrow .pr{margin-left:auto;font:11px ui-monospace,monospace;color:var(--muted)}
.setrow{display:flex;justify-content:space-between;gap:14px;padding:12px 2px;border-bottom:1px solid var(--line)}
.setrow b{font-size:13px}.setrow .v{font-family:ui-monospace,monospace;font-size:12px;color:var(--muted)}
.tabs{display:flex;gap:3px;border-bottom:1px solid var(--line);margin:6px 0 14px}
.tabs a{font-size:13px;font-weight:600;color:var(--muted);padding:9px 14px;border-bottom:2px solid transparent}
.tabs a.on{color:var(--accent);border-bottom-color:var(--accent)}
.btn{display:inline-flex;align-items:center;gap:6px;font:600 12.5px -apple-system,sans-serif;border-radius:8px;padding:8px 13px;border:1px solid var(--line);background:#fff;color:var(--text);cursor:pointer}
.btn.pri{background:var(--accent);border-color:var(--accent);color:#fff}.btn.gh{border-color:#bfe6dd;color:var(--teal)}.btn.dn{border-color:#f4c9bd;color:#c2492e}
form.inline{display:inline}
.ask{display:flex;gap:8px;margin:10px 0 18px}.ask input{flex:1;font:13px ui-monospace,monospace;padding:10px 12px;border:1px solid var(--line);border-radius:9px;background:#fff;color:var(--text)}
footer{margin-top:26px;color:#a89c86;font:11px ui-monospace,monospace}
"#;

const SPRITE: &str = r##"<svg width="0" height="0" style="position:absolute" aria-hidden="true">
<symbol id="i-proj" viewBox="0 0 24 24"><rect x="4" y="4" width="7" height="7" rx="1.5"/><rect x="13" y="4" width="7" height="7" rx="1.5"/><rect x="4" y="13" width="7" height="7" rx="1.5"/><rect x="13" y="13" width="7" height="7" rx="1.5"/></symbol>
<symbol id="i-inc" viewBox="0 0 24 24"><path d="M12 4 L21 19 H3 Z"/><path d="M12 10v4"/><path d="M12 17h.01"/></symbol>
<symbol id="i-src" viewBox="0 0 24 24"><path d="M12 3v10"/><path d="M8 9l4 4 4-4"/><path d="M4 15v3a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2v-3"/></symbol>
<symbol id="i-exp" viewBox="0 0 24 24"><circle cx="11" cy="11" r="6"/><path d="M20 20l-4-4"/></symbol>
<symbol id="i-pat" viewBox="0 0 24 24"><circle cx="7" cy="6" r="2"/><circle cx="7" cy="18" r="2"/><circle cx="17" cy="8" r="2"/><path d="M7 8v8"/><path d="M17 10c0 4-10 1-10 6"/></symbol>
<symbol id="i-ask" viewBox="0 0 24 24"><path d="M21 15a2 2 0 0 1-2 2H8l-4 4V5a2 2 0 0 1 2-2h13a2 2 0 0 1 2 2z"/><path d="M9.5 9a2.5 2.5 0 1 1 3.5 2.3c-.6.3-1 .9-1 1.7"/><path d="M12 16h.01"/></symbol>
<symbol id="i-set" viewBox="0 0 24 24"><path d="M4 8h9"/><circle cx="15" cy="8" r="2"/><path d="M17 8h3"/><path d="M4 16h3"/><circle cx="9" cy="16" r="2"/><path d="M11 16h9"/></symbol>
</svg>"##;

fn rail(active: &str) -> String {
    let item = |ic: &str, label: &str, href: &str, key: &str| {
        let on = if key == active { " on" } else { "" };
        format!("<a class=\"rico{on}\" href=\"{href}\"><svg><use href=\"#{ic}\"/></svg>{label}</a>")
    };
    format!(
        "<nav class=rail>{}{}{}{}{}{}{}</nav>",
        item("i-proj", "Projects", "/", "proj"),
        item("i-inc", "Incidents", "/incidents", "inc"),
        item("i-src", "Sources", "/sources", "src"),
        item("i-exp", "Explore", "/explore", "exp"),
        item("i-pat", "Patches", "/patches", "pat"),
        item("i-ask", "Ask", "/ask", "ask"),
        item("i-set", "Settings", "/settings", "set"),
    )
}

fn shell(rail_active: &str, crumb: &str, title: &str, body: &str) -> String {
    format!(
        "<!doctype html><html><head><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\">\
<title>VigilAI · {}</title><style>{CSS}</style></head><body>{SPRITE}\
<div class=top><span class=brand>Vigil<b>AI</b></span><span class=crumb>{}</span><span class=sp></span>\
<span class=pill>self-hosted</span></div><div class=shell>{}<main class=main>{}</main></div></body></html>",
        esc(title), esc(crumb), rail(rail_active), body
    )
}

fn subnav(project: &str, active: &str, scope_other: &str) -> String {
    let p = urlenc(project);
    let it = |label: &str, seg: &str, key: &str| {
        let href = if seg.is_empty() { format!("/p/{p}") } else { format!("/p/{p}/{seg}") };
        let on = if key == active { " class=on" } else { "" };
        format!("<a{on} href=\"{href}\">{label}</a>")
    };
    format!(
        "<div class=subnav>{}{}{}{}{}{}{}</div>",
        it("Overview", "", "ov"),
        it("Incidents", "incidents", "inc"),
        it("Sources", "sources", "src"),
        it("Explore", "explore", "exp"),
        it("Rules", "rules", "rules"),
        it("Config", "config", "cfg"),
        scope_other, // optional "view across all projects" link
    )
}

// ---- response model -------------------------------------------------------
pub struct Resp { pub status: u16, pub ctype: String, pub body: String, pub location: Option<String> }
fn html(b: String) -> Resp { Resp { status: 200, ctype: "text/html; charset=utf-8".into(), body: b, location: None } }
fn redirect(to: String) -> Resp { Resp { status: 303, ctype: "text/html".into(), body: String::new(), location: Some(to) } }

/// Route a request. `method` is GET/POST; `form` is the POST body (urlencoded).
pub fn route(store: &Store, default_project: &str, method: &str, path: &str, form: &str) -> Result<Resp> {
    let parts: Vec<String> = path.split('?').next().unwrap_or("/").trim_matches('/')
        .split('/').filter(|s| !s.is_empty()).map(urldec).collect();
    let p: Vec<&str> = parts.iter().map(|s| s.as_str()).collect();
    let query = path.split('?').nth(1).unwrap_or("");

    if method == "POST" {
        return handle_post(store, &p, form);
    }
    let r = match p.as_slice() {
        [] => html(page_portfolio(store)?),
        ["api", "incidents"] => {
            let arr: Vec<_> = store.list_incidents()?.iter().map(|i| serde_json::json!({
                "id": i.id, "severity": i.severity, "status": i.status, "count": i.count,
                "blast": i.blast_radius, "has_finding": i.has_finding, "signature": i.signature})).collect();
            Resp { status: 200, ctype: "application/json".into(), body: serde_json::to_string(&arr)?, location: None }
        }
        ["incidents"] => html(page_all_incidents(store)?),
        ["sources"] => html(page_all_sources(store)?),
        ["explore"] => html(page_all_explore(store)?),
        ["patches"] => html(page_all_patches(store)?),
        ["ask"] => html(page_ask(store, None)?),
        ["settings"] => html(page_settings_sys(store)?),
        ["p", proj] => html(page_overview(store, proj)?),
        ["p", proj, "incidents"] => html(page_incidents(store, proj)?),
        ["p", proj, "incident", id] => {
            let tab = match form_get(query, "tab").as_deref() {
                Some("evidence") => "evidence",
                Some("fix") => "fix",
                _ => "cause",
            };
            html(page_incident(store, proj, id.parse().unwrap_or(0), tab)?)
        }
        ["p", proj, "sources"] => html(page_sources(store, proj)?),
        ["p", proj, "explore"] => html(page_explore(store, proj)?),
        ["p", proj, "rules"] => html(page_rules(store, proj)?),
        ["p", proj, "config"] => html(page_config(store, proj)?),
        _ => html(shell("", "not found", "not found", "<h1>Not found</h1><p class=sub><a href=/>← portfolio</a></p>")),
    };
    let _ = default_project;
    Ok(r)
}

fn handle_post(store: &Store, p: &[&str], form: &str) -> Result<Resp> {
    match p {
        ["p", proj, "incident", id, "feedback"] => {
            let id: i64 = id.parse().unwrap_or(0);
            let verdict = form_get(form, "verdict").unwrap_or_default();
            if let Some((row, _)) = store.get_incident(proj, id)? {
                let reason = "via web UI";
                if verdict == "accept" {
                    store.set_verdict(id, "accept", reason)?;
                    store.set_route(proj, &row.fingerprint, &row.signature, "escalate", "feedback")?;
                    store.set_incident_status(id, "resolved")?;
                    store.audit(proj, id, "feedback", "accept", reason)?;
                } else if verdict == "reject" {
                    store.set_verdict(id, "reject", reason)?;
                    store.set_route(proj, &row.fingerprint, &row.signature, "mute", "feedback")?;
                    store.set_incident_status(id, "dismissed")?;
                    store.audit(proj, id, "feedback", "reject", reason)?;
                }
            }
            Ok(redirect(format!("/p/{}/incident/{}", urlenc(proj), id)))
        }
        ["p", proj, "rules"] => {
            let tid = form_get(form, "template").unwrap_or_default();
            let route = form_get(form, "route").unwrap_or_default();
            let sig = form_get(form, "sig").unwrap_or_default();
            if !tid.is_empty() && matches!(route.as_str(), "mute" | "watch" | "escalate") {
                store.set_route(proj, &tid, &sig, &route, "manual")?;
            }
            Ok(redirect(format!("/p/{}/rules", urlenc(proj))))
        }
        ["p", proj, "pause"] => {
            let act = form_get(form, "action").unwrap_or_default();
            store.set_paused(proj, act == "pause")?;
            Ok(redirect(format!("/p/{}", urlenc(proj))))
        }
        _ => Ok(redirect("/".into())),
    }
}

// ---- system (all-projects) pages -----------------------------------------

fn page_portfolio(store: &Store) -> Result<String> {
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
    if projects.is_empty() { cards.push_str("<div class=empty>No projects — <code>vigil project add</code>.</div>"); }
    let body = format!(
        "<h1>Portfolio</h1><p class=sub>every system VigilAI watches · one control plane, isolated projects</p>\
<div class=stats><div class=stat><b>{}</b><span>projects</span></div><div class=stat><b>{events}</b><span>events</span></div>\
<div class=stat><b>{open}</b><span>open incidents</span></div></div><div class=pgrid>{cards}</div>",
        projects.len()
    );
    Ok(shell("proj", "portfolio", "Portfolio", &body))
}

fn incidents_table(_store: &Store, rows_src: &[(String, vigil_engine::store::IncidentRow)], show_project: bool) -> String {
    let mut rows = String::new();
    for (proj, i) in rows_src {
        let fix = if i.has_finding { "✓" } else { "—" };
        let projcol = if show_project { format!("<td class=mono>{}</td>", esc(proj)) } else { String::new() };
        rows.push_str(&format!(
            "<tr class=clk onclick=\"location='/p/{}/incident/{}'\"><td><span class=\"pill {}\">{}</span></td>{}\
<td>{}</td><td class=n>{}</td><td class=n>{}</td><td class=c>{fix}</td><td class=sig title=\"{}\">{}</td></tr>",
            urlenc(proj), i.id, sev_pill(&i.severity), i.severity, projcol, esc(&i.status), i.count, i.blast_radius,
            esc(&i.signature), esc(&preview(&i.signature, 100))
        ));
    }
    if rows_src.is_empty() { rows.push_str("<tr><td colspan=7 class=empty>No incidents — healthy.</td></tr>"); }
    let projhead = if show_project { "<col class=md>" } else { "" };
    let projth = if show_project { "<th>Project</th>" } else { "" };
    format!(
        "<div class=tablewrap><table><colgroup><col class=sm>{projhead}<col class=md><col class=sm><col class=sm><col class=sm><col class=sigcol></colgroup>\
<tr><th>Sev</th>{projth}<th>Status</th><th>Count</th><th>Blast</th><th>RCA</th><th>Signature</th></tr>{rows}</table></div>"
    )
}

fn page_all_incidents(store: &Store) -> Result<String> {
    let mut all = Vec::new();
    for pr in store.list_projects()? {
        for i in store.list_incidents_for(&pr.name)? { all.push((pr.name.clone(), i)); }
    }
    all.sort_by(|a, b| sev_rank(&a.1.severity).cmp(&sev_rank(&b.1.severity)).then(b.1.count.cmp(&a.1.count)));
    let open = all.iter().filter(|(_, i)| i.status == "open").count();
    let body = format!(
        "<h1>Incidents · all projects</h1><p class=sub>every open incident across the portfolio · click a row to investigate</p>\
<div class=stats><div class=stat><b>{open}</b><span>open</span></div><div class=stat><b>{}</b><span>total</span></div></div>{}",
        all.len(), incidents_table(store, &all, true)
    );
    Ok(shell("inc", "incidents · all", "Incidents", &body))
}

fn sev_rank(s: &str) -> u8 { match s { "SEV1" => 1, "SEV2" => 2, "SEV3" => 3, _ => 4 } }

fn page_all_sources(store: &Store) -> Result<String> {
    let mut rows = String::new();
    let mut n = 0;
    for pr in store.list_projects()? {
        for s in store.list_sources(&pr.name)? {
            n += 1;
            rows.push_str(&format!(
                "<div class=srcrow><span class=pill green>watching</span><span class=nm>{}</span><span class=pr><a href=\"/p/{}\">{}</a></span></div>",
                esc(&s), urlenc(&pr.name), esc(&pr.name)));
        }
    }
    if n == 0 { rows.push_str("<div class=empty>No sources.</div>"); }
    let body = format!("<h1>Sources · all projects</h1><p class=sub>every watched log stream · {n} across the portfolio</p>{rows}");
    Ok(shell("src", "sources · all", "Sources", &body))
}

fn page_all_explore(store: &Store) -> Result<String> {
    let mut rows = String::new();
    for pr in store.list_projects()? {
        for r in store.load_policy(&pr.name)?.into_iter().take(20) {
            rows.push_str(&format!("<tr><td class=mono>{}</td><td><span class=\"pill {}\">{}</span></td><td class=sig title=\"{}\">{}</td></tr>",
                esc(&pr.name), route_pill(r.route.as_str()), r.route.as_str(), esc(&r.signature), esc(&preview(&r.signature, 100))));
        }
    }
    if rows.is_empty() { rows.push_str("<tr><td colspan=3 class=empty>No templates yet.</td></tr>"); }
    let body = format!(
        "<h1>Explore · all projects</h1><p class=sub>mined templates &amp; routes across the portfolio · ask in NL on <a href=/ask>Ask</a></p>\
<div class=tablewrap><table><colgroup><col class=md><col class=sm><col class=sigcol></colgroup>\
<tr><th>Project</th><th>Route</th><th>Template</th></tr>{rows}</table></div>"
    );
    Ok(shell("exp", "explore · all", "Explore", &body))
}

fn patches_html(store: &Store, project: &str, show_project: bool) -> Result<(usize, String)> {
    let mut out = String::new();
    let mut n = 0;
    let projects: Vec<String> = if show_project { store.list_projects()?.into_iter().map(|p| p.name).collect() } else { vec![project.to_string()] };
    for proj in projects {
        for i in store.list_incidents_for(&proj)? {
            if let Some(f) = store.get_finding(i.id)? {
                if !f.patch.is_empty() {
                    n += 1;
                    let where_ = if f.branch.is_empty() { "—".to_string() } else { esc(&f.branch) };
                    let pj = if show_project { format!("<span class=pill>{}</span>", esc(&proj)) } else { String::new() };
                    out.push_str(&format!(
                        "<div class=card><div class=kv>{pj}<span class=\"pill {}\">{}</span><span class=\"pill {}\">{}</span><span class=pill>{}</span>\
<a class=\"pill blue\" href=\"/p/{}/incident/{}?tab=fix\">incident {}</a></div><div class=sub style=\"margin:6px 0\">{}</div>{}</div>",
                        sev_pill(&i.severity), i.severity,
                        if f.validation.contains('✓') { "green" } else { "amber" },
                        esc(if f.validation.is_empty() { "not validated" } else { &f.validation }),
                        where_, urlenc(&proj), i.id, i.id, esc(&preview(&i.signature, 90)), render_diff(&f.patch)));
                }
            }
        }
    }
    Ok((n, out))
}

fn page_all_patches(store: &Store) -> Result<String> {
    let (n, out) = patches_html(store, "", true)?;
    let body = format!(
        "<h1>Patches · all projects</h1><p class=sub>validated fixes VigilAI proposed — review before merge</p>{}",
        if n == 0 { "<div class=empty>No proposed patches yet. Raise <code>--autonomy propose</code> to have VigilAI open fixes.</div>".into() } else { out }
    );
    Ok(shell("pat", "patches · all", "Patches", &body))
}

fn page_ask(store: &Store, answer: Option<&str>) -> Result<String> {
    // Read-only NL surface: render the context + the exact CLI to run (a live
    // web ask would spend an engine call per request — kept to the CLI/daemon).
    let mut ctx = String::new();
    for pr in store.list_projects()? {
        let (o, top) = store.open_incident_count(&pr.name)?;
        ctx.push_str(&format!("<tr><td class=mono>{}</td><td class=n>{}</td><td class=sig title=\"{}\">{}</td></tr>",
            esc(&pr.name), o, esc(top.as_deref().unwrap_or("")), esc(&preview(top.as_deref().unwrap_or("—"), 90))));
    }
    let ans = answer.map(|a| format!("<div class=card><div class=cause>{}</div></div>", esc(a))).unwrap_or_default();
    let body = format!(
        "<h1>Ask VigilAI</h1><p class=sub>natural-language questions over your incidents — grounded &amp; cited</p>\
<form class=ask method=get action=/ask><input name=q placeholder=\"why are 502s up since the deploy?\" value=\"\"><button class=\"btn pri\" type=submit>Ask</button></form>\
{ans}<p class=sub>The answer runs on your chosen engine. From a shell: <code>vigil ask \"…\" --project &lt;name&gt;</code></p>\
<h2>Current context</h2><div class=tablewrap><table><colgroup><col class=md><col class=sm><col class=sigcol></colgroup>\
<tr><th>Project</th><th>Open</th><th>Top incident</th></tr>{ctx}</table></div>"
    );
    Ok(shell("ask", "ask", "Ask", &body))
}

fn page_settings_sys(store: &Store) -> Result<String> {
    let (events, open) = store.counts()?;
    let tel = store.get_setting("telemetry")?.unwrap_or_else(|| "unset".into());
    let np = store.list_projects()?.len();
    let row = |k: &str, v: String| format!("<div class=setrow><b>{}</b><span class=v>{}</span></div>", k, v);
    let body = format!(
        "<h1>Settings · system</h1><p class=sub>portfolio-wide defaults &amp; do-no-harm caps · per-project overrides live in a project's Config</p>\
<h2>Defaults</h2><div class=card>{}{}{}{}</div>\
<h2>Resource budget · do-no-harm</h2><div class=card>{}{}{}{}</div>",
        row("Projects", format!("{np}")),
        row("Events stored", format!("{events}")),
        row("Open incidents", format!("{open}")),
        row("Telemetry", format!("{} <span class=sub>(off by default; no egress unless VIGIL_TELEMETRY_ENDPOINT is set)</span>", esc(&tel))),
        row("Data plane", "read-only — reads logs &amp; repo; never queries prod DB".into()),
        row("Resource caps", "CPU/mem budget (`vigil run --max-rss-mb`); sheds its own load before the app".into()),
        row("Credential ceiling", "scoped git token only — opens PRs, never deploys".into()),
        row("Kill switch", "`vigil pause [project|*]`".into()),
    );
    Ok(shell("set", "settings · system", "Settings", &body))
}

// ---- project pages --------------------------------------------------------

fn page_overview(store: &Store, project: &str) -> Result<String> {
    let incs = store.list_incidents_for(project)?;
    let open = incs.iter().filter(|i| i.status == "open").count();
    let (calls, toks) = store.usage(project)?;
    let policy = store.load_policy(project)?;
    let srcs = store.list_sources(project)?;
    let paused = store.is_paused(project)?;
    let recent = incs.iter().take(5).map(|i| format!(
        "<tr class=clk onclick=\"location='/p/{}/incident/{}'\"><td><span class=\"pill {}\">{}</span></td><td class=n>{}</td><td class=sig title=\"{}\">{}</td></tr>",
        urlenc(project), i.id, sev_pill(&i.severity), i.severity, i.count, esc(&i.signature), esc(&preview(&i.signature, 90)))).collect::<String>();
    let pausebtn = if paused {
        "<form class=inline method=post action=pause><input type=hidden name=action value=resume><button class=\"btn gh\">▶ Resume</button></form>"
    } else {
        "<form class=inline method=post action=pause><input type=hidden name=action value=pause><button class=btn>⏸ Pause</button></form>"
    };
    let body = format!(
        "{}<h1>{} <span class=sub>overview</span></h1>\
<div class=stats><div class=stat><b>{open}</b><span>open</span></div><div class=stat><b>{}</b><span>sources</span></div>\
<div class=stat><b>{}</b><span>rules</span></div><div class=stat><b>{calls}</b><span>engine calls</span></div>\
<div class=stat><b>~{toks}</b><span>est tokens</span></div></div>\
<div class=kv>{} <a class=btn href=\"/p/{}/incidents\">All incidents →</a></div>\
<h2>Recent incidents</h2><div class=tablewrap><table><colgroup><col class=sm><col class=sm><col class=sigcol></colgroup>\
<tr><th>Sev</th><th>Count</th><th>Signature</th></tr>{}</table></div>",
        subnav(project, "ov", ""), esc(project), srcs.len(), policy.len(), pausebtn, urlenc(project),
        if incs.is_empty() { "<tr><td colspan=3 class=empty>healthy</td></tr>".into() } else { recent }
    );
    Ok(shell_proj(project, &body, "Overview"))
}

fn shell_proj(project: &str, body: &str, title: &str) -> String {
    shell("proj", &format!("portfolio / {project}"), &format!("{project} · {title}"), body)
}

fn page_incidents(store: &Store, project: &str) -> Result<String> {
    let rows: Vec<(String, _)> = store.list_incidents_for(project)?.into_iter().map(|i| (project.to_string(), i)).collect();
    let scope = "<span class=scope><a class=on href=#>this project</a><a href=/incidents>all projects</a></span>";
    let body = format!("{}<h1>{} · incidents</h1>{}", subnav(project, "inc", scope), esc(project), incidents_table(store, &rows, false));
    Ok(shell_proj(project, &body, "Incidents"))
}

fn page_incident(store: &Store, project: &str, id: i64, tab: &str) -> Result<String> {
    let Some((i, evidence)) = store.get_incident(project, id)? else {
        return Ok(shell_proj(project, "<h1>Incident not found</h1>", "Incident"));
    };
    let finding = store.get_finding(id)?;
    let p = urlenc(project);
    let tabbar = format!(
        "<div class=tabs><a class=\"{}\" href=\"/p/{p}/incident/{id}?tab=cause\">Cause</a>\
<a class=\"{}\" href=\"/p/{p}/incident/{id}?tab=evidence\">Evidence</a>\
<a class=\"{}\" href=\"/p/{p}/incident/{id}?tab=fix\">Fix</a></div>",
        if tab == "cause" { "on" } else { "" }, if tab == "evidence" { "on" } else { "" }, if tab == "fix" { "on" } else { "" }
    );
    let mut body = format!(
        "<p class=sub><a href=\"/p/{p}/incidents\">← incidents</a></p>\
<h1><span class=\"pill {}\">{}</span> &nbsp;{}</h1><p class=sub>×{} occurrences · blast {} · {}</p>{tabbar}",
        sev_pill(&i.severity), i.severity, esc(&preview(&i.signature, 90)), i.count, i.blast_radius, esc(&i.status)
    );

    match tab {
        "cause" => {
            match &finding {
                Some(f) if !f.cause.is_empty() => {
                    let cites = f.citations.split(',').filter(|c| !c.is_empty())
                        .map(|c| format!("<span class=pill>{}</span>", esc(c))).collect::<Vec<_>>().join(" ");
                    body.push_str(&format!(
                        "<div class=card><div class=cause>{}</div><div style=\"margin-top:10px\">confidence {:.2}<div class=confbar><i style=\"width:{}%\"></i></div></div><div class=kv>{}</div></div>",
                        esc(&f.cause), f.confidence, (f.confidence * 100.0) as i64, cites));
                }
                _ => body.push_str("<div class=card><div class=empty>No engine finding (deterministic detection only, or muted/watched).</div></div>"),
            }
            // actions
            body.push_str(&format!(
                "<div class=kv>\
<form class=inline method=post action=\"/p/{p}/incident/{id}/feedback\"><input type=hidden name=verdict value=accept><button class=\"btn pri\">✓ Accept (resolve · keep escalate)</button></form>\
<form class=inline method=post action=\"/p/{p}/incident/{id}/feedback\"><input type=hidden name=verdict value=reject><button class=\"btn dn\">✕ Reject as noise (mute)</button></form>\
<a class=btn href=\"/p/{p}/incident/{id}?tab=fix\">View fix →</a></div>\
<p class=sub>Follow-up in NL: <code>vigil ask \"…\" --project {}</code></p>", esc(project)));
        }
        "evidence" => {
            if let Some(ev) = evidence.as_ref().and_then(|e| serde_json::from_str::<serde_json::Value>(e).ok()) {
                body.push_str("<div class=card>");
                if let Some(rc) = ev.get("recent_change").and_then(|v| v.as_str()) {
                    body.push_str(&format!("<div class=kv><span class=\"pill blue\">recent change</span> <span class=mono>{}</span></div>", esc(&preview(rc, 100))));
                }
                if let Some(pr) = ev.get("host_metrics").and_then(|v| v.get("pressure")).and_then(|v| v.as_str()) {
                    body.push_str(&format!("<div class=kv><span class=\"pill amber\">host pressure</span> <span class=mono>{}</span></div>", esc(pr)));
                }
                if let Some(cl) = ev.get("clusters").and_then(|v| v.as_array()) {
                    body.push_str("<div class=clus style=\"margin-top:8px\">");
                    for c in cl.iter().take(8) {
                        body.push_str(&format!("<div class=cr><span class=cn>×{}</span><span class=ct title=\"{}\">[{}] {}</span></div>",
                            c.get("count").and_then(|v| v.as_i64()).unwrap_or(0),
                            esc(c.get("template").and_then(|v| v.as_str()).unwrap_or("")),
                            esc(c.get("service").and_then(|v| v.as_str()).unwrap_or("")),
                            esc(&preview(c.get("template").and_then(|v| v.as_str()).unwrap_or(""), 90))));
                    }
                    body.push_str("</div>");
                }
                if let Some(stack) = ev.get("stack").and_then(|v| v.as_array()) {
                    for fr in stack.iter().take(6) {
                        body.push_str(&format!("<div class=mono style=\"color:#c2492e;font-size:11.5px\">↳ {}:{}</div>",
                            esc(fr.get("file").and_then(|v| v.as_str()).unwrap_or("")), fr.get("line").and_then(|v| v.as_i64()).unwrap_or(0)));
                    }
                }
                body.push_str(&format!("<div class=kv style=\"margin-top:10px\"><a class=btn href=\"/p/{p}/explore\">Explore templates →</a></div></div>"));
            } else {
                body.push_str("<div class=card><div class=empty>No evidence snapshot (incident predates evidence capture, or deterministic-only).</div></div>");
            }
            // decision timeline
            body.push_str("<h2>Decision timeline</h2><div class=card><ul class=tl>");
            let audit = store.audit_for_incident(id)?;
            if audit.is_empty() { body.push_str("<li><span class=dt>no decisions recorded</span></li>"); }
            for (ts, stage, action, detail) in &audit {
                body.push_str(&format!("<li><div class=ts>{}</div><div class=st>{} · {}</div><div class=dt>{}</div></li>",
                    esc(ts), esc(stage), esc(action), esc(&preview(detail, 120))));
            }
            body.push_str("</ul></div>");
        }
        _ => {
            // fix tab
            match &finding {
                Some(f) if !f.patch.is_empty() => {
                    let where_ = if f.branch.is_empty() { "not proposed (autonomy=notify)".to_string() } else { esc(&f.branch) };
                    body.push_str(&format!(
                        "<div class=card><div class=kv><span class=\"pill {}\">{}</span><span class=pill>{}</span></div>{}</div>",
                        if f.validation.contains('✓') { "green" } else { "amber" },
                        esc(if f.validation.is_empty() { "not validated" } else { &f.validation }), where_, render_diff(&f.patch)));
                }
                _ => body.push_str("<div class=card><div class=empty>No proposed patch. The engine produced a cited cause but withheld a code change (e.g. an environment/provisioning fix), or autonomy is below <code>propose</code>.</div></div>"),
            }
        }
    }
    Ok(shell_proj(project, &body, &format!("incident {id}")))
}

fn render_diff(patch: &str) -> String {
    let mut out = String::from("<pre class=diff>");
    for l in patch.lines() {
        let cls = if l.starts_with("+++") || l.starts_with("---") { "ctx" } else if l.starts_with('+') { "add" } else if l.starts_with('-') { "del" } else { "ctx" };
        out.push_str(&format!("<span class={cls}>{}</span>", esc(l)));
    }
    out.push_str("</pre>");
    out
}

fn page_sources(store: &Store, project: &str) -> Result<String> {
    let srcs = store.list_sources(project)?;
    let mut rows = String::new();
    for s in &srcs { rows.push_str(&format!("<div class=srcrow><span class=\"pill green\">watching</span><span class=nm>{}</span></div>", esc(s))); }
    if srcs.is_empty() { rows.push_str("<div class=empty>No sources.</div>"); }
    let scope = "<span class=scope><a class=on href=#>this project</a><a href=/sources>all projects</a></span>";
    let body = format!(
        "{}<h1>{} · sources</h1><p class=sub>this system's containers/services — one project, many sources</p>{rows}\
<p class=sub style=\"margin-top:12px\">Add more: <code>vigil project add-source {} &lt;path&gt;</code></p>",
        subnav(project, "src", scope), esc(project), esc(project)
    );
    Ok(shell_proj(project, &body, "Sources"))
}

fn page_explore(store: &Store, project: &str) -> Result<String> {
    let policy = store.load_policy(project)?;
    let mut tpl = String::new();
    for r in policy.iter().take(60) {
        tpl.push_str(&format!("<tr><td><span class=\"pill {}\">{}</span></td><td class=sig title=\"{}\">{}</td></tr>",
            route_pill(r.route.as_str()), r.route.as_str(), esc(&r.signature), esc(&preview(&r.signature, 110))));
    }
    if policy.is_empty() { tpl.push_str("<tr><td colspan=2 class=empty>No templates yet.</td></tr>"); }
    let scope = "<span class=scope><a class=on href=#>this project</a><a href=/explore>all projects</a></span>";
    let body = format!(
        "{}<h1>{} · explore</h1><p class=sub>mined templates &amp; their routes · ask in NL on <a href=/ask>Ask</a></p>\
<div class=tablewrap><table><colgroup><col class=sm><col class=sigcol></colgroup><tr><th>Route</th><th>Template</th></tr>{tpl}</table></div>",
        subnav(project, "exp", scope), esc(project)
    );
    Ok(shell_proj(project, &body, "Explore"))
}

fn page_rules(store: &Store, project: &str) -> Result<String> {
    let policy = store.load_policy(project)?;
    let mut rows = String::new();
    for r in &policy {
        let sug = if r.source == "warm-setup" || r.source == "calibration" { " <span class=\"pill blue\">engine</span>" } else { "" };
        // route action buttons (POST → set_route)
        let btn = |to: &str, label: &str, cls: &str| format!(
            "<form class=inline method=post action=\"/p/{}/rules\"><input type=hidden name=template value=\"{}\"><input type=hidden name=sig value=\"{}\"><input type=hidden name=route value=\"{to}\"><button class=\"btn {cls}\" style=\"padding:3px 8px;font-size:11px\">{label}</button></form>",
            urlenc(project), esc(&r.template_id), esc(&r.signature));
        rows.push_str(&format!(
            "<tr><td><span class=\"pill {}\">{}</span></td><td>{}{sug}</td><td class=sig title=\"{}\">{}</td>\
<td>{}{}{}</td></tr>",
            route_pill(r.route.as_str()), r.route.as_str(), esc(&r.source), esc(&r.signature), esc(&preview(&r.signature, 80)),
            btn("escalate", "escalate", "dn"), btn("watch", "watch", ""), btn("mute", "mute", "gh")));
    }
    if policy.is_empty() { rows.push_str("<tr><td colspan=4 class=empty>No rules — run <code>vigil warm</code>.</td></tr>"); }
    let body = format!(
        "{}<h1>{} · Rules</h1><p class=sub>engine-authored Tier-1 policy · 0-token hot path · <span class=\"pill blue\">engine</span> = LLM-authored · click to re-route</p>\
<div class=tablewrap><table><colgroup><col class=sm><col class=md><col class=sigcol><col class=lg></colgroup>\
<tr><th>Route</th><th>Source</th><th>Template</th><th>Set</th></tr>{rows}</table></div>",
        subnav(project, "rules", ""), esc(project)
    );
    Ok(shell_proj(project, &body, "Rules"))
}

fn page_config(store: &Store, project: &str) -> Result<String> {
    let pr = store.get_project(project)?;
    let (calls, toks) = store.usage(project)?;
    let row = |k: &str, v: String| format!("<div class=setrow><b>{}</b><span class=v>{}</span></div>", k, v);
    let body = match pr {
        Some(p) => format!(
            "{}<h1>{} · Config</h1><p class=sub>per-project settings · overrides the portfolio defaults</p>\
<h2>Engine &amp; authority</h2><div class=card>{}{}{}</div>\
<h2>Code &amp; sources</h2><div class=card>{}{}</div>\
<h2>Context &amp; usage</h2><div class=card>{}{}</div>",
            subnav(project, "cfg", ""), esc(project),
            row("Engine", format!("{} <span class=sub>(inherit/override)</span>", esc(&p.engine))),
            row("Autonomy", format!("{} <span class=sub>(notify→report→propose→merge→release · per env)</span>", esc(&p.autonomy))),
            row("Min confidence", format!("{:.2}", p.min_confidence)),
            row("Repo @ SHA", esc(p.repo.as_deref().unwrap_or("— (not connected)"))),
            row("Sources", format!("{}", store.list_sources(project)?.len())),
            row("vigil.md", "project context the engine reads (like CLAUDE.md) — place a vigil.md in the repo".into()),
            row("Engine usage", format!("{calls} calls · ~{toks} est tokens")),
        ),
        None => format!("{}<h1>{} · Config</h1><div class=empty>Project not registered.</div>", subnav(project, "cfg", ""), esc(project)),
    };
    Ok(shell_proj(project, &body, "Config"))
}

/// Live terminal dashboard — clears + redraws each tick. `once` renders one frame.
pub fn tui(db: &str, project: &str, interval: u64, once: bool) -> Result<()> {
    loop {
        let store = Store::open(db)?;
        print!("\x1b[2J\x1b[H{}", text_frame(&store, project)?);
        std::io::stdout().flush().ok();
        if once { break; }
        std::thread::sleep(std::time::Duration::from_secs(interval));
    }
    Ok(())
}

/// 32-hex random token from the OS CSPRNG (no extra deps).
fn gen_token() -> String {
    let mut b = [0u8; 16];
    if let Ok(mut f) = std::fs::File::open("/dev/urandom") {
        let _ = f.read_exact(&mut b);
    }
    b.iter().map(|x| format!("{:02x}", x)).collect()
}

/// Resolve the UI token (Jupyter-style): explicit arg wins and is persisted;
/// else reuse the stored one; else generate, store, and return it.
fn resolve_token(store: &Store, arg: Option<&str>) -> Result<String> {
    if let Some(t) = arg.filter(|t| !t.is_empty()) {
        store.set_setting("ui_token", t)?;
        return Ok(t.to_string());
    }
    if let Some(t) = store.get_setting("ui_token")?.filter(|t| !t.is_empty()) {
        return Ok(t);
    }
    let t = gen_token();
    store.set_setting("ui_token", &t)?;
    Ok(t)
}

fn login_page(bad: bool) -> String {
    let note = if bad { "<p class=sub style=\"color:#c2492e\">Invalid token — try again.</p>" } else { "" };
    let body = format!(
        "<div style=\"max-width:440px;margin:8vh auto;padding:0 20px\">\
<h1 style=\"font-size:24px\">Vigil<span style=\"color:#D9821A\">AI</span></h1>\
<p class=sub>This dashboard is token-protected. Paste the token printed where <code>vigil serve</code> is running \
(or open the URL it gave you).</p>{note}\
<form class=ask method=get action=/><input name=token placeholder=\"token\" autofocus><button class=\"btn pri\" type=submit>Open</button></form>\
<p class=sub style=\"margin-top:14px\">Set your own: <code>vigil serve --token &lt;secret&gt; …</code></p></div>"
    );
    format!("<!doctype html><html><head><meta charset=utf-8><meta name=viewport content=\"width=device-width,initial-scale=1\">\
<title>VigilAI · sign in</title><style>{CSS}</style></head><body>{}</body></html>", body)
}

fn cookie_token(req: &str) -> Option<String> {
    req.lines().find(|l| l.to_ascii_lowercase().starts_with("cookie:"))?
        .split(';').find_map(|kv| {
            let kv = kv.trim_start_matches("Cookie:").trim_start_matches("cookie:").trim();
            kv.strip_prefix("vigil_token=").map(|v| v.to_string())
        })
}

/// Multi-page web UI on localhost — token-gated (Jupyter-style). GET renders;
/// a few POSTs mutate (feedback/route/pause).
pub fn serve(db: &str, project: &str, port: u16, token_arg: Option<String>) -> Result<()> {
    let token = {
        let store = Store::open(db)?;
        resolve_token(&store, token_arg.as_deref())?
    };
    let listener = TcpListener::bind(("127.0.0.1", port))?;
    eprintln!("▶ vigil serve · token-protected (default project={project}, read-only views + safe actions)");
    eprintln!("  open:  http://127.0.0.1:{port}/?token={token}");
    eprintln!("  (set your own with --token; Ctrl-C to stop)");
    for stream in listener.incoming() {
        let mut s = match stream { Ok(s) => s, Err(_) => continue };
        let _ = s.set_read_timeout(Some(std::time::Duration::from_secs(5)));
        let mut buf = vec![0u8; 16384];
        let nread = s.read(&mut buf).unwrap_or(0);
        let req = String::from_utf8_lossy(&buf[..nread]);
        let reqline = req.lines().next().unwrap_or("");
        let method = reqline.split_whitespace().next().unwrap_or("GET");
        let path = reqline.split_whitespace().nth(1).unwrap_or("/");
        let form = req.split("\r\n\r\n").nth(1).unwrap_or("");
        let query = path.split('?').nth(1).unwrap_or("");
        let qtoken = form_get(query, "token");
        let ctoken = cookie_token(&req);

        // --- auth gate ---
        let authed = ctoken.as_deref() == Some(token.as_str()) || qtoken.as_deref() == Some(token.as_str());
        if !authed {
            let page = login_page(qtoken.is_some());
            let head = format!("HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", page.len(), page);
            let _ = s.write_all(head.as_bytes());
            continue;
        }
        // fresh login via ?token= → set cookie and redirect to the clean path
        if qtoken.as_deref() == Some(token.as_str()) && ctoken.as_deref() != Some(token.as_str()) {
            let clean = path.split('?').next().unwrap_or("/");
            let head = format!("HTTP/1.1 303 See Other\r\nLocation: {clean}\r\nSet-Cookie: vigil_token={token}; Path=/; HttpOnly; SameSite=Strict\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
            let _ = s.write_all(head.as_bytes());
            continue;
        }

        let store = Store::open(db)?;
        let resp = route(&store, project, method, path, form).unwrap_or_else(|e| {
            html(format!("<h1>error</h1><pre>{}</pre>", esc(&e.to_string())))
        });
        let head = match &resp.location {
            Some(loc) => format!("HTTP/1.1 303 See Other\r\nLocation: {loc}\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"),
            None => format!("HTTP/1.1 {} OK\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                resp.status, resp.ctype, resp.body.len(), resp.body),
        };
        let _ = s.write_all(head.as_bytes());
    }
    Ok(())
}
