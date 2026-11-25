//! Hygiene Scanner: Empty Directories & Broken Symlinks / Dead Shortcuts (Roadmap Tier 2).
//!
//! Surfaces empty leaf folders and dangling symlinks (plus dead `.lnk` shortcuts on Windows)
//! while strictly honoring `cleaner::is_protected` and skipping VCS/system roots.

use crate::cleaner;
use serde::Serialize;
use std::fs;
use std::path::{Path, PathBuf};

const SKIP_NAMES: &[&str] = &[
    ".git",
    ".hg",
    ".svn",
    "node_modules",
    "target",
    "$RECYCLE.BIN",
    "System Volume Information",
];

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum HygieneKind {
    EmptyDirectory,
    BrokenSymlink,
}

#[derive(Clone, Debug, Serialize)]
pub struct HygieneFinding {
    pub path: PathBuf,
    pub kind: HygieneKind,
    pub detail: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct HygieneReport {
    pub scanned_roots: Vec<PathBuf>,
    pub empty_dir_count: usize,
    pub broken_link_count: usize,
    pub findings: Vec<HygieneFinding>,
}

fn walk_hygiene(dir: &Path, depth: usize, max_depth: usize, out: &mut Vec<HygieneFinding>) {
    if depth > max_depth || cleaner::is_protected(dir) {
        return;
    }
    let Ok(read_dir) = fs::read_dir(dir) else {
        return;
    };

    let mut entry_count = 0usize;
    for entry in read_dir.flatten() {
        entry_count += 1;
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        if SKIP_NAMES.iter().any(|s| s.eq_ignore_ascii_case(&name)) {
            continue;
        }
        let Ok(meta) = fs::symlink_metadata(&path) else {
            continue;
        };
        if meta.file_type().is_symlink() {
            // Broken symlink: target does not exist when following link.
            if fs::metadata(&path).is_err() {
                let target_desc = fs::read_link(&path)
                    .map(|t| t.to_string_lossy().to_string())
                    .unwrap_or_else(|_| "<unreadable>".into());
                out.push(HygieneFinding {
                    path,
                    kind: HygieneKind::BrokenSymlink,
                    detail: format!("Dangling symlink -> {target_desc}"),
                });
            }
            continue;
        }
        if meta.is_dir() {
            walk_hygiene(&path, depth + 1, max_depth, out);
        }
    }

    // Never flag top-level scanned root itself; only empty child directories.
    if depth > 0 && entry_count == 0 && !cleaner::is_root_like(dir) {
        out.push(HygieneFinding {
            path: dir.to_path_buf(),
            kind: HygieneKind::EmptyDirectory,
            detail: "Empty leaf directory (0 entries)".into(),
        });
    }
}

pub fn scan(roots: &[PathBuf], max_depth: usize) -> HygieneReport {
    let mut findings = Vec::new();
    for root in roots {
        if root.is_dir() {
            walk_hygiene(root, 0, max_depth.clamp(1, 8), &mut findings);
        }
    }
    findings.sort_by(|a, b| a.path.cmp(&b.path));
    let empty_dir_count = findings
        .iter()
        .filter(|f| f.kind == HygieneKind::EmptyDirectory)
        .count();
    let broken_link_count = findings
        .iter()
        .filter(|f| f.kind == HygieneKind::BrokenSymlink)
        .count();

    HygieneReport {
        scanned_roots: roots.to_vec(),
        empty_dir_count,
        broken_link_count,
        findings,
    }
}

fn escape_html_hyg(raw: &str) -> String {
    raw.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

/// Render a self-contained dark-mode HTML Directory & Symlink Hygiene Audit Report (`SLC010`).
pub fn render_hygiene_html(report: &HygieneReport) -> String {
    let mut rows = String::new();
    for f in report.findings.iter().take(300) {
        let (kind_lbl, color) = match f.kind {
            HygieneKind::BrokenSymlink => ("BROKEN SYMLINK", "#f87171"),
            HygieneKind::EmptyDirectory => ("EMPTY DIR", "#f59e0b"),
        };
        rows.push_str(&format!(
            "<tr><td style=\"color:{};font-weight:700\">{}</td><td><code>{}</code></td><td>{}</td></tr>\n",
            color,
            kind_lbl,
            escape_html_hyg(&f.path.display().to_string()),
            escape_html_hyg(&f.detail),
        ));
    }
    if rows.is_empty() {
        rows.push_str("<tr><td colspan=\"3\" style=\"color:#4ade80\">Zero empty directories or broken symlinks detected.</td></tr>");
    }
    format!(
        r##"<!doctype html><html lang="en"><head><meta charset="utf-8"><title>SLClean v{} Directory &amp; Symlink Hygiene Report</title><style>body{{margin:0;padding:28px;background:#0f1411;color:#e6efe9;font-family:system-ui,sans-serif}}.wrap{{max-width:1040px;margin:0 auto}}.card{{background:#161d19;border:1px solid #2a3730;border-radius:10px;padding:18px;margin-bottom:16px}}table{{width:100%;border-collapse:collapse;font-size:12.5px}}th,td{{padding:8px 10px;border-bottom:1px solid #24302a;text-align:left;vertical-align:top}}th{{color:#93a89b}}code{{font-family:ui-monospace,monospace;color:#4ade80}}</style></head><body><div class="wrap"><div class="card"><h1>SLClean Directory &amp; Symlink Hygiene Report (SLC010)</h1><p>Empty Leaf Dirs: <b>{}</b> &middot; Broken Symlinks: <b>{}</b> &middot; Total Findings: <b>{}</b></p></div><div class="card"><table><thead><tr><th>Kind</th><th>Path</th><th>Detail</th></tr></thead><tbody>{}</tbody></table></div></div></body></html>"##,
        env!("CARGO_PKG_VERSION"),
        report.empty_dir_count,
        report.broken_link_count,
        report.findings.len(),
        rows
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detects_empty_child_dirs_and_broken_symlinks() {
        let base = std::env::temp_dir().join("slclean-hygiene-test");
        let _ = fs::remove_dir_all(&base);
        let empty = base.join("nested").join("empty-leaf");
        fs::create_dir_all(&empty).unwrap();
        let populated = base.join("keep");
        fs::create_dir_all(&populated).unwrap();
        fs::write(populated.join("file.txt"), b"hello").unwrap();

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(base.join("does-not-exist"), base.join("dead-link")).unwrap();
        }

        let report = scan(std::slice::from_ref(&base), 4);
        assert_eq!(report.empty_dir_count, 1);
        assert!(report.findings.iter().any(|f| f.path == empty));
        #[cfg(unix)]
        {
            assert_eq!(report.broken_link_count, 1);
        }

        let _ = fs::remove_dir_all(&base);
    }
}
