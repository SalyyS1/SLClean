//! Ứng dụng desktop đã đăng ký với Windows: ba nhánh `Uninstall` trong registry (HKLM 64-bit,
//! HKLM 32-bit qua WOW6432Node, HKCU). Mỗi mục cho tên, hãng, phiên bản, thư mục cài, lệnh
//! gỡ. Mục "chết" là mục mà trình gỡ không còn (người dùng xoá tay thư mục) nên Settings của
//! Windows không gỡ được nữa; app xoá khoá registry của nó, và thư mục còn sót nếu có.

use crate::registry::{self, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
use serde::Serialize;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};

const UNINSTALL: &str = r"SOFTWARE\Microsoft\Windows\CurrentVersion\Uninstall";
const UNINSTALL_WOW: &str = r"SOFTWARE\WOW6432Node\Microsoft\Windows\CurrentVersion\Uninstall";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Hive {
    Hklm,
    Hkcu,
}

impl Hive {
    fn handle(self) -> registry::Hkey {
        match self {
            Hive::Hklm => HKEY_LOCAL_MACHINE,
            Hive::Hkcu => HKEY_CURRENT_USER,
        }
    }
}

#[derive(Clone, Debug)]
pub struct DesktopApp {
    pub hive: Hive,
    pub wow64: bool,
    pub key: String,
    pub name: String,
    pub publisher: String,
    pub version: String,
    /// InstallLocation nếu có, không thì thư mục chứa trình gỡ hoặc icon.
    pub install_dir: Option<PathBuf>,
    pub installed: u64,
    /// EstimatedSize của registry (byte); 0 nếu không ghi.
    pub est_bytes: u64,
    pub uninstall: Option<String>,
    pub msi: bool,
    pub dead: bool,
}

impl DesktopApp {
    pub fn id(&self) -> String {
        format!("reg:{}:{}:{}", if self.hive == Hive::Hklm { "hklm" } else { "hkcu" }, if self.wow64 { 1 } else { 0 }, self.key)
    }

    fn sub_path(&self) -> String {
        format!("{}\\{}", if self.wow64 { UNINSTALL_WOW } else { UNINSTALL }, self.key)
    }
}

/// `"C:\x\u.exe" /S` → ("C:\x\u.exe", "/S"); `C:\Program Files\x\u.exe --q` → tách tại `.exe`;
/// `MsiExec.exe /X{…}` → ("MsiExec.exe", "/X{…}"); `winget uninstall …` → ("winget", "uninstall …").
pub fn split_command(s: &str) -> Option<(String, String)> {
    let s = s.trim();
    if s.is_empty() {
        return None;
    }
    if let Some(rest) = s.strip_prefix('"') {
        let end = rest.find('"')?;
        return Some((rest[..end].to_string(), rest[end + 1..].trim().to_string()));
    }
    let lower = s.to_ascii_lowercase();
    let mut start = 0;
    while let Some(i) = lower[start..].find(".exe") {
        let end = start + i + 4;
        if end == s.len() || s[end..].starts_with(char::is_whitespace) {
            return Some((s[..end].to_string(), s[end..].trim().to_string()));
        }
        start = end;
    }
    let mut parts = s.splitn(2, char::is_whitespace);
    let file = parts.next()?.to_string();
    Some((file, parts.next().unwrap_or("").trim().to_string()))
}

fn is_msi(file: &str) -> bool {
    Path::new(file).file_stem().map(|s| s.to_string_lossy().eq_ignore_ascii_case("msiexec")).unwrap_or(false)
}

/// Đường dẫn tuyệt đối tới một .exe, nếu `file` đúng là như vậy.
fn abs_exe(file: &str) -> Option<PathBuf> {
    let p = PathBuf::from(file.trim());
    let is_exe = p.extension().map(|e| e.eq_ignore_ascii_case("exe")).unwrap_or(false);
    (is_exe && p.is_absolute()).then_some(p)
}

/// Ngày trong lịch → giây epoch (thuật toán days_from_civil).
fn ymd_to_epoch(y: i64, m: u32, d: u32) -> u64 {
    let y = if m <= 2 { y - 1 } else { y };
    let era = if y >= 0 { y } else { y - 399 } / 400;
    let yoe = y - era * 400;
    let mp = (m as i64 + 9) % 12;
    let doy = (153 * mp + 2) / 5 + d as i64 - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    (days.max(0) as u64) * 86_400
}

/// InstallDate thường là "20260824"; vài trình cài ghi giây epoch. Định dạng khác → None.
fn parse_install_date(s: &str) -> Option<u64> {
    let t = s.trim();
    if t.len() == 8 && t.bytes().all(|b| b.is_ascii_digit()) {
        let y: i64 = t[..4].parse().ok()?;
        let m: u32 = t[4..6].parse().ok()?;
        let d: u32 = t[6..].parse().ok()?;
        if (1..=12).contains(&m) && (1..=31).contains(&d) {
            return Some(ymd_to_epoch(y, m, d));
        }
        return None;
    }
    let n: u64 = t.parse().ok()?;
    (1_000_000_000..4_000_000_000).contains(&n).then_some(n)
}

/// InstallLocation có thể có ngoặc kép, `\` cuối, hay `/` thay `\` (Riot): chuẩn về dạng Windows.
fn clean_dir(s: &str) -> Option<PathBuf> {
    let t = s.trim().trim_matches('"').replace('/', "\\");
    let t = t.trim_end_matches('\\').trim();
    (!t.is_empty() && Path::new(t).is_absolute()).then(|| PathBuf::from(t))
}

fn read_one(hive: Hive, wow64: bool, key: &registry::Key, name: &str) -> Option<DesktopApp> {
    let display = key.string("DisplayName")?;
    if key.dword("SystemComponent") == Some(1) || key.string("ParentKeyName").is_some() || key.string("ReleaseType").is_some() {
        return None;
    }
    let uninstall = key.string("UninstallString").or_else(|| key.string("QuietUninstallString"));
    let location = key.string("InstallLocation").and_then(|s| clean_dir(&s));
    let (file, _) = uninstall.as_deref().and_then(split_command).unwrap_or_default();
    let msi = !file.is_empty() && is_msi(&file);
    let un_exe = abs_exe(&file);
    let icon_exe = key.string("DisplayIcon").and_then(|s| {
        let s = s.trim_matches('"');
        let s = s.rsplit_once(',').map(|(a, b)| if b.trim().trim_start_matches('-').bytes().all(|c| c.is_ascii_digit()) { a } else { s }).unwrap_or(s);
        abs_exe(s.trim().trim_matches('"'))
    });
    let loc_exists = location.as_ref().map(|p| p.is_dir()).unwrap_or(false);
    // "Chết" = Settings của Windows không gỡ được nữa: trình gỡ là một exe đã mất (dù thư mục cài
    // còn hay không), hoặc lệnh gỡ không phải exe (winget/powershell) mà thư mục cài đã mất.
    // MSI gỡ qua msiexec với mã sản phẩm nên không bao giờ "chết" theo nghĩa này.
    let dead = if msi {
        false
    } else if let Some(p) = &un_exe {
        !p.is_file()
    } else {
        location.is_some() && !loc_exists
    };
    // Web app cài từ trình duyệt (Edge/Chrome PWA) gỡ qua chính trình duyệt: không nhận thư mục
    // của trình duyệt làm thư mục cài, kẻo mượn luôn dung lượng và lịch sử chạy của nó.
    let browser_pwa = abs_exe(&file)
        .and_then(|p| p.file_stem().map(|s| s.to_string_lossy().to_ascii_lowercase()))
        .map(|s| matches!(s.as_str(), "msedge" | "chrome" | "brave" | "msedge_proxy" | "chrome_proxy" | "vivaldi" | "opera"))
        .unwrap_or(false);
    let install_dir = if browser_pwa {
        None
    } else {
        location
            .clone()
            .or_else(|| un_exe.as_ref().and_then(|p| p.parent().map(Path::to_path_buf)))
            .or_else(|| icon_exe.as_ref().and_then(|p| p.parent().map(Path::to_path_buf)))
            // Thư mục gốc kiểu C:\Windows hay Program Files không phải "thư mục cài" của app nào.
            .filter(|p| !crate::cleaner::is_root_like(p))
    };
    let installed = key.string("InstallDate").and_then(|s| parse_install_date(&s)).or_else(|| key.last_write_epoch()).unwrap_or(0);
    Some(DesktopApp {
        hive,
        wow64,
        key: name.to_string(),
        name: display,
        publisher: key.string("Publisher").unwrap_or_default(),
        version: key.string("DisplayVersion").unwrap_or_default(),
        install_dir,
        installed,
        est_bytes: key.dword("EstimatedSize").map(|kb| kb as u64 * 1024).unwrap_or(0),
        uninstall,
        msi,
        dead,
    })
}

fn read_branch(hive: Hive, wow64: bool, out: &mut Vec<DesktopApp>) {
    let Some(root) = registry::open(hive.handle(), if wow64 { UNINSTALL_WOW } else { UNINSTALL }) else { return };
    for name in root.subkeys() {
        let Some(k) = root.open(&name) else { continue };
        if let Some(app) = read_one(hive, wow64, &k, &name) {
            out.push(app);
        }
    }
}

pub fn list() -> Vec<DesktopApp> {
    let mut out = Vec::new();
    read_branch(Hive::Hklm, false, &mut out);
    read_branch(Hive::Hklm, true, &mut out);
    read_branch(Hive::Hkcu, false, &mut out);
    read_branch(Hive::Hkcu, true, &mut out);
    out
}

/// Tìm lại một mục theo id đã phát cho UI; đọc mới từ registry để phản ánh trạng thái hiện tại.
pub fn find(id: &str) -> Option<DesktopApp> {
    let mut parts = id.splitn(4, ':');
    if parts.next()? != "reg" {
        return None;
    }
    let hive = match parts.next()? {
        "hklm" => Hive::Hklm,
        "hkcu" => Hive::Hkcu,
        _ => return None,
    };
    let wow64 = parts.next()? == "1";
    let name = parts.next()?;
    let root = registry::open(hive.handle(), if wow64 { UNINSTALL_WOW } else { UNINSTALL })?;
    let k = root.open(name)?;
    read_one(hive, wow64, &k, name)
}

pub fn exists(app: &DesktopApp) -> bool {
    registry::open(app.hive.handle(), &app.sub_path()).is_some()
}

/// Chạy trình gỡ của hãng (nó tự xin UAC nếu cần) và chờ nó cùng tiến trình con kết thúc.
/// Mã lỗi ngắn: "no-uninstaller", "uninstaller-missing", "uac-cancelled", "launch-failed: …".
pub fn run_uninstaller(app: &DesktopApp) -> Result<(), String> {
    let cmd = app.uninstall.as_deref().ok_or("no-uninstaller")?;
    let (file, params) = split_command(cmd).ok_or("no-uninstaller")?;
    if let Some(p) = abs_exe(&file) {
        if !p.is_file() {
            return Err("uninstaller-missing".into());
        }
    }
    // Lệnh dạng công cụ (winget/powershell/cmd…) chạy qua cmd để có console cho người dùng xem.
    let (file, params) = if abs_exe(&file).is_some() || is_msi(&file) { (file, params) } else { ("cmd.exe".to_string(), format!("/c \"{cmd}\"")) };
    let ps_quote = |s: &str| s.replace('\'', "''");
    let mut script = format!("$p = Start-Process -FilePath '{}' -PassThru -Wait", ps_quote(&file));
    if !params.is_empty() {
        script = format!("$p = Start-Process -FilePath '{}' -ArgumentList '{}' -PassThru -Wait", ps_quote(&file), ps_quote(&params));
    }
    let out = std::process::Command::new("powershell")
        .args(["-NoProfile", "-NonInteractive", "-Command", &format!("{script}; exit $p.ExitCode")])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
        .map_err(|e| format!("launch-failed: {e}"))?;
    let err = String::from_utf8_lossy(&out.stderr);
    if err.contains("canceled by the user") || err.contains("cancelled by the user") {
        return Err("uac-cancelled".into());
    }
    if err.contains("cannot find the file") || err.contains("cannot find the path") {
        return Err("uninstaller-missing".into());
    }
    if !out.status.success() && !err.trim().is_empty() {
        return Err(format!("launch-failed: {}", err.lines().next().unwrap_or("").trim()));
    }
    Ok(())
}

/// Xoá khoá registry của mục chết. HKLM cần quyền admin (lỗi 5 → "needs-admin").
pub fn remove_entry(app: &DesktopApp) -> Result<(), String> {
    match registry::delete_tree(app.hive.handle(), &app.sub_path()) {
        Ok(()) => Ok(()),
        Err(5) => Err("needs-admin".into()),
        Err(2) => Ok(()),
        Err(code) => Err(format!("registry error {code}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_uninstall_strings() {
        assert_eq!(split_command(r#""C:\Program Files\X\unins000.exe" /SILENT"#), Some((r"C:\Program Files\X\unins000.exe".into(), "/SILENT".into())));
        assert_eq!(split_command(r"C:\Program Files\X\Uninstall.exe --uninstall"), Some((r"C:\Program Files\X\Uninstall.exe".into(), "--uninstall".into())));
        assert_eq!(split_command("MsiExec.exe /X{ABC}"), Some(("MsiExec.exe".into(), "/X{ABC}".into())));
        assert_eq!(split_command("winget uninstall --product-code x"), Some(("winget".into(), "uninstall --product-code x".into())));
        assert_eq!(split_command("   "), None);
        assert!(is_msi("MsiExec.exe"));
        assert!(!is_msi(r"C:\x\setup.exe"));
        assert!(abs_exe("MsiExec.exe").is_none());
        assert!(abs_exe(r"D:\Game\x\uninstall.exe").is_some());
    }

    #[test]
    fn parses_install_dates() {
        assert_eq!(parse_install_date("20260101"), Some(1_767_225_600));
        assert_eq!(parse_install_date("1767631222"), Some(1_767_631_222));
        assert_eq!(parse_install_date("Tue May 12 22:42:48 2026"), None);
        assert_eq!(parse_install_date("20261399"), None);
    }

    #[test]
    fn install_location_is_normalised() {
        assert_eq!(clean_dir(r"D:/Game/Riot Games/Riot Client"), Some(PathBuf::from(r"D:\Game\Riot Games\Riot Client")));
        assert_eq!(clean_dir(r#""C:\Users\x\AppData\Local\Caudex""#), Some(PathBuf::from(r"C:\Users\x\AppData\Local\Caudex")));
        assert_eq!(clean_dir(r"C:\Program Files\7-Zip\"), Some(PathBuf::from(r"C:\Program Files\7-Zip")));
        assert_eq!(clean_dir("  "), None);
        assert_eq!(clean_dir("relative"), None);
    }

    #[test]
    fn lists_something_and_ids_round_trip() {
        let apps = list();
        assert!(!apps.is_empty());
        let a = &apps[0];
        let again = find(&a.id()).expect("find by id");
        assert_eq!(again.name, a.name);
        assert!(exists(a));
    }
}
