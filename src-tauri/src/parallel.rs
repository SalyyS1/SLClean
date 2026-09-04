//! Chạy một loạt công việc trên số thread cố định. Dùng thay cho rayon pool toàn cục:
//! mỗi lần đo dung lượng là một walk dài, nếu hàng chục walk cùng tranh một pool 12 thread
//! thì jwalk trả lỗi "threadpool busy" và báo 0 byte. Ở đây mỗi walk chạy tuần tự trên
//! đúng một thread của ta, nên số việc song song luôn bằng số thread và không bao giờ đói.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;

/// Số thread mặc định: đủ để bù thời gian chờ ổ đĩa mà không làm ngập nó.
pub fn default_workers() -> usize {
    std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 12)
}

/// Áp `f` lên từng phần tử; trả kết quả theo thứ tự hoàn thành (không phải thứ tự đầu vào).
pub fn map<T, R, F>(items: Vec<T>, workers: usize, f: F) -> Vec<R>
where
    T: Send,
    R: Send,
    F: Fn(T) -> R + Send + Sync,
{
    let n = items.len();
    if n == 0 {
        return Vec::new();
    }
    let workers = workers.clamp(1, n);
    // Mỗi phần tử được đúng một thread nhận nhờ chỉ số nguyên tử tăng dần.
    let next = AtomicUsize::new(0);
    let slots: Vec<Mutex<Option<T>>> = items.into_iter().map(|t| Mutex::new(Some(t))).collect();
    let out = Mutex::new(Vec::with_capacity(n));
    std::thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| loop {
                let i = next.fetch_add(1, Ordering::Relaxed);
                if i >= n {
                    break;
                }
                let Some(item) = slots[i].lock().unwrap().take() else { continue };
                let r = f(item);
                out.lock().unwrap().push(r);
            });
        }
    });
    out.into_inner().unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_item_runs_exactly_once() {
        let out = map((0..500u64).collect(), 12, |x| x * 2);
        assert_eq!(out.len(), 500);
        let mut sorted = out;
        sorted.sort_unstable();
        assert_eq!(sorted, (0..500u64).map(|x| x * 2).collect::<Vec<_>>());
    }

    #[test]
    fn empty_input_is_fine() {
        assert!(map(Vec::<u8>::new(), 8, |x| x).is_empty());
    }
}
