//! Chế độ dòng lệnh không giao diện (headless CLI) cho CI, quản trị viên và người dùng
//! PowerShell/Terminal: liệt kê quy tắc, xuất báo cáo JSON dung lượng cache/artifact, hoặc
//! chạy xem trước (`--dry-run`) các mục `Safe`. Vẫn giữ nguyên tắc cốt lõi của SLClean:
//! không bao giờ tự xoá nếu người gọi không truyền tường minh `--confirm-delete`.

use crate::{artifacts, catalog, catalog_specs, cleaner, drives, elevation, parallel, project_roots, settings, sizer};
use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::AtomicBool;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, PartialEq, Eq)]
pub enum CliCommand {
    Help,
    Version,
    ListSpecs { out: Option<PathBuf> },
    ScanJson { include_artifacts: bool, out: Option<PathBuf> },
    CleanSafe { dry_run: bool, to_trash: bool, out: Option<PathBuf> },
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
    drives: Vec<drives::Drive>,
    catalog: Vec<crate::CatalogItem>,
    artifacts: Vec<artifacts::Artifact>,
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

/// Phân tích danh sách đối số (bỏ argv[0]). Trả `Ok(None)` nếu không có cờ CLI để khởi động GUI.
pub fn parse_args(args: &[String]) -> Result<Option<CliCommand>, String> {
    if args.is_empty() {
        return Ok(None);
    }
    // Cờ nội bộ `--after-pid <pid>` dùng khi mở lại GUI với quyền admin.
    if args.len() == 2 && args[0] == crate::single_instance::AFTER_PID_FLAG {
        return Ok(None);
    }

    let mut mode: Option<&str> = None;
    let mut include_artifacts = false;
    let mut confirm_delete = false;
    let mut explicit_dry_run = false;
    let mut to_trash = false;
    let mut out: Option<PathBuf> = None;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "-h" | "--help" => return Ok(Some(CliCommand::Help)),
            "-V" | "--version" => return Ok(Some(CliCommand::Version)),
            "--list-specs" => mode = Some("list-specs"),
            "--scan-json" => mode = Some("scan-json"),
            "--clean-safe" => mode = Some("clean-safe"),
            "--include-artifacts" => include_artifacts = true,
            "--dry-run" => explicit_dry_run = true,
            "--confirm-delete" => confirm_delete = true,
            "--trash" => to_trash = true,
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
        None => Ok(None),
        Some("list-specs") => Ok(Some(CliCommand::ListSpecs { out })),
        Some("scan-json") => Ok(Some(CliCommand::ScanJson { include_artifacts, out })),
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

fn emit_output(text: &str, out: Option<&PathBuf>) -> Result<(), String> {
    if let Some(path) = out {
        if let Some(parent) = path.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
            }
        }
        std::fs::write(path, text).map_err(|e| e.to_string())
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
        crate::CatalogItem { entry, bytes: total.bytes, files: total.files, denied: total.denied }
    });
    items.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    items
}

fn help_text() -> String {
    format!(
        "SLClean {} - Local-first Windows cache & artifact inspector\n\n\
         USAGE:\n  \
           slclean.exe                        Launch interactive desktop UI\n  \
           slclean.exe --version              Print version and exit\n  \
           slclean.exe --list-specs           Print all static detection rules as JSON\n  \
           slclean.exe --scan-json [OPTIONS]  Run read-only parallel scan and output JSON\n  \
           slclean.exe --clean-safe [OPTIONS] Preview or clean safe caches as JSON\n\n\
         OPTIONS:\n  \
           --include-artifacts   Include project build artifacts in --scan-json\n  \
           --dry-run             Preview safe cache cleanup without deleting (default)\n  \
           --confirm-delete      Execute safe cache cleanup (required to delete files)\n  \
           --trash               Send cleaned safe caches to the Recycle Bin\n  \
           --out <FILE>          Write JSON output to FILE instead of stdout\n  \
           -h, --help            Print this help message\n",
        env!("CARGO_PKG_VERSION")
    )
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
    let result = match parsed {
        CliCommand::Help => emit_output(&help_text(), None),
        CliCommand::Version => emit_output(&format!("SLClean {}", env!("CARGO_PKG_VERSION")), None),
        CliCommand::ListSpecs { out } => {
            let specs = catalog_specs::spec_metadata();
            serde_json::to_string_pretty(&specs)
                .map_err(|e| e.to_string())
                .and_then(|json| emit_output(&json, out.as_ref()))
        }
        CliCommand::ScanJson { include_artifacts, out } => {
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
            let scanned_at = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
            let report = CliScanReport {
                version: env!("CARGO_PKG_VERSION"),
                scanned_at,
                elevated: elevation::is_elevated(),
                total_reclaimable_bytes: safe_bytes + rebuild_bytes + review_bytes + artifact_bytes,
                safe_bytes,
                rebuild_bytes,
                review_bytes,
                artifact_bytes,
                drives: drives::list_drives(),
                catalog: catalog_items,
                artifacts: found_artifacts,
            };
            serde_json::to_string_pretty(&report)
                .map_err(|e| e.to_string())
                .and_then(|json| emit_output(&json, out.as_ref()))
        }
        CliCommand::CleanSafe { dry_run, to_trash, out } => {
            let elevated = elevation::is_elevated();
            let catalog_items = measure_catalog();
            let eligible: Vec<&crate::CatalogItem> = catalog_items
                .iter()
                .filter(|it| it.entry.safety == catalog::Safety::Safe && it.bytes > 0 && (elevated || !it.entry.needs_admin))
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
                for it in &eligible {
                    let req = cleaner::CleanItem {
                        id: it.entry.id.clone(),
                        paths: it.entry.paths.clone(),
                        keep_root: it.entry.keep_root,
                    };
                    results.push(cleaner::clean_one(&req, to_trash));
                }
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
            serde_json::to_string_pretty(&report)
                .map_err(|e| e.to_string())
                .and_then(|json| emit_output(&json, out.as_ref()))
        }
    };

    match result {
        Ok(()) => Some(0),
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
    fn scan_and_list_flags_parse_cleanly() {
        assert_eq!(
            parse_args(&vec_str(&["--scan-json", "--include-artifacts", "--out", "report.json"])).unwrap(),
            Some(CliCommand::ScanJson { include_artifacts: true, out: Some(PathBuf::from("report.json")) })
        );
        assert_eq!(
            parse_args(&vec_str(&["--list-specs"])).unwrap(),
            Some(CliCommand::ListSpecs { out: None })
        );
    }

    #[test]
    fn clean_safe_defaults_to_dry_run_unless_confirmed() {
        assert_eq!(
            parse_args(&vec_str(&["--clean-safe"])).unwrap(),
            Some(CliCommand::CleanSafe { dry_run: true, to_trash: false, out: None })
        );
        assert_eq!(
            parse_args(&vec_str(&["--clean-safe", "--confirm-delete", "--trash"])).unwrap(),
            Some(CliCommand::CleanSafe { dry_run: false, to_trash: true, out: None })
        );
        // Explicit --dry-run always wins even if --confirm-delete was also passed.
        assert_eq!(
            parse_args(&vec_str(&["--clean-safe", "--confirm-delete", "--dry-run"])).unwrap(),
            Some(CliCommand::CleanSafe { dry_run: true, to_trash: false, out: None })
        );
    }

    #[test]
    fn unknown_flags_are_rejected() {
        assert!(parse_args(&vec_str(&["--bogus"])).is_err());
        assert!(parse_args(&vec_str(&["--scan-json", "--out"])).is_err());
    }
}
