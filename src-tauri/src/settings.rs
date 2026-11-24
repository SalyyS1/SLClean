//! Cài đặt người dùng lưu ở `%APPDATA%\<identifier>\settings.json`: ngôn ngữ, thư mục dự án
//! thêm/bỏ, chế độ xoá. Thiếu file hoặc file hỏng thì dùng mặc định, không báo lỗi.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
#[cfg(windows)]
use tauri::{AppHandle, Manager};

#[derive(Clone, Debug, Serialize, Deserialize, Default)]
#[serde(default)]
pub struct Settings {
    /// "vi" | "en"; None = theo ngôn ngữ hệ thống.
    pub language: Option<String>,
    /// Thư mục dự án người dùng thêm, quét cùng các thư mục tự phát hiện.
    pub extra_roots: Vec<PathBuf>,
    /// Đường dẫn (và mọi thứ bên dưới) không bao giờ quét artifact.
    pub excluded_roots: Vec<PathBuf>,
    /// Mặc định đưa vào Thùng rác thay vì xoá thẳng.
    pub to_trash: bool,
    /// Chế độ Đơn giản: ẩn nhãn kỹ thuật, hiện giải thích thân thiện (không bao giờ tự tick dữ liệu người dùng).
    pub simple_mode: bool,
    /// Ngưỡng chỗ trống tối thiểu tùy chọn (GB) để cảnh báo / kiểm tra ngân sách ổ đĩa.
    pub free_space_budget_gb: Option<u64>,
}

#[cfg(windows)]
fn file(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("settings.json"))
}

#[cfg(windows)]
pub fn load(app: &AppHandle) -> Settings {
    file(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

/// Đọc cấu hình trực tiếp từ `%APPDATA%\vn.salyyy.slclean\settings.json` khi chạy chế độ CLI
/// không khởi tạo `AppHandle` của Tauri.
pub fn load_offline() -> Settings {
    dirs::config_dir()
        .map(|d| d.join("vn.salyyy.slclean").join("settings.json"))
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

#[cfg(windows)]
pub fn save(app: &AppHandle, settings: &Settings) -> Result<(), String> {
    let path = file(app).ok_or("Không tìm được thư mục cấu hình.")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string_pretty(settings).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| e.to_string())
}

/// Ngôn ngữ hiệu lực: cài đặt nếu có, không thì theo locale hệ thống (vi-VN → vi, còn lại en).
pub fn effective_language(settings: &Settings) -> String {
    if let Some(l) = &settings.language {
        if l == "vi" || l == "en" {
            return l.clone();
        }
    }
    system_language()
}

pub fn system_language() -> String {
    let loc = sys_locale::get_locale().unwrap_or_default().to_ascii_lowercase();
    if loc.starts_with("vi") { "vi".into() } else { "en".into() }
}

#[derive(Clone, Debug, Serialize)]
pub struct SettingsAudit {
    pub effective_language: String,
    pub to_trash_default: bool,
    pub simple_mode: bool,
    pub free_space_budget_gb: Option<u64>,
    pub extra_root_count: usize,
    pub excluded_root_count: usize,
    pub missing_extra_roots: Vec<String>,
    pub missing_excluded_roots: Vec<String>,
    pub redundant_exclusions: Vec<String>,
    pub healthy: bool,
}

/// Inspect active SLClean settings and validate extra/excluded paths (strictly read-only).
pub fn audit_settings(settings: &Settings) -> SettingsAudit {
    let missing_extra: Vec<String> = settings
        .extra_roots
        .iter()
        .filter(|p| !p.exists())
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    let missing_excluded: Vec<String> = settings
        .excluded_roots
        .iter()
        .filter(|p| !p.exists())
        .map(|p| p.to_string_lossy().into_owned())
        .collect();
    let mut redundant = Vec::new();
    for (i, a) in settings.excluded_roots.iter().enumerate() {
        for (j, b) in settings.excluded_roots.iter().enumerate() {
            if i != j && a.starts_with(b) && a != b {
                redundant.push(format!("{} (already under {})", a.display(), b.display()));
                break;
            }
        }
    }
    let healthy = missing_extra.is_empty() && missing_excluded.is_empty() && redundant.is_empty();
    SettingsAudit {
        effective_language: effective_language(settings),
        to_trash_default: settings.to_trash,
        simple_mode: settings.simple_mode,
        free_space_budget_gb: settings.free_space_budget_gb,
        extra_root_count: settings.extra_roots.len(),
        excluded_root_count: settings.excluded_roots.len(),
        missing_extra_roots: missing_extra,
        missing_excluded_roots: missing_excluded,
        redundant_exclusions: redundant,
        healthy,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn audits_missing_and_redundant_exclusion_paths() {
        let base = std::env::temp_dir();
        let nested = base.join("slclean_nested_exclusion_sub");
        let missing = base.join("slclean_does_not_exist_991122");
        let s = Settings {
            language: Some("en".into()),
            extra_roots: vec![missing.clone()],
            excluded_roots: vec![base.clone(), nested],
            to_trash: true,
            simple_mode: true,
            free_space_budget_gb: Some(25),
        };
        let rep = audit_settings(&s);
        assert_eq!(rep.effective_language, "en");
        assert_eq!(rep.missing_extra_roots.len(), 1);
        assert_eq!(rep.redundant_exclusions.len(), 1);
        assert!(!rep.healthy);
    }
}
