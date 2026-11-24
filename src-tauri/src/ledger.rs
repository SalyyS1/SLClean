//! Clean Run Ledger & Audit Trail (Roadmap §1.2).
//!
//! Persists a structured record of every cleanup run (items removed, byte totals,
//! whether items were sent to Recycle Bin or permanently deleted, and per-item
//! outcomes) into `clean-ledger.json`. Retaining up to 100 runs lets users verify
//! lifetime reclaimed space and inspect which runs are recoverable from Recycle Bin.

use crate::cleaner::{CleanItem, CleanResult};
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

const MAX_RUNS: usize = 100;

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CleanRunItem {
    pub id: String,
    pub paths: Vec<PathBuf>,
    pub freed: u64,
    pub removed: u64,
    pub skipped: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct CleanRun {
    pub run_id: String,
    pub timestamp: u64,
    pub to_trash: bool,
    pub elevated: bool,
    pub source: String,
    pub total_freed: u64,
    pub total_removed: u64,
    pub total_skipped: u64,
    pub recoverable_via_trash: bool,
    pub items: Vec<CleanRunItem>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize, PartialEq)]
#[serde(default)]
pub struct CleanLedger {
    pub runs: Vec<CleanRun>,
}

#[derive(Clone, Debug, Serialize, PartialEq)]
pub struct LedgerSummary {
    pub total_runs: usize,
    pub lifetime_freed_bytes: u64,
    pub lifetime_removed_entries: u64,
    pub trash_runs: usize,
    pub permanent_runs: usize,
    pub last_run_timestamp: Option<u64>,
    pub last_run_freed_bytes: u64,
}

impl CleanLedger {
    pub fn record(
        &mut self,
        items: &[CleanItem],
        results: &[CleanResult],
        to_trash: bool,
        elevated: bool,
        source: &str,
    ) -> CleanRun {
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let run_items: Vec<CleanRunItem> = results
            .iter()
            .map(|r| {
                let paths = items
                    .iter()
                    .find(|i| i.id == r.id)
                    .map(|i| i.paths.clone())
                    .unwrap_or_default();
                CleanRunItem {
                    id: r.id.clone(),
                    paths,
                    freed: r.freed,
                    removed: r.removed,
                    skipped: r.skipped,
                    error: r.error.clone(),
                }
            })
            .collect();

        let total_freed = run_items.iter().map(|i| i.freed).sum();
        let total_removed = run_items.iter().map(|i| i.removed).sum();
        let total_skipped = run_items.iter().map(|i| i.skipped).sum();

        let run = CleanRun {
            run_id: format!("run-{ts}-{}", self.runs.len() + 1),
            timestamp: ts,
            to_trash,
            elevated,
            source: source.to_string(),
            total_freed,
            total_removed,
            total_skipped,
            recoverable_via_trash: to_trash && total_removed > 0,
            items: run_items,
        };

        self.runs.push(run.clone());
        if self.runs.len() > MAX_RUNS {
            let overflow = self.runs.len() - MAX_RUNS;
            self.runs.drain(0..overflow);
        }
        run
    }

    pub fn summary(&self) -> LedgerSummary {
        let lifetime_freed_bytes = self.runs.iter().map(|r| r.total_freed).sum();
        let lifetime_removed_entries = self.runs.iter().map(|r| r.total_removed).sum();
        let trash_runs = self.runs.iter().filter(|r| r.to_trash).count();
        let permanent_runs = self.runs.len().saturating_sub(trash_runs);
        let last = self.runs.last();
        LedgerSummary {
            total_runs: self.runs.len(),
            lifetime_freed_bytes,
            lifetime_removed_entries,
            trash_runs,
            permanent_runs,
            last_run_timestamp: last.map(|r| r.timestamp),
            last_run_freed_bytes: last.map(|r| r.total_freed).unwrap_or(0),
        }
    }
}

pub fn default_ledger_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("vn.salyyy.slclean").join("clean-ledger.json"))
}

pub fn load_from(path: &Path) -> CleanLedger {
    std::fs::read_to_string(path)
        .ok()
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

pub fn save_to(path: &Path, ledger: &CleanLedger) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }
    let tmp = path.with_extension("json.tmp");
    let json = serde_json::to_string_pretty(ledger).map_err(|e| e.to_string())?;
    std::fs::write(&tmp, json).map_err(|e| e.to_string())?;
    std::fs::rename(&tmp, path).map_err(|e| {
        let _ = std::fs::remove_file(&tmp);
        e.to_string()
    })
}

pub fn load_default() -> CleanLedger {
    default_ledger_path()
        .map(|p| load_from(&p))
        .unwrap_or_default()
}

pub fn append_default(
    items: &[CleanItem],
    results: &[CleanResult],
    to_trash: bool,
    elevated: bool,
    source: &str,
) -> Option<CleanRun> {
    let path = default_ledger_path()?;
    let mut ledger = load_from(&path);
    let run = ledger.record(items, results, to_trash, elevated, source);
    let _ = save_to(&path, &ledger);
    Some(run)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ledger_records_summarizes_and_roundtrips_cleanly() {
        let dir = std::env::temp_dir().join("slclean-ledger-test");
        let _ = std::fs::remove_dir_all(&dir);
        let file = dir.join("clean-ledger.json");

        let mut ledger = CleanLedger::default();
        let items = vec![CleanItem {
            id: "npm".into(),
            paths: vec![PathBuf::from("/tmp/npm-cache")],
            keep_root: true,
        }];
        let results = vec![CleanResult {
            id: "npm".into(),
            freed: 500_000_000,
            removed: 120,
            skipped: 0,
            error: None,
        }];

        let run = ledger.record(&items, &results, true, false, "cli");
        assert!(run.recoverable_via_trash);
        assert_eq!(run.total_freed, 500_000_000);

        save_to(&file, &ledger).expect("save ledger");
        let reloaded = load_from(&file);
        assert_eq!(reloaded.runs.len(), 1);
        let summary = reloaded.summary();
        assert_eq!(summary.lifetime_freed_bytes, 500_000_000);
        assert_eq!(summary.trash_runs, 1);
        assert_eq!(summary.permanent_runs, 0);

        let _ = std::fs::remove_dir_all(&dir);
    }
}
