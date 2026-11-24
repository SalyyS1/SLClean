//! Headless CLI mode for CI runners, administrators, and PowerShell/Terminal users:
//! list static catalog rules, emit JSON or OASIS SARIF v2.1.0 reports for caches/artifacts,
//! audit content-addressed duplicate AI model weights and build blobs (`--scan-dupes`),
//! enforce CI runner disk budgets (`--max-reclaimable-gb`), or preview/execute safe cleanups.

use crate::{
    artifacts, blob_dupes, catalog, catalog_specs, cleaner, dead_links, drives, elevation,
    env_reclaimer, ledger, parallel, project_roots, projects, report_export, sarif, settings,
    sizer,
};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

const AFTER_PID_FLAG: &str = "--after-pid";
/// Exit code returned when `--max-reclaimable-gb` threshold is exceeded.
pub const EXIT_BUDGET_EXCEEDED: i32 = 3;

#[derive(Debug, PartialEq)]
pub enum CliCommand {
    Help,
    Version,
    ListSpecs {
        out: Option<PathBuf>,
    },
    ScanJson {
        include_artifacts: bool,
        max_reclaimable_gb: Option<f64>,
        sarif: Option<PathBuf>,
        report_html: Option<PathBuf>,
        report_csv: Option<PathBuf>,
        out: Option<PathBuf>,
    },
    ScanProjects {
        roots: Vec<PathBuf>,
        stale_days: u64,
        max_reclaimable_gb: Option<f64>,
        sarif: Option<PathBuf>,
        report_html: Option<PathBuf>,
        out: Option<PathBuf>,
    },
    ScanHygiene {
        roots: Vec<PathBuf>,
        sarif: Option<PathBuf>,
        out: Option<PathBuf>,
    },
    Ledger {
        out: Option<PathBuf>,
    },
    ScanDupes {
        roots: Vec<PathBuf>,
        min_dupe_bytes: u64,
        trust_cas_names: bool,
        max_reclaimable_gb: Option<f64>,
        sarif: Option<PathBuf>,
        out: Option<PathBuf>,
    },
    ScanEnvs {
        roots: Vec<PathBuf>,
        stale_days: u64,
        reclaim_orphans: bool,
        dry_run: bool,
        to_trash: bool,
        max_reclaimable_gb: Option<f64>,
        sarif: Option<PathBuf>,
        out: Option<PathBuf>,
    },
    CleanSafe {
        dry_run: bool,
        to_trash: bool,
        out: Option<PathBuf>,
    },
}

#[derive(Serialize)]
struct CliScanReport {
    version: &'static str,
    scanned_at: u64,
    elevated: bool,
    total_reclaimable_bytes: u64,
    safe_bytes: u64,
    rebuild_bytes: u64,
    review_bytes: u64,
    artifact_bytes: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    budget_gate: Option<sarif::BudgetGate>,
    drives: Vec<drives::Drive>,
    catalog: Vec<crate::CatalogItem>,
    artifacts: Vec<artifacts::Artifact>,
}

#[derive(Serialize)]
struct CliDupesReport {
    version: &'static str,
    scanned_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    budget_gate: Option<sarif::BudgetGate>,
    #[serde(flatten)]
    scan: blob_dupes::BlobDupReport,
}

#[derive(Serialize)]
struct CliEnvsReport {
    version: &'static str,
    scanned_at: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    budget_gate: Option<sarif::BudgetGate>,
    #[serde(skip_serializing_if = "Option::is_none")]
    reclaim_summary: Option<env_reclaimer::EnvReclaimSummary>,
    #[serde(flatten)]
    scan: env_reclaimer::EnvScanReport,
}

#[derive(Serialize)]
struct CliCleanCandidate {
    id: String,
    label_en: String,
    group: String,
    bytes: u64,
    files: u64,
    paths: Vec<PathBuf>,
}

#[derive(Serialize)]
struct CliCleanReport {
    version: &'static str,
    dry_run: bool,
    to_trash: bool,
    elevated: bool,
    candidate_count: usize,
    total_candidate_bytes: u64,
    freed_bytes: u64,
    removed_entries: u64,
    skipped_entries: u64,
    candidates: Vec<CliCleanCandidate>,
    results: Vec<cleaner::CleanResult>,
}

fn parse_non_negative_f64(flag: &str, raw: &str) -> Result<f64, String> {
    let val: f64 = raw
        .parse()
        .map_err(|_| format!("invalid numeric value for {flag}: {raw}"))?;
    if !val.is_finite() || val < 0.0 {
        return Err(format!("{flag} requires a finite non-negative number, got {raw}"));
    }
    Ok(val)
}

/// Parse CLI arguments (excluding `argv[0]`). Returns `Ok(None)` when no CLI mode flag is set so GUI launches.
pub fn parse_args(args: &[String]) -> Result<Option<CliCommand>, String> {
    if args.is_empty() {
        return Ok(None);
    }
    if args.len() == 2 && args[0] == AFTER_PID_FLAG {
        return Ok(None);
    }

    let mut mode: Option<&str> = None;
    let mut include_artifacts = false;
    let mut confirm_delete = false;
    let mut explicit_dry_run = false;
    let mut to_trash = false;
    let mut trust_cas_names = false;
    let mut dupes_roots: Vec<PathBuf> = Vec::new();
    let mut env_roots: Vec<PathBuf> = Vec::new();
    let mut stale_days: u64 = env_reclaimer::DEFAULT_STALE_DAYS;
    let mut reclaim_orphans = false;
    let mut min_dupe_bytes: u64 = blob_dupes::DEFAULT_MIN_BLOB_BYTES;
    let mut max_reclaimable_gb: Option<f64> = None;
    let mut sarif_out: Option<PathBuf> = None;
    let mut report_html: Option<PathBuf> = None;
    let mut report_csv: Option<PathBuf> = None;
    let mut out: Option<PathBuf> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => return Ok(Some(CliCommand::Help)),
            "-V" | "--version" => return Ok(Some(CliCommand::Version)),
            "--list-specs" => mode = Some("list-specs"),
            "--scan-json" => mode = Some("scan-json"),
            "--scan-dupes" => mode = Some("scan-dupes"),
            "--scan-envs" => mode = Some("scan-envs"),
            "--scan-projects" => mode = Some("scan-projects"),
            "--scan-hygiene" => mode = Some("scan-hygiene"),
            "--ledger" => mode = Some("ledger"),
            "--clean-safe" => mode = Some("clean-safe"),
            "--include-artifacts" => include_artifacts = true,
            "--trust-cas-names" => trust_cas_names = true,
            "--reclaim-orphans" => reclaim_orphans = true,
            "--dry-run" => explicit_dry_run = true,
            "--confirm-delete" => confirm_delete = true,
            "--trash" => to_trash = true,
            "--report-html" => {
                i += 1;
                let Some(p) = args.get(i) else {
                    return Err("missing path after --report-html".into());
                };
                report_html = Some(PathBuf::from(p));
            }
            "--report-csv" => {
                i += 1;
                let Some(p) = args.get(i) else {
                    return Err("missing path after --report-csv".into());
                };
                report_csv = Some(PathBuf::from(p));
            }
            "--env-root" | "--project-root" | "--hygiene-root" => {
                i += 1;
                let Some(p) = args.get(i) else {
                    return Err("missing directory after --env-root".into());
                };
                env_roots.push(PathBuf::from(p));
            }
            "--stale-days" => {
                i += 1;
                let Some(raw) = args.get(i) else {
                    return Err("missing integer after --stale-days".into());
                };
                stale_days = raw
                    .parse::<u64>()
                    .map_err(|_| format!("invalid non-negative integer for --stale-days: {raw}"))?;
            }
            "--dupes-root" => {
                i += 1;
                let Some(p) = args.get(i) else {
                    return Err("missing directory after --dupes-root".into());
                };
                dupes_roots.push(PathBuf::from(p));
            }
            "--min-dupe-mb" => {
                i += 1;
                let Some(raw) = args.get(i) else {
                    return Err("missing value after --min-dupe-mb".into());
                };
                let mb = parse_non_negative_f64("--min-dupe-mb", raw)?;
                min_dupe_bytes = ((mb * 1024.0 * 1024.0).round() as u64).max(1);
            }
            "--max-reclaimable-gb" => {
                i += 1;
                let Some(raw) = args.get(i) else {
                    return Err("missing value after --max-reclaimable-gb".into());
                };
                max_reclaimable_gb = Some(parse_non_negative_f64("--max-reclaimable-gb", raw)?);
            }
            "--sarif" => {
                i += 1;
                let Some(p) = args.get(i) else {
                    return Err("missing path after --sarif".into());
                };
                sarif_out = Some(PathBuf::from(p));
            }
            "--out" => {
                i += 1;
                let Some(p) = args.get(i) else {
                    return Err("missing path after --out".into());
                };
                out = Some(PathBuf::from(p));
            }
            "--after-pid" => {
                i += 1;
            }
            other if other.starts_with('-') => {
                return Err(format!("unknown flag: {other} (try --help)"));
            }
            _ => {}
        }
        i += 1;
    }

    match mode {
        None => {
            if max_reclaimable_gb.is_some()
                || sarif_out.is_some()
                || !dupes_roots.is_empty()
                || !env_roots.is_empty()
                || reclaim_orphans
            {
                return Err("specify a command mode such as --scan-json, --scan-dupes, or --scan-envs (try --help)".into());
            }
            Ok(None)
        }
        Some("list-specs") => Ok(Some(CliCommand::ListSpecs { out })),
        Some("scan-json") => Ok(Some(CliCommand::ScanJson {
            include_artifacts,
            max_reclaimable_gb,
            sarif: sarif_out,
            report_html,
            report_csv,
            out,
        })),
        Some("scan-projects") => Ok(Some(CliCommand::ScanProjects {
            roots: env_roots,
            stale_days,
            max_reclaimable_gb,
            sarif: sarif_out,
            report_html,
            out,
        })),
        Some("scan-hygiene") => Ok(Some(CliCommand::ScanHygiene {
            roots: env_roots,
            sarif: sarif_out,
            out,
        })),
        Some("ledger") => Ok(Some(CliCommand::Ledger { out })),
        Some("scan-dupes") => Ok(Some(CliCommand::ScanDupes {
            roots: dupes_roots,
            min_dupe_bytes,
            trust_cas_names,
            max_reclaimable_gb,
            sarif: sarif_out,
            out,
        })),
        Some("scan-envs") => {
            let dry_run = explicit_dry_run || !confirm_delete;
            Ok(Some(CliCommand::ScanEnvs {
                roots: env_roots,
                stale_days,
                reclaim_orphans,
                dry_run,
                to_trash,
                max_reclaimable_gb,
                sarif: sarif_out,
                out,
            }))
        }
        Some("clean-safe") => {
            let dry_run = explicit_dry_run || !confirm_delete;
            Ok(Some(CliCommand::CleanSafe { dry_run, to_trash, out }))
        }
        Some(_) => Ok(None),
    }
}

#[cfg(windows)]
fn attach_parent_console() {
    #[link(name = "kernel32")]
    extern "system" {
        fn AttachConsole(process_id: u32) -> i32;
    }
    const ATTACH_PARENT_PROCESS: u32 = u32::MAX;
    unsafe {
        let _ = AttachConsole(ATTACH_PARENT_PROCESS);
    }
}

#[cfg(not(windows))]
fn attach_parent_console() {}

fn write_file_creating_dirs(path: &PathBuf, text: &str) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        if !parent.as_os_str().is_empty() {
            std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
        }
    }
    std::fs::write(path, text).map_err(|e| e.to_string())
}

fn emit_output(text: &str, out: Option<&PathBuf>) -> Result<(), String> {
    if let Some(path) = out {
        write_file_creating_dirs(path, text)
    } else {
        println!("{text}");
        Ok(())
    }
}

fn measure_catalog() -> Vec<crate::CatalogItem> {
    let cancel = AtomicBool::new(false);
    let entries = catalog::existing_entries();
    let mut items = parallel::map(entries, parallel::default_workers(), |entry| {
        let mut total = sizer::DirStats::default();
        for p in &entry.paths {
            let st = sizer::dir_stats(p, &cancel);
            total.bytes += st.bytes;
            total.files += st.files;
            total.denied += st.denied;
        }
        crate::CatalogItem {
            entry,
            bytes: total.bytes,
            files: total.files,
            denied: total.denied,
        }
    });
    items.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    items
}

pub fn help_text() -> String {
    format!(
        "SLClean {} - Local-first cache, artifact & AI weight blob inspector\n\n\
         USAGE:\n  \
           slclean                           Launch interactive desktop UI (Windows)\n  \
           slclean --version                 Print version and exit\n  \
           slclean --list-specs              Print all static detection rules as JSON\n  \
           slclean --scan-json [OPTIONS]     Run read-only parallel cache scan as JSON/SARIF/HTML/CSV\n  \
           slclean --scan-dupes [OPTIONS]    Audit duplicate HuggingFace/Ollama/artifact blobs\n  \
           slclean --scan-envs [OPTIONS]     Audit or reclaim orphaned/stale envs & targets\n  \
           slclean --scan-projects [OPTIONS] Group build artifacts by project with .git mtime staleness\n  \
           slclean --scan-hygiene [OPTIONS]  Find empty leaf directories and dangling symlinks\n  \
           slclean --ledger [OPTIONS]        Inspect persistent cleanup audit ledger & lifetime freed\n  \
           slclean --clean-safe [OPTIONS]    Preview or clean safe caches as JSON\n\n\
         OPTIONS:\n  \
           --include-artifacts        Include project build artifacts in --scan-json\n  \
           --report-html <FILE>       Export single-file HTML executive audit report\n  \
           --report-csv <FILE>        Export RFC 4180 CSV audit spreadsheet\n  \
           --project-root <DIR>       Root directory for --scan-projects / --scan-hygiene\n  \
           --dupes-root <DIR>         Scan DIR for duplicate blobs (repeatable; defaults to AI caches)\n  \
           --min-dupe-mb <MB>         Minimum blob size in MiB for --scan-dupes (default: 4)\n  \
           --trust-cas-names          Use verified sha256-<hex> / blobs/<hex> digests without re-reading\n  \
           --env-root <DIR>           Scan DIR for orphaned/stale environments (repeatable)\n  \
           --stale-days <DAYS>        Inactivity threshold in days for --scan-envs (default: 30)\n  \
           --reclaim-orphans          Reclaim broken/manifest-less orphan envs in --scan-envs\n  \
           --max-reclaimable-gb <GB>  Exit with code 3 if reclaimable/duplicate GiB exceeds threshold\n  \
           --sarif <FILE>             Write OASIS SARIF v2.1.0 report to FILE\n  \
           --dry-run                  Preview safe cache cleanup without deleting (default)\n  \
           --confirm-delete           Execute safe cache cleanup (required to delete files)\n  \
           --trash                    Send cleaned safe caches to the Recycle Bin / Trash\n  \
           --out <FILE>               Write JSON output to FILE instead of stdout\n  \
           -h, --help                 Print this help message\n",
        env!("CARGO_PKG_VERSION")
    )
}

pub fn execute_command(cmd: CliCommand) -> Result<i32, String> {
    match cmd {
        CliCommand::Help => {
            emit_output(&help_text(), None)?;
            Ok(0)
        }
        CliCommand::Version => {
            emit_output(&format!("SLClean {}", env!("CARGO_PKG_VERSION")), None)?;
            Ok(0)
        }
        CliCommand::ListSpecs { out } => {
            let specs = catalog_specs::spec_metadata();
            let json = serde_json::to_string_pretty(&specs).map_err(|e| e.to_string())?;
            emit_output(&json, out.as_ref())?;
            Ok(0)
        }
        CliCommand::ScanJson {
            include_artifacts,
            max_reclaimable_gb,
            sarif,
            report_html,
            report_csv,
            out,
        } => {
            let catalog_items = measure_catalog();
            let mut safe_bytes = 0u64;
            let mut rebuild_bytes = 0u64;
            let mut review_bytes = 0u64;
            for item in &catalog_items {
                match item.entry.safety {
                    catalog::Safety::Safe => safe_bytes += item.bytes,
                    catalog::Safety::Rebuild => rebuild_bytes += item.bytes,
                    catalog::Safety::Review => review_bytes += item.bytes,
                }
            }
            let found_artifacts = if include_artifacts {
                if let Some(roots) = catalog::Roots::detect() {
                    let catalog_paths: Vec<PathBuf> = catalog::existing_entries()
                        .into_iter()
                        .filter(|e| e.group != "temp")
                        .flat_map(|e| e.paths)
                        .collect();
                    let saved_settings = settings::load_offline();
                    let plan = project_roots::plan(&roots, &saved_settings, &catalog_paths);
                    artifacts::find_artifacts(&plan, &Arc::new(AtomicBool::new(false)), |_| {})
                } else {
                    Vec::new()
                }
            } else {
                Vec::new()
            };
            let artifact_bytes: u64 = found_artifacts.iter().map(|a| a.bytes).sum();
            let gated_reclaimable = safe_bytes + rebuild_bytes + artifact_bytes;
            let budget_gate = max_reclaimable_gb.map(|gb| sarif::BudgetGate::evaluate(gb, gated_reclaimable));

            if let Some(sarif_path) = sarif.as_ref() {
                let sarif_doc = sarif::build_scan_sarif(&catalog_items, &found_artifacts, budget_gate.as_ref());
                let sarif_json = serde_json::to_string_pretty(&sarif_doc).map_err(|e| e.to_string())?;
                write_file_creating_dirs(sarif_path, &sarif_json)?;
            }

            let scanned_at = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            if let Some(html_path) = report_html.as_ref() {
                let proj_rep = if found_artifacts.is_empty() {
                    None
                } else {
                    Some(projects::group_artifacts_by_project(&found_artifacts, projects::DEFAULT_STALE_PROJECT_DAYS, Some(scanned_at)))
                };
                let led = ledger::load_default().summary();
                let html = report_export::render_html(&catalog_items, proj_rep.as_ref(), Some(&led), scanned_at);
                write_file_creating_dirs(html_path, &html)?;
            }
            if let Some(csv_path) = report_csv.as_ref() {
                let csv = report_export::render_csv(&catalog_items, &found_artifacts);
                write_file_creating_dirs(csv_path, &csv)?;
            }
            let budget_exceeded = budget_gate.as_ref().map(|g| g.exceeded).unwrap_or(false);
            let report = CliScanReport {
                version: env!("CARGO_PKG_VERSION"),
                scanned_at,
                elevated: elevation::is_elevated(),
                total_reclaimable_bytes: safe_bytes + rebuild_bytes + review_bytes + artifact_bytes,
                safe_bytes,
                rebuild_bytes,
                review_bytes,
                artifact_bytes,
                budget_gate: budget_gate.clone(),
                drives: drives::list_drives(),
                catalog: catalog_items,
                artifacts: found_artifacts,
            };
            let json = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
            emit_output(&json, out.as_ref())?;

            if let Some(gate) = budget_gate.filter(|_| budget_exceeded) {
                eprintln!(
                    "slclean budget exceeded: {:.3} GiB reclaimable exceeds --max-reclaimable-gb {:.3}",
                    gate.actual_reclaimable_gb, gate.max_reclaimable_gb
                );
                return Ok(EXIT_BUDGET_EXCEEDED);
            }
            Ok(0)
        }
        CliCommand::ScanDupes {
            roots,
            min_dupe_bytes,
            trust_cas_names,
            max_reclaimable_gb,
            sarif,
            out,
        } => {
            let effective_roots = if roots.is_empty() {
                blob_dupes::default_ai_and_artifact_roots()
            } else {
                roots
            };
            let scan = blob_dupes::scan_blobs(
                &effective_roots,
                min_dupe_bytes,
                trust_cas_names,
                &AtomicBool::new(false),
            );
            let budget_gate = max_reclaimable_gb.map(|gb| sarif::BudgetGate::evaluate(gb, scan.total_wasted_bytes));
            let budget_exceeded = budget_gate.as_ref().map(|g| g.exceeded).unwrap_or(false);

            if let Some(sarif_path) = sarif.as_ref() {
                let sarif_doc = sarif::build_dupes_sarif(&scan, budget_gate.as_ref());
                let sarif_json = serde_json::to_string_pretty(&sarif_doc).map_err(|e| e.to_string())?;
                write_file_creating_dirs(sarif_path, &sarif_json)?;
            }

            let scanned_at = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let report = CliDupesReport {
                version: env!("CARGO_PKG_VERSION"),
                scanned_at,
                budget_gate: budget_gate.clone(),
                scan,
            };
            let json = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
            emit_output(&json, out.as_ref())?;

            if let Some(gate) = budget_gate.filter(|_| budget_exceeded) {
                eprintln!(
                    "slclean duplicate budget exceeded: {:.3} GiB wasted exceeds --max-reclaimable-gb {:.3}",
                    gate.actual_reclaimable_gb, gate.max_reclaimable_gb
                );
                return Ok(EXIT_BUDGET_EXCEEDED);
            }
            Ok(0)
        }
        CliCommand::ScanEnvs {
            roots,
            stale_days,
            reclaim_orphans,
            dry_run,
            to_trash,
            max_reclaimable_gb,
            sarif,
            out,
        } => {
            let effective_roots = if roots.is_empty() {
                env_reclaimer::default_env_roots()
            } else {
                roots
            };
            let scan = env_reclaimer::scan_environments(
                &effective_roots,
                stale_days,
                &AtomicBool::new(false),
            );
            let reclaim_summary = if reclaim_orphans {
                Some(env_reclaimer::reclaim_environments(
                    &scan, dry_run, true, to_trash,
                ))
            } else {
                None
            };
            let remaining_reclaimable = if let Some(ref sum) = reclaim_summary {
                if !sum.dry_run {
                    scan.total_reclaimable_bytes.saturating_sub(sum.freed_bytes)
                } else {
                    scan.total_reclaimable_bytes
                }
            } else {
                scan.total_reclaimable_bytes
            };

            let budget_gate =
                max_reclaimable_gb.map(|gb| sarif::BudgetGate::evaluate(gb, remaining_reclaimable));
            let budget_exceeded = budget_gate.as_ref().map(|g| g.exceeded).unwrap_or(false);

            if let Some(sarif_path) = sarif.as_ref() {
                let sarif_doc = sarif::build_envs_sarif(&scan, budget_gate.as_ref());
                let sarif_json =
                    serde_json::to_string_pretty(&sarif_doc).map_err(|e| e.to_string())?;
                write_file_creating_dirs(sarif_path, &sarif_json)?;
            }

            let scanned_at = SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .map(|d| d.as_secs())
                .unwrap_or(0);
            let report = CliEnvsReport {
                version: env!("CARGO_PKG_VERSION"),
                scanned_at,
                budget_gate: budget_gate.clone(),
                reclaim_summary,
                scan,
            };
            let json = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
            emit_output(&json, out.as_ref())?;

            if let Some(gate) = budget_gate.filter(|_| budget_exceeded) {
                eprintln!(
                    "slclean environment budget exceeded: {:.3} GiB reclaimable exceeds --max-reclaimable-gb {:.3}",
                    gate.actual_reclaimable_gb, gate.max_reclaimable_gb
                );
                return Ok(EXIT_BUDGET_EXCEEDED);
            }
            Ok(0)
        }
        CliCommand::CleanSafe {
            dry_run,
            to_trash,
            out,
        } => {
            let elevated = elevation::is_elevated();
            let catalog_items = measure_catalog();
            let eligible: Vec<&crate::CatalogItem> = catalog_items
                .iter()
                .filter(|it| {
                    it.entry.safety == catalog::Safety::Safe
                        && it.bytes > 0
                        && (elevated || !it.entry.needs_admin)
                })
                .collect();
            let candidates: Vec<CliCleanCandidate> = eligible
                .iter()
                .map(|it| CliCleanCandidate {
                    id: it.entry.id.clone(),
                    label_en: it.entry.label.en.clone(),
                    group: it.entry.group.clone(),
                    bytes: it.bytes,
                    files: it.files,
                    paths: it.entry.paths.clone(),
                })
                .collect();
            let total_candidate_bytes: u64 = candidates.iter().map(|c| c.bytes).sum();
            let mut results = Vec::new();
            if !dry_run {
                let mut reqs = Vec::new();
                for it in &eligible {
                    let req = cleaner::CleanItem {
                        id: it.entry.id.clone(),
                        paths: it.entry.paths.clone(),
                        keep_root: it.entry.keep_root,
                    };
                    results.push(cleaner::clean_one(&req, to_trash));
                    reqs.push(req);
                }
                let _ = ledger::append_default(&reqs, &results, to_trash, elevated, "cli");
            }
            let freed_bytes: u64 = results.iter().map(|r| r.freed).sum();
            let removed_entries: u64 = results.iter().map(|r| r.removed).sum();
            let skipped_entries: u64 = results.iter().map(|r| r.skipped).sum();
            let report = CliCleanReport {
                version: env!("CARGO_PKG_VERSION"),
                dry_run,
                to_trash,
                elevated,
                candidate_count: candidates.len(),
                total_candidate_bytes,
                freed_bytes,
                removed_entries,
                skipped_entries,
                candidates,
                results,
            };
            let json = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
            emit_output(&json, out.as_ref())?;
            Ok(0)
        }
        CliCommand::ScanProjects {
            roots,
            stale_days,
            max_reclaimable_gb,
            sarif,
            report_html,
            out,
        } => {
            let plan = if roots.is_empty() {
                let saved = settings::load_offline();
                if let Some(r) = catalog::Roots::detect() {
                    project_roots::plan(&r, &saved, &[])
                } else {
                    project_roots::ScanPlan { roots: Vec::new(), excluded: Vec::new() }
                }
            } else {
                project_roots::ScanPlan { roots, excluded: Vec::new() }
            };
            let found = artifacts::find_artifacts(&plan, &Arc::new(AtomicBool::new(false)), |_| {});
            let now = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let proj_report = projects::group_artifacts_by_project(&found, stale_days, Some(now));
            let budget_gate = max_reclaimable_gb.map(|gb| sarif::BudgetGate::evaluate(gb, proj_report.stale_reclaimable_bytes));
            if let Some(sarif_path) = sarif.as_ref() {
                let doc = sarif::build_projects_sarif(&proj_report, budget_gate.as_ref());
                write_file_creating_dirs(sarif_path, &serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?)?;
            }
            if let Some(html_path) = report_html.as_ref() {
                let led = ledger::load_default().summary();
                let html = report_export::render_html(&[], Some(&proj_report), Some(&led), now);
                write_file_creating_dirs(html_path, &html)?;
            }
            let json = serde_json::to_string_pretty(&proj_report).map_err(|e| e.to_string())?;
            emit_output(&json, out.as_ref())?;
            if budget_gate.as_ref().map(|g| g.exceeded).unwrap_or(false) {
                return Ok(EXIT_BUDGET_EXCEEDED);
            }
            Ok(0)
        }
        CliCommand::ScanHygiene { roots, sarif, out } => {
            let effective = if roots.is_empty() {
                dirs::home_dir().into_iter().collect()
            } else {
                roots
            };
            let report = dead_links::scan(&effective, 5);
            if let Some(sarif_path) = sarif.as_ref() {
                let doc = sarif::build_hygiene_sarif(&report);
                write_file_creating_dirs(sarif_path, &serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?)?;
            }
            let json = serde_json::to_string_pretty(&report).map_err(|e| e.to_string())?;
            emit_output(&json, out.as_ref())?;
            Ok(0)
        }
        CliCommand::Ledger { out } => {
            let led = ledger::load_default();
            let payload = serde_json::json!({
                "version": env!("CARGO_PKG_VERSION"),
                "summary": led.summary(),
                "runs": led.runs,
            });
            emit_output(&serde_json::to_string_pretty(&payload).map_err(|e| e.to_string())?, out.as_ref())?;
            Ok(0)
        }
    }
}

pub fn try_run_from_env() -> Option<i32> {
    let raw: Vec<String> = std::env::args().skip(1).collect();
    let parsed = match parse_args(&raw) {
        Ok(None) => return None,
        Ok(Some(cmd)) => cmd,
        Err(err) => {
            attach_parent_console();
            eprintln!("slclean: {err}");
            return Some(2);
        }
    };

    attach_parent_console();
    match execute_command(parsed) {
        Ok(code) => Some(code),
        Err(err) => {
            eprintln!("slclean error: {err}");
            Some(1)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn vec_str(items: &[&str]) -> Vec<String> {
        items.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn gui_launch_args_return_none() {
        assert_eq!(parse_args(&[]).unwrap(), None);
        assert_eq!(parse_args(&vec_str(&["--after-pid", "1234"])).unwrap(), None);
    }

    #[test]
    fn scan_and_dupes_flags_parse_cleanly() {
        assert_eq!(
            parse_args(&vec_str(&[
                "--scan-json",
                "--include-artifacts",
                "--max-reclaimable-gb",
                "12.5",
                "--sarif",
                "audit.sarif",
                "--out",
                "report.json"
            ]))
            .unwrap(),
            Some(CliCommand::ScanJson {
                include_artifacts: true,
                max_reclaimable_gb: Some(12.5),
                sarif: Some(PathBuf::from("audit.sarif")),
                report_html: None,
                report_csv: None,
                out: Some(PathBuf::from("report.json")),
            })
        );
        assert_eq!(
            parse_args(&vec_str(&[
                "--scan-dupes",
                "--dupes-root",
                "/tmp/hf",
                "--dupes-root",
                "/tmp/ollama",
                "--min-dupe-mb",
                "2",
                "--trust-cas-names",
                "--max-reclaimable-gb",
                "5"
            ]))
            .unwrap(),
            Some(CliCommand::ScanDupes {
                roots: vec![PathBuf::from("/tmp/hf"), PathBuf::from("/tmp/ollama")],
                min_dupe_bytes: 2 * 1024 * 1024,
                trust_cas_names: true,
                max_reclaimable_gb: Some(5.0),
                sarif: None,
                out: None,
            })
        );
    }

    #[test]
    fn clean_safe_defaults_to_dry_run_unless_confirmed() {
        assert_eq!(
            parse_args(&vec_str(&["--clean-safe"])).unwrap(),
            Some(CliCommand::CleanSafe {
                dry_run: true,
                to_trash: false,
                out: None,
            })
        );
        assert_eq!(
            parse_args(&vec_str(&["--clean-safe", "--confirm-delete", "--trash"])).unwrap(),
            Some(CliCommand::CleanSafe {
                dry_run: false,
                to_trash: true,
                out: None,
            })
        );
        assert_eq!(
            parse_args(&vec_str(&["--clean-safe", "--confirm-delete", "--dry-run"])).unwrap(),
            Some(CliCommand::CleanSafe {
                dry_run: true,
                to_trash: false,
                out: None,
            })
        );
    }

    #[test]
    fn scan_dupes_end_to_end_writes_json_sarif_and_enforces_budget_exit_code() {
        let root = std::env::temp_dir().join(format!("slclean-cli-dupes-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);

        let payload = vec![0x5Au8; 64 * 1024];
        let a = root.join(".ollama/models/blobs/model-a.gguf");
        let b = root.join("Downloads/model-a-copy.gguf");
        std::fs::create_dir_all(a.parent().unwrap()).unwrap();
        std::fs::create_dir_all(b.parent().unwrap()).unwrap();
        std::fs::write(&a, &payload).unwrap();
        std::fs::write(&b, &payload).unwrap();

        let json_out = root.join("out/dupes.json");
        let sarif_out = root.join("out/dupes.sarif");

        // 1. High budget (1 GiB) -> exit code 0
        let code_ok = execute_command(CliCommand::ScanDupes {
            roots: vec![root.clone()],
            min_dupe_bytes: 1024,
            trust_cas_names: false,
            max_reclaimable_gb: Some(1.0),
            sarif: Some(sarif_out.clone()),
            out: Some(json_out.clone()),
        })
        .unwrap();
        assert_eq!(code_ok, 0);

        // 2. Zero budget (0.0 GiB) -> exit code 3 (EXIT_BUDGET_EXCEEDED) and SLC005 in SARIF
        let code_fail = execute_command(CliCommand::ScanDupes {
            roots: vec![root.clone()],
            min_dupe_bytes: 1024,
            trust_cas_names: false,
            max_reclaimable_gb: Some(0.0),
            sarif: Some(sarif_out.clone()),
            out: Some(json_out.clone()),
        })
        .unwrap();
        assert_eq!(code_fail, EXIT_BUDGET_EXCEEDED);

        let json_val: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&json_out).unwrap()).unwrap();
        assert_eq!(json_val["total_wasted_bytes"].as_u64(), Some(64 * 1024));
        assert_eq!(json_val["budget_gate"]["exceeded"].as_bool(), Some(true));

        let sarif_val: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&sarif_out).unwrap()).unwrap();
        assert_eq!(sarif_val["version"].as_str(), Some("2.1.0"));
        let results = sarif_val["runs"][0]["results"].as_array().unwrap();
        assert!(results.iter().any(|r| r["ruleId"] == "SLC004"));
        assert!(results.iter().any(|r| r["ruleId"] == "SLC005"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn unknown_and_invalid_flags_are_rejected() {
        assert!(parse_args(&vec_str(&["--bogus"])).is_err());
        assert!(parse_args(&vec_str(&["--scan-json", "--out"])).is_err());
        assert!(parse_args(&vec_str(&["--scan-dupes", "--max-reclaimable-gb", "-1"])).is_err());
        assert!(parse_args(&vec_str(&["--max-reclaimable-gb", "5"])).is_err());
        assert!(parse_args(&vec_str(&["--scan-envs", "--stale-days", "abc"])).is_err());
    }

    #[test]
    fn scan_projects_hygiene_and_html_report_end_to_end() {
        let root = std::env::temp_dir().join(format!("slclean-cli-v070-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);

        let proj = root.join("old-rust-plugin");
        let target_dir = proj.join("target");
        std::fs::create_dir_all(&target_dir).unwrap();
        std::fs::write(proj.join("Cargo.toml"), b"[package]\nname = \"demo\"\n").unwrap();
        std::fs::write(target_dir.join("artifact.bin"), vec![0x42u8; 32 * 1024]).unwrap();

        let empty_leaf = root.join("empty-folder");
        std::fs::create_dir_all(&empty_leaf).unwrap();

        let proj_json = root.join("out/projects.json");
        let proj_sarif = root.join("out/projects.sarif");
        let proj_html = root.join("out/projects.html");
        let code = execute_command(CliCommand::ScanProjects {
            roots: vec![root.clone()],
            stale_days: 0,
            max_reclaimable_gb: Some(1.0),
            sarif: Some(proj_sarif.clone()),
            report_html: Some(proj_html.clone()),
            out: Some(proj_json.clone()),
        })
        .unwrap();
        assert_eq!(code, 0);
        let parsed: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&proj_json).unwrap()).unwrap();
        assert_eq!(parsed["project_count"].as_u64(), Some(1));
        assert!(std::fs::read_to_string(&proj_html).unwrap().contains("old-rust-plugin"));
        assert!(std::fs::read_to_string(&proj_sarif).unwrap().contains("SLC009"));

        let hyg_json = root.join("out/hygiene.json");
        let hyg_sarif = root.join("out/hygiene.sarif");
        let h_code = execute_command(CliCommand::ScanHygiene {
            roots: vec![root.clone()],
            sarif: Some(hyg_sarif.clone()),
            out: Some(hyg_json.clone()),
        })
        .unwrap();
        assert_eq!(h_code, 0);
        assert!(std::fs::read_to_string(&hyg_sarif).unwrap().contains("SLC010"));

        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn scan_envs_end_to_end_writes_json_sarif_and_enforces_budget_gate() {
        let root = std::env::temp_dir().join(format!("slclean-cli-envs-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);

        let orphan_nm = root.join("abandoned-app").join("node_modules").join("pkg");
        std::fs::create_dir_all(&orphan_nm).unwrap();
        std::fs::write(orphan_nm.join("index.js"), vec![b'z'; 96 * 1024]).unwrap();

        let vhdx = root.join("docker").join("docker_data.vhdx");
        std::fs::create_dir_all(vhdx.parent().unwrap()).unwrap();
        std::fs::write(&vhdx, vec![0u8; 64 * 1024]).unwrap();

        let json_out = root.join("out/envs.json");
        let sarif_out = root.join("out/envs.sarif");

        let parsed = parse_args(&vec_str(&[
            "--scan-envs",
            "--env-root",
            root.to_str().unwrap(),
            "--stale-days",
            "14",
            "--max-reclaimable-gb",
            "0",
            "--sarif",
            sarif_out.to_str().unwrap(),
            "--out",
            json_out.to_str().unwrap(),
        ]))
        .unwrap()
        .unwrap();

        let code = execute_command(parsed).unwrap();
        assert_eq!(code, EXIT_BUDGET_EXCEEDED);

        let json_val: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&json_out).unwrap()).unwrap();
        assert_eq!(json_val["orphan_count"].as_u64(), Some(1));
        assert_eq!(json_val["guarded_vhdx_count"].as_u64(), Some(1));
        assert_eq!(json_val["budget_gate"]["exceeded"].as_bool(), Some(true));

        let sarif_val: serde_json::Value =
            serde_json::from_str(&std::fs::read_to_string(&sarif_out).unwrap()).unwrap();
        let results = sarif_val["runs"][0]["results"].as_array().unwrap();
        assert!(results.iter().any(|r| r["ruleId"] == "SLC005"));
        assert!(results.iter().any(|r| r["ruleId"] == "SLC006"));
        assert!(results.iter().any(|r| r["ruleId"] == "SLC008"));

        let _ = std::fs::remove_dir_all(&root);
    }
}
