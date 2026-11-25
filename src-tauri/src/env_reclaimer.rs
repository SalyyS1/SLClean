//! Orphaned developer environment, stale toolchain target, and container layer
//! age-weighted reclaimer (`--scan-envs`, SARIF rules `SLC006`..`SLC008`).
//!
//! Discovers abandoned Python virtualenvs (`pyvenv.cfg` with missing base interpreter
//! or missing project manifest), orphaned `node_modules` directories, stale Rust
//! `target/` / Gradle / CMake build outputs, and dormant container image layers while
//! guarding active WSL2/Docker `.vhdx` virtual disks against destructive deletion.

use crate::{catalog, cleaner, project_roots, settings, sizer};
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

pub const DEFAULT_STALE_DAYS: u64 = 30;
const SECS_PER_DAY: f64 = 86_400.0;
const GIB_F64: f64 = 1024.0 * 1024.0 * 1024.0;
const CACHEDIR_SIGNATURE_PREFIX: &str = "Signature: 8a477f597d28d172789f06886806bc55";
const MAX_WALK_DEPTH: usize = 6;

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct EnvCandidate {
    pub path: PathBuf,
    pub category: &'static str,
    pub ecosystem: &'static str,
    pub sarif_rule: &'static str,
    pub status: &'static str,
    pub bytes: u64,
    pub files: u64,
    pub last_modified_epoch: u64,
    pub age_days: f64,
    pub priority_score: f64,
    pub reclaim_safe: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
    pub remediation: String,
}

#[derive(Clone, Debug, Default, PartialEq, Serialize)]
pub struct EnvScanReport {
    pub roots_scanned: Vec<PathBuf>,
    pub stale_days_threshold: u64,
    pub candidate_count: usize,
    pub orphan_count: usize,
    pub stale_count: usize,
    pub guarded_vhdx_count: usize,
    pub total_reclaimable_bytes: u64,
    pub orphan_reclaimable_bytes: u64,
    pub guarded_vhdx_bytes: u64,
    pub candidates: Vec<EnvCandidate>,
}

#[derive(Clone, Debug, PartialEq, Serialize)]
pub struct EnvReclaimSummary {
    pub dry_run: bool,
    pub only_orphans: bool,
    pub to_trash: bool,
    pub targeted_count: usize,
    pub targeted_bytes: u64,
    pub freed_bytes: u64,
    pub removed_entries: u64,
    pub skipped_entries: u64,
}

pub fn default_env_roots() -> Vec<PathBuf> {
    let mut out = Vec::new();
    if let Some(roots) = catalog::Roots::detect() {
        let saved = settings::load_offline();
        let plan = project_roots::plan(&roots, &saved, &[]);
        for r in plan.roots {
            if r.exists() && !out.contains(&r) {
                out.push(r);
            }
        }
        // Common centralized virtualenv / Conda / container storage homes
        for extra in [
            roots.home.join(".virtualenvs"),
            roots.home.join(".conda").join("envs"),
            roots.home.join("miniconda3").join("envs"),
            roots.home.join("anaconda3").join("envs"),
            roots.local.join("pypoetry").join("Cache").join("virtualenvs"),
            roots.local.join("hatch").join("env").join("virtual"),
            roots.local.join("Docker").join("wsl"),
            roots.local.join("Packages"),
        ] {
            if extra.exists() && !out.contains(&extra) {
                out.push(extra);
            }
        }
    }
    out
}

fn now_epoch_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn metadata_mtime_secs(meta: &fs::Metadata) -> u64 {
    meta.modified()
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

/// Computes the freshest modification timestamp across a directory root and its immediate children
/// so touching any top-level marker or subdir updates liveness without an extra deep stat pass.
fn freshest_shallow_mtime(dir: &Path) -> u64 {
    let mut newest = fs::metadata(dir).map(|m| metadata_mtime_secs(&m)).unwrap_or(0);
    if let Ok(entries) = fs::read_dir(dir) {
        for entry in entries.flatten().take(64) {
            if let Ok(meta) = entry.metadata() {
                let ts = metadata_mtime_secs(&meta);
                if ts > newest {
                    newest = ts;
                }
            }
        }
    }
    newest
}

pub fn compute_priority_score(bytes: u64, age_days: f64, orphan_weight: f64, reclaim_safe: bool) -> f64 {
    if !reclaim_safe || bytes == 0 || orphan_weight <= 0.0 {
        return 0.0;
    }
    let gib = (bytes as f64) / GIB_F64;
    let age_factor = (1.0 + age_days.max(0.0)).ln().max(0.25);
    let raw = gib * age_factor * orphan_weight;
    (raw * 10_000.0).round() / 10_000.0
}

fn parse_pyvenv_home(pyvenv_cfg: &Path) -> Option<(PathBuf, Option<String>)> {
    let content = fs::read_to_string(pyvenv_cfg).ok()?;
    let mut home_path: Option<PathBuf> = None;
    let mut version: Option<String> = None;
    for line in content.lines() {
        let Some((k, v)) = line.split_once('=') else {
            continue;
        };
        let key = k.trim();
        let val = v.trim();
        if key.eq_ignore_ascii_case("home") && !val.is_empty() {
            home_path = Some(PathBuf::from(val));
        } else if key.eq_ignore_ascii_case("version") && !val.is_empty() {
            version = Some(val.to_string());
        }
    }
    home_path.map(|h| (h, version))
}

fn has_python_project_manifest(parent: &Path) -> bool {
    const MANIFESTS: &[&str] = &[
        "pyproject.toml",
        "requirements.txt",
        "requirements-dev.txt",
        "Pipfile",
        "setup.py",
        "setup.cfg",
        "manage.py",
        "uv.lock",
        "poetry.lock",
    ];
    MANIFESTS.iter().any(|m| parent.join(m).is_file())
}

fn is_centralized_venv_store(parent: &Path) -> bool {
    let norm = parent.to_string_lossy().to_ascii_lowercase().replace('\\', "/");
    norm.ends_with("/.virtualenvs")
        || norm.contains("/pypoetry/cache/virtualenvs")
        || norm.contains("/hatch/env/virtual")
        || norm.ends_with("/envs")
}

fn has_valid_cachedir_tag(dir: &Path) -> bool {
    let tag_path = dir.join("CACHEDIR.TAG");
    let Ok(text) = fs::read_to_string(tag_path) else {
        return false;
    };
    text.lines()
        .take(4)
        .any(|l| l.trim().starts_with(CACHEDIR_SIGNATURE_PREFIX))
}

fn is_verified_rust_target(dir: &Path) -> bool {
    if has_valid_cachedir_tag(dir) {
        return true;
    }
    dir.join(".rustc_info.json").is_file()
        || dir.join("debug").join(".fingerprint").is_dir()
        || dir.join("release").join(".fingerprint").is_dir()
}

fn is_vhdx_virtual_disk(name_lower: &str) -> bool {
    name_lower.ends_with(".vhdx")
}

fn is_container_layer_archive(name_lower: &str, parent_norm: &str) -> bool {
    (name_lower.ends_with(".tar") || name_lower.ends_with(".tar.gz") || name_lower.ends_with(".oci.tar"))
        && (parent_norm.contains("/docker")
            || parent_norm.contains("/containers")
            || parent_norm.contains("/buildkit")
            || parent_norm.contains("/wsl")
            || parent_norm.contains("/oci"))
}

struct ClassifiedDir {
    category: &'static str,
    ecosystem: &'static str,
    sarif_rule: &'static str,
    status: &'static str,
    orphan_weight: f64,
    reclaim_safe: bool,
    detail: Option<String>,
    remediation: String,
}

fn classify_directory(dir: &Path, age_days: f64, stale_days: u64) -> Option<ClassifiedDir> {
    let name = dir.file_name()?.to_str()?;
    let name_lower = name.to_ascii_lowercase();
    let parent = dir.parent();
    let is_stale = age_days >= (stale_days as f64);

    // 1. Python virtualenv (`pyvenv.cfg` present)
    let pyvenv_cfg = dir.join("pyvenv.cfg");
    if pyvenv_cfg.is_file() {
        if let Some((home_dir, ver)) = parse_pyvenv_home(&pyvenv_cfg) {
            if !home_dir.exists() {
                let ver_label = ver.map(|v| format!(" (Python {v})")).unwrap_or_default();
                return Some(ClassifiedDir {
                    category: "python_venv",
                    ecosystem: "python",
                    sarif_rule: "SLC006",
                    status: "broken_interpreter",
                    orphan_weight: 2.75,
                    reclaim_safe: true,
                    detail: Some(format!(
                        "Base interpreter home '{}'{} no longer exists on disk.",
                        home_dir.display(),
                        ver_label
                    )),
                    remediation: "Delete broken virtualenv and recreate with 'uv venv' or 'python -m venv'.".into(),
                });
            }
        }
        if let Some(p) = parent {
            if !is_centralized_venv_store(p) && !has_python_project_manifest(p) {
                return Some(ClassifiedDir {
                    category: "python_venv",
                    ecosystem: "python",
                    sarif_rule: "SLC006",
                    status: "missing_project_manifest",
                    orphan_weight: 2.50,
                    reclaim_safe: true,
                    detail: Some("No sibling pyproject.toml, requirements.txt, Pipfile, or setup.py found.".into()),
                    remediation: "Remove orphaned Python virtualenv left behind after project removal or rename.".into(),
                });
            }
        }
        if is_stale {
            return Some(ClassifiedDir {
                category: "python_venv",
                ecosystem: "python",
                sarif_rule: "SLC006",
                status: "stale_inactive",
                orphan_weight: 1.0,
                reclaim_safe: true,
                detail: Some(format!("Python virtualenv inactive for {:.0} days.", age_days)),
                remediation: "Remove inactive virtualenv and recreate on demand from lockfile.".into(),
            });
        }
        return None;
    }

    // 2. Conda environment (`conda-meta/history` present)
    if dir.join("conda-meta").is_dir() {
        if is_stale {
            return Some(ClassifiedDir {
                category: "conda_env",
                ecosystem: "conda",
                sarif_rule: "SLC006",
                status: "stale_inactive",
                orphan_weight: 1.15,
                reclaim_safe: true,
                detail: Some(format!("Conda environment inactive for {:.0} days.", age_days)),
                remediation: "Export environment.yml if needed and remove via 'conda env remove -p <path>'.".into(),
            });
        }
        return None;
    }

    // 3. Node.js `node_modules`
    if name_lower == "node_modules" {
        let has_pkg = parent.map(|p| p.join("package.json").is_file()).unwrap_or(false);
        if !has_pkg {
            return Some(ClassifiedDir {
                category: "node_modules",
                ecosystem: "nodejs",
                sarif_rule: "SLC006",
                status: "missing_project_manifest",
                orphan_weight: 2.50,
                reclaim_safe: true,
                detail: Some("Sibling package.json is missing; orphaned node_modules tree.".into()),
                remediation: "Delete orphaned node_modules directory.".into(),
            });
        }
        if is_stale {
            return Some(ClassifiedDir {
                category: "node_modules",
                ecosystem: "nodejs",
                sarif_rule: "SLC006",
                status: "stale_inactive",
                orphan_weight: 1.0,
                reclaim_safe: true,
                detail: Some(format!("node_modules untouched for {:.0} days.", age_days)),
                remediation: "Reclaim node_modules and restore later with 'npm ci' or 'pnpm install'.".into(),
            });
        }
        return None;
    }

    // 4. Rust Cargo `target/` (verified via CACHEDIR.TAG or .fingerprint)
    if name_lower == "target" && is_verified_rust_target(dir) {
        let has_cargo = parent.map(|p| p.join("Cargo.toml").is_file()).unwrap_or(false);
        if !has_cargo {
            return Some(ClassifiedDir {
                category: "rust_target",
                ecosystem: "rust",
                sarif_rule: "SLC007",
                status: "missing_project_manifest",
                orphan_weight: 2.50,
                reclaim_safe: true,
                detail: Some("Verified Cargo build target lacks a sibling Cargo.toml manifest.".into()),
                remediation: "Delete orphaned Rust target directory.".into(),
            });
        }
        if is_stale {
            return Some(ClassifiedDir {
                category: "rust_target",
                ecosystem: "rust",
                sarif_rule: "SLC007",
                status: "stale_inactive",
                orphan_weight: 1.15,
                reclaim_safe: true,
                detail: Some(format!("Cargo build target inactive for {:.0} days.", age_days)),
                remediation: "Run 'cargo clean' or delete target/ to reclaim compiled intermediates.".into(),
            });
        }
        return None;
    }

    // 5. Gradle project cache (`.gradle` with `caches` or task history)
    if name_lower == ".gradle" && (dir.join("caches").is_dir() || dir.join("configuration-cache").is_dir() || dir.join("nb-cache").is_dir() || parent.map(|p| p.join("build.gradle").is_file() || p.join("build.gradle.kts").is_file() || p.join("settings.gradle").is_file() || p.join("settings.gradle.kts").is_file()).unwrap_or(false)) {
        let has_gradle_manifest = parent
            .map(|p| {
                p.join("build.gradle").is_file()
                    || p.join("build.gradle.kts").is_file()
                    || p.join("settings.gradle").is_file()
                    || p.join("settings.gradle.kts").is_file()
            })
            .unwrap_or(false);
        let is_user_gradle_home = parent
            .and_then(|p| p.file_name())
            .map(|n| !n.is_empty())
            .unwrap_or(false)
            && dir.join("caches").is_dir()
            && dir.join("wrapper").is_dir();
        if !is_user_gradle_home && !has_gradle_manifest {
            return Some(ClassifiedDir {
                category: "gradle_cache",
                ecosystem: "jvm",
                sarif_rule: "SLC007",
                status: "missing_project_manifest",
                orphan_weight: 2.40,
                reclaim_safe: true,
                detail: Some("Local .gradle directory has no sibling Gradle build script.".into()),
                remediation: "Delete orphaned project .gradle directory.".into(),
            });
        }
        if !is_user_gradle_home && is_stale {
            return Some(ClassifiedDir {
                category: "gradle_cache",
                ecosystem: "jvm",
                sarif_rule: "SLC007",
                status: "stale_inactive",
                orphan_weight: 1.0,
                reclaim_safe: true,
                detail: Some(format!("Project .gradle cache inactive for {:.0} days.", age_days)),
                remediation: "Remove stale project-local .gradle cache directory.".into(),
            });
        }
        return None;
    }

    // 6. CMake out-of-source build dir (`CMakeCache.txt` present)
    if dir.join("CMakeCache.txt").is_file() {
        let has_cmake = parent.map(|p| p.join("CMakeLists.txt").is_file()).unwrap_or(false);
        if !has_cmake {
            return Some(ClassifiedDir {
                category: "cmake_build",
                ecosystem: "cpp",
                sarif_rule: "SLC007",
                status: "missing_project_manifest",
                orphan_weight: 2.40,
                reclaim_safe: true,
                detail: Some("CMake build directory has no sibling CMakeLists.txt.".into()),
                remediation: "Remove orphaned CMake build directory.".into(),
            });
        }
        if is_stale {
            return Some(ClassifiedDir {
                category: "cmake_build",
                ecosystem: "cpp",
                sarif_rule: "SLC007",
                status: "stale_inactive",
                orphan_weight: 1.0,
                reclaim_safe: true,
                detail: Some(format!("CMake build directory inactive for {:.0} days.", age_days)),
                remediation: "Delete stale CMake build output directory and reconfigure when needed.".into(),
            });
        }
        return None;
    }

    // 7. Unmounted dormant container layer / BuildKit cache directories
    if (name_lower == "buildkit" || name_lower == "layerdb" || name_lower == "image-cache") && is_stale {
        return Some(ClassifiedDir {
            category: "container_layer",
            ecosystem: "containers",
            sarif_rule: "SLC008",
            status: "stale_inactive",
            orphan_weight: 1.25,
            reclaim_safe: true,
            detail: Some(format!("Dormant container/BuildKit layer cache untouched for {:.0} days.", age_days)),
            remediation: "Prune unused layers via 'docker builder prune -f' or 'docker image prune -a'.".into(),
        });
    }

    None
}

fn should_skip_dir_name(name_lower: &str) -> bool {
    matches!(
        name_lower,
        ".git"
            | ".hg"
            | ".svn"
            | "windows"
            | "program files"
            | "program files (x86)"
            | "$recycle.bin"
            | "system volume information"
            | "proc"
            | "sys"
            | "dev"
    )
}

fn walk_env_root(
    dir: &Path,
    depth: usize,
    now_secs: u64,
    stale_days: u64,
    cancel: &AtomicBool,
    out: &mut Vec<EnvCandidate>,
) {
    if cancel.load(Ordering::Relaxed) || depth > MAX_WALK_DEPTH {
        return;
    }

    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let path = entry.path();
        let Ok(ft) = entry.file_type() else {
            continue;
        };
        if ft.is_symlink() {
            continue;
        }

        let Some(name_str) = path.file_name().and_then(|n| n.to_str()) else {
            continue;
        };
        let name_lower = name_str.to_ascii_lowercase();

        if ft.is_file() {
            let Ok(meta) = entry.metadata() else {
                continue;
            };
            let bytes = meta.len();
            if bytes == 0 {
                continue;
            }
            let mtime = metadata_mtime_secs(&meta);
            let age_days = ((now_secs.saturating_sub(mtime) as f64) / SECS_PER_DAY * 10.0).round() / 10.0;

            if is_vhdx_virtual_disk(&name_lower) {
                out.push(EnvCandidate {
                    path,
                    category: "wsl_vhdx",
                    ecosystem: "wsl-docker",
                    sarif_rule: "SLC008",
                    status: "locked_vhdx_guard",
                    bytes,
                    files: 1,
                    last_modified_epoch: mtime,
                    age_days,
                    priority_score: 0.0,
                    reclaim_safe: false,
                    detail: Some(
                        "WSL2/Docker virtual disk (.vhdx) is protected by SLClean lock guard and never deleted."
                            .into(),
                    ),
                    remediation:
                        "Shut down WSL ('wsl --shutdown') and compact sparse blocks with 'Optimize-VHD -Path <file.vhdx> -Mode Full'."
                            .into(),
                });
                continue;
            }

            let parent_norm = dir.to_string_lossy().to_ascii_lowercase().replace('\\', "/");
            if is_container_layer_archive(&name_lower, &parent_norm) && age_days >= (stale_days as f64) {
                let score = compute_priority_score(bytes, age_days, 1.35, true);
                out.push(EnvCandidate {
                    path,
                    category: "container_layer",
                    ecosystem: "containers",
                    sarif_rule: "SLC008",
                    status: "stale_inactive",
                    bytes,
                    files: 1,
                    last_modified_epoch: mtime,
                    age_days,
                    priority_score: score,
                    reclaim_safe: true,
                    detail: Some(format!("Dormant OCI/Docker image archive untouched for {:.0} days.", age_days)),
                    remediation: "Remove stale exported container image tarball if already pushed to a registry.".into(),
                });
            }
            continue;
        }

        if !ft.is_dir() || should_skip_dir_name(&name_lower) {
            continue;
        }

        let mtime = freshest_shallow_mtime(&path);
        let age_days = ((now_secs.saturating_sub(mtime) as f64) / SECS_PER_DAY * 10.0).round() / 10.0;

        if let Some(classified) = classify_directory(&path, age_days, stale_days) {
            let stats = sizer::dir_stats(&path, cancel);
            if stats.bytes > 0 {
                let score = compute_priority_score(
                    stats.bytes,
                    age_days,
                    classified.orphan_weight,
                    classified.reclaim_safe,
                );
                out.push(EnvCandidate {
                    path,
                    category: classified.category,
                    ecosystem: classified.ecosystem,
                    sarif_rule: classified.sarif_rule,
                    status: classified.status,
                    bytes: stats.bytes,
                    files: stats.files,
                    last_modified_epoch: mtime,
                    age_days,
                    priority_score: score,
                    reclaim_safe: classified.reclaim_safe,
                    detail: classified.detail,
                    remediation: classified.remediation,
                });
            }
            // Do not recurse inside a matched environment or build target.
            continue;
        }

        // Skip descending into active (non-stale, valid) heavy dependency trees so scans stay fast.
        if matches!(name_lower.as_str(), "node_modules" | ".venv" | "venv")
            || (name_lower == "target" && is_verified_rust_target(&path))
            || path.join("pyvenv.cfg").is_file()
            || path.join("conda-meta").is_dir()
        {
            continue;
        }

        walk_env_root(&path, depth + 1, now_secs, stale_days, cancel, out);
    }
}

pub fn scan_environments(
    roots: &[PathBuf],
    stale_days: u64,
    cancel: &AtomicBool,
) -> EnvScanReport {
    scan_environments_at(roots, stale_days, now_epoch_secs(), cancel)
}

pub fn scan_environments_at(
    roots: &[PathBuf],
    stale_days: u64,
    now_secs: u64,
    cancel: &AtomicBool,
) -> EnvScanReport {
    let mut candidates = Vec::new();
    let mut scanned = Vec::new();

    for root in roots {
        if !root.exists() {
            continue;
        }
        scanned.push(root.clone());
        walk_env_root(root, 0, now_secs, stale_days, cancel, &mut candidates);
    }

    // Deduplicate by canonical/normalized path in case overlapping roots were supplied.
    candidates.sort_by(|a, b| a.path.cmp(&b.path));
    candidates.dedup_by(|a, b| a.path == b.path);

    // Rank by descending age-weighted priority score, then descending bytes, then path.
    candidates.sort_by(|a, b| {
        b.priority_score
            .partial_cmp(&a.priority_score)
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| b.bytes.cmp(&a.bytes))
            .then_with(|| a.path.cmp(&b.path))
    });

    let mut orphan_count = 0usize;
    let mut stale_count = 0usize;
    let mut guarded_vhdx_count = 0usize;
    let mut total_reclaimable_bytes = 0u64;
    let mut orphan_reclaimable_bytes = 0u64;
    let mut guarded_vhdx_bytes = 0u64;

    for c in &candidates {
        match c.status {
            "broken_interpreter" | "missing_project_manifest" => {
                orphan_count += 1;
                if c.reclaim_safe {
                    total_reclaimable_bytes += c.bytes;
                    orphan_reclaimable_bytes += c.bytes;
                }
            }
            "stale_inactive" => {
                stale_count += 1;
                if c.reclaim_safe {
                    total_reclaimable_bytes += c.bytes;
                }
            }
            "locked_vhdx_guard" => {
                guarded_vhdx_count += 1;
                guarded_vhdx_bytes += c.bytes;
            }
            _ => {}
        }
    }

    EnvScanReport {
        roots_scanned: scanned,
        stale_days_threshold: stale_days,
        candidate_count: candidates.len(),
        orphan_count,
        stale_count,
        guarded_vhdx_count,
        total_reclaimable_bytes,
        orphan_reclaimable_bytes,
        guarded_vhdx_bytes,
        candidates,
    }
}

pub fn reclaim_environments(
    report: &EnvScanReport,
    dry_run: bool,
    only_orphans: bool,
    to_trash: bool,
) -> EnvReclaimSummary {
    let eligible: Vec<&EnvCandidate> = report
        .candidates
        .iter()
        .filter(|c| {
            c.reclaim_safe
                && (!only_orphans
                    || matches!(c.status, "broken_interpreter" | "missing_project_manifest"))
        })
        .collect();

    let targeted_bytes: u64 = eligible.iter().map(|c| c.bytes).sum();
    let mut freed_bytes = 0u64;
    let mut removed_entries = 0u64;
    let mut skipped_entries = 0u64;

    if !dry_run {
        for (idx, c) in eligible.iter().enumerate() {
            let req = cleaner::CleanItem {
                id: format!("env-{idx}"),
                paths: vec![c.path.clone()],
                keep_root: false,
            };
            let res = cleaner::clean_one(&req, to_trash);
            freed_bytes += res.freed;
            removed_entries += res.removed;
            skipped_entries += res.skipped;
        }
    }

    EnvReclaimSummary {
        dry_run,
        only_orphans,
        to_trash,
        targeted_count: eligible.len(),
        targeted_bytes,
        freed_bytes,
        removed_entries,
        skipped_entries,
    }
}

fn escape_html(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Render a self-contained dark-mode HTML audit report of orphaned/stale dev environments and guarded VHDX disks.
pub fn render_envs_html(report: &EnvScanReport) -> String {
    let total_mb = report.total_reclaimable_bytes as f64 / (1024.0 * 1024.0);
    let vhdx_mb = report.guarded_vhdx_bytes as f64 / (1024.0 * 1024.0);
    let mut rows = String::new();
    for c in report.candidates.iter().take(200) {
        let mb = c.bytes as f64 / (1024.0 * 1024.0);
        let badge_color = if !c.reclaim_safe { "#38bdf8" } else if c.status == "stale_inactive" { "#f59e0b" } else { "#f87171" };
        rows.push_str(&format!(
            "<tr><td><span style=\"color:{};font-weight:700\">{}</span></td><td>{}</td><td>{}</td><td>{:.1} MiB</td><td>{:.0}d</td><td><code>{}</code></td><td>{}</td></tr>\n",
            badge_color,
            escape_html(c.sarif_rule),
            escape_html(c.category),
            escape_html(c.status),
            mb,
            c.age_days,
            escape_html(&c.path.display().to_string()),
            escape_html(&c.remediation),
        ));
    }
    if rows.is_empty() {
        rows.push_str("<tr><td colspan=\"7\">No orphaned or stale development environments detected.</td></tr>");
    }
    format!(
        r##"<!doctype html><html lang="en"><head><meta charset="utf-8"><title>SLClean v{} Orphaned &amp; Stale Environments Report</title><style>body{{margin:0;padding:28px;background:#0f1411;color:#e6efe9;font-family:system-ui,sans-serif}}.wrap{{max-width:1120px;margin:0 auto}}.card{{background:#161d19;border:1px solid #2a3730;border-radius:10px;padding:18px;margin-bottom:16px}}table{{width:100%;border-collapse:collapse;font-size:12.5px}}th,td{{padding:8px 10px;border-bottom:1px solid #24302a;text-align:left;vertical-align:top}}th{{color:#93a89b}}code{{font-family:ui-monospace,monospace;color:#4ade80}}</style></head><body><div class="wrap"><div class="card"><h1>SLClean Orphaned &amp; Stale Dev Environments Report</h1><p>Orphans: <b>{}</b> &middot; Stale: <b>{}</b> &middot; Reclaimable: <b>{:.1} MiB</b> &middot; Guarded VHDX: <b>{} ({:.1} MiB, protected)</b> &middot; Stale Threshold: <b>{} days</b></p></div><div class="card"><table><thead><tr><th>Rule</th><th>Category</th><th>Status</th><th>Size</th><th>Age</th><th>Path</th><th>Remediation</th></tr></thead><tbody>{}</tbody></table></div></div></body></html>"##,
        env!("CARGO_PKG_VERSION"),
        report.orphan_count,
        report.stale_count,
        total_mb,
        report.guarded_vhdx_count,
        vhdx_mb,
        report.stale_days_threshold,
        rows
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_broken_pyvenv_orphaned_node_modules_cargo_target_and_guards_vhdx() {
        let root = std::env::temp_dir().join(format!("slclean-envs-test-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);

        // 1. Python venv with non-existent base interpreter -> SLC006 broken_interpreter
        let broken_venv = root.join("proj-a").join(".venv");
        fs::create_dir_all(broken_venv.join("lib")).unwrap();
        fs::write(
            root.join("proj-a").join("pyproject.toml"),
            b"[project]\nname = \"proj-a\"\n",
        )
        .unwrap();
        let missing_py = root.join("nonexistent-python-3.11");
        fs::write(
            broken_venv.join("pyvenv.cfg"),
            format!("home = {}\nversion = 3.11.8\n", missing_py.display()),
        )
        .unwrap();
        fs::write(broken_venv.join("lib").join("pkg.py"), vec![b'x'; 32 * 1024]).unwrap();

        // 2. Orphaned node_modules with no sibling package.json -> SLC006 missing_project_manifest
        let orphan_nm = root.join("deleted-webapp").join("node_modules").join("leftpad");
        fs::create_dir_all(&orphan_nm).unwrap();
        fs::write(orphan_nm.join("index.js"), vec![b'a'; 48 * 1024]).unwrap();

        // 3. Active recent node_modules WITH sibling package.json -> ignored (not stale, not orphan)
        let active_web = root.join("active-webapp");
        fs::create_dir_all(active_web.join("node_modules").join("react")).unwrap();
        fs::write(active_web.join("package.json"), b"{\"name\":\"active\"}").unwrap();
        fs::write(
            active_web.join("node_modules").join("react").join("index.js"),
            vec![b'r'; 16 * 1024],
        )
        .unwrap();

        // 4. Verified Rust target with CACHEDIR.TAG and no Cargo.toml -> SLC007 missing_project_manifest
        let orphan_target = root.join("old-crate").join("target");
        fs::create_dir_all(orphan_target.join("debug")).unwrap();
        fs::write(
            orphan_target.join("CACHEDIR.TAG"),
            format!("{CACHEDIR_SIGNATURE_PREFIX}\n# Created by cargo\n"),
        )
        .unwrap();
        fs::write(orphan_target.join("debug").join("libfoo.rlib"), vec![b'c'; 64 * 1024]).unwrap();

        // 5. Generic user folder named "target" without CACHEDIR.TAG or .fingerprint -> MUST NOT match
        let user_target_docs = root.join("sales-q4").join("target");
        fs::create_dir_all(&user_target_docs).unwrap();
        fs::write(user_target_docs.join("revenue.csv"), b"month,usd\n11,50000\n").unwrap();

        // 6. WSL2 ext4.vhdx -> SLC008 locked_vhdx_guard (reclaim_safe == false!)
        let wsl_dir = root.join("wsl-distro");
        fs::create_dir_all(&wsl_dir).unwrap();
        fs::write(wsl_dir.join("ext4.vhdx"), vec![0u8; 128 * 1024]).unwrap();

        let now_future = now_epoch_secs() + 5 * 86_400; // 5 days later (< 30d stale threshold)
        let rep = scan_environments_at(&[root.clone()], 30, now_future, &AtomicBool::new(false));

        assert_eq!(rep.orphan_count, 3);
        assert_eq!(rep.stale_count, 0);
        assert_eq!(rep.guarded_vhdx_count, 1);
        assert_eq!(rep.candidate_count, 4);

        // User's sales-q4/target and active-webapp/node_modules must never appear!
        assert!(!rep.candidates.iter().any(|c| c.path.ends_with("sales-q4/target") || c.path.ends_with("sales-q4\\target")));
        assert!(!rep.candidates.iter().any(|c| c.path.to_string_lossy().contains("active-webapp")));

        // Verify SLC006, SLC007, and SLC008 are all accurately assigned
        assert!(rep.candidates.iter().any(|c| c.category == "python_venv" && c.status == "broken_interpreter" && c.sarif_rule == "SLC006" && c.reclaim_safe));
        assert!(rep.candidates.iter().any(|c| c.category == "node_modules" && c.status == "missing_project_manifest" && c.sarif_rule == "SLC006" && c.reclaim_safe));
        assert!(rep.candidates.iter().any(|c| c.category == "rust_target" && c.status == "missing_project_manifest" && c.sarif_rule == "SLC007" && c.reclaim_safe));
        let vhdx = rep.candidates.iter().find(|c| c.category == "wsl_vhdx").unwrap();
        assert_eq!(vhdx.sarif_rule, "SLC008");
        assert!(!vhdx.reclaim_safe);
        assert_eq!(vhdx.priority_score, 0.0);

        // Reclaiming orphans deletes the 3 orphans while keeping ext4.vhdx and sales-q4/target untouched!
        let summary = reclaim_environments(&rep, false, true, false);
        assert_eq!(summary.targeted_count, 3);
        assert!(summary.freed_bytes >= (32 + 48 + 64) * 1024);
        assert!(!broken_venv.exists());
        assert!(!orphan_target.exists());
        assert!(wsl_dir.join("ext4.vhdx").exists());
        assert!(user_target_docs.join("revenue.csv").exists());

        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn age_weighted_priority_ranks_orphans_and_older_targets_higher() {
        let one_gib = 1024 * 1024 * 1024;
        let recent_stale = compute_priority_score(one_gib, 30.0, 1.0, true);
        let ancient_stale = compute_priority_score(one_gib, 180.0, 1.0, true);
        let orphan_same_age = compute_priority_score(one_gib, 30.0, 2.5, true);
        let locked_vhdx = compute_priority_score(10 * one_gib, 365.0, 2.5, false);

        assert!(ancient_stale > recent_stale);
        assert!(orphan_same_age > ancient_stale);
        assert_eq!(locked_vhdx, 0.0);
    }
}
