//! Tính dung lượng một thư mục bằng `read_dir` thuần với ngăn xếp tường minh. Trên Windows,
//! `DirEntry::metadata` và `file_type` lấy từ dữ liệu FindNextFile đã có sẵn, không tốn thêm
//! syscall cho từng file, nên nhanh hơn walker chung. Bỏ qua reparse point (junction/symlink)
//! để không đếm trùng và không đi lạc sang ổ khác; thư mục không đọc được đếm vào `denied`.
//!
//! Mỗi lần đo chạy tuần tự trên một thread; song song hoá nằm ở tầng trên (`parallel::map`).

use std::fs;
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};

#[derive(Clone, Copy, Debug, Default)]
pub struct DirStats {
    pub bytes: u64,
    pub files: u64,
    /// Số thư mục/mục không đọc được (quyền truy cập), để UI báo "một phần".
    pub denied: u64,
}

/// Số file giữa hai lần báo tiến trình.
const PROGRESS_EVERY: u64 = 4096;

pub fn dir_stats(root: &Path, cancel: &AtomicBool) -> DirStats {
    dir_stats_with(root, cancel, |_| {})
}

/// Như `dir_stats` nhưng gọi `on_progress` định kỳ với số liệu tạm để UI hiện số đang tăng.
pub fn dir_stats_with(root: &Path, cancel: &AtomicBool, mut on_progress: impl FnMut(&DirStats)) -> DirStats {
    let mut stats = DirStats::default();
    if cancel.load(Ordering::Relaxed) {
        return stats;
    }
    let mut stack = vec![root.to_path_buf()];
    let mut since_report = 0u64;
    while let Some(dir) = stack.pop() {
        if cancel.load(Ordering::Relaxed) {
            break;
        }
        let read = match fs::read_dir(&dir) {
            Ok(it) => it,
            Err(_) => {
                stats.denied += 1;
                continue;
            }
        };
        for entry in read {
            let Ok(entry) = entry else {
                stats.denied += 1;
                continue;
            };
            let Ok(ft) = entry.file_type() else {
                stats.denied += 1;
                continue;
            };
            if ft.is_symlink() {
                continue;
            }
            if ft.is_dir() {
                stack.push(entry.path());
            } else if ft.is_file() {
                if let Ok(m) = entry.metadata() {
                    stats.bytes += m.len();
                    stats.files += 1;
                    since_report += 1;
                    if since_report >= PROGRESS_EVERY {
                        since_report = 0;
                        on_progress(&stats);
                    }
                }
            }
        }
    }
    stats
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn fixture(name: &str, dirs: usize) -> PathBuf {
        let root = std::env::temp_dir().join(format!("slclean-sizer-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&root);
        for i in 0..dirs {
            let d = root.join(format!("d{i}/e/f"));
            fs::create_dir_all(&d).unwrap();
            fs::write(d.join("x.bin"), vec![1u8; 1000]).unwrap();
        }
        root
    }

    #[test]
    fn counts_bytes_and_files_of_nested_tree() {
        let root = fixture("basic", 40);
        let st = dir_stats(&root, &AtomicBool::new(false));
        assert_eq!(st.bytes, 40_000, "{st:?}");
        assert_eq!(st.files, 40, "{st:?}");
        assert_eq!(st.denied, 0, "{st:?}");
        let _ = fs::remove_dir_all(&root);
    }

    #[test]
    fn missing_dir_is_denied_not_panic() {
        let st = dir_stats(Path::new(r"C:\definitely\not\here-slclean"), &AtomicBool::new(false));
        assert_eq!(st.bytes, 0);
        assert_eq!(st.denied, 1);
    }

    #[test]
    fn progress_reports_partial_then_final_is_complete() {
        let root = fixture("progress", 10);
        // 10 file < PROGRESS_EVERY: không có báo cáo giữa chừng, nhưng tổng cuối vẫn đúng.
        let mut reports = 0;
        let st = dir_stats_with(&root, &AtomicBool::new(false), |_| reports += 1);
        assert_eq!(reports, 0);
        assert_eq!(st.files, 10);
        let _ = fs::remove_dir_all(&root);
    }

    /// 60 lần đo cùng lúc trên pool riêng phải cho cùng kết quả (không có "threadpool busy").
    #[test]
    fn many_concurrent_walks_never_starve() {
        let root = fixture("starve", 40);
        let cancel = AtomicBool::new(false);
        let results = crate::parallel::map((0..60).collect::<Vec<_>>(), 60, |_| dir_stats(&root, &cancel));
        for st in &results {
            assert_eq!(st.bytes, 40_000, "{st:?}");
            assert_eq!(st.denied, 0, "{st:?}");
        }
        let _ = fs::remove_dir_all(&root);
    }
}
