//! Cross-platform Startup Impact & Broken Entry Auditor (`SLC011` / MCP `slclean_audit_startup_entries`).
//!
//! Strictly read-only: classifies Windows startup entries (`Run`, Startup folder, Scheduled Tasks)
//! by boot overhead (`high` / `medium` / `low` / `system-protected` / `broken-target`),
//! estimates cumulative login latency overhead in milliseconds, and flags stale enabled launchers
//! so users and AI agents can safely decide what to toggle off (never auto-deleting).

use serde::{Deserialize, Serialize};
use std::path::Path;

#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct StartupAuditInput {
    pub name: String,
    pub command: String,
    #[serde(default = "default_true")]
    pub enabled: bool,
    #[serde(default)]
    pub broken: bool,
    #[serde(default)]
    pub system: bool,
    #[serde(default)]
    pub stale: bool,
}

fn default_true() -> bool {
    true
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "kebab-case")]
pub enum BootImpact {
    High,
    Medium,
    Low,
    Broken,
    Protected,
}

#[derive(Clone, Debug, Serialize)]
pub struct StartupAuditItem {
    pub name: String,
    pub command: String,
    pub enabled: bool,
    pub impact: BootImpact,
    pub category: String,
    pub estimated_boot_ms: u32,
    pub recommended_action: String,
    pub reason: String,
}

#[derive(Clone, Debug, Serialize)]
pub struct StartupAuditReport {
    pub total_entries: usize,
    pub enabled_count: usize,
    pub high_impact_count: usize,
    pub updater_count: usize,
    pub broken_count: usize,
    pub stale_enabled_count: usize,
    pub estimated_boot_overhead_ms: u64,
    pub boot_health_score: u8,
    pub grade: String,
    pub items: Vec<StartupAuditItem>,
}

const SYSTEM_MARKERS: &[&str] = &[
    "securityhealthsystray",
    "windows defender",
    "\\windows\\system32\\",
    "/windows/system32/",
    "ctfmon.exe",
    "igfxtray",
    "rtkauduservice",
    "realtek",
    "nvbackend",
    "amdnoise",
];

const HEAVY_LAUNCHERS: &[(&str, &str, u32)] = &[
    ("docker desktop", "container-runtime", 3400),
    ("docker.exe", "container-runtime", 3200),
    ("teams", "chat-electron", 2600),
    ("ms-teams", "chat-electron", 2600),
    ("discord", "chat-electron", 2200),
    ("slack", "chat-electron", 2100),
    ("adobe", "creative-suite", 2400),
    ("creative cloud", "creative-suite", 2400),
    ("ccxprocess", "creative-suite", 1800),
    ("steam", "game-launcher", 1900),
    ("epicgameslauncher", "game-launcher", 2100),
    ("riotclient", "game-launcher", 1800),
    ("battle.net", "game-launcher", 1700),
    ("overwolf", "game-overlay", 2000),
    ("spotify", "media-electron", 1700),
    ("onedrive", "cloud-sync", 1500),
    ("dropbox", "cloud-sync", 1400),
    ("googledrivefs", "cloud-sync", 1400),
    ("postman", "dev-electron", 1800),
    ("zoom", "chat-electron", 1400),
    ("operagx", "browser-preload", 1600),
    ("chrome.exe", "browser-preload", 1300),
    ("msedge.exe", "browser-preload", 1200),
];

const UPDATER_MARKERS: &[&str] = &[
    "update.exe",
    "googleupdate",
    "edgeupdate",
    "adobearm",
    "jusched",
    "javaupdate",
    "braveupdate",
    "mozilla maintenance",
    "autoupdater",
];

fn extract_executable_token(cmd: &str) -> Option<&str> {
    let trimmed = cmd.trim();
    if trimmed.is_empty() {
        return None;
    }
    if let Some(rest) = trimmed.strip_prefix('"') {
        rest.split('"').next().filter(|s| !s.is_empty())
    } else {
        trimmed.split_whitespace().next()
    }
}

/// Classify one startup entry without modifying the OS.
pub fn audit_entry(entry: &StartupAuditInput, verify_local_paths: bool) -> StartupAuditItem {
    let hay = format!("{} {}", entry.name, entry.command).to_ascii_lowercase();
    let path_missing = if verify_local_paths {
        extract_executable_token(&entry.command)
            .map(|tok| {
                let is_local_abs = if cfg!(windows) {
                    tok.contains(":\\") || tok.starts_with("\\\\")
                } else {
                    tok.starts_with('/')
                };
                is_local_abs && !Path::new(tok).exists()
            })
            .unwrap_or(false)
    } else {
        false
    };
    let is_broken = entry.broken || path_missing;
    let is_system = entry.system || SYSTEM_MARKERS.iter().any(|m| hay.contains(m));

    if is_broken {
        return StartupAuditItem {
            name: entry.name.clone(),
            command: entry.command.clone(),
            enabled: entry.enabled,
            impact: BootImpact::Broken,
            category: "broken-target".to_string(),
            estimated_boot_ms: 0,
            recommended_action: "safe-to-remove-dead-entry".to_string(),
            reason: "Target executable is missing; removing this dead entry keeps Run keys clean.".to_string(),
        };
    }

    if is_system {
        return StartupAuditItem {
            name: entry.name.clone(),
            command: entry.command.clone(),
            enabled: entry.enabled,
            impact: BootImpact::Protected,
            category: "system-protected".to_string(),
            estimated_boot_ms: if entry.enabled { 120 } else { 0 },
            recommended_action: "keep-enabled".to_string(),
            reason: "Core Windows / security / driver component; SLClean never recommends disabling it.".to_string(),
        };
    }

    for (needle, category, ms) in HEAVY_LAUNCHERS {
        if hay.contains(needle) {
            let est = if entry.enabled { *ms } else { 0 };
            let action = if !entry.enabled {
                "already-disabled"
            } else if entry.stale {
                "toggle-off-stale-heavy"
            } else {
                "consider-toggle-off"
            };
            let reason = if entry.stale {
                format!("Heavy {} auto-starts at login (~{} ms) but has not been opened recently.", category, ms)
            } else {
                format!("Heavy {} adds ~{} ms to Windows login; launch on demand to speed up boot.", category, ms)
            };
            return StartupAuditItem {
                name: entry.name.clone(),
                command: entry.command.clone(),
                enabled: entry.enabled,
                impact: BootImpact::High,
                category: (*category).to_string(),
                estimated_boot_ms: est,
                recommended_action: action.to_string(),
                reason,
            };
        }
    }

    if UPDATER_MARKERS.iter().any(|m| hay.contains(m)) {
        return StartupAuditItem {
            name: entry.name.clone(),
            command: entry.command.clone(),
            enabled: entry.enabled,
            impact: BootImpact::Medium,
            category: "background-updater".to_string(),
            estimated_boot_ms: if entry.enabled { 450 } else { 0 },
            recommended_action: if entry.enabled {
                "optional-toggle-off".to_string()
            } else {
                "already-disabled".to_string()
            },
            reason: "Background updater polls network/disk on every login; apps still update when opened.".to_string(),
        };
    }

    let action = if !entry.enabled {
        "already-disabled"
    } else if entry.stale {
        "consider-toggle-off"
    } else {
        "keep-if-needed"
    };
    StartupAuditItem {
        name: entry.name.clone(),
        command: entry.command.clone(),
        enabled: entry.enabled,
        impact: BootImpact::Low,
        category: "standard".to_string(),
        estimated_boot_ms: if entry.enabled { 180 } else { 0 },
        recommended_action: action.to_string(),
        reason: if entry.stale {
            "Low-overhead entry, but unused for >30 days; safe to toggle off reversibly.".to_string()
        } else {
            "Lightweight startup entry.".to_string()
        },
    }
}

/// Audit a slice of startup entries and compute an executive A-F boot health grade.
pub fn audit_entries(entries: &[StartupAuditInput], verify_local_paths: bool) -> StartupAuditReport {
    let mut items = Vec::with_capacity(entries.len());
    let mut enabled_count = 0usize;
    let mut high_impact_count = 0usize;
    let mut updater_count = 0usize;
    let mut broken_count = 0usize;
    let mut stale_enabled_count = 0usize;
    let mut estimated_boot_overhead_ms = 0u64;

    for e in entries {
        let item = audit_entry(e, verify_local_paths);
        if item.enabled {
            enabled_count += 1;
            estimated_boot_overhead_ms += item.estimated_boot_ms as u64;
            if e.stale && item.impact != BootImpact::Broken && item.impact != BootImpact::Protected {
                stale_enabled_count += 1;
            }
        }
        match item.impact {
            BootImpact::High if item.enabled => high_impact_count += 1,
            BootImpact::Medium if item.enabled => updater_count += 1,
            BootImpact::Broken => broken_count += 1,
            _ => {}
        }
        items.push(item);
    }

    let penalty = (high_impact_count as u32 * 14)
        + (stale_enabled_count as u32 * 8)
        + (updater_count as u32 * 4)
        + (broken_count as u32 * 6)
        + (estimated_boot_overhead_ms / 1500) as u32;
    let boot_health_score = 100u32.saturating_sub(penalty).min(100) as u8;
    let grade = match boot_health_score {
        90..=100 => "A",
        78..=89 => "B",
        62..=77 => "C",
        45..=61 => "D",
        _ => "F",
    }
    .to_string();

    StartupAuditReport {
        total_entries: items.len(),
        enabled_count,
        high_impact_count,
        updater_count,
        broken_count,
        stale_enabled_count,
        estimated_boot_overhead_ms,
        boot_health_score,
        grade,
        items,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_heavy_system_updater_and_broken_entries() {
        let entries = vec![
            StartupAuditInput {
                name: "Discord".into(),
                command: r#""C:\Users\me\AppData\Local\Discord\Update.exe" --processStart Discord.exe"#.into(),
                enabled: true,
                broken: false,
                system: false,
                stale: true,
            },
            StartupAuditInput {
                name: "SecurityHealth".into(),
                command: r"C:\Windows\System32\SecurityHealthSystray.exe".into(),
                enabled: true,
                broken: false,
                system: true,
                stale: false,
            },
            StartupAuditInput {
                name: "OldApp".into(),
                command: "/nonexistent/slclean/oldapp.bin".into(),
                enabled: true,
                broken: false,
                system: false,
                stale: false,
            },
            StartupAuditInput {
                name: "GoogleUpdate".into(),
                command: r"C:\Program Files\Google\Update\GoogleUpdate.exe /c".into(),
                enabled: true,
                broken: false,
                system: false,
                stale: false,
            },
        ];

        let report = audit_entries(&entries, true);
        assert_eq!(report.total_entries, 4);
        assert_eq!(report.high_impact_count, 1);
        assert_eq!(report.broken_count, 1);
        assert_eq!(report.updater_count, 1);
        assert_eq!(report.stale_enabled_count, 1);
        assert_eq!(report.items[0].impact, BootImpact::High);
        assert_eq!(report.items[0].recommended_action, "toggle-off-stale-heavy");
        assert_eq!(report.items[1].impact, BootImpact::Protected);
        assert_eq!(report.items[2].impact, BootImpact::Broken);
        assert_eq!(report.items[3].impact, BootImpact::Medium);
        assert!(report.boot_health_score < 85);
    }

    #[test]
    fn clean_startup_scores_grade_a() {
        let entries = vec![StartupAuditInput {
            name: "SecurityHealth".into(),
            command: r"C:\Windows\System32\SecurityHealthSystray.exe".into(),
            enabled: true,
            broken: false,
            system: true,
            stale: false,
        }];
        let report = audit_entries(&entries, false);
        assert_eq!(report.grade, "A");
        assert!(report.boot_health_score >= 95);
    }
}
