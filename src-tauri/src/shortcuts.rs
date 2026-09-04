//! Đọc file .lnk (định dạng MS-SHLLINK) không qua COM: đích chạy (exe) và AppUserModelID
//! trong khối property. Cần vì Windows ghi lần mở của nhiều app (Chrome, VS Code, Discord…)
//! theo AppUserModelID của shortcut chứ không theo đường dẫn exe; shortcut Start Menu / Taskbar
//! là chỗ nối hai thứ đó lại. Chỉ đọc, không bao giờ ghi.

use crate::registry::expand_env;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// fmtid của property AppUserModel (`{9F4C2855-9F79-4B39-A8D0-E1D42DE1D5F3}`) theo thứ tự byte
/// trong file; pid 5 là AppUserModel.ID.
const AUMID_FMTID: [u8; 16] = [0x55, 0x28, 0x4C, 0x9F, 0x79, 0x9F, 0x39, 0x4B, 0xA8, 0xD0, 0xE1, 0xD4, 0x2D, 0xE1, 0xD5, 0xF3];
const PID_AUMID: u32 = 5;
/// Chuỗi UTF-16: đếm ký tự (LPWSTR) hoặc đếm byte (BSTR); trình cài dùng cả hai.
const VT_BSTR: u16 = 0x08;
const VT_LPWSTR: u16 = 0x1F;
const BLOCK_ENVIRONMENT: u32 = 0xA000_0001;
const BLOCK_PROPERTY_STORE: u32 = 0xA000_0009;

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Shortcut {
    /// Đường dẫn đích đã mở rộng biến môi trường (chưa kiểm tra tồn tại).
    pub target: Option<String>,
    /// Thư mục làm việc; shortcut trỏ vào shell (không có đích file) vẫn thường có nó.
    pub working_dir: Option<String>,
    /// File icon; nhiều trình cài trỏ thẳng vào exe của app.
    pub icon: Option<String>,
    pub aumid: Option<String>,
}

fn u16_at(b: &[u8], i: usize) -> Option<u16> {
    Some(u16::from_le_bytes(b.get(i..i + 2)?.try_into().ok()?))
}

fn u32_at(b: &[u8], i: usize) -> Option<u32> {
    Some(u32::from_le_bytes(b.get(i..i + 4)?.try_into().ok()?))
}

/// Chuỗi ANSI kết thúc bằng NUL tại `start`.
fn ansi_nul(b: &[u8], start: usize) -> Option<String> {
    let s = b.get(start..)?;
    let end = s.iter().position(|&c| c == 0).unwrap_or(s.len());
    Some(String::from_utf8_lossy(&s[..end]).into_owned())
}

/// Chuỗi UTF-16 kết thúc bằng NUL tại `start`.
fn wide_nul(b: &[u8], start: usize) -> Option<String> {
    let mut units = Vec::new();
    let mut i = start;
    loop {
        let u = u16_at(b, i)?;
        if u == 0 {
            break;
        }
        units.push(u);
        i += 2;
    }
    Some(String::from_utf16_lossy(&units))
}

/// Chuỗi UTF-16 dài đúng `chars` đơn vị tại `start`.
fn wide_n(b: &[u8], start: usize, chars: usize) -> Option<String> {
    let raw = b.get(start..start + chars * 2)?;
    let units: Vec<u16> = raw.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
    Some(String::from_utf16_lossy(&units))
}

/// LinkInfo: LocalBasePath + CommonPathSuffix (bản Unicode nếu header đủ dài).
fn link_info_path(b: &[u8], start: usize) -> Option<String> {
    let header = u32_at(b, start + 4)? as usize;
    let flags = u32_at(b, start + 8)?;
    if flags & 1 == 0 {
        return None;
    }
    let (base, suffix) = if header >= 0x24 {
        let ub = u32_at(b, start + 28)? as usize;
        let us = u32_at(b, start + 32)? as usize;
        (wide_nul(b, start + ub)?, wide_nul(b, start + us).unwrap_or_default())
    } else {
        let ab = u32_at(b, start + 16)? as usize;
        let asf = u32_at(b, start + 24)? as usize;
        (ansi_nul(b, start + ab)?, ansi_nul(b, start + asf).unwrap_or_default())
    };
    let joined = format!("{base}{suffix}");
    (!joined.is_empty()).then_some(joined)
}

/// AppUserModel.ID trong PropertyStoreDataBlock (MS-PROPSTORE, dạng id số).
fn aumid_from_store(b: &[u8]) -> Option<String> {
    let mut pos = 0;
    loop {
        let size = u32_at(b, pos)? as usize;
        if size == 0 {
            return None;
        }
        if b.get(pos + 8..pos + 24)? == AUMID_FMTID {
            let mut p = pos + 24;
            loop {
                let vs = u32_at(b, p)? as usize;
                if vs == 0 {
                    break;
                }
                if u32_at(b, p + 4)? == PID_AUMID {
                    let n = u32_at(b, p + 13)? as usize;
                    let chars = match u16_at(b, p + 9)? {
                        VT_LPWSTR => n,
                        VT_BSTR => n / 2,
                        _ => return None,
                    };
                    return wide_n(b, p + 17, chars).map(|s| s.trim_end_matches('\0').to_string());
                }
                p += vs;
            }
        }
        pos += size;
    }
}

pub fn parse(b: &[u8]) -> Option<Shortcut> {
    if u32_at(b, 0)? != 0x4C {
        return None;
    }
    let flags = u32_at(b, 0x14)?;
    let unicode = flags & 0x80 != 0;
    let mut pos = 0x4C;
    if flags & 0x01 != 0 {
        pos += 2 + u16_at(b, pos)? as usize;
    }
    let mut target = None;
    if flags & 0x02 != 0 {
        target = link_info_path(b, pos);
        pos += u32_at(b, pos)? as usize;
    }
    // StringData theo thứ tự: tên, đường dẫn tương đối, thư mục làm việc, tham số, icon.
    let mut strings: [Option<String>; 5] = Default::default();
    for (i, bit) in [0x04, 0x08, 0x10, 0x20, 0x40].into_iter().enumerate() {
        if flags & bit != 0 {
            let n = u16_at(b, pos)? as usize;
            strings[i] = if unicode { wide_n(b, pos + 2, n) } else { b.get(pos + 2..pos + 2 + n).map(|s| String::from_utf8_lossy(s).into_owned()) };
            pos += 2 + if unicode { n * 2 } else { n };
        }
    }
    let [_, _, working_dir, _, icon] = strings;
    let mut aumid = None;
    let mut env_target = None;
    while let Some(size) = u32_at(b, pos) {
        let size = size as usize;
        if size < 8 {
            break;
        }
        match u32_at(b, pos + 4)? {
            BLOCK_ENVIRONMENT => {
                env_target = wide_nul(b, pos + 8 + 260).filter(|s| !s.is_empty()).or_else(|| ansi_nul(b, pos + 8)).filter(|s| !s.is_empty());
            }
            BLOCK_PROPERTY_STORE => {
                if let Some(block) = b.get(pos + 8..pos + size) {
                    aumid = aumid_from_store(block);
                }
            }
            _ => {}
        }
        pos += size;
    }
    let clean = |s: Option<String>| s.map(|s| expand_env(s.trim().trim_matches('"')).replace('/', "\\")).filter(|s| !s.is_empty());
    Some(Shortcut { target: clean(target.or(env_target)), working_dir: clean(working_dir), icon: clean(icon), aumid })
}

fn is_exe(p: &str) -> bool {
    let p = Path::new(p);
    p.is_absolute() && p.extension().map(|e| e.eq_ignore_ascii_case("exe")).unwrap_or(false)
}

/// Nơi shortcut "chạy": exe đích, không thì exe làm icon, không thì thư mục làm việc (shortcut
/// vào shell:AppsFolder như của Riot không có đích file) ghi theo `thư mục làm việc\tên.lnk`
/// để vẫn khớp tiền tố thư mục cài. Thư mục gốc hệ thống không tính.
fn launch_path(sc: &Shortcut, lnk: &Path) -> Option<PathBuf> {
    if let Some(t) = sc.target.as_deref().filter(|t| is_exe(t)) {
        return Some(PathBuf::from(t));
    }
    if let Some(i) = sc.icon.as_deref().filter(|i| is_exe(i)) {
        return Some(PathBuf::from(i));
    }
    let wd = Path::new(sc.working_dir.as_deref()?);
    (wd.is_absolute() && !crate::cleaner::is_root_like(wd)).then(|| wd.join(lnk.file_name().unwrap_or_default()))
}

/// Mọi shortcut trong Start Menu (chung + riêng), Taskbar/Quick Launch và Desktop, gom theo
/// AppUserModelID, theo đường dẫn .lnk và theo tên file (đã chuẩn hoá) → exe đích.
#[derive(Debug, Default)]
pub struct ShortcutIndex {
    by_aumid: HashMap<String, PathBuf>,
    by_lnk: HashMap<String, PathBuf>,
    by_name: HashMap<String, PathBuf>,
}

fn walk(dir: &Path, depth: u32, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() && !ft.is_symlink() {
            if depth < 5 {
                walk(&p, depth + 1, out);
            }
        } else if p.extension().map(|x| x.eq_ignore_ascii_case("lnk")).unwrap_or(false) {
            out.push(p);
        }
    }
}

fn shortcut_roots() -> Vec<PathBuf> {
    let env = |k: &str| std::env::var_os(k).map(PathBuf::from);
    let mut v = Vec::new();
    if let Some(pd) = env("ProgramData") {
        v.push(pd.join(r"Microsoft\Windows\Start Menu\Programs"));
    }
    if let Some(ad) = env("APPDATA") {
        v.push(ad.join(r"Microsoft\Windows\Start Menu\Programs"));
        v.push(ad.join(r"Microsoft\Internet Explorer\Quick Launch"));
    }
    if let Some(h) = dirs::home_dir() {
        v.push(h.join("Desktop"));
    }
    if let Some(p) = env("PUBLIC") {
        v.push(p.join("Desktop"));
    }
    v
}

impl ShortcutIndex {
    pub fn build() -> ShortcutIndex {
        let mut files = Vec::new();
        for root in shortcut_roots() {
            walk(&root, 0, &mut files);
        }
        let mut idx = ShortcutIndex::default();
        for f in files {
            let Ok(bytes) = std::fs::read(&f) else { continue };
            let Some(sc) = parse(&bytes) else { continue };
            let Some(target) = launch_path(&sc, &f) else { continue };
            idx.by_lnk.entry(f.to_string_lossy().to_ascii_lowercase()).or_insert_with(|| target.clone());
            if let Some(a) = sc.aumid {
                idx.by_aumid.entry(a.to_ascii_lowercase()).or_insert_with(|| target.clone());
            }
            if let Some(stem) = f.file_stem() {
                let n = crate::leftovers::norm(&stem.to_string_lossy());
                if !n.is_empty() {
                    idx.by_name.entry(n).or_insert(target);
                }
            }
        }
        idx
    }

    /// Đổi một mục UserAssist thành đường dẫn exe: `.lnk` → đích của shortcut; AppUserModelID
    /// của app desktop (không có `\` và không có `!` của app Store) → exe có shortcut mang ID đó.
    pub fn resolve(&self, target: &str) -> Option<PathBuf> {
        let lower = target.to_ascii_lowercase();
        if lower.ends_with(".lnk") {
            return self.by_lnk.get(&lower).cloned();
        }
        if !lower.contains('\\') && !lower.contains('!') {
            return self.by_aumid.get(&lower).cloned();
        }
        None
    }

    /// Thư mục chứa exe của shortcut có tên trùng tên app (đã chuẩn hoá).
    pub fn dir_for_name(&self, name: &str) -> Option<PathBuf> {
        let n = crate::leftovers::norm(name);
        if n.len() < 3 {
            return None;
        }
        self.by_name.get(&n).and_then(|p| p.parent().map(Path::to_path_buf))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Dựng một .lnk tối thiểu: LinkInfo với LocalBasePath ANSI (nếu có đích), StringData tên +
    /// thư mục làm việc, và khối property chứa AppUserModel.ID (LPWSTR hoặc BSTR) nếu có.
    fn build_lnk(target: Option<&str>, working_dir: Option<&str>, aumid: Option<(&str, u16)>) -> Vec<u8> {
        let mut b = vec![0u8; 0x4C];
        b[0..4].copy_from_slice(&0x4Cu32.to_le_bytes());
        let mut flags = 0x80u32 | 0x04;
        if target.is_some() {
            flags |= 0x02;
        }
        if working_dir.is_some() {
            flags |= 0x10;
        }
        b[0x14..0x18].copy_from_slice(&flags.to_le_bytes());
        if let Some(target) = target {
            let header = 0x1Cusize;
            let base_off = header;
            let suffix_off = base_off + target.len() + 1;
            let size = suffix_off + 1;
            let mut li = Vec::new();
            for v in [size as u32, header as u32, 1, 0, base_off as u32, 0, suffix_off as u32] {
                li.extend(v.to_le_bytes());
            }
            li.extend(target.as_bytes());
            li.extend([0, 0]);
            b.extend(li);
        }
        b.extend(1u16.to_le_bytes());
        b.extend(('x' as u16).to_le_bytes());
        if let Some(wd) = working_dir {
            let w: Vec<u16> = wd.encode_utf16().collect();
            b.extend((w.len() as u16).to_le_bytes());
            for u in &w {
                b.extend(u.to_le_bytes());
            }
        }
        if let Some((a, vt)) = aumid {
            let wide: Vec<u16> = a.encode_utf16().chain(std::iter::once(0)).collect();
            let mut prop = Vec::new();
            prop.extend(PID_AUMID.to_le_bytes());
            prop.push(0);
            prop.extend(vt.to_le_bytes());
            prop.extend(0u16.to_le_bytes());
            let n = if vt == VT_BSTR { wide.len() * 2 } else { wide.len() };
            prop.extend((n as u32).to_le_bytes());
            for w in &wide {
                prop.extend(w.to_le_bytes());
            }
            let mut storage = Vec::new();
            storage.extend(((4 + prop.len()) as u32).to_le_bytes());
            storage.extend(prop);
            storage.extend(0u32.to_le_bytes());
            let mut st = Vec::new();
            st.extend(((24 + storage.len()) as u32).to_le_bytes());
            st.extend(0x5350_5331u32.to_le_bytes());
            st.extend(AUMID_FMTID);
            st.extend(storage);
            st.extend(0u32.to_le_bytes());
            b.extend(((8 + st.len()) as u32).to_le_bytes());
            b.extend(BLOCK_PROPERTY_STORE.to_le_bytes());
            b.extend(st);
        }
        b.extend(0u32.to_le_bytes());
        b
    }

    #[test]
    fn parses_target_and_aumid() {
        let sc = parse(&build_lnk(Some(r"C:\Apps\Foo\foo.exe"), None, Some(("com.example.Foo", VT_LPWSTR)))).unwrap();
        assert_eq!(sc.target.as_deref(), Some(r"C:\Apps\Foo\foo.exe"));
        assert_eq!(sc.aumid.as_deref(), Some("com.example.Foo"));
        // Trình cài NSIS/Tauri ghi AUMID dạng BSTR (đếm byte) thay vì LPWSTR (đếm ký tự).
        let bstr = parse(&build_lnk(Some(r"C:\Apps\Foo\foo.exe"), Some(r"C:\Apps\Foo"), Some(("com.caudex.dev", VT_BSTR)))).unwrap();
        assert_eq!(bstr.aumid.as_deref(), Some("com.caudex.dev"));
        assert_eq!(bstr.working_dir.as_deref(), Some(r"C:\Apps\Foo"));
        let plain = parse(&build_lnk(Some(r"D:\x\y.exe"), None, None)).unwrap();
        assert_eq!(plain, Shortcut { target: Some(r"D:\x\y.exe".into()), working_dir: None, icon: None, aumid: None });
        assert!(parse(b"not a shortcut").is_none());
        assert!(parse(&[0x4C, 0, 0, 0]).is_none());
    }

    #[test]
    fn launch_path_falls_back_to_working_dir() {
        let lnk = Path::new(r"C:\Users\x\Start Menu\Riot Games\League.lnk");
        let shell_only = parse(&build_lnk(None, Some(r"D:\Game\Riot Games\Riot Client"), None)).unwrap();
        assert_eq!(shell_only.target, None);
        assert_eq!(launch_path(&shell_only, lnk), Some(PathBuf::from(r"D:\Game\Riot Games\Riot Client\League.lnk")));
        let system_dir = parse(&build_lnk(None, Some(r"C:\Windows\System32"), None)).unwrap();
        assert_eq!(launch_path(&system_dir, lnk), None, "system folders are not an app's install dir");
        let with_exe = parse(&build_lnk(Some(r"D:\x\y.exe"), Some(r"D:\elsewhere"), None)).unwrap();
        assert_eq!(launch_path(&with_exe, lnk), Some(PathBuf::from(r"D:\x\y.exe")));
    }

    #[test]
    fn index_resolves_aumid_lnk_and_name() {
        let mut idx = ShortcutIndex::default();
        idx.by_aumid.insert("com.example.foo".into(), PathBuf::from(r"C:\Apps\Foo\foo.exe"));
        idx.by_lnk.insert(r"c:\users\x\start menu\foo.lnk".into(), PathBuf::from(r"C:\Apps\Foo\foo.exe"));
        idx.by_name.insert("foo".into(), PathBuf::from(r"C:\Apps\Foo\foo.exe"));
        assert_eq!(idx.resolve("com.example.Foo"), Some(PathBuf::from(r"C:\Apps\Foo\foo.exe")));
        assert_eq!(idx.resolve(r"C:\Users\x\Start Menu\Foo.lnk"), Some(PathBuf::from(r"C:\Apps\Foo\foo.exe")));
        assert_eq!(idx.resolve("Microsoft.Paint_8wekyb3d8bbwe!App"), None, "Store AUMIDs stay as they are");
        assert_eq!(idx.resolve(r"C:\Apps\Bar\bar.exe"), None, "exe paths are already resolved");
        assert_eq!(idx.dir_for_name("Foo"), Some(PathBuf::from(r"C:\Apps\Foo")));
        assert_eq!(idx.dir_for_name("Bar"), None);
    }

    /// Start Menu của mọi máy Windows có shortcut trỏ tới exe (ví dụ Windows Media Player Legacy).
    #[test]
    fn this_machine_start_menu_has_exe_shortcuts() {
        let idx = ShortcutIndex::build();
        assert!(!idx.by_lnk.is_empty(), "no exe shortcut found under Start Menu / Desktop");
    }

    /// In mọi shortcut máy này đọc được (đích, AUMID) và các file .lnk không đọc ra exe, để soi
    /// parser bằng mắt: `cargo test print_this_machine_shortcuts -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn print_this_machine_shortcuts() {
        let mut files = Vec::new();
        for root in shortcut_roots() {
            walk(&root, 0, &mut files);
        }
        let (mut ok, mut no_target, mut no_parse) = (0, 0, 0);
        for f in &files {
            let Ok(bytes) = std::fs::read(f) else { continue };
            match parse(&bytes) {
                None => {
                    no_parse += 1;
                    println!("  NOPARSE {}", f.display());
                }
                Some(sc) if sc.target.as_deref().map(is_exe).unwrap_or(false) => {
                    ok += 1;
                    println!("  OK {} -> {} | aumid={:?}", f.file_name().unwrap().to_string_lossy(), sc.target.unwrap(), sc.aumid);
                }
                Some(sc) => {
                    no_target += 1;
                    println!("  NOEXE {} -> {:?} | aumid={:?}", f.file_name().unwrap().to_string_lossy(), sc.target, sc.aumid);
                }
            }
        }
        println!("lnk={} exe={} no-exe={} no-parse={}", files.len(), ok, no_target, no_parse);
        let idx = ShortcutIndex::build();
        println!("by_aumid={} by_name={} zoom={:?}", idx.by_aumid.len(), idx.by_name.len(), idx.resolve("zoom.us.Zoom Video Meetings"));
    }
}
