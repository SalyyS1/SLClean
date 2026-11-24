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
    }
}
