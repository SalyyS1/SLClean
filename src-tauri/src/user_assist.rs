//! Lần chạy cuối của từng chương trình theo Windows ghi nhận (UserAssist trong HKCU): mỗi lần
//! mở app qua Explorer/Start/Taskbar, Windows ghi số lần và FILETIME lần cuối. Tên giá trị
//! được mã ROT13, đường dẫn dưới Program Files/Windows được thay bằng GUID thư mục chuẩn.
//! Không cần admin, khác với BAM hay Prefetch.

use crate::registry::{self, filetime_to_epoch, HKEY_CURRENT_USER};
use std::collections::HashMap;
use std::os::windows::ffi::OsStringExt;

/// Khoá chứa chương trình chạy trực tiếp: đường dẫn exe, hoặc AppUserModelID (app Store dạng
/// `Family!App`; app desktop có shortcut mang ID, ví dụ `Chrome`, `Microsoft.VisualStudioCode`).
const EXE_COUNT_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\UserAssist\{CEBFF5CD-ACE2-4F4F-9178-9926F41749EA}\Count";
/// Khoá chứa shortcut .lnk đã mở (Start Menu, Taskbar, Desktop), đường dẫn cũng qua GUID thư mục.
const LNK_COUNT_KEY: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\UserAssist\{F4E57C4B-2036-45F0-A9AB-443BCFE33D9F}\Count";

#[derive(Clone, Debug)]
pub struct Run {
    /// Đường dẫn exe hoặc .lnk đã mở rộng, hoặc AppUserModelID (`Family!App` với app Store).
    pub target: String,
    pub count: u32,
    /// Giây epoch của lần chạy cuối; 0 nếu Windows chưa ghi.
    pub last: u64,
}

fn rot13(s: &str) -> String {
    s.chars()
        .map(|c| match c {
            'a'..='z' => (((c as u8 - b'a' + 13) % 26) + b'a') as char,
            'A'..='Z' => (((c as u8 - b'A' + 13) % 26) + b'A') as char,
            _ => c,
        })
        .collect()
}

#[repr(C)]
struct Guid {
    d1: u32,
    d2: u16,
    d3: u16,
    d4: [u8; 8],
}

fn parse_guid(s: &str) -> Option<Guid> {
    let s = s.trim_matches(|c| c == '{' || c == '}');
    let parts: Vec<&str> = s.split('-').collect();
    if parts.len() != 5 {
        return None;
    }
    let d1 = u32::from_str_radix(parts[0], 16).ok()?;
    let d2 = u16::from_str_radix(parts[1], 16).ok()?;
    let d3 = u16::from_str_radix(parts[2], 16).ok()?;
    let tail = format!("{}{}", parts[3], parts[4]);
    if tail.len() != 16 {
        return None;
    }
    let mut d4 = [0u8; 8];
    for (i, b) in d4.iter_mut().enumerate() {
        *b = u8::from_str_radix(&tail[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(Guid { d1, d2, d3, d4 })
}

#[link(name = "shell32")]
extern "system" {
    fn SHGetKnownFolderPath(rfid: *const Guid, flags: u32, token: isize, out: *mut *mut u16) -> i32;
}
#[link(name = "ole32")]
extern "system" {
    fn CoTaskMemFree(p: *mut u16);
}

/// Đường dẫn của một thư mục chuẩn Windows theo GUID (`{6D809377-…}` → `C:\Program Files`).
pub fn known_folder(guid: &str) -> Option<String> {
    let g = parse_guid(guid)?;
    let mut p: *mut u16 = std::ptr::null_mut();
    let hr = unsafe { SHGetKnownFolderPath(&g, 0, 0, &mut p) };
    if hr != 0 || p.is_null() {
        return None;
    }
    let mut len = 0;
    while unsafe { *p.add(len) } != 0 {
        len += 1;
    }
    let s = std::ffi::OsString::from_wide(unsafe { std::slice::from_raw_parts(p, len) }).to_string_lossy().into_owned();
    unsafe { CoTaskMemFree(p) };
    Some(s)
}

/// Thay tiền tố `{GUID}\` bằng đường dẫn thật; GUID lạ thì giữ nguyên chuỗi.
fn expand_target(name: &str, cache: &mut HashMap<String, Option<String>>) -> String {
    if !name.starts_with('{') {
        return name.to_string();
    }
    let Some(end) = name.find('}') else { return name.to_string() };
    let guid = &name[..=end];
    let rest = &name[end + 1..];
    let folder = cache.entry(guid.to_string()).or_insert_with(|| known_folder(guid));
    match folder {
        Some(f) => format!("{f}{rest}"),
        None => name.to_string(),
    }
}

/// Giải mã một bản ghi UserAssist (72 byte từ Windows 7): số lần chạy ở byte 4, FILETIME lần
/// cuối ở byte 60. Bản ghi ngắn hơn (định dạng cũ) bỏ qua.
fn decode(data: &[u8]) -> Option<(u32, u64)> {
    if data.len() < 68 {
        return None;
    }
    let count = u32::from_le_bytes([data[4], data[5], data[6], data[7]]);
    let ft = u64::from_le_bytes(data[60..68].try_into().ok()?);
    Some((count, if ft == 0 { 0 } else { filetime_to_epoch(ft) }))
}

fn read_hive(sub: &str, cache: &mut HashMap<String, Option<String>>, out: &mut Vec<Run>) {
    let Some(key) = registry::open(HKEY_CURRENT_USER, sub) else { return };
    for raw_name in key.value_names() {
        let name = rot13(&raw_name);
        if name.starts_with("UEME_") {
            continue;
        }
        let Some(data) = key.binary(&raw_name) else { continue };
        let Some((count, last)) = decode(&data) else { continue };
        out.push(Run { target: expand_target(&name, cache), count, last });
    }
}

/// Mọi bản ghi của cả hai khoá (exe/AUMID và .lnk), chưa đổi shortcut thành exe.
pub fn runs() -> Vec<Run> {
    let mut cache = HashMap::new();
    let mut out = Vec::new();
    read_hive(EXE_COUNT_KEY, &mut cache, &mut out);
    read_hive(LNK_COUNT_KEY, &mut cache, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rot13_round_trips_paths() {
        assert_eq!(rot13("Hello"), "Uryyb");
        assert_eq!(rot13(&rot13(r"C:\Users\x\app.exe")), r"C:\Users\x\app.exe");
    }

    #[test]
    fn decodes_count_and_last_run() {
        let mut rec = vec![0u8; 72];
        rec[4..8].copy_from_slice(&7u32.to_le_bytes());
        rec[60..68].copy_from_slice(&134_116_992_000_000_000u64.to_le_bytes());
        assert_eq!(decode(&rec), Some((7, 1_767_225_600)));
        assert_eq!(decode(&rec[..16]), None);
    }

    #[test]
    fn program_files_guid_expands() {
        let mut cache = HashMap::new();
        let s = expand_target(r"{6D809377-6AF0-444B-8957-A3773F02200E}\WinSCP\WinSCP.exe", &mut cache);
        assert!(s.to_ascii_lowercase().ends_with(r"\winscp\winscp.exe"), "{s}");
        assert!(!s.starts_with('{'), "{s}");
        assert_eq!(expand_target("plain", &mut cache), "plain");
    }
}
