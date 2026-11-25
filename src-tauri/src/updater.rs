//! Lightweight GitHub Releases update checker for SLClean.
//! Zero extra crate dependencies: uses TLS-enabled OS HTTP tooling (`powershell` with
//! `CREATE_NO_WINDOW` on Windows, `curl` on Unix) and parses the official GitHub Releases
//! API JSON via `serde_json`. Strictly validates release URLs before opening a browser.

use serde::Serialize;
use std::process::Command;

pub const RELEASES_API_URL: &str = "https://api.github.com/repos/SalyyS1/SLClean/releases/latest";
pub const ALLOWED_URL_PREFIX: &str = "https://github.com/SalyyS1/SLClean/";

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct UpdateInfo {
    pub current_version: String,
    pub latest_version: String,
    pub update_available: bool,
    pub release_name: String,
    pub release_url: String,
    pub setup_url: Option<String>,
    pub portable_url: Option<String>,
    pub published_at: String,
    pub notes_preview: String,
}

/// Parses `v0.61.0`, `0.61.0-rc1`, etc. into `(major, minor, patch)`.
pub fn parse_semver(raw: &str) -> Option<(u64, u64, u64)> {
    let trimmed = raw.trim().trim_start_matches(['v', 'V']);
    let core = trimmed.split(['-', '+']).next()?;
    let mut parts = core.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next().unwrap_or("0").parse::<u64>().ok()?;
    let patch = parts.next().unwrap_or("0").parse::<u64>().ok()?;
    Some((major, minor, patch))
}

/// Returns `true` strictly when `latest` has a higher numeric `(major, minor, patch)` than `current`.
pub fn is_newer_version(current: &str, latest: &str) -> bool {
    match (parse_semver(current), parse_semver(latest)) {
        (Some(c), Some(l)) => l > c,
        _ => false,
    }
}

/// Validates that a URL points strictly inside the official `SalyyS1/SLClean` GitHub repository.
pub fn validate_release_url(url: &str) -> Result<String, String> {
    let trimmed = url.trim();
    if !trimmed.starts_with(ALLOWED_URL_PREFIX) {
        return Err("Refusing to open non-SLClean URL".into());
    }
    if trimmed.chars().any(|c| c.is_whitespace() || c.is_control() || matches!(c, '"' | '\'' | '&' | '|' | ';' | '`')) {
        return Err("URL contains disallowed characters".into());
    }
    Ok(trimmed.to_string())
}

/// Parses the JSON payload from `GET /repos/SalyyS1/SLClean/releases/latest`.
pub fn parse_github_release_json(current_version: &str, body: &str) -> Result<UpdateInfo, String> {
    let v: serde_json::Value = serde_json::from_str(body).map_err(|e| format!("Invalid GitHub JSON: {e}"))?;
    let tag = v
        .get("tag_name")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "Missing tag_name in GitHub release payload".to_string())?;
    let latest_clean = tag.trim().trim_start_matches(['v', 'V']).to_string();
    let current_clean = current_version.trim().trim_start_matches(['v', 'V']).to_string();
    let release_name = v
        .get("name")
        .and_then(|x| x.as_str())
        .unwrap_or(tag)
        .to_string();
    let release_url = v
        .get("html_url")
        .and_then(|x| x.as_str())
        .unwrap_or("https://github.com/SalyyS1/SLClean/releases")
        .to_string();
    let published_at = v
        .get("published_at")
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    let notes_raw = v.get("body").and_then(|x| x.as_str()).unwrap_or("").trim();
    let notes_preview = notes_raw
        .lines()
        .filter(|l| !l.trim().is_empty())
        .take(4)
        .collect::<Vec<_>>()
        .join("\n");

    let mut setup_url = None;
    let mut portable_url = None;
    if let Some(assets) = v.get("assets").and_then(|x| x.as_array()) {
        for a in assets {
            let name = a.get("name").and_then(|x| x.as_str()).unwrap_or("").to_ascii_lowercase();
            let dl = a.get("browser_download_url").and_then(|x| x.as_str()).map(|s| s.to_string());
            if let Some(u) = dl {
                if validate_release_url(&u).is_err() {
                    continue;
                }
                if name.ends_with("-setup.exe") || name.ends_with("_setup.exe") {
                    setup_url = Some(u);
                } else if name.contains("portable") && name.ends_with(".exe") {
                    portable_url = Some(u);
                }
            }
        }
    }

    Ok(UpdateInfo {
        update_available: is_newer_version(&current_clean, &latest_clean),
        current_version: current_clean,
        latest_version: latest_clean,
        release_name,
        release_url,
        setup_url,
        portable_url,
        published_at,
        notes_preview,
    })
}

/// Fetches the latest published GitHub Release for SLClean and compares it to `CARGO_PKG_VERSION`.
pub fn check_latest_release() -> Result<UpdateInfo, String> {
    let body = fetch_latest_json()?;
    parse_github_release_json(env!("CARGO_PKG_VERSION"), &body)
}

#[cfg(windows)]
fn fetch_latest_json() -> Result<String, String> {
    use std::os::windows::process::CommandExt;
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    let ps = format!(
        "[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12; \
         $r = Invoke-WebRequest -UseBasicParsing -TimeoutSec 8 \
         -Headers @{{'User-Agent'='SLClean-Updater';'Accept'='application/vnd.github+json'}} \
         -Uri '{RELEASES_API_URL}'; \
         [Console]::OutputEncoding = [System.Text.Encoding]::UTF8; \
         Write-Output $r.Content"
    );
    let out = Command::new("powershell.exe")
        .args(["-NoProfile", "-NonInteractive", "-Command", &ps])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("Failed to invoke PowerShell update check: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(if err.is_empty() { "Update check failed (offline?)".into() } else { err });
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

#[cfg(not(windows))]
fn fetch_latest_json() -> Result<String, String> {
    let out = Command::new("curl")
        .args([
            "-fsSL",
            "--max-time",
            "8",
            "-H",
            "User-Agent: SLClean-Updater",
            "-H",
            "Accept: application/vnd.github+json",
            RELEASES_API_URL,
        ])
        .output()
        .map_err(|e| format!("Failed to invoke curl update check: {e}"))?;
    if !out.status.success() {
        return Err("Update check failed (offline?)".into());
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

/// Opens a validated SLClean GitHub release/download URL in the system default browser.
pub fn open_release_url(url: &str) -> Result<(), String> {
    let safe = validate_release_url(url)?;
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        Command::new("cmd")
            .args(["/C", "start", "", &safe])
            .creation_flags(CREATE_NO_WINDOW)
            .spawn()
            .map_err(|e| format!("Failed to open browser: {e}"))?;
        Ok(())
    }
    #[cfg(not(windows))]
    {
        let _ = Command::new("xdg-open").arg(&safe).spawn();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn semver_comparison_and_github_release_parsing_work() {
        assert!(is_newer_version("0.60.1", "v0.61.0"));
        assert!(is_newer_version("0.60.1", "1.0.0"));
        assert!(!is_newer_version("0.61.0", "v0.61.0"));
        assert!(!is_newer_version("0.61.0", "v0.60.1"));

        let sample = r###"{
            "tag_name": "v0.62.0",
            "name": "SLClean v0.62.0 — Rem Aqua",
            "html_url": "https://github.com/SalyyS1/SLClean/releases/tag/v0.62.0",
            "published_at": "2026-11-25T10:00:00Z",
            "body": "## What's New\n- Auto update notifier\n- Faster Codex scanner",
            "assets": [
                { "name": "SLClean_0.62.0_x64-setup.exe", "browser_download_url": "https://github.com/SalyyS1/SLClean/releases/download/v0.62.0/SLClean_0.62.0_x64-setup.exe" },
                { "name": "SLClean-portable.exe", "browser_download_url": "https://github.com/SalyyS1/SLClean/releases/download/v0.62.0/SLClean-portable.exe" }
            ]
        }"###;
        let info = parse_github_release_json("0.61.0", sample).unwrap();
        assert!(info.update_available);
        assert_eq!(info.latest_version, "0.62.0");
        assert!(info.setup_url.as_deref().unwrap().ends_with("setup.exe"));
        assert!(info.portable_url.as_deref().unwrap().ends_with("portable.exe"));
        assert!(validate_release_url("https://github.com/SalyyS1/SLClean/releases/tag/v0.62.0").is_ok());
        assert!(validate_release_url("https://evil.example.com/payload.exe").is_err());
        assert!(validate_release_url("https://github.com/SalyyS1/SLClean/releases&calc.exe").is_err());
    }
}
