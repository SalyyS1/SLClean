//! Cài đặt người dùng lưu ở `%APPDATA%\<identifier>\settings.json`: ngôn ngữ, thư mục dự án
//! thêm/bỏ, chế độ xoá. Thiếu file hoặc file hỏng thì dùng mặc định, không báo lỗi.

use serde::{Deserialize, Serialize};
use std::path::PathBuf;
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
}

fn file(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("settings.json"))
}

pub fn load(app: &AppHandle) -> Settings {
    file(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

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
