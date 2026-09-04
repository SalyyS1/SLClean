//! Danh mục các vị trí cache/tạm đã biết trên Windows. Mục tĩnh (đường dẫn suy từ thư mục
//! chuẩn của Windows) nằm ở `catalog_specs.rs`; mục sinh động theo cấu trúc máy (từng profile
//! trình duyệt, app Electron, sản phẩm JetBrains, thư mục tạm ở gốc ổ đĩa, Steam) ở
//! `catalog_dynamic.rs`. Mỗi mục có mức độ an toàn:
//! - `Safe`: xoá không mất gì, công cụ tự tạo lại (cache gói, cache trình duyệt, temp).
//! - `Rebuild`: xoá được nhưng lần build/chạy sau chậm hơn vì phải tải lại.
//! - `Review`: chứa dữ liệu người dùng có thể muốn giữ (lịch sử phiên AI, backup); không tự tick.

use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Safety {
    Safe,
    Rebuild,
    Review,
}

/// Chuỗi hiển thị hai ngôn ngữ; UI chọn theo ngôn ngữ đang dùng.
#[derive(Clone, Debug, Serialize)]
pub struct Text {
    pub vi: String,
    pub en: String,
}

impl Text {
    pub fn new(vi: impl Into<String>, en: impl Into<String>) -> Self {
        Text { vi: vi.into(), en: en.into() }
    }

    /// Cùng một chuỗi cho cả hai ngôn ngữ (tên riêng, đường dẫn).
    pub fn same(s: impl Into<String>) -> Self {
        let s = s.into();
        Text { vi: s.clone(), en: s }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct CatalogEntry {
    pub id: String,
    /// Nhóm hiển thị: ai, package, editor, browser, app, game, system, temp.
    pub group: String,
    pub label: Text,
    pub note: Text,
    /// Thư mục đại diện để hiển thị và mở trong Explorer.
    pub path: PathBuf,
    /// Các thư mục thật sự bị dọn. Thường là một; profile trình duyệt gom nhiều thư mục cache.
    pub paths: Vec<PathBuf>,
    pub safety: Safety,
    /// Xoá nội dung bên trong nhưng giữ thư mục gốc (đa số cache cần thư mục tồn tại).
    pub keep_root: bool,
    /// Chỉ dọn trọn vẹn được khi app chạy với quyền admin (thư mục hệ thống).
    pub needs_admin: bool,
}

/// Các thư mục gốc chuẩn của Windows, lấy từ biến môi trường một lần rồi dùng cho mọi mục.
/// Không có đường dẫn nào ghi cứng theo máy cụ thể.
pub struct Roots {
    pub home: PathBuf,
    pub local: PathBuf,
    pub roaming: PathBuf,
    pub temp: PathBuf,
    /// %SystemRoot%, thường là C:\Windows.
    pub system_root: PathBuf,
    /// %SystemDrive%\, thường là C:\.
    pub system_drive: PathBuf,
    pub program_data: PathBuf,
    /// Gốc của mọi ổ đĩa cố định (C:\, D:\ ...).
    pub drives: Vec<PathBuf>,
}

impl Roots {
    pub fn detect() -> Option<Roots> {
        let env_path = |k: &str| std::env::var_os(k).map(PathBuf::from);
        let system_root = env_path("SystemRoot").unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
        let system_drive = std::env::var_os("SystemDrive")
            .map(|d| PathBuf::from(format!("{}\\", d.to_string_lossy().trim_end_matches('\\'))))
            .unwrap_or_else(|| PathBuf::from(r"C:\"));
        let program_data = env_path("ProgramData").unwrap_or_else(|| system_drive.join("ProgramData"));
        let drives = crate::drives::list_drives()
            .into_iter()
            .map(|d| PathBuf::from(format!("{}\\", d.mount)))
            .collect();
        Some(Roots {
            home: dirs::home_dir()?,
            local: dirs::cache_dir()?,
            roaming: dirs::config_dir()?,
            temp: std::env::temp_dir(),
            system_root,
            system_drive,
            program_data,
            drives,
        })
    }

    /// Thư mục hệ thống chỉ ghi được khi elevated: dưới %SystemRoot%, %ProgramData%, và các
    /// thư mục nâng cấp Windows ở gốc ổ hệ thống ($WinREAgent, $WINDOWS.~BT, Windows.old…).
    pub fn needs_admin(&self, path: &Path) -> bool {
        let p = lower(path);
        let under = |base: &Path| p.starts_with(&format!("{}\\", lower(base).trim_end_matches('\\')));
        if under(&self.system_root) || under(&self.program_data) {
            return true;
        }
        let drive = lower(&self.system_drive);
        match p.strip_prefix(&drive) {
            Some(rest) => rest.starts_with('$') || rest.starts_with("windows.old") || rest.starts_with("config.msi"),
            None => false,
        }
    }
}

pub fn lower(p: &Path) -> String {
    p.to_string_lossy().to_ascii_lowercase()
}

/// Tạo một mục; `paths` rỗng nghĩa là dùng chính `path`. Trả None nếu không thư mục nào tồn tại.
#[allow(clippy::too_many_arguments)]
pub fn make_entry(
    roots: &Roots,
    id: impl Into<String>,
    group: &str,
    label: Text,
    note: Text,
    path: PathBuf,
    paths: Vec<PathBuf>,
    safety: Safety,
    keep_root: bool,
) -> Option<CatalogEntry> {
    let paths: Vec<PathBuf> = if paths.is_empty() { vec![path.clone()] } else { paths }
        .into_iter()
        .filter(|p| p.is_dir())
        .collect();
    if paths.is_empty() {
        return None;
    }
    let needs_admin = paths.iter().any(|p| roots.needs_admin(p));
    Some(CatalogEntry { id: id.into(), group: group.to_string(), label, note, path, paths, safety, keep_root, needs_admin })
}

/// Trả về các mục có tồn tại trên máy này. Mục tĩnh đi trước để khi trùng đường dẫn với
/// mục sinh động thì giữ nhãn/ghi chú đã viết tay; mục sinh động trùng hoàn toàn bị bỏ.
pub fn existing_entries() -> Vec<CatalogEntry> {
    let Some(r) = Roots::detect() else { return Vec::new() };
    let mut all = crate::catalog_specs::entries(&r);
    all.extend(crate::catalog_dynamic::entries(&r));
    let mut seen: HashSet<String> = HashSet::new();
    all.retain_mut(|e| {
        e.paths.retain(|p| !seen.contains(&lower(p)));
        if e.paths.is_empty() {
            return false;
        }
        seen.extend(e.paths.iter().map(|p| lower(p)));
        true
    });
    all
}

#[cfg(test)]
mod tests {
    use super::*;

    fn roots() -> Roots {
        Roots {
            home: PathBuf::from(r"C:\Users\x"),
            local: PathBuf::from(r"C:\Users\x\AppData\Local"),
            roaming: PathBuf::from(r"C:\Users\x\AppData\Roaming"),
            temp: PathBuf::from(r"C:\Users\x\AppData\Local\Temp"),
            system_root: PathBuf::from(r"C:\Windows"),
            system_drive: PathBuf::from(r"C:\"),
            program_data: PathBuf::from(r"C:\ProgramData"),
            drives: vec![PathBuf::from(r"C:\"), PathBuf::from(r"D:\")],
        }
    }

    #[test]
    fn admin_only_for_system_locations() {
        let r = roots();
        for p in [
            r"C:\Windows\Temp",
            r"C:\Windows\SoftwareDistribution\Download",
            r"C:\$WinREAgent",
            r"C:\Windows.old",
            r"C:\Config.Msi",
            r"C:\ProgramData\Package Cache",
        ] {
            assert!(r.needs_admin(Path::new(p)), "{p}");
        }
        for p in [r"C:\Users\x\AppData\Local\Temp", r"D:\Temp", r"D:\$WinREAgent", r"C:\WindowsApps"] {
            assert!(!r.needs_admin(Path::new(p)), "{p}");
        }
    }

    #[test]
    fn catalog_never_contains_duplicate_paths() {
        let entries = existing_entries();
        let mut seen = HashSet::new();
        for e in &entries {
            assert!(!e.paths.is_empty(), "{} has no paths", e.id);
            for p in &e.paths {
                assert!(seen.insert(lower(p)), "duplicate path {p:?} in {}", e.id);
            }
        }
    }
}
