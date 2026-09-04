//! Chọn thư mục gốc để tìm artifact build. Không ghi cứng tên thư mục của máy nào: quét mọi
//! ổ đĩa cố định và thư mục home, bỏ các thư mục hệ thống/app đã cài/kho cache, rồi cộng thêm
//! thư mục người dùng tự thêm và trừ thư mục người dùng loại ra.

use crate::catalog::{lower, Roots};
use crate::settings::Settings;
use std::path::{Path, PathBuf};

/// Kết quả lập kế hoạch quét: các gốc sẽ đi qua và các tiền tố không bao giờ vào.
#[derive(Clone, Debug)]
pub struct ScanPlan {
    pub roots: Vec<PathBuf>,
    pub excluded: Vec<PathBuf>,
}

/// Con trực tiếp của gốc ổ đĩa không bao giờ chứa dự án của người dùng (hoặc là app đã cài).
const DRIVE_BLOCK: &[&str] = &[
    "windows", "program files", "program files (x86)", "programdata", "users", "$recycle.bin",
    "system volume information", "recovery", "perflogs", "windowsapps", "wpsystem", "xboxgames",
    "config.msi", "steam", "steamlibrary", "msys64", "msys2", "cygwin", "cygwin64", "mingw", "mingw64",
    "inetpub", "documents and settings", "windows.old", "$windows.~bt", "$windows.~ws", "$winreagent",
    "intel", "nvidia", "amd", "drivers", "boot", "efi",
];

/// Con trực tiếp của thư mục home không phải nơi chứa dự án (thư mục hệ thống, tool home `.x`).
const HOME_BLOCK: &[&str] = &[
    "appdata", "application data", "cookies", "local settings", "nethood", "printhood", "recent",
    "sendto", "start menu", "templates", "favorites", "links", "searches", "saved games", "contacts",
    "music", "videos", "pictures", "3d objects", "ntuser.dat",
];

pub fn is_blocked_root_child(name: &str, at_drive_root: bool) -> bool {
    let n = name.to_ascii_lowercase();
    if n.starts_with("onedrive") {
        return true;
    }
    if at_drive_root {
        DRIVE_BLOCK.contains(&n.as_str())
    } else {
        n.starts_with('.') || HOME_BLOCK.contains(&n.as_str())
    }
}

pub fn is_under(path: &Path, prefix: &Path) -> bool {
    let p = lower(path);
    let pre = lower(prefix);
    let pre = pre.trim_end_matches('\\');
    p == pre || p.starts_with(&format!("{pre}\\"))
}

/// Gốc tự phát hiện: mọi ổ đĩa cố định và thư mục home.
pub fn discovered_roots(r: &Roots) -> Vec<PathBuf> {
    let mut roots = r.drives.clone();
    roots.push(r.home.clone());
    roots
}

/// Gốc thật sự sẽ quét: tự phát hiện + thêm tay (bỏ trùng, bỏ cái nằm trong cái khác), trừ loại ra.
/// `excluded` gồm thư mục người dùng loại ra và mọi đường dẫn của danh mục cache (đã có mục riêng).
pub fn plan(r: &Roots, settings: &Settings, catalog_paths: &[PathBuf]) -> ScanPlan {
    let mut roots = discovered_roots(r);
    for extra in &settings.extra_roots {
        if extra.is_dir() && !roots.iter().any(|x| is_under(extra, x)) {
            roots.push(extra.clone());
        }
    }
    let mut excluded: Vec<PathBuf> = settings.excluded_roots.iter().filter(|p| p.is_absolute()).cloned().collect();
    excluded.extend(catalog_paths.iter().cloned());
    roots.retain(|root| !excluded.iter().any(|ex| is_under(root, ex)));
    ScanPlan { roots, excluded }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn drive_root_blocks_system_but_not_projects() {
        for n in ["Windows", "Program Files", "$Recycle.Bin", "OneDrive - company", "msys64", "SteamLibrary"] {
            assert!(is_blocked_root_child(n, true), "{n}");
        }
        for n in ["Project", "Work", "dev", "Game", "tmp"] {
            assert!(!is_blocked_root_child(n, true), "{n}");
        }
    }

    #[test]
    fn home_root_blocks_dotdirs_and_shell_folders() {
        for n in [".gradle", ".vscode", "AppData", "Saved Games", "OneDrive"] {
            assert!(is_blocked_root_child(n, false), "{n}");
        }
        for n in ["source", "Desktop", "Documents", "Downloads", "node_modules", "go"] {
            assert!(!is_blocked_root_child(n, false), "{n}");
        }
    }

    #[test]
    fn under_is_case_insensitive_and_boundary_safe() {
        assert!(is_under(Path::new(r"D:\Project\x"), Path::new(r"d:\project")));
        assert!(is_under(Path::new(r"D:\Project"), Path::new(r"D:\Project\")));
        assert!(!is_under(Path::new(r"D:\Projects\x"), Path::new(r"D:\Project")));
    }
}
