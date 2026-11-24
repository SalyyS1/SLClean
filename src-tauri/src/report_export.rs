//! Self-Contained HTML & RFC 4180 CSV Audit Report Exporter (Roadmap §1.6).
//!
//! Produces portable single-file HTML executive summaries (with inline treemap-style
//! visual footprint bars, zero external CDN dependencies, safe HTML escaping) and
//! spreadsheet-ready CSV exports for IT support and CI archives.

use crate::artifacts::Artifact;
use crate::ledger::LedgerSummary;
use crate::projects::ProjectScanReport;
use crate::CatalogItem;

fn esc_html(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

fn esc_csv(raw: &str) -> String {
    if raw.contains([',', '"', '\n', '\r']) {
        format!("\"{}\"", raw.replace('"', "\"\""))
    } else {
        raw.to_string()
    }
}

fn human_bytes(bytes: u64) -> String {
    const GIB: f64 = 1024.0 * 1024.0 * 1024.0;
    const MIB: f64 = 1024.0 * 1024.0;
    const KIB: f64 = 1024.0;
    let b = bytes as f64;
    if b >= GIB {
        format!("{:.2} GiB", b / GIB)
    } else if b >= MIB {
        format!("{:.1} MiB", b / MIB)
    } else if b >= KIB {
        format!("{:.1} KiB", b / KIB)
    } else {
        format!("{bytes} B")
    }
}

pub fn render_csv(catalog: &[CatalogItem], artifacts: &[Artifact]) -> String {
    let mut out = String::from("source,id_or_kind,label,group_or_tool,safety,bytes,human_size,files,path\n");
    for item in catalog {
        if item.bytes == 0 {
            continue;
        }
        let path_str = item
            .entry
            .paths
            .first()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        out.push_str(&format!(
            "catalog,{},{},{},{:?},{},{},{},{}\n",
            esc_csv(&item.entry.id),
            esc_csv(&item.entry.label.en),
            esc_csv(&item.entry.group),
            item.entry.safety,
            item.bytes,
            esc_csv(&human_bytes(item.bytes)),
            item.files,
            esc_csv(&path_str),
        ));
    }
    for art in artifacts {
        if art.bytes == 0 {
            continue;
        }
        out.push_str(&format!(
            "artifact,{},{},{},Rebuild,{},{},{},{}\n",
            esc_csv(&art.kind),
            esc_csv(&art.project.to_string_lossy()),
            esc_csv(&art.tool),
            art.bytes,
            esc_csv(&human_bytes(art.bytes)),
            art.files,
            esc_csv(&art.path.to_string_lossy()),
        ));
    }
    out
}

pub fn render_html(
    catalog: &[CatalogItem],
    projects: Option<&ProjectScanReport>,
    ledger: Option<&LedgerSummary>,
    generated_epoch: u64,
) -> String {
    let catalog_bytes: u64 = catalog.iter().map(|c| c.bytes).sum();
    let project_bytes: u64 = projects.map(|p| p.total_reclaimable_bytes).unwrap_or(0);
    let total_bytes = catalog_bytes + project_bytes;
    let lifetime_freed = ledger.map(|l| l.lifetime_freed_bytes).unwrap_or(0);

    // Top 12 visual treemap/footprint bars
    let mut top_bars: Vec<(String, String, u64)> = catalog
        .iter()
        .filter(|c| c.bytes > 0)
        .map(|c| (c.entry.label.en.clone(), c.entry.group.clone(), c.bytes))
        .collect();
    if let Some(proj) = projects {
        for p in &proj.projects {
            if p.total_bytes > 0 {
                top_bars.push((format!("Project: {}", p.name), "project-artifacts".into(), p.total_bytes));
            }
        }
    }
    top_bars.sort_by(|a, b| b.2.cmp(&a.2));
    top_bars.truncate(12);
    let max_bar = top_bars.first().map(|b| b.2).unwrap_or(1).max(1);

    let mut bars_html = String::new();
    for (label, group, bytes) in &top_bars {
        let pct = ((*bytes as f64 / max_bar as f64) * 100.0).clamp(2.0, 100.0);
        bars_html.push_str(&format!(
            "<div class=\"bar-row\"><div class=\"bar-meta\"><span>{} <small>({})</small></span><strong>{}</strong></div><div class=\"bar-track\"><div class=\"bar-fill\" style=\"width:{:.1}%\"></div></div></div>\n",
            esc_html(label),
            esc_html(group),
            esc_html(&human_bytes(*bytes)),
            pct,
        ));
    }

    let mut rows_html = String::new();
    for item in catalog.iter().filter(|c| c.bytes > 0).take(80) {
        let path_str = item
            .entry
            .paths
            .first()
            .map(|p| p.to_string_lossy().to_string())
            .unwrap_or_default();
        rows_html.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{:?}</td><td class=\"num\">{}</td><td class=\"num\">{}</td><td class=\"mono\">{}</td></tr>\n",
            esc_html(&item.entry.label.en),
            esc_html(&item.entry.group),
            item.entry.safety,
            esc_html(&human_bytes(item.bytes)),
            item.files,
            esc_html(&path_str),
        ));
    }

    let mut proj_html = String::new();
    if let Some(proj) = projects {
        for p in proj.projects.iter().take(40) {
            let status = if p.stale { "STALE" } else { "ACTIVE" };
            proj_html.push_str(&format!(
                "<tr><td>{}</td><td>{}</td><td class=\"num\">{} days</td><td class=\"num\">{}</td><td class=\"mono\">{}</td></tr>\n",
                esc_html(&p.name),
                status,
                p.inactive_days,
                esc_html(&human_bytes(p.total_bytes)),
                esc_html(&p.project.to_string_lossy()),
            ));
        }
    }

    format!(
        "<!doctype html>\n<html lang=\"en\">\n<head>\n<meta charset=\"utf-8\">\n<meta name=\"viewport\" content=\"width=device-width,initial-scale=1\">\n<title>SLClean Disk Audit Report</title>\n<style>\
        body{{margin:0;background:#0b111a;color:#e8f0fa;font-family:system-ui,-apple-system,sans-serif;line-height:1.5;padding:28px}}\
        .wrap{{max-width:1100px;margin:0 auto}}h1{{margin:0 0 6px;font-size:1.8rem}}.sub{{color:#8da2bd;margin:0 0 22px}}\
        .kpis{{display:grid;grid-template-columns:repeat(auto-fit,minmax(210px,1fr));gap:14px;margin-bottom:24px}}\
        .kpi{{background:#131e2c;border:1px solid #24364d;border-radius:12px;padding:16px}}.kpi small{{color:#8da2bd;display:block}}.kpi strong{{font-size:1.5rem;color:#48e8b2}}\
        .card{{background:#131e2c;border:1px solid #24364d;border-radius:12px;padding:18px;margin-bottom:20px}}\
        .bar-row{{margin-bottom:10px}}.bar-meta{{display:flex;justify-content:space-between;font-size:.88rem;margin-bottom:4px}}\
        .bar-track{{height:10px;border-radius:99px;background:#0a1019;overflow:hidden}}.bar-fill{{height:100%;background:linear-gradient(90deg,#48e8b2,#38bdf8)}}\
        table{{width:100%;border-collapse:collapse;font-size:.86rem}}th,td{{text-align:left;padding:8px 10px;border-bottom:1px solid #223247}}\
        th{{color:#8da2bd}}.num{{text-align:right;font-variant-numeric:tabular-nums}}.mono{{font-family:ui-monospace,monospace;font-size:.78rem;color:#9eb5cf;word-break:break-all}}\
        </style>\n</head>\n<body>\n<div class=\"wrap\">\n\
        <h1>SLClean v{} Executive Disk Audit Report</h1>\n\
        <p class=\"sub\">Local-first offline audit · generated epoch {} · zero telemetry · nothing deleted automatically.</p>\n\
        <div class=\"kpis\">\
        <div class=\"kpi\"><small>Total Reclaimable</small><strong>{}</strong></div>\
        <div class=\"kpi\"><small>Cache Catalog</small><strong>{}</strong></div>\
        <div class=\"kpi\"><small>Project Artifacts</small><strong>{}</strong></div>\
        <div class=\"kpi\"><small>Lifetime Freed (Ledger)</small><strong>{}</strong></div>\
        </div>\n\
        <div class=\"card\"><h2>Top Disk Consumers (Treemap Bar View)</h2>{}</div>\n\
        <div class=\"card\"><h2>Catalog Findings</h2><table><thead><tr><th>Item</th><th>Group</th><th>Safety</th><th class=\"num\">Size</th><th class=\"num\">Files</th><th>Path</th></tr></thead><tbody>{}</tbody></table></div>\n\
        {}</div>\n</body>\n</html>\n",
        env!("CARGO_PKG_VERSION"),
        generated_epoch,
        esc_html(&human_bytes(total_bytes)),
        esc_html(&human_bytes(catalog_bytes)),
        esc_html(&human_bytes(project_bytes)),
        esc_html(&human_bytes(lifetime_freed)),
        bars_html,
        rows_html,
        if proj_html.is_empty() {
            String::new()
        } else {
            format!("<div class=\"card\"><h2>Project-Centric Artifact Breakdown</h2><table><thead><tr><th>Project</th><th>Status</th><th class=\"num\">Inactive</th><th class=\"num\">Reclaimable</th><th>Path</th></tr></thead><tbody>{proj_html}</tbody></table></div>")
        }
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::catalog::{CatalogEntry, Safety, Text};
    use std::path::PathBuf;

    #[test]
    fn renders_escaped_html_and_csv_reports() {
        let items = vec![CatalogItem {
            entry: CatalogEntry {
                id: "npm".into(),
                group: "dev".into(),
                label: Text::new("Cache npm", "npm <Cache>"),
                note: Text::same(""),
                path: PathBuf::from("/tmp/npm,cache"),
                paths: vec![PathBuf::from("/tmp/npm,cache")],
                safety: Safety::Safe,
                keep_root: true,
                needs_admin: false,
            },
            bytes: 256 * 1024 * 1024,
            files: 42,
            denied: 0,
        }];
        let csv = render_csv(&items, &[]);
        assert!(csv.contains("\"npm <Cache>\"") || csv.contains("npm <Cache>"));
        assert!(csv.contains("\"/tmp/npm,cache\""));

        let html = render_html(&items, None, None, 1_750_000_000);
        assert!(html.contains("<!doctype html>"));
        assert!(html.contains("npm &lt;Cache&gt;"));
        assert!(html.contains("256.0 MiB"));
    }
}
