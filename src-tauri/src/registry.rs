//! Đọc và xoá registry qua advapi32 trực tiếp, chỉ những gì app cần: mở khoá, liệt kê khoá
//! con và tên giá trị, đọc chuỗi/số/nhị phân, giờ ghi cuối của khoá, xoá cả nhánh. Không kéo
//! thêm crate vì phần này nhỏ và ổn định.

use std::os::windows::ffi::OsStringExt;

pub type Hkey = isize;
pub const HKEY_CURRENT_USER: Hkey = 0x8000_0001u32 as i32 as isize;
pub const HKEY_LOCAL_MACHINE: Hkey = 0x8000_0002u32 as i32 as isize;

const KEY_READ: u32 = 0x2_0019;
const ERROR_SUCCESS: i32 = 0;
const ERROR_NO_MORE_ITEMS: i32 = 259;
const ERROR_MORE_DATA: i32 = 234;
const REG_SZ: u32 = 1;
const REG_EXPAND_SZ: u32 = 2;
const REG_DWORD: u32 = 4;

#[repr(C)]
#[derive(Default)]
struct FileTime {
    lo: u32,
    hi: u32,
}

#[link(name = "advapi32")]
extern "system" {
    fn RegOpenKeyExW(hkey: Hkey, sub: *const u16, options: u32, sam: u32, out: *mut Hkey) -> i32;
    fn RegCloseKey(hkey: Hkey) -> i32;
    fn RegEnumKeyExW(hkey: Hkey, index: u32, name: *mut u16, name_len: *mut u32, reserved: *const u32, class: *mut u16, class_len: *mut u32, last_write: *mut FileTime) -> i32;
    fn RegEnumValueW(hkey: Hkey, index: u32, name: *mut u16, name_len: *mut u32, reserved: *const u32, vtype: *mut u32, data: *mut u8, data_len: *mut u32) -> i32;
    fn RegQueryValueExW(hkey: Hkey, name: *const u16, reserved: *const u32, vtype: *mut u32, data: *mut u8, data_len: *mut u32) -> i32;
    fn RegQueryInfoKeyW(hkey: Hkey, class: *mut u16, class_len: *mut u32, reserved: *const u32, subkeys: *mut u32, max_subkey: *mut u32, max_class: *mut u32, values: *mut u32, max_value_name: *mut u32, max_value: *mut u32, security: *mut u32, last_write: *mut FileTime) -> i32;
    fn RegDeleteTreeW(hkey: Hkey, sub: *const u16) -> i32;
}

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    std::ffi::OsString::from_wide(&buf[..end]).to_string_lossy().into_owned()
}

/// FILETIME (100 ns từ 1601) → giây epoch Unix; 0 nếu rỗng.
pub fn filetime_to_epoch(ft: u64) -> u64 {
    const EPOCH_DIFF: u64 = 11_644_473_600;
    let secs = ft / 10_000_000;
    secs.saturating_sub(EPOCH_DIFF)
}

/// Thay `%VAR%` bằng biến môi trường (REG_EXPAND_SZ). Biến không có thì giữ nguyên.
pub fn expand_env(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut rest = s;
    while let Some(start) = rest.find('%') {
        out.push_str(&rest[..start]);
        let after = &rest[start + 1..];
        match after.find('%') {
            Some(end) => {
                let name = &after[..end];
                match std::env::var(name) {
                    Ok(v) if !name.is_empty() => out.push_str(&v),
                    _ => {
                        out.push('%');
                        out.push_str(name);
                        out.push('%');
                    }
                }
                rest = &after[end + 1..];
            }
            None => {
                out.push('%');
                rest = after;
            }
        }
    }
    out.push_str(rest);
    out
}

/// Khoá đã mở; tự đóng khi rơi khỏi phạm vi.
pub struct Key(Hkey);

impl Drop for Key {
    fn drop(&mut self) {
        unsafe {
            RegCloseKey(self.0);
        }
    }
}

pub fn open(root: Hkey, sub: &str) -> Option<Key> {
    let mut h: Hkey = 0;
    let rc = unsafe { RegOpenKeyExW(root, wide(sub).as_ptr(), 0, KEY_READ, &mut h) };
    (rc == ERROR_SUCCESS).then_some(Key(h))
}

impl Key {
    pub fn open(&self, sub: &str) -> Option<Key> {
        open(self.0, sub)
    }

    pub fn subkeys(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut buf = vec![0u16; 512];
        for i in 0.. {
            let mut len = buf.len() as u32;
            let rc = unsafe { RegEnumKeyExW(self.0, i, buf.as_mut_ptr(), &mut len, std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) };
            match rc {
                ERROR_SUCCESS => out.push(from_wide(&buf[..len as usize])),
                ERROR_NO_MORE_ITEMS => break,
                _ => continue,
            }
        }
        out
    }

    pub fn value_names(&self) -> Vec<String> {
        let mut out = Vec::new();
        let mut buf = vec![0u16; 16_384];
        for i in 0.. {
            let mut len = buf.len() as u32;
            let rc = unsafe { RegEnumValueW(self.0, i, buf.as_mut_ptr(), &mut len, std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) };
            match rc {
                ERROR_SUCCESS => out.push(from_wide(&buf[..len as usize])),
                ERROR_NO_MORE_ITEMS => break,
                _ => continue,
            }
        }
        out
    }

    fn raw(&self, name: &str) -> Option<(u32, Vec<u8>)> {
        let name = wide(name);
        let mut vtype = 0u32;
        let mut len = 0u32;
        let rc = unsafe { RegQueryValueExW(self.0, name.as_ptr(), std::ptr::null(), &mut vtype, std::ptr::null_mut(), &mut len) };
        if rc != ERROR_SUCCESS && rc != ERROR_MORE_DATA {
            return None;
        }
        let mut data = vec![0u8; len as usize + 2];
        let mut len2 = data.len() as u32;
        let rc = unsafe { RegQueryValueExW(self.0, name.as_ptr(), std::ptr::null(), &mut vtype, data.as_mut_ptr(), &mut len2) };
        if rc != ERROR_SUCCESS {
            return None;
        }
        data.truncate(len2 as usize);
        Some((vtype, data))
    }

    /// REG_SZ / REG_EXPAND_SZ (đã thay biến môi trường); kiểu khác trả None. Chuỗi rỗng → None.
    pub fn string(&self, name: &str) -> Option<String> {
        let (vtype, data) = self.raw(name)?;
        if vtype != REG_SZ && vtype != REG_EXPAND_SZ {
            return None;
        }
        let u16s: Vec<u16> = data.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        let s = from_wide(&u16s);
        let s = if vtype == REG_EXPAND_SZ { expand_env(&s) } else { s };
        let s = s.trim().to_string();
        (!s.is_empty()).then_some(s)
    }

    pub fn dword(&self, name: &str) -> Option<u32> {
        let (vtype, data) = self.raw(name)?;
        (vtype == REG_DWORD && data.len() >= 4).then(|| u32::from_le_bytes([data[0], data[1], data[2], data[3]]))
    }

    pub fn binary(&self, name: &str) -> Option<Vec<u8>> {
        self.raw(name).map(|(_, d)| d)
    }

    /// Giờ ghi cuối của khoá (giây epoch); dùng làm ngày cài đặt dự phòng.
    pub fn last_write_epoch(&self) -> Option<u64> {
        let mut ft = FileTime::default();
        let rc = unsafe {
            RegQueryInfoKeyW(self.0, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), &mut ft)
        };
        (rc == ERROR_SUCCESS).then(|| filetime_to_epoch(((ft.hi as u64) << 32) | ft.lo as u64))
    }
}

/// Xoá khoá `sub` dưới `root` cùng mọi khoá con. Err(5) = không có quyền (HKLM cần admin).
pub fn delete_tree(root: Hkey, sub: &str) -> Result<(), i32> {
    let rc = unsafe { RegDeleteTreeW(root, wide(sub).as_ptr()) };
    if rc == ERROR_SUCCESS { Ok(()) } else { Err(rc) }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn expands_env_and_keeps_unknown() {
        std::env::set_var("SLCLEAN_TEST_VAR", r"C:\x");
        assert_eq!(expand_env(r"%SLCLEAN_TEST_VAR%\y"), r"C:\x\y");
        assert_eq!(expand_env("%NOPE_SLCLEAN%\\z"), "%NOPE_SLCLEAN%\\z");
        assert_eq!(expand_env("50%"), "50%");
    }

    #[test]
    fn filetime_epoch_matches_known_date() {
        // 2026-01-01T00:00:00Z = 1767225600 epoch = 134_116_992_000_000_000 FILETIME.
        assert_eq!(filetime_to_epoch(134_116_992_000_000_000), 1_767_225_600);
        assert_eq!(filetime_to_epoch(0), 0);
    }

    #[test]
    fn reads_a_well_known_key() {
        let k = open(HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion").expect("open");
        assert!(k.string("ProductName").is_some());
        assert!(k.dword("CurrentMajorVersionNumber").is_some());
        assert!(k.last_write_epoch().unwrap_or(0) > 1_000_000_000);
        assert!(!open(HKEY_LOCAL_MACHINE, r"SOFTWARE\Microsoft\Windows NT\CurrentVersion").unwrap().subkeys().is_empty());
    }
}
