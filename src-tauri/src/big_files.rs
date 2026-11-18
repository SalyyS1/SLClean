//! File đơn lẻ lớn và bản đồ thư mục. Sổ cái chỉ đo được cả thư mục cache, nên một file 4 GB
//! nằm lẻ trong Downloads không bao giờ lộ ra. Một lần đi cây trả cả hai: danh sách file ≥
//! ngưỡng, và tổng dung lượng theo từng thư mục ở vài cấp đầu để lần ra chỗ chứa.
//!
//! Không cần admin và không phụ thuộc NTFS (WizTree cần cả hai), đổi lại phải đi từng thư mục
//! nên chậm hơn — vì vậy quét chạy nền, phát dần và huỷ được.

use crate::project_roots::is_under;
use serde::Serialize;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// Ngưỡng mặc định nếu UI không gửi: 100 MB.
pub const DEFAULT_THRESHOLD: u64 = 100 * 1024 * 1024;
/// Số cấp thư mục giữ trong bản đồ, tính từ gốc quét. Sâu hơn thì gộp vào cha.
const MAP_DEPTH: usize = 3;
/// Số file giữa hai lần báo tiến trình.
const PROGRESS_EVERY: u64 = 8192;

/// Con trực tiếp của gốc ổ đĩa không bao giờ nên đụng tới. Hẹp hơn danh sách của tab artifact
/// rất nhiều: file lớn nằm đúng ở Program Files (game), Users (video, ISO, máy ảo) — chặn
/// những chỗ đó thì tab này chẳng tìm được gì.
const SKIP_AT_DRIVE_ROOT: &[&str] = &[
    "windows", "windows.old", "$windows.~bt", "$windows.~ws", "$winreagent", "$recycle.bin",
    "system volume information", "recovery", "config.msi", "perflogs", "documents and settings",
];

/// File hệ thống Windows tự quản: nêu ra để giải thích chỗ trống đi đâu, nhưng khoá không cho
/// tick. Xoá tay là hỏng máy; muốn bỏ thì phải tắt tính năng tương ứng của Windows.
const SYSTEM_FILES: &[&str] = &["pagefile.sys", "hiberfil.sys", "swapfile.sys", "dumpstack.log", "dumpstack.log.tmp"];

#[derive(Clone, Debug, Serialize)]
pub struct BigFile {
    /// Đường dẫn đầy đủ; cũng là id ở UI.
    pub path: PathBuf,
    pub bytes: u64,
    /// Sửa lần cuối (giây epoch); 0 nếu không đọc được.
    pub modified: u64,
    /// Đuôi file chữ thường, không có dấu chấm; rỗng nếu không có.
    pub ext: String,
    /// File Windows / WSL / Docker tự quản: hiện để giải thích, không cho tick xoá.
    pub locked: bool,
    /// Loại khoá ("system" | "vhdx") để UI hiện đúng hướng dẫn thu gọn thay vì xoá tay.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub lock_kind: Option<String>,
}

/// Một thư mục trong bản đồ. `bytes` là tổng của cả cây con, kể cả phần sâu hơn `MAP_DEPTH`.
#[derive(Clone, Debug, Serialize)]
pub struct DirNode {
    pub path: PathBuf,
    pub bytes: u64,
    pub files: u64,
    pub children: Vec<DirNode>,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct BigScan {
    pub files: Vec<BigFile>,
    pub map: Vec<DirNode>,
    /// Thư mục không đọc được (thiếu quyền), để UI báo kết quả chỉ là một phần.
    pub denied: u64,
    pub scanned_files: u64,
    pub scanned_bytes: u64,
}

fn is_system_file(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    SYSTEM_FILES.contains(&lower.as_str())
}

/// Ổ đĩa ảo của WSL2 hoặc Docker Desktop (`ext4.vhdx`, `docker_data.vhdx`, `swap.vhdx`…).
/// Xoá thẳng file này sẽ mất trắng cả bản phân phối Linux hoặc volume Docker, nên phải khoá
/// cả ở UI lẫn `cleaner::is_protected` và hướng dẫn người dùng thu gọn (compact) an toàn.
pub fn is_managed_vhdx(path: &Path) -> bool {
    let s = path.to_string_lossy().replace('\\', "/");
    let norm = Path::new(&s);
    if !ext_of(norm).eq_ignore_ascii_case("vhdx") {
        return false;
    }
    let name = norm.file_name().map(|n| n.to_string_lossy().to_ascii_lowercase()).unwrap_or_default();
    if matches!(name.as_str(), "ext4.vhdx" | "docker_data.vhdx" | "swap.vhdx") {
        return true;
    }
    let norm_s = s.to_ascii_lowercase();
    norm_s.contains("/docker/wsl/")
        || norm_s.contains("/appdata/local/wsl/")
        || (norm_s.contains("/appdata/local/packages/") && norm_s.contains("/localstate/"))
}

fn classify_lock(name: &str, path: &Path) -> Option<String> {
    if is_system_file(name) {
        Some("system".into())
    } else if is_managed_vhdx(path) {
        Some("vhdx".into())
    } else {
        None
    }
}

fn skip_at_drive_root(name: &str) -> bool {
    SKIP_AT_DRIVE_ROOT.contains(&name.to_ascii_lowercase().as_str())
}

/// Gốc quét có phải gốc ổ đĩa không (`D:\`), để biết có áp danh sách chặn con trực tiếp không.
fn is_drive_root(p: &Path) -> bool {
    p.parent().is_none()
}

fn modified_epoch(m: &std::fs::Metadata) -> u64 {
    m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
}

fn ext_of(p: &Path) -> String {
    p.extension().map(|e| e.to_string_lossy().to_ascii_lowercase()).unwrap_or_default()
}

/// Số cấp của `path` so với `root`; `root` chính nó là 0.
fn depth_from(root: &Path, path: &Path) -> usize {
    path.strip_prefix(root).map(|r| r.components().count()).unwrap_or(0)
}

/// Đi một gốc: thu file ≥ `threshold` và cộng dồn dung lượng cho từng thư mục tổ tiên trong
/// phạm vi `MAP_DEPTH`. `on_file` được gọi cho từng file lớn tìm thấy, để UI hiện dần.
fn walk_root(root: &Path, threshold: u64, excluded: &[PathBuf], cancel: &AtomicBool, out: &mut BigScan, totals: &mut HashMap<PathBuf, (u64, u64)>, mut on_progress: impl FnMut(&BigScan), on_file: &mut impl FnMut(&BigFile)) {
    let mut stack = vec![root.to_path_buf()];
    let mut since_report = 0u64;
    while let Some(dir) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            return;
        }
        let read = match std::fs::read_dir(&dir) {
            Ok(it) => it,
            Err(_) => {
                out.denied += 1;
                continue;
            }
        };
        let at_drive_root = is_drive_root(&dir);
        for entry in read.flatten() {
            let Ok(ft) = entry.file_type() else {
                out.denied += 1;
                continue;
            };
            // Junction/symlink: đi vào sẽ đếm trùng hoặc lạc sang ổ khác.
            if ft.is_symlink() {
                continue;
            }
            let path = entry.path();
            let name = entry.file_name().to_string_lossy().into_owned();
            if ft.is_dir() {
                if at_drive_root && skip_at_drive_root(&name) {
                    continue;
                }
                if excluded.iter().any(|ex| is_under(&path, ex)) {
                    continue;
                }
                stack.push(path);
                continue;
            }
            if !ft.is_file() {
                continue;
            }
            let Ok(m) = entry.metadata() else {
                out.denied += 1;
                continue;
            };
            let bytes = m.len();
            out.scanned_files += 1;
            out.scanned_bytes += bytes;
            since_report += 1;

            // Cộng cỡ file cho mọi thư mục tổ tiên trong phạm vi bản đồ. File nằm sâu hơn
            // `MAP_DEPTH` vẫn được tính, chỉ là tính vào tổ tiên gần nhất còn trong phạm vi.
            let mut cur = path.parent();
            while let Some(d) = cur {
                if !is_under(d, root) {
                    break;
                }
                if depth_from(root, d) <= MAP_DEPTH {
                    let e = totals.entry(d.to_path_buf()).or_insert((0, 0));
                    e.0 += bytes;
                    e.1 += 1;
                }
                cur = d.parent();
            }

            if bytes >= threshold {
                let lock_kind = classify_lock(&name, &path);
                let f = BigFile { bytes, modified: modified_epoch(&m), ext: ext_of(&path), locked: lock_kind.is_some(), lock_kind, path };
                on_file(&f);
                out.files.push(f);
            }
            if since_report >= PROGRESS_EVERY {
                since_report = 0;
                on_progress(out);
            }
        }
    }
}

/// Dựng cây từ bảng tổng: mỗi thư mục là con của tổ tiên gần nhất cũng có trong bảng.
fn build_tree(root: &Path, totals: &HashMap<PathBuf, (u64, u64)>) -> Option<DirNode> {
    fn node(path: &Path, root: &Path, totals: &HashMap<PathBuf, (u64, u64)>) -> DirNode {
        let (bytes, files) = totals.get(path).copied().unwrap_or((0, 0));
        let mut children: Vec<DirNode> = totals
            .keys()
            .filter(|p| p.parent() == Some(path) && depth_from(root, p) <= MAP_DEPTH)
            .map(|p| node(p, root, totals))
            .collect();
        children.sort_by(|a, b| b.bytes.cmp(&a.bytes));
        DirNode { path: path.to_path_buf(), bytes, files, children }
    }
    totals.contains_key(root).then(|| node(root, root, totals))
}

/// Quét các gốc, trả file lớn (lớn nhất trước) và bản đồ thư mục theo gốc.
pub fn scan(roots: &[PathBuf], excluded: &[PathBuf], threshold: u64, cancel: &AtomicBool, mut on_progress: impl FnMut(&BigScan), mut on_file: impl FnMut(&BigFile)) -> BigScan {
    let mut out = BigScan::default();
    for root in roots {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let mut totals: HashMap<PathBuf, (u64, u64)> = HashMap::new();
        walk_root(root, threshold, excluded, cancel, &mut out, &mut totals, &mut on_progress, &mut on_file);
        if let Some(node) = build_tree(root, &totals) {
            out.map.push(node);
        }
    }
    out.files.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    out.map.sort_by(|a, b| b.bytes.cmp(&a.bytes));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("slclean-big-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("a/b/c/d")).unwrap();
        std::fs::create_dir_all(root.join("small")).unwrap();
        std::fs::write(root.join("a/big.bin"), vec![7u8; 4000]).unwrap();
        std::fs::write(root.join("a/b/mid.iso"), vec![7u8; 2000]).unwrap();
        // Nằm sâu hơn MAP_DEPTH: vẫn phải được tính vào tổ tiên còn trong phạm vi.
        std::fs::write(root.join("a/b/c/d/deep.bin"), vec![7u8; 3000]).unwrap();
        std::fs::write(root.join("small/tiny.txt"), vec![7u8; 10]).unwrap();
        root
    }

    fn scan_at(root: &Path, threshold: u64) -> BigScan {
        scan(&[root.to_path_buf()], &[], threshold, &AtomicBool::new(false), |_| {}, |_| {})
    }

    #[test]
    fn only_files_at_or_above_threshold_are_listed() {
        let root = fixture("threshold");
        let s = scan_at(&root, 2000);
        let names: Vec<String> = s.files.iter().map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert_eq!(names, vec!["big.bin", "deep.bin", "mid.iso"], "sắp lớn trước, bỏ file dưới ngưỡng");
        assert_eq!(s.files[0].bytes, 4000);
        assert_eq!(s.files[0].ext, "bin");
        assert_eq!(s.files[2].ext, "iso");
        assert!(s.files.iter().all(|f| f.modified > 1_600_000_000), "phải đọc được ngày sửa");
        assert_eq!(s.scanned_files, 4, "vẫn đi qua mọi file để dựng bản đồ");
        assert_eq!(s.scanned_bytes, 9010);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn map_totals_include_files_deeper_than_the_cut() {
        let root = fixture("map");
        let s = scan_at(&root, 1_000_000);
        assert!(s.files.is_empty(), "ngưỡng cao thì không có file nào được nêu");
        let top = &s.map[0];
        assert_eq!(top.bytes, 9010, "gốc phải cộng đủ mọi file, kể cả file sâu");
        assert_eq!(top.files, 4);
        let a = top.children.iter().find(|c| c.path.ends_with("a")).expect("thư mục a");
        assert_eq!(a.bytes, 9000, "a gồm cả deep.bin nằm sâu hơn mức cắt");
        let small = top.children.iter().find(|c| c.path.ends_with("small")).expect("thư mục small");
        assert_eq!(small.bytes, 10);
        assert!(top.children[0].bytes >= top.children[1].bytes, "con sắp theo cỡ giảm dần");
        // Cắt đúng độ sâu: không có nút nào quá MAP_DEPTH cấp từ gốc.
        fn max_depth(n: &DirNode, root: &Path) -> usize {
            n.children.iter().map(|c| max_depth(c, root)).max().unwrap_or(depth_from(root, &n.path))
        }
        assert!(max_depth(top, &root) <= MAP_DEPTH, "cây phải cắt ở {MAP_DEPTH} cấp");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn excluded_folders_are_never_entered() {
        let root = fixture("excluded");
        let s = scan(&[root.clone()], &[root.join("a")], 1000, &AtomicBool::new(false), |_| {}, |_| {});
        assert!(s.files.is_empty(), "mọi file lớn đều nằm trong a: {:?}", s.files);
        assert_eq!(s.scanned_files, 1, "chỉ còn tiny.txt ngoài a");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn cancelling_stops_the_walk() {
        let root = fixture("cancel");
        let cancel = AtomicBool::new(true);
        let s = scan(&[root.clone()], &[], 1000, &cancel, |_| {}, |_| {});
        assert!(s.files.is_empty() && s.scanned_files == 0);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn windows_managed_files_are_locked_and_system_dirs_skipped() {
        assert!(is_system_file("pagefile.sys") && is_system_file("HIBERFIL.SYS") && is_system_file("swapfile.sys"));
        assert!(!is_system_file("mymovie.mkv"));
        assert!(is_managed_vhdx(Path::new(r"C:\Users\x\AppData\Local\Packages\CanonicalGroupLimited.Ubuntu_79rhkp\LocalState\ext4.vhdx")));
        assert!(is_managed_vhdx(Path::new(r"C:\Users\x\AppData\Local\Docker\wsl\disk\docker_data.vhdx")));
        assert!(is_managed_vhdx(Path::new(r"D:\WSL\Ubuntu24\ext4.vhdx")));
        assert!(!is_managed_vhdx(Path::new(r"C:\Users\x\Downloads\WinServer2025_Eval.vhdx")));
        assert!(skip_at_drive_root("Windows") && skip_at_drive_root("$Recycle.Bin") && skip_at_drive_root("System Volume Information"));
        // Chỗ chứa file lớn thật sự phải đi vào được, khác hẳn danh sách chặn của tab artifact.
        assert!(!skip_at_drive_root("Program Files") && !skip_at_drive_root("Users") && !skip_at_drive_root("Games"));
    }

    #[test]
    fn callbacks_report_each_big_file_as_it_is_found() {
        let root = fixture("callback");
        let mut seen = Vec::new();
        scan(&[root.clone()], &[], 2000, &AtomicBool::new(false), |_| {}, |f| seen.push(f.bytes));
        seen.sort_unstable();
        assert_eq!(seen, vec![2000, 3000, 4000], "mỗi file lớn phải được phát ngay khi gặp");
        let _ = std::fs::remove_dir_all(&root);
    }

    /// `cargo test -- --ignored --nocapture print_this_machine_big_files`
    #[test]
    #[ignore]
    fn print_this_machine_big_files() {
        let roots: Vec<PathBuf> = crate::drives::list_drives().into_iter().map(|d| PathBuf::from(format!("{}\\", d.mount))).collect();
        let t0 = std::time::Instant::now();
        let s = scan(&roots, &[], DEFAULT_THRESHOLD, &AtomicBool::new(false), |_| {}, |_| {});
        println!("quét {} file / {} MB trong {:?}, {} thư mục bị từ chối", s.scanned_files, s.scanned_bytes / 1024 / 1024, t0.elapsed(), s.denied);
        for f in s.files.iter().take(25) {
            println!("{:>10} MB {}{}", f.bytes / 1024 / 1024, f.path.display(), if f.locked { "  [khoá]" } else { "" });
        }
        for m in &s.map {
            println!("MAP {} = {} MB", m.path.display(), m.bytes / 1024 / 1024);
            for c in m.children.iter().take(6) {
                println!("   {:>10} MB {}", c.bytes / 1024 / 1024, c.path.display());
            }
        }
    }
}
