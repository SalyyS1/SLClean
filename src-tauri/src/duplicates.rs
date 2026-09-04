//! Tìm file giống hệt nhau (từng byte) để người dùng tự chọn bản nào bỏ. Ba pha lọc để hash
//! ít nhất có thể: gom theo cỡ → băm 4 KB đầu → băm toàn file. File chỉ có một mình ở cỡ của
//! nó thì không bao giờ bị đọc.
//!
//! Không bao giờ tự tick bản nào. Phần lớn tool miễn phí "chọn hộ" bản để xoá; ở đây app chỉ
//! nêu nhóm, đánh dấu bản cũ nhất/mới nhất, còn quyết định là của người dùng.

use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};

/// File nhỏ hơn ngưỡng này không xét: bản sao của chúng nhiều vô kể mà chẳng lấy lại được bao
/// nhiêu chỗ, trong khi băm hết thì rất lâu.
pub const MIN_SIZE: u64 = 1024 * 1024;
/// Số byte đọc ở pha hai. Đủ để loại gần hết cặp chỉ tình cờ cùng cỡ.
const HEAD_BYTES: usize = 4096;
/// Đọc theo khối khi băm toàn file, để file 4 GB không phải nằm hết trong RAM.
const CHUNK: usize = 256 * 1024;

/// Tên thư mục mà file trùng nhau là chuyện đương nhiên, xoá là hỏng: mã nguồn, kho gói của
/// công cụ lập trình. Một jar nằm trong mười cache Gradle là **đúng thiết kế**; muốn lấy lại
/// chỗ thì dọn cả cache ở tab Dọn dẹp, chứ không phải xoá lẻ từng file rồi hỏng build.
const SKIP_DIRS: &[&str] = &[
    "node_modules", ".git", "windowsapps", "winsxs", "$recycle.bin", "system volume information",
    "packages", ".cargo", ".rustup", "target", ".gradle", ".m2", ".nuget", ".ivy2", ".pub-cache",
    ".pnpm-store", ".paper-remapped", "__pycache__", ".venv", "site-packages",
];

/// Vùng do chương trình quản, không phải chỗ người dùng để file của mình. Bản trùng ở đây gần
/// như luôn là **cố ý**: hai phiên bản Edge cài song song có chung `msedge.dll`, app tự cập
/// nhật giữ lại bản exe cũ. Xoá là hỏng app, mà danh sách thì ngập những mục không ai nên đụng
/// tới — đo trên máy thật: 1556 nhóm, phần lớn là Edge, runtime và bản build cũ của app.
///
/// Người gọi tự cộng danh sách này vào `excluded` (xem `lib.rs`) thay vì `scan` tự thêm, để
/// test chạy được trong `%TEMP%` — vốn nằm ngay dưới `LOCALAPPDATA`.
pub fn program_areas() -> Vec<PathBuf> {
    ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432", "ProgramData", "LOCALAPPDATA", "APPDATA", "SystemRoot"]
        .iter()
        .filter_map(|k| std::env::var(k).ok())
        .map(PathBuf::from)
        .collect()
}

#[derive(Clone, Debug, Serialize)]
pub struct DupFile {
    pub path: PathBuf,
    pub modified: u64,
    /// Có một bản khác cùng nhóm nằm ngay cùng thư mục (thường là "copy.ext", dễ quyết nhất).
    pub same_dir_as_sibling: bool,
}

#[derive(Clone, Debug, Serialize)]
pub struct DupGroup {
    /// Cỡ của một bản (byte); mọi file trong nhóm đều bằng nhau.
    pub bytes: u64,
    pub files: Vec<DupFile>,
    /// Lấy lại được bao nhiêu nếu chỉ giữ một bản.
    pub wasted: u64,
}

#[derive(Clone, Debug, Default, Serialize)]
pub struct DupScan {
    pub groups: Vec<DupGroup>,
    /// Tổng lãng phí của mọi nhóm.
    pub wasted: u64,
    pub scanned_files: u64,
    /// Số file thực sự phải băm toàn bộ, để biết ba pha lọc có hiệu quả không.
    pub hashed_files: u64,
}

fn skip_dir(name: &str) -> bool {
    SKIP_DIRS.contains(&name.to_ascii_lowercase().as_str())
}

fn modified_epoch(m: &std::fs::Metadata) -> u64 {
    m.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
}

/// Băm `HEAD_BYTES` byte đầu. File ngắn hơn thì băm những gì có.
fn head_hash(path: &Path) -> Option<[u8; 32]> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut buf = vec![0u8; HEAD_BYTES];
    let mut read = 0;
    while read < buf.len() {
        match f.read(&mut buf[read..]) {
            Ok(0) => break,
            Ok(n) => read += n,
            Err(_) => return None,
        }
    }
    let mut h = Sha256::new();
    h.update(&buf[..read]);
    Some(h.finalize().into())
}

fn full_hash(path: &Path, cancel: &AtomicBool) -> Option<[u8; 32]> {
    let mut f = std::fs::File::open(path).ok()?;
    let mut h = Sha256::new();
    let mut buf = vec![0u8; CHUNK];
    loop {
        if cancel.load(Ordering::Relaxed) {
            return None;
        }
        match f.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => h.update(&buf[..n]),
            Err(_) => return None,
        }
    }
    Some(h.finalize().into())
}

/// Gom mọi file ≥ `min_size` theo cỡ. Chỉ cỡ nào có từ hai file trở lên mới đi tiếp.
fn by_size(roots: &[PathBuf], excluded: &[PathBuf], min_size: u64, cancel: &AtomicBool, out: &mut DupScan) -> HashMap<u64, Vec<PathBuf>> {
    let mut sizes: HashMap<u64, Vec<PathBuf>> = HashMap::new();
    let mut stack: Vec<PathBuf> = roots.to_vec();
    while let Some(dir) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let Ok(read) = std::fs::read_dir(&dir) else { continue };
        for entry in read.flatten() {
            let Ok(ft) = entry.file_type() else { continue };
            if ft.is_symlink() {
                continue;
            }
            let path = entry.path();
            if ft.is_dir() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if skip_dir(&name) || excluded.iter().any(|ex| crate::project_roots::is_under(&path, ex)) {
                    continue;
                }
                stack.push(path);
                continue;
            }
            if !ft.is_file() {
                continue;
            }
            let Ok(m) = entry.metadata() else { continue };
            out.scanned_files += 1;
            // File rỗng thì bản nào cũng "trùng" bản nào — vô nghĩa, và xoá chẳng được gì.
            if m.len() >= min_size.max(1) {
                sizes.entry(m.len()).or_default().push(path);
            }
        }
    }
    sizes.retain(|_, v| v.len() > 1);
    sizes
}

/// Gom theo khoá băm; chỉ giữ khoá có từ hai file trở lên.
fn group_by_hash(paths: Vec<PathBuf>, mut hash: impl FnMut(&Path) -> Option<[u8; 32]>) -> Vec<Vec<PathBuf>> {
    let mut buckets: HashMap<[u8; 32], Vec<PathBuf>> = HashMap::new();
    for p in paths {
        if let Some(h) = hash(&p) {
            buckets.entry(h).or_default().push(p);
        }
    }
    buckets.into_values().filter(|v| v.len() > 1).collect()
}

fn make_group(bytes: u64, paths: Vec<PathBuf>) -> DupGroup {
    let mut files: Vec<DupFile> = paths
        .iter()
        .map(|p| {
            let modified = std::fs::metadata(p).map(|m| modified_epoch(&m)).unwrap_or(0);
            let same_dir_as_sibling = paths.iter().any(|q| q != p && q.parent() == p.parent());
            DupFile { path: p.clone(), modified, same_dir_as_sibling }
        })
        .collect();
    // Cũ nhất trước: bản đầu tiên thường là bản gốc, các bản sau là bản sao.
    files.sort_by(|a, b| a.modified.cmp(&b.modified).then_with(|| a.path.cmp(&b.path)));
    let wasted = bytes * (files.len() as u64 - 1);
    DupGroup { bytes, files, wasted }
}

/// Quét các gốc, trả nhóm file giống hệt nhau, nhóm lãng phí nhiều nhất trước.
pub fn scan(roots: &[PathBuf], excluded: &[PathBuf], min_size: u64, cancel: &AtomicBool, mut on_group: impl FnMut(&DupGroup)) -> DupScan {
    let mut out = DupScan::default();
    let sizes = by_size(roots, excluded, min_size, cancel, &mut out);
    for (bytes, paths) in sizes {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        // Pha 2: cùng cỡ chưa chắc cùng nội dung; 4 KB đầu loại gần hết cặp tình cờ.
        for candidates in group_by_hash(paths, head_hash) {
            if cancel.load(Ordering::Relaxed) {
                break;
            }
            // Pha 3: chỉ những file qua được pha 2 mới bị đọc toàn bộ.
            let hashed = candidates.len() as u64;
            let full = group_by_hash(candidates, |p| full_hash(p, cancel));
            out.hashed_files += hashed;
            for paths in full {
                let g = make_group(bytes, paths);
                on_group(&g);
                out.wasted += g.wasted;
                out.groups.push(g);
            }
        }
    }
    out.groups.sort_by(|a, b| b.wasted.cmp(&a.wasted));
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write(path: &Path, byte: u8, len: usize) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, vec![byte; len]).unwrap();
    }

    /// Cây thử: hai bản y hệt của "a" (một bản cùng thư mục), một file cùng cỡ nhưng khác nội
    /// dung, một cặp trùng nằm trong node_modules (phải bỏ qua), và một file nhỏ.
    fn fixture(name: &str) -> PathBuf {
        let root = std::env::temp_dir().join(format!("slclean-dup-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let big = 2 * MIN_SIZE as usize;
        write(&root.join("one/a.bin"), 7, big);
        write(&root.join("one/a copy.bin"), 7, big);
        write(&root.join("two/a.bin"), 7, big);
        // Cùng cỡ, khác nội dung: không được gom chung.
        write(&root.join("two/impostor.bin"), 9, big);
        // Trùng nhau nhưng nằm trong thư mục bị bỏ qua.
        write(&root.join("proj/node_modules/x.bin"), 3, big);
        write(&root.join("proj/node_modules/y.bin"), 3, big);
        // Dưới ngưỡng: bỏ qua dù trùng.
        write(&root.join("small/p.txt"), 1, 10);
        write(&root.join("small/q.txt"), 1, 10);
        root
    }

    fn scan_at(root: &Path) -> DupScan {
        scan(&[root.to_path_buf()], &[], MIN_SIZE, &AtomicBool::new(false), |_| {})
    }

    #[test]
    fn groups_only_byte_identical_files() {
        let root = fixture("basic");
        let s = scan_at(&root);
        assert_eq!(s.groups.len(), 1, "chỉ một nhóm hợp lệ: {:?}", s.groups);
        let g = &s.groups[0];
        assert_eq!(g.files.len(), 3, "ba bản của a.bin");
        assert_eq!(g.bytes, 2 * MIN_SIZE);
        assert_eq!(g.wasted, 2 * 2 * MIN_SIZE, "giữ một bản thì lấy lại được hai bản");
        assert_eq!(s.wasted, g.wasted);
        let names: Vec<String> = g.files.iter().map(|f| f.path.file_name().unwrap().to_string_lossy().into_owned()).collect();
        assert!(names.iter().all(|n| n.starts_with('a')), "impostor cùng cỡ nhưng khác nội dung: {names:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn skips_small_files_and_package_dirs() {
        let root = fixture("skip");
        let s = scan_at(&root);
        let all: Vec<String> = s.groups.iter().flat_map(|g| g.files.iter()).map(|f| f.path.to_string_lossy().to_ascii_lowercase()).collect();
        assert!(!all.iter().any(|p| p.contains("node_modules")), "trùng trong node_modules là đúng thiết kế, không phải rác");
        assert!(!all.iter().any(|p| p.ends_with(".txt")), "file dưới ngưỡng không được xét");
        // Kho gói của công cụ lập trình: cùng một jar nằm trong nhiều cache là bình thường.
        for d in ["node_modules", ".gradle", ".m2", ".nuget", "site-packages", ".paper-remapped"] {
            assert!(skip_dir(d), "{d} phải bị bỏ qua");
        }
        assert!(!skip_dir("Downloads") && !skip_dir("Documents"), "thư mục của người dùng thì phải quét");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn marks_siblings_in_the_same_folder() {
        let root = fixture("sibling");
        let s = scan_at(&root);
        let g = &s.groups[0];
        let by = |name: &str| g.files.iter().find(|f| f.path.ends_with(name)).unwrap();
        assert!(by("one\\a.bin").same_dir_as_sibling, "hai bản cùng nằm trong one\\");
        assert!(by("one\\a copy.bin").same_dir_as_sibling);
        assert!(!by("two\\a.bin").same_dir_as_sibling, "bản trong two\\ đứng một mình ở thư mục đó");
        // Sắp cũ nhất trước để người dùng nhận ra bản gốc.
        assert!(g.files[0].modified <= g.files[1].modified);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn three_phase_filter_avoids_hashing_lonely_sizes() {
        let root = std::env::temp_dir().join(format!("slclean-dup-phases-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        // Ba file, ba cỡ khác nhau: không cỡ nào có bạn → không file nào bị băm.
        write(&root.join("a.bin"), 1, MIN_SIZE as usize + 1);
        write(&root.join("b.bin"), 2, MIN_SIZE as usize + 2);
        write(&root.join("c.bin"), 3, MIN_SIZE as usize + 3);
        let s = scan_at(&root);
        assert!(s.groups.is_empty());
        assert_eq!(s.scanned_files, 3);
        assert_eq!(s.hashed_files, 0, "cỡ chỉ có một file thì không bao giờ phải đọc");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn excluded_roots_are_not_visited() {
        let root = fixture("excluded");
        let s = scan(&[root.clone()], &[root.join("two")], MIN_SIZE, &AtomicBool::new(false), |_| {});
        assert_eq!(s.groups.len(), 1);
        assert_eq!(s.groups[0].files.len(), 2, "bản trong two\\ bị loại khỏi nhóm");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn cancelling_stops_before_any_work() {
        let root = fixture("cancel");
        let s = scan(&[root.clone()], &[], MIN_SIZE, &AtomicBool::new(true), |_| {});
        assert!(s.groups.is_empty() && s.scanned_files == 0);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn program_areas_cover_the_installer_owned_folders() {
        let areas = program_areas();
        assert!(!areas.is_empty(), "máy Windows nào cũng có ít nhất Program Files");
        let has = |needle: &str| areas.iter().any(|p| p.to_string_lossy().to_ascii_lowercase().contains(needle));
        assert!(has("program files"), "{areas:?}");
        assert!(has("appdata"), "{areas:?}");
    }

    /// Quét đúng một thư mục do biến `SLCLEAN_DUP_ROOT` chỉ ra, để soi một fixture cụ thể:
    /// `SLCLEAN_DUP_ROOT=D:\ZzDupFixture cargo test -- --ignored --nocapture print_duplicates_of_root`
    #[test]
    #[ignore]
    fn print_duplicates_of_root() {
        let Ok(root) = std::env::var("SLCLEAN_DUP_ROOT") else {
            println!("đặt SLCLEAN_DUP_ROOT trước đã");
            return;
        };
        let s = scan(&[PathBuf::from(&root)], &program_areas(), MIN_SIZE, &AtomicBool::new(false), |_| {});
        println!("{root}: đi qua {} file, băm {}, {} nhóm", s.scanned_files, s.hashed_files, s.groups.len());
        for g in &s.groups {
            for f in &g.files {
                println!("  {} MB {}", g.bytes / 1024 / 1024, f.path.display());
            }
        }
    }

    /// `cargo test --release -- --ignored --nocapture print_this_machine_duplicates`
    #[test]
    #[ignore]
    fn print_this_machine_duplicates() {
        let roots: Vec<PathBuf> = crate::drives::list_drives().into_iter().map(|d| PathBuf::from(format!("{}\\", d.mount))).collect();
        let excluded = program_areas();
        let t0 = std::time::Instant::now();
        let s = scan(&roots, &excluded, MIN_SIZE, &AtomicBool::new(false), |_| {});
        println!("đi qua {} file, băm {} file, {} nhóm, lãng phí {} MB, mất {:?}", s.scanned_files, s.hashed_files, s.groups.len(), s.wasted / 1024 / 1024, t0.elapsed());
        for g in s.groups.iter().take(20) {
            println!("  {}x {:>8} MB → {:>8} MB  {}", g.files.len(), g.bytes / 1024 / 1024, g.wasted / 1024 / 1024, g.files[0].path.display());
        }
    }

    #[test]
    fn each_group_is_reported_as_it_is_found() {
        let root = fixture("callback");
        let mut seen = 0;
        scan(&[root.clone()], &[], MIN_SIZE, &AtomicBool::new(false), |g| { seen += g.files.len(); });
        assert_eq!(seen, 3, "nhóm phải được phát ngay khi tìm ra");
        let _ = std::fs::remove_dir_all(&root);
    }
}
