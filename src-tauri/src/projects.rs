//! Project-Centric Build Artifact Reclaimer (Roadmap §1.4).
//!
//! Groups detected build artifacts (`target/`, `node_modules/`, `build/`, `.venv/`,
//! `.gradle/`, `dist/`, etc.) by their owning project directory and reads last-touched
//! timestamps directly from `.git/HEAD` + `.git/refs/` (zero `git` subprocess spawns)
//! so developers can spot 6-month-old untouched projects holding gigabytes of artifacts.

use crate::artifacts::Artifact;
use serde::Serialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const DEFAULT_STALE_PROJECT_DAYS: u64 = 30;

#[derive(Clone, Debug, Serialize)]
pub struct ProjectArtifactSlice {
    pub path: PathBuf,
    pub kind: String,
    pub tool: String,
    pub bytes: u64,
    pub files: u64,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProjectGroup {
    pub project: PathBuf,
    pub name: String,
    pub is_git_repo: bool,
    pub total_bytes: u64,
    pub total_files: u64,
    pub last_activity_epoch: u64,
    pub inactive_days: u64,
    pub stale: bool,
    pub artifacts: Vec<ProjectArtifactSlice>,
}

#[derive(Clone, Debug, Serialize)]
pub struct ProjectScanReport {
    pub stale_threshold_days: u64,
    pub project_count: usize,
    pub stale_project_count: usize,
    pub total_reclaimable_bytes: u64,
    pub stale_reclaimable_bytes: u64,
    pub projects: Vec<ProjectGroup>,
}

fn file_mtime_epoch(path: &Path) -> Option<u64> {
    std::fs::metadata(path)
        .ok()
        .and_then(|m| m.modified().ok())
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
}

/// Inspect `.git/HEAD`, `.git/index`, and `.git/refs/heads/*` directly without spawning git.
pub fn detect_git_activity_epoch(project_dir: &Path) -> (bool, Option<u64>) {
    let git_dir = project_dir.join(".git");
    if !git_dir.exists() {
        return (false, None);
    }
    let mut latest: Option<u64> = None;
    let mut consider = |p: PathBuf| {
        if let Some(ts) = file_mtime_epoch(&p) {
            latest = Some(latest.map_or(ts, |cur| cur.max(ts)));
        }
    };
    consider(git_dir.join("HEAD"));
    consider(git_dir.join("index"));
    consider(git_dir.join("FETCH_HEAD"));
    consider(git_dir.join("COMMIT_EDITMSG"));

    let heads_dir = git_dir.join("refs").join("heads");
    if let Ok(entries) = std::fs::read_dir(&heads_dir) {
        for entry in entries.flatten().take(32) {
            consider(entry.path());
        }
    }
    (true, latest)
}

/// Group flat artifacts by parent project directory and classify inactivity.
pub fn group_artifacts_by_project(
    artifacts: &[Artifact],
    stale_threshold_days: u64,
    now_epoch: Option<u64>,
) -> ProjectScanReport {
    let now = now_epoch.unwrap_or_else(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0)
    });

    let mut by_project: BTreeMap<PathBuf, Vec<&Artifact>> = BTreeMap::new();
    for art in artifacts {
        by_project.entry(art.project.clone()).or_default().push(art);
    }

    let mut projects: Vec<ProjectGroup> = by_project
        .into_iter()
        .map(|(project, list)| {
            let name = project
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .filter(|n| !n.is_empty())
                .unwrap_or_else(|| project.to_string_lossy().to_string());

            let (is_git_repo, git_epoch) = detect_git_activity_epoch(&project);
            let max_art_epoch = list.iter().map(|a| a.modified).max().unwrap_or(0);
            let last_activity_epoch = git_epoch.unwrap_or(max_art_epoch).max(max_art_epoch);
            let inactive_days = if last_activity_epoch == 0 || now <= last_activity_epoch {
                0
            } else {
                (now - last_activity_epoch) / 86_400
            };

            let mut slices: Vec<ProjectArtifactSlice> = list
                .iter()
                .map(|a| ProjectArtifactSlice {
                    path: a.path.clone(),
                    kind: a.kind.clone(),
                    tool: a.tool.clone(),
                    bytes: a.bytes,
                    files: a.files,
                })
                .collect();
            slices.sort_by(|a, b| b.bytes.cmp(&a.bytes));

            let total_bytes: u64 = slices.iter().map(|s| s.bytes).sum();
            let total_files: u64 = slices.iter().map(|s| s.files).sum();
            let stale = inactive_days >= stale_threshold_days;

            ProjectGroup {
                project,
                name,
                is_git_repo,
                total_bytes,
                total_files,
                last_activity_epoch,
                inactive_days,
                stale,
                artifacts: slices,
            }
        })
        .collect();

    // Sort by stale first, then descending reclaimable bytes.
    projects.sort_by(|a, b| b.total_bytes.cmp(&a.total_bytes));

    let total_reclaimable_bytes = projects.iter().map(|p| p.total_bytes).sum();
    let stale_reclaimable_bytes = projects.iter().filter(|p| p.stale).map(|p| p.total_bytes).sum();
    let stale_project_count = projects.iter().filter(|p| p.stale).count();

    ProjectScanReport {
        stale_threshold_days,
        project_count: projects.len(),
        stale_project_count,
        total_reclaimable_bytes,
        stale_reclaimable_bytes,
        projects,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn groups_multiple_artifacts_per_project_and_flags_stale_projects() {
        let now = 1_750_000_000u64;
        let old_ts = now - 90 * 86_400; // 90 days inactive
        let fresh_ts = now - 2 * 86_400; // 2 days inactive

        let artifacts = vec![
            Artifact {
                path: PathBuf::from("/work/old-rpg/target"),
                project: PathBuf::from("/work/old-rpg"),
                kind: "target".into(),
                tool: "Cargo".into(),
                bytes: 3_000_000_000,
                files: 1200,
                modified: old_ts,
            },
            Artifact {
                path: PathBuf::from("/work/old-rpg/node_modules"),
                project: PathBuf::from("/work/old-rpg"),
                kind: "node_modules".into(),
                tool: "npm / pnpm / yarn".into(),
                bytes: 1_200_000_000,
                files: 15000,
                modified: old_ts,
            },
            Artifact {
                path: PathBuf::from("/work/active-app/dist"),
                project: PathBuf::from("/work/active-app"),
                kind: "dist".into(),
                tool: "bundler output".into(),
                bytes: 50_000_000,
                files: 40,
                modified: fresh_ts,
            },
        ];

        let report = group_artifacts_by_project(&artifacts, 30, Some(now));
        assert_eq!(report.project_count, 2);
        assert_eq!(report.stale_project_count, 1);
        assert_eq!(report.projects[0].name, "old-rpg");
        assert!(report.projects[0].stale);
        assert_eq!(report.projects[0].total_bytes, 4_200_000_000);
        assert_eq!(report.projects[0].artifacts.len(), 2);
        assert_eq!(report.stale_reclaimable_bytes, 4_200_000_000);
    }
}
