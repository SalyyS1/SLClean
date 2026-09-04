//! App Microsoft Store (MSIX/AppX) của người dùng hiện tại, đọc từ kho gói trong HKCU
//! (`AppModel\Repository\Packages`), không cần PowerShell hay admin. Tên hiển thị dạng
//! `ms-resource:` được giải qua SHLoadIndirectString. Gỡ bằng Remove-AppxPackage cho user
//! hiện tại (không cần admin).

use crate::registry::{self, HKEY_CURRENT_USER};
use std::collections::HashSet;
use std::os::windows::process::CommandExt;
use std::path::PathBuf;

const REPO: &str = r"Software\Classes\Local Settings\Software\Microsoft\Windows\CurrentVersion\AppModel\Repository\Packages";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Debug)]
pub struct StoreApp {
    pub full_name: String,
    /// `Microsoft.Paint_8wekyb3d8bbwe`: khớp với thư mục Local\Packages và AUMID trong UserAssist.
    pub family: String,
    pub display: String,
    pub publisher: String,
    pub version: String,
    pub root: PathBuf,
}

impl StoreApp {
    pub fn id(&self) -> String {
        format!("appx:{}", self.full_name)
    }
}

#[link(name = "shlwapi")]
extern "system" {
    fn SHLoadIndirectString(src: *const u16, out: *mut u16, cch: u32, reserved: *const std::ffi::c_void) -> i32;
}

fn load_indirect(src: &str) -> Option<String> {
    let mut buf = vec![0u16; 512];
    let hr = unsafe { SHLoadIndirectString(registry::wide(src).as_ptr(), buf.as_mut_ptr(), buf.len() as u32, std::ptr::null()) };
    if hr != 0 {
        return None;
    }
    let end = buf.iter().position(|&c| c == 0).unwrap_or(0);
    let s = String::from_utf16_lossy(&buf[..end]).trim().to_string();
    (!s.is_empty()).then_some(s)
}

/// Giải `ms-resource:…` của gói `full` thành chuỗi hiển thị. Dạng ngắn (`ms-resource:AppName`)
/// được thử dưới `/resources/` rồi ở gốc, theo cách Windows tự tra.
fn resolve(full: &str, name: &str, raw: &str) -> Option<String> {
    if raw.starts_with("@{") {
        return load_indirect(raw);
    }
    let body = raw.strip_prefix("ms-resource:")?;
    if body.starts_with("//") {
        return load_indirect(&format!("@{{{full}?{raw}}}"));
    }
    let body = body.trim_start_matches('/');
    load_indirect(&format!("@{{{full}?ms-resource://{name}/resources/{body}}}")).or_else(|| load_indirect(&format!("@{{{full}?ms-resource://{name}/{body}}}")))
}

fn display_text(full: &str, name: &str, raw: &str) -> Option<String> {
    if raw.starts_with("ms-resource:") || raw.starts_with("@{") {
        resolve(full, name, raw)
    } else {
        let t = raw.trim();
        (!t.is_empty()).then(|| t.to_string())
    }
}

/// Giá trị của thẻ XML `<tag>…</tag>` đầu tiên trong manifest (đủ cho hai thẻ tên).
fn xml_tag(text: &str, tag: &str) -> Option<String> {
    let open = format!("<{tag}>");
    let close = format!("</{tag}>");
    let s = text.find(&open)? + open.len();
    let e = text[s..].find(&close)? + s;
    Some(text[s..e].trim().to_string())
}

/// `Name_Version_Arch_Resource_Publisher`; gói tài nguyên/tách (Resource khác rỗng) không phải app.
fn parse_full(full: &str) -> Option<(String, String, String, String, String)> {
    let p: Vec<&str> = full.split('_').collect();
    if p.len() != 5 {
        return None;
    }
    Some((p[0].to_string(), p[1].to_string(), p[2].to_string(), p[3].to_string(), p[4].to_string()))
}

fn looks_like_guid(s: &str) -> bool {
    s.len() == 36 && s.bytes().filter(|&b| b == b'-').count() == 4
}

pub fn list() -> Vec<StoreApp> {
    let Some(repo) = registry::open(HKEY_CURRENT_USER, REPO) else { return Vec::new() };
    let system_root = std::env::var_os("SystemRoot").map(PathBuf::from).unwrap_or_else(|| PathBuf::from(r"C:\Windows"));
    let sys_lower = system_root.to_string_lossy().to_ascii_lowercase();
    let mut out = Vec::new();
    for full in repo.subkeys() {
        let Some(k) = repo.open(&full) else { continue };
        if k.dword("Framework") == Some(1) {
            continue;
        }
        let Some((name, version, _arch, resource, publisher_id)) = parse_full(&full) else { continue };
        if !resource.is_empty() || looks_like_guid(&name) {
            continue;
        }
        let Some(root) = k.string("PackageRootFolder").map(PathBuf::from) else { continue };
        if root.to_string_lossy().to_ascii_lowercase().starts_with(&sys_lower) {
            continue;
        }
        let manifest = std::fs::read_to_string(root.join("AppxManifest.xml")).unwrap_or_default();
        let display = k
            .string("DisplayName")
            .and_then(|raw| display_text(&full, &name, &raw))
            .or_else(|| xml_tag(&manifest, "DisplayName").and_then(|raw| display_text(&full, &name, &raw)))
            .unwrap_or_else(|| name.rsplit('.').next().unwrap_or(&name).to_string());
        let publisher = xml_tag(&manifest, "PublisherDisplayName").and_then(|raw| display_text(&full, &name, &raw)).unwrap_or_default();
        out.push(StoreApp { full_name: full.clone(), family: format!("{name}_{publisher_id}"), display, publisher, version, root });
    }
    out
}

/// Mọi family đang có gói (kể cả framework) để nhận ra thư mục Local\Packages mồ côi.
pub fn installed_families() -> HashSet<String> {
    let Some(repo) = registry::open(HKEY_CURRENT_USER, REPO) else { return HashSet::new() };
    repo.subkeys()
        .into_iter()
        .filter_map(|full| parse_full(&full).map(|(name, _, _, _, pubid)| format!("{name}_{pubid}").to_ascii_lowercase()))
        .collect()
}

pub fn find(id: &str) -> Option<StoreApp> {
    let full = id.strip_prefix("appx:")?;
    list().into_iter().find(|a| a.full_name == full)
}

pub fn exists(full_name: &str) -> bool {
    registry::open(HKEY_CURRENT_USER, &format!("{REPO}\\{full_name}")).is_some()
}

/// Gỡ gói cho user hiện tại. Gói hệ thống không gỡ được sẽ báo lỗi từ Windows.
pub fn remove(full_name: &str) -> Result<(), String> {
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &format!("Remove-AppxPackage -Package '{}'", full_name.replace('\'', "''"))])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("launch-failed: {e}"))?;
    if out.status.success() {
        return Ok(());
    }
    let err = String::from_utf8_lossy(&out.stderr);
    let line = err.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    Err(format!("store-refused: {line}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn full_name_parsing() {
        let (n, v, a, r, p) = parse_full("Microsoft.Paint_11.2510.31.0_x64__8wekyb3d8bbwe").unwrap();
        assert_eq!((n.as_str(), v.as_str(), a.as_str(), r.as_str(), p.as_str()), ("Microsoft.Paint", "11.2510.31.0", "x64", "", "8wekyb3d8bbwe"));
        assert_eq!(parse_full("Microsoft.ScreenSketch_11.0_neutral_split.scale-150_8wekyb3d8bbwe").unwrap().3, "split.scale-150");
        assert!(parse_full("nonsense").is_none());
        assert!(looks_like_guid("1527c705-839a-4832-9118-54d4bd6a0c89"));
    }

    #[test]
    fn xml_tag_extracts_first() {
        let m = "<Package><Properties><DisplayName>Paint</DisplayName><PublisherDisplayName>Microsoft</PublisherDisplayName></Properties></Package>";
        assert_eq!(xml_tag(m, "DisplayName").as_deref(), Some("Paint"));
        assert_eq!(xml_tag(m, "PublisherDisplayName").as_deref(), Some("Microsoft"));
        assert!(xml_tag(m, "Logo").is_none());
    }

    #[test]
    fn lists_user_packages_with_readable_names() {
        let apps = list();
        assert!(!apps.is_empty());
        // Không mục nào còn để nguyên chuỗi ms-resource chưa giải.
        assert!(apps.iter().all(|a| !a.display.starts_with("ms-resource") && !a.display.starts_with("@{")), "{:?}", apps.iter().filter(|a| a.display.starts_with("ms-resource")).map(|a| &a.full_name).collect::<Vec<_>>());
        assert!(installed_families().contains(&apps[0].family.to_ascii_lowercase()));
    }
}
