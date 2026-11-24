//! Executive Workstation Disk Health Diagnosis (`SLClean --doctor` & `slclean_doctor` MCP tool).
//!
//! Strictly read-only: evaluates fixed drive free-space pressure, Clean Run Ledger history,
//! and produces a 0..100 score + A–F grade with prioritized non-destructive next steps.

use crate::drives::{self, Drive};
use crate::ledger::{self, LedgerSummary};
use serde::Serialize;

#[derive(Clone, Debug, Serialize)]
pub struct DrivePressure {
    pub mount: String,
    pub total_bytes: u64,
    pub free_bytes: u64,
    pub used_pct: f64,
    pub status: &'static str,
}

#[derive(Clone, Debug, Serialize)]
pub struct DiskHealthReport {
    pub version: &'static str,
    pub grade: &'static str,
    pub score: u8,
    pub drive_count: usize,
    pub total_capacity_bytes: u64,
    pub total_free_bytes: u64,
    pub lowest_free_pct: f64,
    pub critical_drives: usize,
    pub warning_drives: usize,
    pub drives: Vec<DrivePressure>,
    pub ledger: LedgerSummary,
    pub recommendations: Vec<String>,
}

pub fn grade_meets_threshold(actual: &str, min_grade: &str) -> bool {
    fn rank(g: &str) -> u8 {
        match g.trim().to_ascii_uppercase().as_str() {
            "A" => 5,
            "B" => 4,
            "C" => 3,
            "D" => 2,
            _ => 1,
        }
    }
    rank(actual) >= rank(min_grade)
}

pub fn evaluate(drive_list: &[Drive], ledger_summary: LedgerSummary) -> DiskHealthReport {
    let mut total_cap = 0u64;
    let mut total_free = 0u64;
    let mut lowest_free_pct = 100.0_f64;
    let mut critical = 0usize;
    let mut warning = 0usize;
    let mut pressures = Vec::with_capacity(drive_list.len());

    for d in drive_list {
        total_cap = total_cap.saturating_add(d.total);
        total_free = total_free.saturating_add(d.free);
        let free_pct = if d.total > 0 {
            (d.free as f64 / d.total as f64) * 100.0
        } else {
            100.0
        };
        if free_pct < lowest_free_pct {
            lowest_free_pct = free_pct;
        }
        let used_pct = (100.0 - free_pct).clamp(0.0, 100.0);
        let status = if free_pct < 5.0 {
            critical += 1;
            "CRITICAL"
        } else if free_pct < 15.0 {
            warning += 1;
            "WARNING"
        } else {
            "HEALTHY"
        };
        pressures.push(DrivePressure {
            mount: d.mount.clone(),
            total_bytes: d.total,
            free_bytes: d.free,
            used_pct: (used_pct * 10.0).round() / 10.0,
            status,
        });
    }

    let mut score_f = 100.0_f64;
    score_f -= (critical as f64) * 35.0;
    score_f -= (warning as f64) * 15.0;
    if lowest_free_pct < 20.0 {
        score_f -= (20.0 - lowest_free_pct) * 1.2;
    }
    let score = score_f.clamp(0.0, 100.0).round() as u8;
    let grade = match score {
        90..=100 => "A",
        80..=89 => "B",
        65..=79 => "C",
        50..=64 => "D",
        _ => "F",
    };

    let mut recs = Vec::new();
    if critical > 0 {
        recs.push("CRITICAL free space (<5%): run `SLClean --scan-projects --stale-days 30` and `SLClean --scan-dupes --trust-cas-names` immediately to reclaim stale build targets and duplicate AI weights.".into());
    } else if warning > 0 {
        recs.push("Low free space (<15%): inspect Clean tab 'By project' view and Large Files Squarified Treemap.".into());
    }
    if ledger_summary.total_runs == 0 {
        recs.push("No cleanup runs recorded yet; preview safe caches non-destructively via `SLClean --clean-safe --dry-run`.".into());
    }
    if recs.is_empty() {
        recs.push("All fixed drives have healthy free headroom (>=15%).".into());
    }

    DiskHealthReport {
        version: env!("CARGO_PKG_VERSION"),
        grade,
        score,
        drive_count: pressures.len(),
        total_capacity_bytes: total_cap,
        total_free_bytes: total_free,
        lowest_free_pct: (lowest_free_pct * 10.0).round() / 10.0,
        critical_drives: critical,
        warning_drives: warning,
        drives: pressures,
        ledger: ledger_summary,
        recommendations: recs,
    }
}

pub fn run_default() -> DiskHealthReport {
    let list = drives::list_drives();
    let ledger_summary = ledger::load_default().summary();
    evaluate(&list, ledger_summary)
}

fn html_esc(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for ch in s.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            c => out.push(c),
        }
    }
    out
}

fn fmt_gib(bytes: u64) -> String {
    format!("{:.2} GiB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
}

/// Render a self-contained dark-mode executive HTML workstation storage digest (read-only).
pub fn render_digest_html(rep: &DiskHealthReport) -> String {
    let accent = match rep.grade {
        "A" | "B" => "#48e8b2",
        "C" => "#fbbf24",
        _ => "#f85149",
    };
    let mut drive_rows = String::new();
    for d in &rep.drives {
        drive_rows.push_str(&format!(
            "<tr><td><code>{}</code></td><td>{}</td><td>{}</td><td>{:.1}%</td><td><b>{}</b></td></tr>\n",
            html_esc(&d.mount),
            fmt_gib(d.total_bytes),
            fmt_gib(d.free_bytes),
            d.used_pct,
            html_esc(d.status),
        ));
    }
    let mut rec_items = String::new();
    for r in &rep.recommendations {
        rec_items.push_str(&format!("<li>{}</li>\n", html_esc(r)));
    }
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>SLClean v{} Executive Storage Digest</title>\
<style>body{{font-family:system-ui,sans-serif;background:#0b1116;color:#e6edf3;margin:0;padding:28px}}\
.wrap{{max-width:860px;margin:0 auto}}.card{{background:#131b24;border:1px solid #263445;border-radius:12px;padding:18px;margin-bottom:16px}}\
.grade{{font-size:44px;font-weight:800;color:{accent}}}table{{width:100%;border-collapse:collapse}}\
th,td{{padding:8px 10px;border-bottom:1px solid #223040;text-align:left;font-size:13px}}th{{color:#8b9bb0}}</style></head>\
<body><div class=\"wrap\">\
<div class=\"card\"><h1>SLClean v{} — Executive Workstation Storage Digest</h1>\
<p class=\"grade\">Grade {} ({}/100)</p>\
<p>Drives: {} · Total Free: {} / {} · Lowest Free: {:.1}% · Lifetime Reclaimed: {} ({} runs)</p></div>\
<div class=\"card\"><h2>Drive Pressure</h2><table><thead><tr><th>Mount</th><th>Capacity</th><th>Free</th><th>Used %</th><th>Status</th></tr></thead>\
<tbody>{}</tbody></table></div>\
<div class=\"card\"><h2>Recommended Non-Destructive Next Steps</h2><ul>{}</ul></div>\
</div></body></html>",
        html_esc(rep.version),
        html_esc(rep.version),
        html_esc(rep.grade),
        rep.score,
        rep.drive_count,
        fmt_gib(rep.total_free_bytes),
        fmt_gib(rep.total_capacity_bytes),
        rep.lowest_free_pct,
        fmt_gib(rep.ledger.lifetime_freed_bytes),
        rep.ledger.total_runs,
        drive_rows,
        rec_items,
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn grades_healthy_drives_higher_than_critical_drives() {
        let empty_ledger = ledger::CleanLedger::default().summary();
        let healthy = evaluate(
            &[Drive {
                mount: "C:".into(),
                name: "System".into(),
                total: 1000,
                free: 600,
            }],
            empty_ledger.clone(),
        );
        let crit = evaluate(
            &[Drive {
                mount: "C:".into(),
                name: "System".into(),
                total: 1000,
                free: 20,
            }],
            empty_ledger,
        );
        assert_eq!(healthy.grade, "A");
        assert!(healthy.score > crit.score);
        assert_eq!(crit.critical_drives, 1);
        let html = render_digest_html(&crit);
        assert!(html.contains("<!doctype html>"));
        assert!(html.contains("CRITICAL"));
        assert!(grade_meets_threshold("A", "B"));
        assert!(grade_meets_threshold("B", "B"));
        assert!(!grade_meets_threshold("C", "B"));
    }
}
