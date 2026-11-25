//! Smart AI-Agent Junk, Ephemeral Test Residue & Leftover Installer Decision Engine.
//!
//! Solves two everyday developer/Windows pain points:
//! 1. AI coding agents (Claude Code, Codex, Cursor, Windsurf, Antigravity, Stella, Orca) leave
//!    behind debug logs, shell snapshots, test outputs (`playwright-report`, `.pytest_cache`,
//!    `test-results`, `htmlcov`, `.vitest`), throwaway `tmp-*`/`scratch-*` dirs, crash dumps,
//!    and orphaned `.worktrees/*`.
//! 2. Users install and uninstall miscellaneous apps, leaving behind forgotten `.exe`/`.msi`/`.msix`
//!    installers in `Downloads` and `Temp`/`SquirrelTemp`.
//!
//! Every finding receives a clear 3-tier decision verdict (`safe_delete` | `review` | `keep`),
//! a confidence score (`0..=100`), and plain-language Vietnamese + English reasons explaining
//! WHAT it is, WHY it is safe or unsafe to delete, and WHAT happens if removed.

use crate::sizer;
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::AtomicBool;
use std::time::{SystemTime, UNIX_EPOCH};

pub const DEFAULT_STALE_DAYS: u64 = 7;
pub const DEFAULT_MIN_INSTALLER_BYTES: u64 = 512 * 1024;
const MAX_WALK_DEPTH: usize = 6;
const MAX_FINDINGS: usize = 400;

#[derive(Clone, Debug, Serialize)]
pub struct AiJunkFinding {
    pub id: String,
    pub path: PathBuf,
    pub name: String,
    /// "ai_agent_residue" | "ai_test_junk" | "ai_worktree" | "documents_junk" | "leftover_installer"
    pub category: String,
    /// "safe_delete" | "review" | "keep"
    pub verdict: String,
    pub confidence: u8,
    pub bytes: u64,
    pub files: u64,
    pub age_days: u64,
    pub modified: u64,
    pub git_dirty: bool,
    /// "SLC016" (AI residue/worktree) | "SLC017" (ephemeral test/crash junk) | "SLC018" (leftover installer) | "SLC019" (Codex/AI junk in Documents)
    pub rule_id: String,
    pub reason_vi: String,
    pub reason_en: String,
    pub impact_vi: String,
    pub impact_en: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct AiJunkReport {
    pub scanned_at: u64,
    pub stale_days: u64,
    pub scanned_roots: Vec<PathBuf>,
    pub total_count: usize,
    pub total_bytes: u64,
    pub safe_delete_count: usize,
    pub safe_delete_bytes: u64,
    pub review_count: usize,
    pub review_bytes: u64,
    pub keep_count: usize,
    pub keep_bytes: u64,
    pub findings: Vec<AiJunkFinding>,
}

/// Regenerable test runner caches & output reports — 100% safe to delete.
const SAFE_TEST_DIRS: &[(&str, &str, &str)] = &[
    (".pytest_cache", "Cache chạy test của pytest", "Pytest execution cache"),
    (".mypy_cache", "Cache kiểm tra kiểu của mypy", "Mypy type-checker cache"),
    (".ruff_cache", "Cache lint của Ruff", "Ruff linter cache"),
    (".tox", "Môi trường test tạm của tox", "Tox ephemeral test environments"),
    (".nox", "Môi trường test tạm của nox", "Nox ephemeral test environments"),
    ("htmlcov", "Báo cáo HTML độ phủ test (coverage)", "HTML test coverage report"),
    (".coverage", "Dữ liệu độ phủ test tạm", "Temporary test coverage data"),
    (".nyc_output", "Dữ liệu coverage tạm của NYC/Istanbul", "NYC/Istanbul temporary coverage output"),
    ("test-results", "Thư mục kết quả test tự động (Playwright/JUnit/CI)", "Automated test results output (Playwright/JUnit/CI)"),
    ("playwright-report", "Báo cáo HTML & trace chạy thử của Playwright", "Playwright HTML test & trace report"),
    ("blob-report", "Dữ liệu gộp báo cáo test Playwright", "Playwright merged blob test report"),
    (".vitest", "Cache chạy test của Vitest", "Vitest test runner cache"),
    (".jest-cache", "Cache chạy test của Jest", "Jest test runner cache"),
    ("allure-results", "Dữ liệu báo cáo test tạm của Allure", "Allure temporary test report output"),
];

const SCRATCH_PREFIXES: &[&str] = &[
    "tmp-",
    "tmp_",
    "test-tmp",
    "scratch-",
    "scratch_",
    "ai-test-",
    "codex-test",
    "claude-test",
    "sandbox-",
    "demo-test",
    "mckit-test",
];

const WORKTREE_PARENTS: &[&str] = &[".worktrees", "worktrees"];

const INSTALLER_EXTENSIONS: &[&str] = &["msi", "msix", "appx", "msp", "dmg", "pkg", "deb", "rpm", "appimage"];

const EXE_INSTALLER_HINTS: &[&str] = &[
    "setup", "install", "installer", "updater", "redist", "vcredist",
    "win64", "win32", "x64", "x86", "arm64", "portable", "offline", "bootstrapper",
];

const SKIP_WALK_DIRS: &[&str] = &[
    ".git",
    "node_modules",
    "windows",
    "program files",
    "program files (x86)",
    "programdata",
    "$recycle.bin",
    "system volume information",
    "windowsapps",
    ".cargo",
    ".rustup",
    "venv",
    ".venv",
    "site-packages",
    "__pycache__",
];

fn now_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn modified_epoch(path: &Path) -> u64 {
    std::fs::metadata(path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn age_days_from(modified: u64, now: u64) -> u64 {
    if modified == 0 || modified >= now {
        0
    } else {
        (now - modified) / 86_400
    }
}

fn path_key(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/").to_ascii_lowercase()
}

fn measure_path(path: &Path) -> (u64, u64) {
    if path.is_file() {
        let len = std::fs::metadata(path).map(|m| m.len()).unwrap_or(0);
        return (len, 1);
    }
    let cancel = AtomicBool::new(false);
    let st = sizer::dir_stats(path, &cancel);
    (st.bytes, st.files)
}

/// Checks if a directory with a `.git` entry has uncommitted local modifications.
/// Never fails catastrophically: if `git` is unavailable, conservatively returns `false`
/// unless `.git` is missing.
fn is_git_dirty(dir: &Path) -> bool {
    let dot_git = dir.join(".git");
    if !dot_git.exists() {
        return false;
    }
    let mut cmd = Command::new("git");
    cmd.args(["-C", &dir.to_string_lossy(), "status", "--porcelain"]);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000); // CREATE_NO_WINDOW
    }
    match cmd.output() {
        Ok(out) if out.status.success() => !String::from_utf8_lossy(&out.stdout).trim().is_empty(),
        _ => false,
    }
}

fn is_crash_or_debug_file(name_lower: &str) -> bool {
    name_lower.starts_with("hs_err_pid") && name_lower.ends_with(".log")
        || name_lower.starts_with("replay_pid") && name_lower.ends_with(".log")
        || name_lower.ends_with(".hprof")
        || name_lower.ends_with(".dmp")
        || name_lower.starts_with("npm-debug.log")
        || name_lower.starts_with("pnpm-debug.log")
        || name_lower == "yarn-error.log"
}

fn is_installer_file(path: &Path, name_lower: &str) -> bool {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .map(|s| s.to_ascii_lowercase())
        .unwrap_or_default();
    if INSTALLER_EXTENSIONS.contains(&ext.as_str()) {
        return true;
    }
    if ext == "exe" {
        if EXE_INSTALLER_HINTS.iter().any(|h| name_lower.contains(h)) {
            return true;
        }
        // Versioned installer filenames like `app-1.2.3.exe` or `tool_2.4.exe`
        let has_version_sep = name_lower.contains("-v") || name_lower.contains("_v")
            || name_lower
                .as_bytes()
                .windows(3)
                .any(|w| (w[0] == b'-' || w[0] == b'_') && w[1].is_ascii_digit() && (w[2] == b'.' || w[2].is_ascii_digit()));
        return has_version_sep;
    }
    false
}

fn push_unique(
    findings: &mut Vec<AiJunkFinding>,
    seen: &mut HashSet<String>,
    finding: AiJunkFinding,
) {
    if findings.len() >= MAX_FINDINGS {
        return;
    }
    let key = path_key(&finding.path);
    if seen.insert(key) {
        findings.push(finding);
    }
}

/// Scans well-known AI agent state/log/snapshot/scratch directories inside `home`.
fn scan_ai_agent_home(
    home: &Path,
    stale_days: u64,
    now: u64,
    seen: &mut HashSet<String>,
    out: &mut Vec<AiJunkFinding>,
) {
    // (relative path, safe_always, vi_desc, en_desc, impact_vi, impact_en)
    let direct_targets: &[(&str, bool, &str, &str, &str, &str)] = &[
        (
            ".claude/debug",
            true,
            "Nhật ký debug nội bộ của Claude Code",
            "Claude Code internal debug logs",
            "Xóa an toàn 100% — chỉ là log gỡ lỗi cũ, không ảnh hưởng lịch sử chat hay cấu hình.",
            "100% safe to delete — diagnostic logs only; conversations and settings stay intact.",
        ),
        (
            ".claude/shell-snapshots",
            true,
            "Ảnh chụp biến môi trường shell tạm của Claude Code",
            "Claude Code temporary shell environment snapshots",
            "Xóa an toàn 100% — Claude Code tự tạo lại khi mở phiên mới.",
            "100% safe to delete — regenerated automatically on next Claude Code session.",
        ),
        (
            ".claude/todos",
            true,
            "File danh sách công việc tạm của các phiên Claude Code cũ",
            "Ephemeral task checklists from past Claude Code sessions",
            "Xóa an toàn — không ảnh hưởng mã nguồn hay kỹ năng (skills).",
            "Safe to delete — does not touch source code or installed skills.",
        ),
        (
            ".claude/file-history",
            false,
            "Bản sao lưu lịch sử chỉnh sửa file tự động của Claude Code (dễ phình hàng GB)",
            "Claude Code automatic file-edit backup history (can grow to gigabytes)",
            "Chỉ mất khả năng hoàn tác (rewind) các phiên chat cũ trong Claude; code hiện tại trên đĩa không bị ảnh hưởng.",
            "Only removes old Claude rewind snapshots; current workspace files on disk are untouched.",
        ),
        (
            ".codex/log",
            true,
            "Nhật ký chạy của OpenAI Codex CLI",
            "OpenAI Codex CLI execution logs",
            "Xóa an toàn 100% — chỉ là log chạy cũ.",
            "100% safe to delete — past execution logs only.",
        ),
        (
            ".codex/tmp",
            true,
            "Thư mục tạm khi Codex chạy lệnh/test",
            "Temporary scratch directory used by Codex runs",
            "Xóa an toàn 100% — dữ liệu tạm sau khi chạy xong.",
            "100% safe to delete — ephemeral scratch data.",
        ),
        (
            ".codex/sessions",
            false,
            "Bản ghi lịch sử hội thoại JSONL cũ của Codex CLI",
            "Stored Codex CLI JSONL session transcripts",
            "Xóa sẽ mất lịch sử resume các phiên Codex cũ; không ảnh hưởng mã nguồn.",
            "Removes old Codex resume transcripts; source code is untouched.",
        ),
        (
            ".cursor/logs",
            true,
            "Nhật ký hoạt động cũ của Cursor",
            "Cursor IDE historical logs",
            "Xóa an toàn 100% — log chẩn đoán cũ.",
            "100% safe to delete — old diagnostic logs.",
        ),
        (
            ".windsurf/logs",
            true,
            "Nhật ký hoạt động cũ của Windsurf",
            "Windsurf IDE historical logs",
            "Xóa an toàn 100% — log chẩn đoán cũ.",
            "100% safe to delete — old diagnostic logs.",
        ),
        (
            ".stella/logs",
            true,
            "Nhật ký cũ của Stella ADE",
            "Stella ADE historical logs",
            "Xóa an toàn 100% — chỉ là file log.",
            "100% safe to delete — log files only.",
        ),
        (
            ".orca/logs",
            true,
            "Nhật ký cũ của Orca ADE",
            "Orca ADE historical logs",
            "Xóa an toàn 100% — chỉ là file log.",
            "100% safe to delete — log files only.",
        ),
    ];

    for &(rel, safe_always, desc_vi, desc_en, imp_vi, imp_en) in direct_targets {
        let target = home.join(rel);
        if !target.is_dir() {
            continue;
        }
        let (bytes, files) = measure_path(&target);
        if bytes == 0 && files == 0 {
            continue;
        }
        let modified = modified_epoch(&target);
        let age_days = age_days_from(modified, now);
        let (verdict, confidence) = if safe_always {
            ("safe_delete", 96u8)
        } else if age_days >= stale_days.max(14) {
            ("review", 75u8)
        } else {
            ("keep", 80u8)
        };
        let reason_vi = if safe_always {
            format!("{desc_vi} ({files} file). XÓA AN TOÀN 100%.")
        } else if verdict == "review" {
            format!("{desc_vi} (đã {age_days} ngày không ghi mới). XEM LẠI nếu không cần mở lại lịch sử cũ.")
        } else {
            format!("{desc_vi} (mới dùng {age_days} ngày trước). NÊN GIỮ để tiếp tục phiên làm việc.")
        };
        let reason_en = if safe_always {
            format!("{desc_en} ({files} files). 100% SAFE TO DELETE.")
        } else if verdict == "review" {
            format!("{desc_en} (untouched for {age_days}d). REVIEW if old history is no longer needed.")
        } else {
            format!("{desc_en} (active {age_days}d ago). KEEP while active sessions depend on it.")
        };

        push_unique(
            out,
            seen,
            AiJunkFinding {
                id: format!("aijunk:{}", path_key(&target)),
                name: rel.to_string(),
                path: target,
                category: "ai_agent_residue".into(),
                verdict: verdict.into(),
                confidence,
                bytes,
                files,
                age_days,
                modified,
                git_dirty: false,
                rule_id: "SLC016".into(),
                reason_vi,
                reason_en,
                impact_vi: imp_vi.into(),
                impact_en: imp_en.into(),
            },
        );
    }

    // Scan ~/.claude/projects/*/tool-results (huge AI tool output dumps)
    let claude_projects = home.join(".claude/projects");
    if let Ok(entries) = std::fs::read_dir(&claude_projects) {
        for entry in entries.flatten() {
            let tr = entry.path().join("tool-results");
            if !tr.is_dir() {
                continue;
            }
            let (bytes, files) = measure_path(&tr);
            if bytes == 0 {
                continue;
            }
            let modified = modified_epoch(&tr);
            let age_days = age_days_from(modified, now);
            let verdict = if age_days >= 2 { "safe_delete" } else { "review" };
            push_unique(
                out,
                seen,
                AiJunkFinding {
                    id: format!("aijunk:{}", path_key(&tr)),
                    name: format!(
                        "claude tool-results ({})",
                        entry.file_name().to_string_lossy()
                    ),
                    path: tr,
                    category: "ai_agent_residue".into(),
                    verdict: verdict.into(),
                    confidence: if verdict == "safe_delete" { 94 } else { 70 },
                    bytes,
                    files,
                    age_days,
                    modified,
                    git_dirty: false,
                    rule_id: "SLC016".into(),
                    reason_vi: format!(
                        "Kết quả đầu ra tool cũ của Claude Code ({files} file, {age_days} ngày). Dữ liệu tạm dễ phình lớn."
                    ),
                    reason_en: format!(
                        "Cached Claude Code tool output blobs ({files} files, {age_days}d old). Ephemeral output dump."
                    ),
                    impact_vi: "Xóa an toàn — không ảnh hưởng code dự án, chỉ dọn log kết quả lệnh cũ.".into(),
                    impact_en: "Safe to remove — project files are untouched; only old tool dumps are cleared.".into(),
                },
            );
        }
    }

    // Scan ~/.claude/worktrees/* and ~/.codex/worktrees/*
    for wt_rel in [".claude/worktrees", ".codex/worktrees", ".orca/worktrees", ".stella/worktrees"] {
        let wt_root = home.join(wt_rel);
        if wt_root.is_dir() {
            scan_worktree_children(&wt_root, stale_days, now, seen, out);
        }
    }
}

fn scan_worktree_children(
    wt_parent: &Path,
    stale_days: u64,
    now: u64,
    seen: &mut HashSet<String>,
    out: &mut Vec<AiJunkFinding>,
) {
    let Ok(entries) = std::fs::read_dir(wt_parent) else { return };
    for entry in entries.flatten() {
        let p = entry.path();
        if !entry.file_type().map(|t| t.is_dir() && !t.is_symlink()).unwrap_or(false) {
            continue;
        }
        let name = entry.file_name().to_string_lossy().to_string();
        let (bytes, files) = measure_path(&p);
        let modified = modified_epoch(&p);
        let age_days = age_days_from(modified, now);
        let dirty = is_git_dirty(&p);
        let (verdict, confidence, reason_vi, reason_en, imp_vi, imp_en) = if dirty {
            (
                "keep",
                95u8,
                format!("Worktree AI `{name}` ĐANG CÓ THAY ĐỔI CHƯA COMMIT trong git — NÊN GIỮ LẠI!"),
                format!("AI worktree `{name}` HAS UNCOMMITTED GIT CHANGES — KEEP or commit first!"),
                "Nếu xóa ngay bạn sẽ MẤT code AI vừa sửa chưa commit.",
                "Deleting now will LOSE uncommitted edits inside this worktree.",
            )
        } else if age_days >= stale_days {
            (
                "safe_delete",
                92u8,
                format!("Worktree AI `{name}` sạch (0 file chưa commit) & bỏ trống {age_days} ngày — XÓA AN TOÀN."),
                format!("AI worktree `{name}` is clean (0 uncommitted changes) and idle {age_days}d — SAFE TO DELETE."),
                "Không có thay đổi chưa lưu; xóa giải phóng toàn bộ bản sao repo/build thừa.",
                "No uncommitted work; removing reclaims the duplicated repo & build artifacts.",
            )
        } else {
            (
                "review",
                70u8,
                format!("Worktree AI `{name}` sạch nhưng mới dùng {age_days} ngày trước — XEM LẠI xem agent còn cần không."),
                format!("AI worktree `{name}` is clean but modified {age_days}d ago — REVIEW before removing."),
                "Không có thay đổi chưa commit, nhưng phiên AI gần đây có thể đang trỏ vào đây.",
                "Clean working tree, though a recent AI session may still reference this path.",
            )
        };
        push_unique(
            out,
            seen,
            AiJunkFinding {
                id: format!("aiwt:{}", path_key(&p)),
                path: p,
                name: format!("worktree/{name}"),
                category: "ai_worktree".into(),
                verdict: verdict.into(),
                confidence,
                bytes,
                files,
                age_days,
                modified,
                git_dirty: dirty,
                rule_id: "SLC016".into(),
                reason_vi,
                reason_en,
                impact_vi: imp_vi.into(),
                impact_en: imp_en.into(),
            },
        );
    }
}

/// Walks project/workspace roots looking for test caches, crash dumps, AI scratch dirs, and `.worktrees`.
fn walk_test_junk(
    dir: &Path,
    depth: usize,
    stale_days: u64,
    now: u64,
    seen: &mut HashSet<String>,
    out: &mut Vec<AiJunkFinding>,
) {
    if depth > MAX_WALK_DEPTH || out.len() >= MAX_FINDINGS {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if out.len() >= MAX_FINDINGS {
            break;
        }
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let lower = name.to_ascii_lowercase();

        if ft.is_file() {
            if is_crash_or_debug_file(&lower) {
                let (bytes, files) = measure_path(&path);
                let modified = modified_epoch(&path);
                let age_days = age_days_from(modified, now);
                push_unique(
                    out,
                    seen,
                    AiJunkFinding {
                        id: format!("aidump:{}", path_key(&path)),
                        path,
                        name,
                        category: "ai_test_junk".into(),
                        verdict: "safe_delete".into(),
                        confidence: 98,
                        bytes,
                        files,
                        age_days,
                        modified,
                        git_dirty: false,
                        rule_id: "SLC017".into(),
                        reason_vi: "File log crash / heap dump (.hprof / .dmp / hs_err) sinh ra khi test bị lỗi — XÓA AN TOÀN 100%.".into(),
                        reason_en: "Crash dump or debug log (.hprof / .dmp / hs_err) left from a failed run — 100% SAFE TO DELETE.".into(),
                        impact_vi: "Không ảnh hưởng bất kỳ ứng dụng hay mã nguồn nào.".into(),
                        impact_en: "Zero impact on any application or source code.".into(),
                    },
                );
            }
            continue;
        }

        if !ft.is_dir() {
            continue;
        }

        // Check .worktrees parent folders
        if WORKTREE_PARENTS.contains(&lower.as_str()) {
            scan_worktree_children(&path, stale_days, now, seen, out);
            continue;
        }

        // Check known regenerable test cache/report folders
        if let Some(&(_, vi_desc, en_desc)) = SAFE_TEST_DIRS.iter().find(|&&(k, _, _)| k == lower) {
            let (bytes, files) = measure_path(&path);
            if bytes > 0 || files > 0 {
                let modified = modified_epoch(&path);
                let age_days = age_days_from(modified, now);
                push_unique(
                    out,
                    seen,
                    AiJunkFinding {
                        id: format!("aitest:{}", path_key(&path)),
                        path,
                        name,
                        category: "ai_test_junk".into(),
                        verdict: "safe_delete".into(),
                        confidence: 98,
                        bytes,
                        files,
                        age_days,
                        modified,
                        git_dirty: false,
                        rule_id: "SLC017".into(),
                        reason_vi: format!("{vi_desc} (bỏ trống {age_days} ngày). XÓA AN TOÀN 100% — tự tạo lại khi chạy test."),
                        reason_en: format!("{en_desc} (idle {age_days}d). 100% SAFE TO DELETE — automatically rebuilt on next test run."),
                        impact_vi: "Chạy lại lệnh test sẽ tự sinh lại, KHÔNG mất bất kỳ code nào.".into(),
                        impact_en: "Re-running tests recreates this folder automatically; zero source code loss.".into(),
                    },
                );
            }
            continue;
        }

        // Check Cypress videos / screenshots
        if lower == "cypress" {
            for sub in ["videos", "screenshots"] {
                let sp = path.join(sub);
                if sp.is_dir() {
                    let (bytes, files) = measure_path(&sp);
                    if bytes > 0 {
                        let modified = modified_epoch(&sp);
                        let age_days = age_days_from(modified, now);
                        push_unique(
                            out,
                            seen,
                            AiJunkFinding {
                                id: format!("aitest:{}", path_key(&sp)),
                                path: sp,
                                name: format!("cypress/{sub}"),
                                category: "ai_test_junk".into(),
                                verdict: "safe_delete".into(),
                                confidence: 97,
                                bytes,
                                files,
                                age_days,
                                modified,
                                git_dirty: false,
                                rule_id: "SLC017".into(),
                                reason_vi: format!("Video/ảnh chụp chạy test Cypress ({files} file, {age_days} ngày) — XÓA AN TOÀN 100%."),
                                reason_en: format!("Cypress test capture artifacts ({files} files, {age_days}d old) — 100% SAFE TO DELETE."),
                                impact_vi: "Chỉ là video/ảnh quay lại lần test cũ, không ảnh hưởng code.".into(),
                                impact_en: "Past test recordings only; zero impact on source code.".into(),
                            },
                        );
                    }
                }
            }
            continue;
        }

        // Check AI-created scratch / throwaway test folders (e.g. tmp-*, codex-test*, scratch-*)
        if SCRATCH_PREFIXES.iter().any(|pref| lower.starts_with(pref)) {
            let (bytes, files) = measure_path(&path);
            let modified = modified_epoch(&path);
            let age_days = age_days_from(modified, now);
            let dirty = is_git_dirty(&path);
            let (verdict, confidence, reason_vi, reason_en, imp_vi, imp_en) = if dirty {
                (
                    "keep",
                    94u8,
                    format!("Thư mục test tạm `{name}` có git chưa commit — NÊN GIỮ hoặc kiểm tra trước."),
                    format!("Scratch folder `{name}` has uncommitted git changes — KEEP or inspect first."),
                    "Chứa file sửa đổi chưa commit.",
                    "Contains uncommitted git modifications.",
                )
            } else if age_days >= stale_days {
                (
                    "safe_delete",
                    91u8,
                    format!("Thư mục test rác / thử nghiệm `{name}` đã bỏ trống {age_days} ngày, không có thay đổi git chưa lưu — XÓA AN TOÀN."),
                    format!("Ephemeral test/scratch directory `{name}` idle for {age_days}d with no uncommitted git work — SAFE TO DELETE."),
                    "Thư mục thử nghiệm cũ do AI/dev tạo tạm; nên đưa vào Thùng rác để dọn nhẹ máy.",
                    "Old throwaway test directory; safe to recycle and reclaim space.",
                )
            } else if age_days < 2 {
                (
                    "keep",
                    82u8,
                    format!("Thư mục tạm `{name}` vừa được sửa gần đây ({age_days} ngày) — NÊN GIỮ đề phòng phiên AI đang chạy."),
                    format!("Scratch folder `{name}` was touched recently ({age_days}d ago) — KEEP while active."),
                    "Có thể đang được một phiên test hoặc AI sử dụng.",
                    "May still be in use by an active AI or test run.",
                )
            } else {
                (
                    "review",
                    72u8,
                    format!("Thư mục test tạm `{name}` ({age_days} ngày, {files} file) — XEM LẠI xem còn cần kết quả thử nghiệm không."),
                    format!("Temporary test folder `{name}` ({age_days}d old, {files} files) — REVIEW before deleting."),
                    "Mở xem nhanh để chắc không còn file cần giữ lại.",
                    "Quickly inspect to confirm no needed test output remains.",
                )
            };
            push_unique(
                out,
                seen,
                AiJunkFinding {
                    id: format!("aiscratch:{}", path_key(&path)),
                    path,
                    name,
                    category: "ai_test_junk".into(),
                    verdict: verdict.into(),
                    confidence,
                    bytes,
                    files,
                    age_days,
                    modified,
                    git_dirty: dirty,
                    rule_id: "SLC017".into(),
                    reason_vi,
                    reason_en,
                    impact_vi: imp_vi.into(),
                    impact_en: imp_en.into(),
                },
            );
            continue;
        }

        if SKIP_WALK_DIRS.contains(&lower.as_str()) {
            continue;
        }
        walk_test_junk(&path, depth + 1, stale_days, now, seen, out);
    }
}

const DOCS_AI_DIR_PREFIXES: &[&str] = &[
    "codex",
    ".codex",
    "claude-",
    "claude_",
    "ai-test",
    "ai_test",
    "agent-",
    "openai-",
    "tmp-",
    "tmp_",
    "temp-",
    "test-",
    "test_",
    "scratch",
    "sandbox",
    "playground",
    "demo-",
    "repro-",
    "bench-",
    "mock-",
    "smoke-",
    "verify-",
    "check-",
    "mckit-",
];

const DOCS_PROTECTED_EXTS: &[&str] = &[
    "doc", "docx", "xls", "xlsx", "ppt", "pptx", "pdf", "odt", "ods", "odp", "rtf",
    "psd", "ai", "fig", "mp4", "mkv", "mov", "mp3", "wav", "flac", "zip", "rar", "7z",
];

const DOCS_JUNK_FILE_EXTS: &[&str] = &["log", "jsonl", "tmp", "bak", "dmp", "hprof", "trace", "har"];

const DOCS_DEV_ARTIFACT_MARKERS: &[&str] = &[
    "node_modules",
    "target",
    ".pytest_cache",
    "playwright-report",
    "test-results",
    "__pycache__",
    ".venv",
    "htmlcov",
    ".vitest",
];

/// Deeply inspects Windows/Unix `Documents` (and `OneDrive/Documents`) for Codex/AI throwaway
/// test directories, stray dev build outputs, and loose test/debug dump files — while strictly
/// protecting personal Office/PDF/media documents.
fn scan_documents_codex_junk(
    docs_dir: &Path,
    stale_days: u64,
    now: u64,
    seen: &mut HashSet<String>,
    out: &mut Vec<AiJunkFinding>,
) {
    let Ok(entries) = std::fs::read_dir(docs_dir) else { return };
    for entry in entries.flatten() {
        if out.len() >= MAX_FINDINGS {
            break;
        }
        let Ok(ft) = entry.file_type() else { continue };
        if ft.is_symlink() {
            continue;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let lower = name.to_ascii_lowercase();

        if ft.is_file() {
            let ext = path
                .extension()
                .and_then(|e| e.to_str())
                .map(|s| s.to_ascii_lowercase())
                .unwrap_or_default();
            if DOCS_PROTECTED_EXTS.contains(&ext.as_str()) {
                continue;
            }
            let is_junk_ext = DOCS_JUNK_FILE_EXTS.contains(&ext.as_str());
            let is_codex_test_file = (lower.starts_with("codex")
                || lower.starts_with("test_")
                || lower.starts_with("test-")
                || lower.starts_with("debug_")
                || lower.starts_with("tmp_")
                || lower.starts_with("trace_")
                || lower.starts_with("output_"))
                && matches!(ext.as_str(), "txt" | "json" | "jsonl" | "log" | "html" | "py" | "js" | "ts" | "ps1" | "bat" | "md");
            if !is_junk_ext && !is_codex_test_file {
                continue;
            }
            let (bytes, files) = measure_path(&path);
            if bytes == 0 {
                continue;
            }
            let modified = modified_epoch(&path);
            let age_days = age_days_from(modified, now);
            let (verdict, confidence) = if is_junk_ext || age_days >= stale_days {
                ("safe_delete", 94u8)
            } else {
                ("review", 72u8)
            };
            push_unique(
                out,
                seen,
                AiJunkFinding {
                    id: format!("docsjunk:{}", path_key(&path)),
                    path,
                    name: format!("Documents/{name}"),
                    category: "documents_junk".into(),
                    verdict: verdict.into(),
                    confidence,
                    bytes,
                    files,
                    age_days,
                    modified,
                    git_dirty: false,
                    rule_id: "SLC019".into(),
                    reason_vi: format!(
                        "File rác test/log của Codex hoặc AI nằm trong Documents ({age_days} ngày). Không phải tài liệu Office/PDF cá nhân — {}.",
                        if verdict == "safe_delete" { "XÓA AN TOÀN 100%" } else { "XEM LẠI trước khi xóa" }
                    ),
                    reason_en: format!(
                        "Stray Codex/AI test or log file inside Documents ({age_days}d old). Not a personal Office/PDF document — {}.",
                        if verdict == "safe_delete" { "100% SAFE TO DELETE" } else { "REVIEW before deleting" }
                    ),
                    impact_vi: "Không ảnh hưởng tài liệu cá nhân hay mã nguồn dự án chính.".into(),
                    impact_en: "Zero impact on personal documents or main project repositories.".into(),
                },
            );
            continue;
        }

        if !ft.is_dir() {
            continue;
        }
        // Skip standard Windows Documents subfolders (My Music, My Pictures, PowerShell, etc.)
        if matches!(
            lower.as_str(),
            "my music" | "my pictures" | "my videos" | "windowspowershell" | "powershell" | "iisexpress" | "visual studio 2022" | "custom office templates"
        ) {
            continue;
        }

        let name_looks_codex_test = DOCS_AI_DIR_PREFIXES.iter().any(|p| lower.starts_with(p));
        let found_markers: Vec<&str> = DOCS_DEV_ARTIFACT_MARKERS
            .iter()
            .copied()
            .filter(|m| path.join(m).exists())
            .collect();

        if !name_looks_codex_test && found_markers.is_empty() {
            continue;
        }

        let (bytes, files) = measure_path(&path);
        if bytes == 0 && files == 0 {
            continue;
        }
        let modified = modified_epoch(&path);
        let age_days = age_days_from(modified, now);
        let dirty = is_git_dirty(&path);
        let marker_note = if found_markers.is_empty() {
            String::new()
        } else {
            format!(" [chứa {}]", found_markers.join(", "))
        };
        let marker_note_en = if found_markers.is_empty() {
            String::new()
        } else {
            format!(" [contains {}]", found_markers.join(", "))
        };

        let (verdict, confidence, reason_vi, reason_en, imp_vi, imp_en) = if dirty {
            (
                "keep",
                94u8,
                format!("Thư mục `{name}` trong Documents{marker_note} ĐANG CÓ CODE GIT CHƯA COMMIT — NÊN GIỮ / commit trước!"),
                format!("Folder `{name}` in Documents{marker_note_en} HAS UNCOMMITTED GIT CHANGES — KEEP or commit first!"),
                "Xóa ngay có thể làm mất thay đổi code chưa lưu.",
                "Deleting now may lose uncommitted edits.",
            )
        } else if name_looks_codex_test && (age_days >= stale_days || !found_markers.is_empty()) {
            (
                "safe_delete",
                95u8,
                format!("Thư mục test/rác Codex `{name}` nằm lạc trong Documents{marker_note} ({files} file, bỏ trống {age_days} ngày) — XÓA AN TOÀN 100%!"),
                format!("Stray Codex/AI test folder `{name}` in Documents{marker_note_en} ({files} files, idle {age_days}d) — 100% SAFE TO DELETE!"),
                "Thư mục chạy thử của Codex/AI bị bỏ quên trong Documents; đưa vào Thùng rác giúp nhẹ ổ C ngay lập tức.",
                "Abandoned AI/Codex scratch folder in Documents; recycling immediately reclaims disk space.",
            )
        } else if !found_markers.is_empty() && age_days >= stale_days {
            (
                "review",
                80u8,
                format!("Thư mục `{name}` trong Documents chứa artifact build/test nặng{marker_note} (bỏ trống {age_days} ngày) — XEM LẠI xem có phải dự án test cũ không."),
                format!("Folder `{name}` in Documents holds heavy build/test artifacts{marker_note_en} (idle {age_days}d) — REVIEW if it is an old test project."),
                "Mở nhanh Explorer kiểm tra; nếu là dự án thử nghiệm cũ thì xóa an toàn.",
                "Quickly inspect in Explorer; safe to remove if it was a throwaway test repo.",
            )
        } else {
            (
                "review",
                70u8,
                format!("Thư mục `{name}` trong Documents có dấu hiệu test/Codex (mới chạm {age_days} ngày trước) — XEM LẠI trước khi xóa."),
                format!("Folder `{name}` in Documents looks like a recent Codex/test workspace ({age_days}d ago) — REVIEW before removing."),
                "Có thể phiên Codex gần đây vừa tạo ra.",
                "May have been created by a recent Codex session.",
            )
        };

        push_unique(
            out,
            seen,
            AiJunkFinding {
                id: format!("docsjunk:{}", path_key(&path)),
                path,
                name: format!("Documents/{name}"),
                category: "documents_junk".into(),
                verdict: verdict.into(),
                confidence,
                bytes,
                files,
                age_days,
                modified,
                git_dirty: dirty,
                rule_id: "SLC019".into(),
                reason_vi,
                reason_en,
                impact_vi: imp_vi.into(),
                impact_en: imp_en.into(),
            },
        );
    }
}

/// Scans `Downloads`, `Desktop`, and installer temp directories for forgotten setup packages.
fn scan_installer_dir(
    dir: &Path,
    stale_days: u64,
    min_bytes: u64,
    now: u64,
    seen: &mut HashSet<String>,
    out: &mut Vec<AiJunkFinding>,
) {
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        if out.len() >= MAX_FINDINGS {
            break;
        }
        let Ok(ft) = entry.file_type() else { continue };
        if !ft.is_file() || ft.is_symlink() {
            continue;
        }
        let path = entry.path();
        let name = entry.file_name().to_string_lossy().to_string();
        let lower = name.to_ascii_lowercase();
        if !is_installer_file(&path, &lower) {
            continue;
        }
        let bytes = std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
        if bytes < min_bytes {
            continue;
        }
        let modified = modified_epoch(&path);
        let age_days = age_days_from(modified, now);
        let (verdict, confidence, reason_vi, reason_en) = if age_days >= stale_days {
            (
                "safe_delete",
                93u8,
                format!("Bộ cài đặt `{name}` đã tải {age_days} ngày trước. Nếu đã cài xong hoặc đã gỡ app thì XÓA AN TOÀN 100%."),
                format!("Installer package `{name}` downloaded {age_days}d ago. If already installed or uninstalled, 100% SAFE TO DELETE."),
            )
        } else {
            (
                "review",
                70u8,
                format!("Bộ cài đặt `{name}` mới tải {age_days} ngày trước — XEM LẠI xem bạn đã cài xong chưa trước khi xóa."),
                format!("Installer package `{name}` downloaded {age_days}d ago — REVIEW to confirm installation finished."),
            )
        };
        push_unique(
            out,
            seen,
            AiJunkFinding {
                id: format!("inst:{}", path_key(&path)),
                path,
                name,
                category: "leftover_installer".into(),
                verdict: verdict.into(),
                confidence,
                bytes,
                files: 1,
                age_days,
                modified,
                git_dirty: false,
                rule_id: "SLC018".into(),
                reason_vi,
                reason_en,
                impact_vi: "Xóa bộ cài KHÔNG gỡ ứng dụng đã cài trên máy; cần thì luôn tải lại được bản mới nhất.".into(),
                impact_en: "Deleting the installer does NOT uninstall the app; you can always redownload the latest setup.".into(),
            },
        );
    }
}

/// Runs a targeted scan over explicit roots (used by CLI, MCP, and unit tests).
pub fn scan_with_options(
    project_roots: &[PathBuf],
    installer_dirs: &[PathBuf],
    ai_home: Option<&Path>,
    stale_days: u64,
    min_installer_bytes: u64,
    now_override: Option<u64>,
) -> AiJunkReport {
    let now = now_override.unwrap_or_else(now_epoch);
    let mut findings = Vec::new();
    let mut seen = HashSet::new();
    let mut scanned_roots = Vec::new();

    if let Some(home) = ai_home {
        if home.is_dir() {
            scanned_roots.push(home.to_path_buf());
            scan_ai_agent_home(home, stale_days, now, &mut seen, &mut findings);
            for docs_rel in ["Documents", "OneDrive/Documents"] {
                let d = home.join(docs_rel);
                if d.is_dir() {
                    scanned_roots.push(d.clone());
                    scan_documents_codex_junk(&d, stale_days, now, &mut seen, &mut findings);
                }
            }
        }
    }

    for root in project_roots {
        if root.is_dir() {
            scanned_roots.push(root.clone());
            walk_test_junk(root, 0, stale_days, now, &mut seen, &mut findings);
        }
    }

    for inst_dir in installer_dirs {
        if inst_dir.is_dir() {
            scanned_roots.push(inst_dir.clone());
            scan_installer_dir(inst_dir, stale_days, min_installer_bytes, now, &mut seen, &mut findings);
        }
    }

    // Sort: safe_delete first, then review, then keep; largest bytes first within each verdict.
    findings.sort_by(|a, b| {
        let rank = |v: &str| match v {
            "safe_delete" => 0u8,
            "review" => 1u8,
            _ => 2u8,
        };
        rank(&a.verdict)
            .cmp(&rank(&b.verdict))
            .then_with(|| b.bytes.cmp(&a.bytes))
            .then_with(|| a.path.cmp(&b.path))
    });

    let mut total_bytes = 0u64;
    let mut safe_delete_count = 0usize;
    let mut safe_delete_bytes = 0u64;
    let mut review_count = 0usize;
    let mut review_bytes = 0u64;
    let mut keep_count = 0usize;
    let mut keep_bytes = 0u64;

    for f in &findings {
        total_bytes = total_bytes.saturating_add(f.bytes);
        match f.verdict.as_str() {
            "safe_delete" => {
                safe_delete_count += 1;
                safe_delete_bytes = safe_delete_bytes.saturating_add(f.bytes);
            }
            "review" => {
                review_count += 1;
                review_bytes = review_bytes.saturating_add(f.bytes);
            }
            _ => {
                keep_count += 1;
                keep_bytes = keep_bytes.saturating_add(f.bytes);
            }
        }
    }

    AiJunkReport {
        scanned_at: now,
        stale_days,
        scanned_roots,
        total_count: findings.len(),
        total_bytes,
        safe_delete_count,
        safe_delete_bytes,
        review_count,
        review_bytes,
        keep_count,
        keep_bytes,
        findings,
    }
}

/// Runs a default user-machine scan covering AI home dirs, Downloads/Temp installers, and dev roots.
pub fn scan_default(extra_roots: &[PathBuf], stale_days: u64) -> AiJunkReport {
    let home = dirs::home_dir();
    let mut project_roots: Vec<PathBuf> = extra_roots.to_vec();
    let mut installer_dirs: Vec<PathBuf> = Vec::new();

    if let Some(ref h) = home {
        if project_roots.is_empty() {
            for candidate in ["work", "projects", "Projects", "dev", "Dev", "code", "Code", "source", "repos", "Documents"] {
                let p = h.join(candidate);
                if p.is_dir() {
                    project_roots.push(p);
                }
            }
        }
        for inst in ["Downloads", "Desktop", "AppData/Local/SquirrelTemp", "AppData/Local/Downloaded Installations"] {
            let p = h.join(inst);
            if p.is_dir() {
                installer_dirs.push(p);
            }
        }
    }
    let sys_temp = std::env::temp_dir();
    if sys_temp.is_dir() {
        installer_dirs.push(sys_temp);
    }

    scan_with_options(
        &project_roots,
        &installer_dirs,
        home.as_deref(),
        stale_days,
        DEFAULT_MIN_INSTALLER_BYTES,
        None,
    )
}

fn html_esc(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
}

fn fmt_mb(bytes: u64) -> String {
    if bytes >= 1024 * 1024 * 1024 {
        format!("{:.2} GiB", bytes as f64 / (1024.0 * 1024.0 * 1024.0))
    } else if bytes >= 1024 * 1024 {
        format!("{:.1} MiB", bytes as f64 / (1024.0 * 1024.0))
    } else if bytes >= 1024 {
        format!("{:.1} KiB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} B")
    }
}

pub fn render_html(report: &AiJunkReport) -> String {
    let mut rows = String::new();
    for f in &report.findings {
        let badge = match f.verdict.as_str() {
            "safe_delete" => "<span style=\"color:#4ade80;font-weight:700\">SAFE DELETE</span>",
            "review" => "<span style=\"color:#fbbf24;font-weight:700\">REVIEW</span>",
            _ => "<span style=\"color:#60a5fa;font-weight:700\">KEEP</span>",
        };
        rows.push_str(&format!(
            "<tr><td>{}</td><td><code>{}</code></td><td>{}</td><td>{}</td><td>{}d</td><td><b>{}</b><br><small>{}</small><br><small style=\"color:#93a89b\">{}</small></td></tr>\n",
            badge,
            html_esc(&f.category),
            html_esc(&fmt_mb(f.bytes)),
            f.confidence,
            f.age_days,
            html_esc(&f.name),
            html_esc(&f.reason_en),
            html_esc(&f.path.to_string_lossy()),
        ));
    }
    if rows.is_empty() {
        rows.push_str("<tr><td colspan=\"6\" style=\"color:#4ade80\">No AI test junk or leftover installers detected.</td></tr>");
    }
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><title>SLClean AI Junk &amp; Leftover Decision Report</title>\
        <style>body{{margin:0;padding:28px;background:#0f1411;color:#e6efe9;font-family:system-ui,sans-serif}}.wrap{{max-width:1120px;margin:0 auto}}.card{{background:#161d19;border:1px solid #2a3730;border-radius:10px;padding:18px;margin-bottom:16px}}table{{width:100%;border-collapse:collapse;font-size:12.5px}}th,td{{padding:8px 10px;border-bottom:1px solid #24302a;text-align:left;vertical-align:top}}th{{color:#93a89b}}code{{font-family:ui-monospace,monospace;color:#4ade80}}</style>\
        </head><body><div class=\"wrap\">\
        <div class=\"card\"><h1>SLClean AI-Agent Junk &amp; Leftover Decision Report</h1>\
        <p>Safe Delete: <b style=\"color:#4ade80\">{} ({})</b> &middot; Review: <b style=\"color:#fbbf24\">{} ({})</b> &middot; Keep: <b style=\"color:#60a5fa\">{} ({})</b></p></div>\
        <div class=\"card\"><table><thead><tr><th>Verdict</th><th>Category</th><th>Size</th><th>Conf%</th><th>Age</th><th>Item &amp; Decision Reason</th></tr></thead><tbody>{}</tbody></table></div>\
        </div></body></html>",
        report.safe_delete_count,
        html_esc(&fmt_mb(report.safe_delete_bytes)),
        report.review_count,
        html_esc(&fmt_mb(report.review_bytes)),
        report.keep_count,
        html_esc(&fmt_mb(report.keep_bytes)),
        rows
    )
}

pub fn render_badge_json(report: &AiJunkReport) -> serde_json::Value {
    let safe_mb = report.safe_delete_bytes as f64 / (1024.0 * 1024.0);
    let color = if report.safe_delete_bytes >= 500 * 1024 * 1024 {
        "orange"
    } else if report.safe_delete_count > 0 {
        "yellow"
    } else {
        "brightgreen"
    };
    serde_json::json!({
        "schemaVersion": 1,
        "label": "slclean ai-junk",
        "message": format!("{} safe ({:.1} MiB) · {} review · {} keep", report.safe_delete_count, safe_mb, report.review_count, report.keep_count),
        "color": color,
    })
}

pub fn render_sarif(report: &AiJunkReport) -> serde_json::Value {
    let results: Vec<serde_json::Value> = report
        .findings
        .iter()
        .filter(|f| f.verdict != "keep")
        .map(|f| {
            let level = if f.verdict == "safe_delete" { "warning" } else { "note" };
            serde_json::json!({
                "ruleId": f.rule_id,
                "level": level,
                "message": { "text": format!("[{} {}%] {} ({}) — {}", f.verdict.to_uppercase(), f.confidence, f.name, fmt_mb(f.bytes), f.reason_en) },
                "locations": [{
                    "physicalLocation": {
                        "artifactLocation": { "uri": f.path.to_string_lossy().replace('\\', "/") }
                    }
                }]
            })
        })
        .collect();
    serde_json::json!({
        "$schema": "https://json.schemastore.org/sarif-2.1.0.json",
        "version": "2.1.0",
        "runs": [{
            "tool": {
                "driver": {
                    "name": "SLClean",
                    "version": env!("CARGO_PKG_VERSION"),
                    "rules": [
                        { "id": "SLC016", "name": "AiAgentResidueOrWorktree", "shortDescription": { "text": "Stale AI agent debug logs, tool dumps, or orphaned git worktrees" } },
                        { "id": "SLC017", "name": "EphemeralTestJunkOrCrashDump", "shortDescription": { "text": "Regenerable test runner caches, crash dumps, or abandoned scratch directories" } },
                        { "id": "SLC018", "name": "LeftoverInstallerPackage", "shortDescription": { "text": "Forgotten setup installer files (.exe/.msi/.msix) left in Downloads or Temp" } },
                        { "id": "SLC019", "name": "DocumentsAiTestResidue", "shortDescription": { "text": "Stray Codex/AI test folders, build caches, or debug logs accumulated inside Documents" } }
                    ]
                }
            },
            "results": results
        }]
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_ai_test_junk_installers_and_dirty_git_worktree() {
        let base = std::env::temp_dir().join(format!("slclean_ai_junk_test_{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let proj = base.join("projects/demo");
        let downloads = base.join("Downloads");
        let fake_home = base.join("home");

        // 1. Regenerable test cache & crash dump -> safe_delete (SLC017)
        std::fs::create_dir_all(proj.join(".pytest_cache")).unwrap();
        std::fs::write(proj.join(".pytest_cache/v.bin"), vec![1u8; 4096]).unwrap();
        std::fs::create_dir_all(proj.join("playwright-report")).unwrap();
        std::fs::write(proj.join("playwright-report/index.html"), "<html>ok</html>").unwrap();
        std::fs::write(proj.join("hs_err_pid1234.log"), "fatal jvm crash").unwrap();

        // 2. Claude debug residue -> safe_delete (SLC016)
        std::fs::create_dir_all(fake_home.join(".claude/debug")).unwrap();
        std::fs::write(fake_home.join(".claude/debug/session.log"), vec![2u8; 2048]).unwrap();

        // 3. Leftover installer -> safe_delete when stale_days = 0 (SLC018)
        std::fs::create_dir_all(&downloads).unwrap();
        std::fs::write(downloads.join("RandomTool-Setup-x64.exe"), vec![3u8; 2048]).unwrap();

        // 4. Dirty git worktree under .worktrees/agent-wip -> MUST be KEEP!
        let wt = proj.join(".worktrees/agent-wip");
        std::fs::create_dir_all(&wt).unwrap();
        let _ = Command::new("git").args(["init", "-q"]).current_dir(&wt).status();
        std::fs::write(wt.join("uncommitted.txt"), "do not delete me").unwrap();

        // 5. Stray Codex test directory & log inside Documents -> safe_delete (SLC019), personal .docx protected!
        let docs = fake_home.join("Documents");
        std::fs::create_dir_all(docs.join("codex-test-runner/node_modules")).unwrap();
        std::fs::write(docs.join("codex-test-runner/node_modules/pkg.js"), vec![5u8; 4096]).unwrap();
        std::fs::write(docs.join("codex_debug.log"), "codex trace output").unwrap();
        std::fs::write(docs.join("MyImportantReport.docx"), "personal document").unwrap();

        // Use future `now` (+20 days) so fresh files appear 20 days old (stale_days=7)
        let future_now = now_epoch() + 20 * 86_400;
        let rep = scan_with_options(
            &[base.join("projects")],
            &[downloads],
            Some(&fake_home),
            7,
            1024,
            Some(future_now),
        );

        assert!(rep.safe_delete_count >= 7, "expected >=7 safe_delete items, got {:?}", rep.findings);
        assert!(rep.findings.iter().any(|f| f.name == ".pytest_cache" && f.verdict == "safe_delete" && f.rule_id == "SLC017"));
        assert!(rep.findings.iter().any(|f| f.name == "playwright-report" && f.verdict == "safe_delete"));
        assert!(rep.findings.iter().any(|f| f.name == "hs_err_pid1234.log" && f.verdict == "safe_delete"));
        assert!(rep.findings.iter().any(|f| f.name == ".claude/debug" && f.verdict == "safe_delete" && f.rule_id == "SLC016"));
        assert!(rep.findings.iter().any(|f| f.name == "RandomTool-Setup-x64.exe" && f.verdict == "safe_delete" && f.rule_id == "SLC018"));
        assert!(rep.findings.iter().any(|f| f.category == "documents_junk" && f.name == "Documents/codex-test-runner" && f.verdict == "safe_delete" && f.rule_id == "SLC019"));
        assert!(rep.findings.iter().any(|f| f.category == "documents_junk" && f.name == "Documents/codex_debug.log" && f.verdict == "safe_delete"));
        assert!(!rep.findings.iter().any(|f| f.name.contains("MyImportantReport.docx")), "personal Office documents must NEVER be flagged");

        let wt_item = rep.findings.iter().find(|f| f.category == "ai_worktree").expect("worktree finding");
        assert!(wt_item.git_dirty, "dirty worktree must be detected");
        assert_eq!(wt_item.verdict, "keep", "dirty worktree must NEVER be marked safe_delete");

        let html = render_html(&rep);
        assert!(html.contains("<!doctype html>"));
        assert!(html.contains("SAFE DELETE"));
        assert!(html.contains("KEEP"));

        let sarif = render_sarif(&rep);
        assert_eq!(sarif["version"], "2.1.0");
        assert_eq!(sarif["runs"][0]["tool"]["driver"]["rules"].as_array().unwrap().len(), 4);

        let badge = render_badge_json(&rep);
        assert_eq!(badge["label"], "slclean ai-junk");

        let _ = std::fs::remove_dir_all(&base);
    }
}
