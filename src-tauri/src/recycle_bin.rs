//! Thùng rác qua hai lời gọi Win32 có sẵn: `SHQueryRecycleBinW` đọc tổng số mục và dung lượng
//! từ chỉ mục của shell (một lời gọi, không phụ thuộc số mục), `SHEmptyRecycleBinW` dọn sạch.
//!
//! Trước đây dùng crate `trash` liệt kê từng mục qua COM rồi hỏi kích cỡ từng cái; với vài
//! nghìn mục mất nhiều giây và vì lệnh Tauri đồng bộ chạy trên main thread nên cửa sổ bị
//! "Not responding" ngay sau khi quét xong hoặc sau mỗi lần dọn.

use serde::Serialize;

#[derive(Clone, Copy, Serialize)]
pub struct RecycleBin {
    pub items: u64,
    pub bytes: u64,
}

/// SHQUERYRBINFO: trên x64 các trường i64 căn 8 byte (kích cỡ 24); trên x86 shell32 dùng
/// pack(1) (kích cỡ 20). `cb_size` phải điền đúng kích cỡ này trước khi gọi.
#[repr(C)]
#[cfg_attr(target_arch = "x86", repr(packed(1)))]
#[derive(Default)]
struct ShQueryRbInfo {
    cb_size: u32,
    size: i64,
    num_items: i64,
}

#[link(name = "shell32")]
extern "system" {
    fn SHQueryRecycleBinW(root_path: *const u16, info: *mut ShQueryRbInfo) -> i32;
    fn SHEmptyRecycleBinW(hwnd: isize, root_path: *const u16, flags: u32) -> i32;
}

const S_OK: i32 = 0;
/// Shell trả mã này khi Thùng rác đã trống sẵn; không phải lỗi với người dùng.
const E_UNEXPECTED: i32 = 0x8000_FFFFu32 as i32;
const SHERB_NOCONFIRMATION: u32 = 0x1;
const SHERB_NOSOUND: u32 = 0x4;

/// Tổng của Thùng rác trên mọi ổ đĩa (root_path = null).
pub fn query() -> Result<RecycleBin, String> {
    let mut info = ShQueryRbInfo { cb_size: std::mem::size_of::<ShQueryRbInfo>() as u32, ..Default::default() };
    let hr = unsafe { SHQueryRecycleBinW(std::ptr::null(), &mut info) };
    if hr != S_OK {
        return Err(format!("SHQueryRecycleBin 0x{:08x}", hr as u32));
    }
    // Đọc qua biến cục bộ vì trên x86 struct bị pack, không tạo tham chiếu vào trường được.
    let (size, num_items) = (info.size, info.num_items);
    Ok(RecycleBin { items: num_items.max(0) as u64, bytes: size.max(0) as u64 })
}

/// Dọn sạch Thùng rác trên mọi ổ đĩa, không hỏi lại, không phát âm thanh; Windows vẫn hiện hộp
/// tiến trình của shell khi có nhiều mục. Thùng rác trống sẵn không phải lỗi.
pub fn empty() -> Result<(), String> {
    let hr = unsafe { SHEmptyRecycleBinW(0, std::ptr::null(), SHERB_NOCONFIRMATION | SHERB_NOSOUND) };
    if hr == S_OK || hr == E_UNEXPECTED {
        Ok(())
    } else {
        Err(format!("SHEmptyRecycleBin 0x{:08x}", hr as u32))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn struct_layout_matches_shell32() {
        let expected = if cfg!(target_arch = "x86") { 20 } else { 24 };
        assert_eq!(std::mem::size_of::<ShQueryRbInfo>(), expected);
    }

    #[test]
    fn query_answers_without_enumerating() {
        let t0 = std::time::Instant::now();
        let rb = query().expect("shell should answer");
        assert!(t0.elapsed().as_millis() < 2000, "took {:?}", t0.elapsed());
        // Thùng rác trống có 0 mục và 0 byte; có mục thì byte có thể là 0 nếu toàn file rỗng.
        assert!(rb.items > 0 || rb.bytes == 0);
    }
}
