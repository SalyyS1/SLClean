//! Xoá nội dung đã chọn và báo cáo từng mục: giải phóng bao nhiêu byte, bao nhiêu file
//! bị bỏ qua (đang mở / không có quyền). Chế độ Thùng rác dùng crate `trash`; chế độ
//! xoá thẳng đi từng file để một file kẹt không làm hỏng cả mục. Một mục có thể gồm
//! nhiều thư mục (profile trình duyệt gom Cache, Code Cache, GPUCache…).

use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Path, PathBuf};

#[derive(Clone, Debug, Deserialize)]
pub struct CleanItem {
    pub id: String,
    pub paths: Vec<PathBuf>,
    /// true: xoá bên trong, giữ thư mục gốc. false: xoá cả thư mục gốc.
    pub keep_root: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct CleanResult {
    pub id: String,
    pub freed: u64,
    pub removed: u64,
    pub skipped: u64,
    pub error: Option<String>,
}

/// Thư mục con của Program Files thuộc Windows, Microsoft hoặc runtime dùng chung: không bao
/// giờ xoá và không bao giờ coi là "thư mục thừa". App của hãng thứ ba nằm ngoài danh sách này.
pub const PROGRAM_FILES_KEEP: &[&str] = &[
    "common files", "internet explorer", "windows defender", "windows defender advanced threat protection", "windows mail",
    "windows media player", "windows multimedia platform", "windows nt", "windows photo viewer", "windows portable devices",
    "windows security", "windows sidebar", "windowsapps", "windowspowershell", "modifiablewindowsapps", "msbuild", "dotnet",
    "reference assemblies", "microsoft", "microsoft sdks", "microsoft visual studio", "microsoft sql server", "microsoft.net",
    "microsoft xna", "microsoft update health tools", "microsoft gameinput", "microsoft office", "uninstall information",
    "installshield installation information", "application verifier", "windows kits", "package cache", "packagemanagement",
    "intel", "nvidia corporation", "amd", "realtek", "hp", "dell", "lenovo", "asus", "google", "mozilla maintenance service",
    "bonjour", "java", "eclipse adoptium", "common", "rempl", "wsl", "git", "nodejs", "python", "go", "rust",
];

fn lower(p: &Path) -> String {
    p.to_string_lossy().to_ascii_lowercase().trim_end_matches('\\').to_string()
}

/// Hai danh sách đường dẫn (chữ thường) suy từ biến môi trường: `exact` là các gốc chỉ được
/// bảo vệ đúng chính nó (Program Files, ProgramData, home, AppData\Local…); `prefix` là các cây
/// bị bảo vệ toàn bộ (System32, .ssh, thư mục Windows/runtime trong Program Files…).
fn guard_lists() -> (Vec<String>, Vec<String>) {
    let env = |k: &str| std::env::var_os(k).map(|v| lower(Path::new(&v)));
    let mut exact: Vec<String> = Vec::new();
    let mut prefix: Vec<String> = Vec::new();
    if let Some(sr) = env("SystemRoot") {
        exact.push(sr.clone());
        for s in ["system32", "syswow64", "winsxs", "boot", "fonts", "servicing"] {
            prefix.push(format!("{sr}\\{s}"));
        }
    }
    // Program Files: gốc và các thư mục Windows/runtime dùng chung được bảo vệ; thư mục app của
    // hãng thứ ba thì không, để dọn được thư mục cài còn sót sau khi gỡ (tab Thư mục thừa).
    for k in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
        if let Some(p) = env(k) {
            exact.push(p.clone());
            for s in PROGRAM_FILES_KEEP {
                prefix.push(format!("{p}\\{s}"));
            }
        }
    }
    if let Some(pd) = env("ProgramData") {
        exact.push(pd);
    }
    if let Some(h) = dirs::home_dir() {
        let h = lower(&h);
        exact.push(h.clone());
        for sub in [
            "appdata", "appdata\\local", "appdata\\roaming", "appdata\\locallow", "documents", "desktop", "downloads",
            "pictures", "videos", "music", "onedrive", ".claude", ".codex", ".cargo", ".gradle", ".m2", ".rustup", ".nuget",
            ".vscode", ".cursor", ".gemini", ".config", "go", "source", "appdata\\local\\programs", "appdata\\local\\microsoft",
        ] {
            exact.push(format!("{h}\\{sub}"));
        }
        for sub in [".ssh", ".gnupg", "appdata\\local\\microsoft\\windowsapps"] {
            prefix.push(format!("{h}\\{sub}"));
        }
    }
    (exact, prefix)
}

/// Đường dẫn không bao giờ được xoá dù UI gửi gì: gốc ổ đĩa và một cấp dưới gốc, thư mục
/// Windows/Program Files/ProgramData, thư mục home và các thư mục con chuẩn, tool home
/// (.cargo, .gradle, .claude…), app đã cài (Local\Programs, WindowsApps). Tất cả suy từ biến
/// môi trường, không ghi cứng theo máy.
pub(crate) fn is_protected(path: &Path) -> bool {
    if is_root_like(path) {
        return true;
    }
    let target = lower(path);
    let (_, prefix) = guard_lists();
    prefix.iter().any(|p| target == *p || target.starts_with(&format!("{p}\\")))
}

/// Đường dẫn là một "gốc" chứ không thể là thư mục cài của app nào: gốc ổ đĩa, một cấp dưới gốc
/// (C:\Windows, C:\Program Files), các gốc trong `exact`, và mọi thứ dưới %SystemRoot%. Khác
/// `is_protected` ở chỗ thư mục runtime trong Program Files (nodejs, Git, Java…) vẫn là thư mục
/// cài hợp lệ để đo và đối chiếu, dù không bao giờ được xoá.
pub(crate) fn is_root_like(path: &Path) -> bool {
    if !path.is_absolute() {
        return true;
    }
    // Trên Windows `C:\Windows` có 3 component: tiền tố `C:`, gốc `\`, tên.
    if path.components().count() <= 3 {
        return true;
    }
    let target = lower(path);
    let (exact, _) = guard_lists();
    if exact.iter().any(|p| *p == target) {
        return true;
    }
    std::env::var_os("SystemRoot").map(|v| lower(Path::new(&v))).map(|sr| target.starts_with(&format!("{sr}\\"))).unwrap_or(false)
}

fn clear_readonly(path: &Path) {
    if let Ok(meta) = fs::symlink_metadata(path) {
        let mut perm = meta.permissions();
        if perm.readonly() {
            perm.set_readonly(false);
            let _ = fs::set_permissions(path, perm);
        }
    }
}

fn remove_file_retry(path: &Path) -> bool {
    if fs::remove_file(path).is_ok() {
        return true;
    }
    clear_readonly(path);
    fs::remove_file(path).is_ok()
}

/// Xoá thẳng một cây thư mục (không đi theo symlink/junction). Trả (freed, removed, skipped).
fn remove_tree(path: &Path) -> (u64, u64, u64) {
    let (mut freed, mut removed, mut skipped) = (0u64, 0u64, 0u64);
    let meta = match fs::symlink_metadata(path) {
        Ok(m) => m,
        Err(_) => return (0, 0, 1),
    };
    if meta.file_type().is_symlink() {
        // Junction/symlink: chỉ bỏ liên kết, không đụng đích. Junction thư mục cần remove_dir.
        let ok = fs::remove_dir(path).is_ok() || remove_file_retry(path);
        return if ok { (0, 1, 0) } else { (0, 0, 1) };
    }
    if !meta.is_dir() {
        let len = meta.len();
        return if remove_file_retry(path) { (len, 1, 0) } else { (0, 0, 1) };
    }
    let entries = match fs::read_dir(path) {
        Ok(it) => it,
        Err(_) => return (0, 0, 1),
    };
    for entry in entries.flatten() {
        let (f, r, s) = remove_tree(&entry.path());
        freed += f;
        removed += r;
        skipped += s;
    }
    if skipped == 0 {
        clear_readonly(path);
        if fs::remove_dir(path).is_ok() {
            removed += 1;
        } else {
            skipped += 1;
        }
    }
    (freed, removed, skipped)
}

fn children(path: &Path) -> Vec<PathBuf> {
    fs::read_dir(path).map(|it| it.flatten().map(|e| e.path()).collect()).unwrap_or_default()
}

fn clean_path(root: &Path, keep_root: bool, to_trash: bool, res: &mut CleanResult) {
    if is_protected(root) {
        res.error = Some("protected".into());
        return;
    }
    if !root.exists() {
        return;
    }
    let targets: Vec<PathBuf> = if keep_root { children(root) } else { vec![root.to_path_buf()] };
    if to_trash {
        for t in &targets {
            let size = if t.is_dir() {
                crate::sizer::dir_stats(t, &std::sync::atomic::AtomicBool::new(false)).bytes
            } else {
                fs::metadata(t).map(|m| m.len()).unwrap_or(0)
            };
            match trash::delete(t) {
                Ok(()) => {
                    res.freed += size;
                    res.removed += 1;
                }
                Err(e) => {
                    res.skipped += 1;
                    if res.error.is_none() {
                        res.error = Some(format!("trash: {e}"));
                    }
                }
            }
        }
    } else {
        for t in &targets {
            let (f, r, s) = remove_tree(t);
            res.freed += f;
            res.removed += r;
            res.skipped += s;
        }
    }
}

/// Dọn một mục. `error` là mã ngắn để UI dịch: "protected", "missing", "skipped", "trash: …".
pub fn clean_one(item: &CleanItem, to_trash: bool) -> CleanResult {
    let mut res = CleanResult { id: item.id.clone(), freed: 0, removed: 0, skipped: 0, error: None };
    if item.paths.iter().any(|p| is_protected(p)) {
        res.error = Some("protected".into());
        return res;
    }
    if !item.paths.iter().any(|p| p.exists()) {
        res.error = Some("missing".into());
        return res;
    }
    for p in &item.paths {
        clean_path(p, item.keep_root, to_trash, &mut res);
    }
    if res.skipped > 0 && res.error.is_none() {
        res.error = Some("skipped".into());
    }
    res
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Cây mẫu: root/a.txt (5B), root/sub/b.bin (1000B, read-only), root/sub/deep/c (3B).
    fn make_tree(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("slclean-test-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        fs::create_dir_all(root.join("sub/deep")).unwrap();
        fs::write(root.join("a.txt"), b"hello").unwrap();
        fs::write(root.join("sub/b.bin"), vec![0u8; 1000]).unwrap();
        let mut p = fs::metadata(root.join("sub/b.bin")).unwrap().permissions();
        p.set_readonly(true);
        fs::set_permissions(root.join("sub/b.bin"), p).unwrap();
        fs::write(root.join("sub/deep/c"), b"abc").unwrap();
        root
    }

    fn item(paths: Vec<PathBuf>, keep_root: bool) -> CleanItem {
        CleanItem { id: "t".into(), paths, keep_root }
    }

    #[test]
    fn keep_root_empties_folder_but_leaves_it() {
        let root = make_tree("keep");
        let res = clean_one(&item(vec![root.clone()], true), false);
        assert_eq!(res.freed, 1008, "{res:?}");
        assert_eq!(res.skipped, 0, "{res:?}");
        assert!(res.error.is_none(), "{res:?}");
        assert!(root.exists());
        assert_eq!(fs::read_dir(&root).unwrap().count(), 0);
        fs::remove_dir(&root).unwrap();
    }

    #[test]
    fn full_delete_removes_root_too() {
        let root = make_tree("full");
        let res = clean_one(&item(vec![root.clone()], false), false);
        assert_eq!(res.freed, 1008, "{res:?}");
        assert!(!root.exists());
    }

    #[test]
    fn multi_path_item_cleans_every_path() {
        let a = make_tree("multi-a");
        let b = make_tree("multi-b");
        let res = clean_one(&item(vec![a.clone(), b.clone()], true), false);
        assert_eq!(res.freed, 2016, "{res:?}");
        assert!(a.exists() && b.exists());
        let _ = fs::remove_dir_all(&a);
        let _ = fs::remove_dir_all(&b);
    }

    #[test]
    fn locked_file_is_skipped_not_fatal() {
        let root = make_tree("locked");
        // Rust mở file với share mode đầy đủ nên xoá vẫn được; khoá thật bằng share_mode(0).
        use std::os::windows::fs::OpenOptionsExt;
        let held = fs::File::options().read(true).share_mode(0).open(root.join("sub/deep/c")).unwrap();
        let res = clean_one(&item(vec![root.clone()], false), false);
        drop(held);
        assert!(res.skipped >= 1, "{res:?}");
        assert_eq!(res.error.as_deref(), Some("skipped"), "{res:?}");
        assert!(root.join("sub/deep/c").exists());
        assert!(!root.join("a.txt").exists());
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn protected_paths_are_refused() {
        let sr = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let pf = std::env::var("ProgramFiles").unwrap_or_else(|_| r"C:\Program Files".into());
        for p in [r"C:\".to_string(), sr.clone(), format!("{sr}\\System32"), format!("{sr}\\System32\\drivers"), pf.clone(), format!("{pf}\\Google"), format!("{pf}\\Common Files\\x"), "relative\\path".into()] {
            assert!(is_protected(Path::new(&p)), "{p}");
        }
        // Thư mục app hãng thứ ba trong Program Files phải dọn được (thư mục thừa sau khi gỡ).
        assert!(!is_protected(Path::new(&format!("{pf}\\Some Vendor App"))));
        // Thư mục runtime trong Program Files không xoá được nhưng vẫn là thư mục cài để đo.
        assert!(is_protected(Path::new(&format!("{pf}\\nodejs"))));
        assert!(!is_root_like(Path::new(&format!("{pf}\\nodejs"))));
        assert!(!is_root_like(Path::new(&format!("{pf}\\Git"))));
        for p in [pf.clone(), sr.clone(), format!("{sr}\\System32"), format!("{sr}\\Installer\\x"), r"C:\".to_string()] {
            assert!(is_root_like(Path::new(&p)), "{p}");
        }
        let home = dirs::home_dir().unwrap();
        assert!(is_protected(&home));
        assert!(is_protected(&home.join("AppData\\Local")));
        assert!(is_protected(&home.join(".cargo")));
        assert!(is_protected(&home.join(".ssh\\keys")));
        assert!(!is_protected(&home.join("AppData\\Local\\Temp")));
        assert!(!is_protected(&home.join(".cargo\\registry\\cache")));
        assert!(!is_protected(Path::new(r"D:\Project\x\node_modules")));
        let res = clean_one(&item(vec![home.clone()], true), false);
        assert_eq!(res.removed, 0);
        assert_eq!(res.error.as_deref(), Some("protected"));
    }
}
