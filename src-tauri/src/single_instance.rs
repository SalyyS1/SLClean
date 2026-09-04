//! Chỉ cho một bản SLClean chạy mỗi lần. WebView2 dùng chung một thư mục dữ liệu cho mỗi app,
//! nên bản thứ hai không tạo được webview và chỉ hiện ra cửa sổ đen vô dụng. Bản mới sẽ đưa
//! cửa sổ đang có ra trước rồi tự thoát. Bước chuyển sang quyền admin dùng `--after-pid` để
//! chờ bản cũ thoát hẳn trước khi khởi tạo webview.

use std::ffi::OsStr;
use std::time::{Duration, Instant};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System};

/// Phải khớp `app.windows[0].title` trong tauri.conf.json để tìm lại cửa sổ cũ.
const WINDOW_TITLE: &str = "SLClean";
pub const AFTER_PID_FLAG: &str = "--after-pid";

#[link(name = "user32")]
extern "system" {
    fn FindWindowW(class_name: *const u16, window_name: *const u16) -> isize;
    fn SetForegroundWindow(hwnd: isize) -> i32;
    fn ShowWindow(hwnd: isize, cmd: i32) -> i32;
}

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn slclean_pids() -> Vec<u32> {
    let mut sys = System::new();
    sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing());
    let exe = std::env::current_exe()
        .ok()
        .and_then(|p| p.file_name().map(OsStr::to_os_string))
        .unwrap_or_else(|| OsStr::new("slclean.exe").to_os_string());
    sys.processes()
        .values()
        .filter(|p| p.name().eq_ignore_ascii_case(&exe))
        .map(|p| p.pid().as_u32())
        .collect()
}

/// Đọc pid từ `--after-pid <n>` nếu có.
pub fn after_pid_arg() -> Option<u32> {
    let mut args = std::env::args();
    while let Some(a) = args.next() {
        if a == AFTER_PID_FLAG {
            return args.next().and_then(|v| v.parse().ok());
        }
    }
    None
}

/// Chờ `pid` thoát, tối đa `timeout`. Trả false nếu hết thời gian mà nó vẫn còn.
pub fn wait_for_exit(pid: u32, timeout: Duration) -> bool {
    let start = Instant::now();
    while start.elapsed() < timeout {
        if !slclean_pids().contains(&pid) {
            return true;
        }
        std::thread::sleep(Duration::from_millis(120));
    }
    false
}

/// true nếu đã có bản khác đang chạy; khi đó đã cố đưa cửa sổ của nó ra trước
/// (có thể thất bại nếu bản kia chạy quyền admin — Windows chặn, vẫn nên thoát).
pub fn focus_existing_and_should_exit() -> bool {
    let me = std::process::id();
    if !slclean_pids().iter().any(|&p| p != me) {
        return false;
    }
    let title = wide(WINDOW_TITLE);
    let hwnd = unsafe { FindWindowW(std::ptr::null(), title.as_ptr()) };
    if hwnd != 0 {
        const SW_RESTORE: i32 = 9;
        unsafe {
            ShowWindow(hwnd, SW_RESTORE);
            SetForegroundWindow(hwnd);
        }
    }
    true
}
