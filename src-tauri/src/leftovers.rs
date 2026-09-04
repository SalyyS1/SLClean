//! Thư mục thừa của app đã gỡ: dữ liệu còn nằm trong AppData (Local, Roaming, LocalLow),
//! ProgramData, Program Files, Local\Programs và Local\Packages sau khi app không còn đăng ký.
//! Một thư mục bị coi là "mồ côi" khi không app nào đang cài trỏ vào nó, không có exe đang
//! chạy từ đó, và không app nào đang cài có tên/hãng khớp với tên thư mục. Tên thư mục hệ
//! thống và tool dev quen thuộc không bao giờ được nêu.

use crate::apps::Usage;
use crate::catalog::{lower, Roots, Text};
use crate::installed_apps::DesktopApp;
use crate::store_apps::StoreApp;
use serde::Serialize;
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

#[derive(Clone, Debug, Serialize)]
pub struct Leftover {
    pub id: String,
    pub path: PathBuf,
    /// "appdata" | "programdata" | "programs" | "packages"
    pub area: String,
    pub note: Text,
    /// Lần ghi cuối của thư mục (giây epoch).
    pub modified: u64,
    /// Thư mục có exe bên trong: có thể là app portable, xem kỹ hơn.
    pub has_exe: bool,
    /// Lần Windows thấy một exe trong thư mục này được mở (giây epoch); 0 = chưa từng.
    pub last_used: u64,
    /// Chỉ xoá được khi chạy với quyền admin (Program Files, ProgramData).
    pub needs_admin: bool,
}

/// Tên (chữ thường) không bao giờ được nêu dù không app nào nhận: thư mục Windows/shell,
/// kho công cụ dev, và mọi thứ đã có mục riêng trong danh mục cache.
const NEVER: &[&str] = &[
    "apps", "deployment", "backup", "sentry", "soundresearch", "next-swc", "prisma", "turborepo", "clink", "fastmcp", "pip-audit",
    "update-informer-rs", "cache", "caches", "logs", "log", "crashpad", "crashreports", "squirrel", "identitycache", "vn.salyyy.slclean",
    "microsoft", "windows", "packages", "programs", "temp", "tmp", "cache", "crashdumps", "d3dscache", "comms",
    "connecteddevicesplatform", "publishers", "peernetworking", "virtualstore", "history", "temporary internet files",
    "application data", "isolatedstorage", "identitycache", "placeholdertilelogofolder", "package cache", "packagemanagement",
    "downloaded installations", "squirreltemp", "sun", "oracle", "apple", "apple computer", "apple inc", "intel", "nvidia", "amd",
    "realtek", "hp", "dell", "lenovo", "asus", "acer", "google", "mozilla", "adobe", "common", "common files", "desktop",
    "documents", "start menu", "templates", "ssh", "regid.1991-06.com.microsoft", "softwaredistribution", "usoprivate", "usoshared",
    "microsoft onedrive", "microsoft devdiv", "microsoft help", "microsoft visual studio", "microsoft sql server", "windowsapps",
    "modifiablewindowsapps", "windows defender", "windows nt", "windows mail", "windows media player", "windows photo viewer",
    "windows sidebar", "windows security", "windowspowershell", "internet explorer", "reference assemblies", "msbuild", "dotnet",
    "uninstall information", "installshield installation information", "boost_interprocess", "elevateddiagnostics", "speech",
    "speech_onecore", "fontconfig", "cef", "electron", "electron-builder", "node-gyp", "npm", "npm-cache", "pnpm", "pnpm-cache",
    "pnpm-state", "yarn", "pip", "uv", "go", "go-build", "gopls", "goimports", "cargo", "rustup", "gradle", "maven", "kotlin",
    "jetbrains", "code", "cursor", "windsurf", "nuget", "composer", "deno", "bun", "python", "docker", "wsl", "tauri",
    "ms-playwright", "ms-playwright-go", "puppeteer", "cypress", "unity", "unrealengine", "epicgameslauncher", "steam",
    "riot games", "riot vanguard", "modrinth", "modrinthapp", "prismlauncher", ".minecraft", "minecraftinstaller", "discord",
    "discordptb", "discordcanary", "claude", "anthropicclaude", "openai", "orca", "codex", "gemini", "huggingface", "ollama",
    "lm-studio", "obsidian", "notion", "slack", "figma", "postman", "spotify", "telegram desktop", "zoom", "teamviewer",
    "overwolf", "ow-electron", "7-zip", "winrar", "git", "github cli", "github", "vlc", "obs-studio", "obs-studio-hook",
    "onedrive", "onedrivetemp", "tailscale", "cloudflared", "recent", "sendto", "printhood", "nethood", "cookies", "local settings",
];

fn name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

/// Chuẩn hoá tên để so app ↔ thư mục: chữ thường, bỏ khoảng trắng/dấu, bỏ hậu tố phiên bản.
pub fn norm(s: &str) -> String {
    let mut out = String::new();
    for c in s.chars() {
        if c.is_alphanumeric() {
            out.extend(c.to_lowercase());
        }
    }
    out
}

/// Từ khoá nhận diện từ tên app/hãng: tên đầy đủ đã chuẩn hoá và từ đầu tiên đủ dài.
fn keywords(s: &str) -> Vec<String> {
    let mut v = Vec::new();
    let full = norm(s);
    if full.len() >= 3 {
        v.push(full);
    }
    let first: String = s.split(|c: char| c == ' ' || c == '-' || c == '_' || c == '.').next().unwrap_or("").to_string();
    let first = norm(&first);
    if first.len() >= 4 && !matches!(first.as_str(), "microsoft" | "google" | "windows" | "the" | "apple" | "adobe") {
        v.push(first);
    }
    v
}

/// Mọi dấu hiệu của app đang cài: thư mục cài (chữ thường) và từ khoá tên/hãng.
pub struct Owners {
    dirs: Vec<String>,
    words: HashSet<String>,
    families: HashSet<String>,
}

impl Owners {
    pub fn build(desktop: &[DesktopApp], store: &[StoreApp], families: HashSet<String>, usage: &Usage) -> Owners {
        let mut dirs = Vec::new();
        let mut words = HashSet::new();
        // App portable / không đăng ký nhưng vẫn được mở trong 6 tháng qua: coi như đang dùng.
        let since = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0).saturating_sub(180 * 86_400);
        for stem in usage.exe_stems(since) {
            words.extend(keywords(&stem));
        }
        for d in desktop {
            if d.dead {
                continue;
            }
            if let Some(p) = &d.install_dir {
                dirs.push(lower(p).trim_end_matches('\\').to_string());
            }
            words.extend(keywords(&d.name));
            words.extend(keywords(&d.publisher));
        }
        for s in store {
            dirs.push(lower(&s.root).trim_end_matches('\\').to_string());
            words.extend(keywords(&s.display));
            words.extend(keywords(&s.publisher));
            words.extend(keywords(s.family.split('_').next().unwrap_or("")));
            for part in s.family.split('_').next().unwrap_or("").split('.') {
                words.extend(keywords(part));
            }
        }
        Owners { dirs, words, families }
    }

    fn owns_dir(&self, dir: &Path) -> bool {
        let d = lower(dir).trim_end_matches('\\').to_string();
        self.dirs.iter().any(|o| *o == d || o.starts_with(&format!("{d}\\")) || d.starts_with(&format!("{o}\\")))
    }

    fn name_matches(&self, dir_name: &str) -> bool {
        let n = norm(dir_name);
        if n.is_empty() {
            return true;
        }
        // "com.caudex.dev" → caudex; "9router" → 9router; "vn.salyyy.slclean" → slclean.
        let mut cands: Vec<String> = vec![n.clone()];
        for part in dir_name.split(|c: char| c == '.' || c == '-' || c == '_' || c == ' ') {
            let p = norm(part);
            if p.len() >= 4 {
                cands.push(p);
            }
        }
        cands.iter().any(|c| self.words.contains(c) || self.words.iter().any(|w| w.len() >= 5 && (c.starts_with(w) || w.starts_with(c.as_str()))))
    }
}

fn has_exe(dir: &Path) -> bool {
    std::fs::read_dir(dir)
        .map(|it| it.flatten().any(|e| e.path().extension().map(|x| x.eq_ignore_ascii_case("exe")).unwrap_or(false)))
        .unwrap_or(false)
}

fn modified_epoch(p: &Path) -> u64 {
    std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
}

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|it| it.flatten().filter(|e| e.file_type().map(|t| t.is_dir() && !t.is_symlink()).unwrap_or(false)).map(|e| e.path()).collect())
        .unwrap_or_default();
    v.sort();
    v
}

fn note_for(area: &str) -> Text {
    match area {
        "programs" => Text::new(
            "Thư mục cài trong Program Files / Local\\Programs mà không app nào còn đăng ký. Thường là app đã gỡ nhưng để sót.",
            "Install folder under Program Files / Local\\Programs with no registered app. Usually left behind by an uninstall.",
        ),
        "packages" => Text::new(
            "Dữ liệu của app Store đã gỡ (Local\\Packages). Cài lại app sẽ tạo thư mục mới.",
            "Data of an uninstalled Store app (Local\\Packages). Reinstalling creates a fresh folder.",
        ),
        "programdata" => Text::new(
            "Dữ liệu dùng chung của app không còn cài. Xem qua nếu có license/key bên trong.",
            "Shared data of an app that is no longer installed. Check for licenses or keys inside.",
        ),
        _ => Text::new(
            "Cài đặt và dữ liệu của app không còn cài. Cài lại app sẽ mất cấu hình cũ nếu xoá.",
            "Settings and data of an app that is no longer installed. Reinstalling won't recover old settings if deleted.",
        ),
    }
}

/// Thư mục con của `base` không thuộc app nào; `area` gắn nhãn và ghi chú.
/// `needs_admin`: vùng chỉ ghi được khi elevated (Program Files, ProgramData).
fn scan_area(base: &Path, area: &str, owners: &Owners, usage: &Usage, never: &[&str], needs_admin: bool, out: &mut Vec<Leftover>) {
    for dir in subdirs(base) {
        let dn = name_of(&dir);
        let dl = dn.to_ascii_lowercase();
        if dl.starts_with('.') || dl.starts_with('$') || never.contains(&dl.as_str()) || NEVER.contains(&dl.as_str()) {
            continue;
        }
        if area == "packages" {
            if owners.families.contains(&dl) {
                continue;
            }
            // Thư mục sandbox của trình duyệt / "Microsoft.*" hệ thống không phải app người dùng gỡ.
            if dl.starts_with("cr.sb.") || dl.starts_with("fx.sb.") || dl.starts_with("windows_") || dl.starts_with("microsoft.") || dl.starts_with("microsoftwindows.") || dl.starts_with("windows.") || dl.starts_with("microsoftcorporationii.") || dl.starts_with("activesync") {
                continue;
            }
        } else if owners.owns_dir(&dir) || owners.name_matches(&dn) {
            continue;
        }
        if usage.is_running_under(&dir) {
            continue;
        }
        let (last_used, _) = usage.under_dir(&dir);
        out.push(Leftover { id: format!("left:{}", lower(&dir)), path: dir.clone(), area: area.into(), note: note_for(area), modified: modified_epoch(&dir), has_exe: has_exe(&dir), last_used, needs_admin });
    }
}

pub fn scan(r: &Roots, owners: &Owners, usage: &Usage) -> Vec<Leftover> {
    let mut out = Vec::new();
    let local_low = r.home.join("AppData/LocalLow");
    for base in [&r.local, &r.roaming, &local_low] {
        scan_area(base, "appdata", owners, usage, &[], false, &mut out);
    }
    scan_area(&r.program_data, "programdata", owners, usage, &[], true, &mut out);
    scan_area(&r.local.join("Programs"), "programs", owners, usage, &[], false, &mut out);
    let mut pf: Vec<PathBuf> = ["ProgramFiles", "ProgramFiles(x86)"].iter().filter_map(|k| std::env::var_os(k).map(PathBuf::from)).collect();
    pf.dedup();
    for base in pf {
        scan_area(&base, "programs", owners, usage, crate::cleaner::PROGRAM_FILES_KEEP, true, &mut out);
    }
    scan_area(&r.local.join("Packages"), "packages", owners, usage, &[], false, &mut out);
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::installed_apps::Hive;

    fn owners() -> Owners {
        let d = DesktopApp {
            hive: Hive::Hkcu, wow64: false, key: "k".into(), name: "Blockbench 4.12".into(), publisher: "JannisX11".into(), version: "".into(),
            install_dir: Some(PathBuf::from(r"C:\Users\x\AppData\Local\Programs\Blockbench")), installed: 0, est_bytes: 0, uninstall: None, msi: false, dead: false,
        };
        let dead = DesktopApp { name: "Nexo Maker".into(), dead: true, install_dir: Some(PathBuf::from(r"C:\Users\x\AppData\Local\Programs\NexoMaker")), ..d.clone() };
        let s = StoreApp { full_name: "Microsoft.Paint_1_x64__8wekyb3d8bbwe".into(), family: "Microsoft.Paint_8wekyb3d8bbwe".into(), display: "Paint".into(), publisher: "Microsoft Corporation".into(), version: "1".into(), root: PathBuf::from(r"C:\Program Files\WindowsApps\Microsoft.Paint_1_x64__8wekyb3d8bbwe") };
        Owners::build(&[d, dead], &[s], HashSet::from(["microsoft.paint_8wekyb3d8bbwe".to_string()]), &Usage::empty())
    }

    #[test]
    fn folder_of_installed_app_is_owned_by_dir_or_name() {
        let o = owners();
        assert!(o.owns_dir(Path::new(r"C:\Users\x\AppData\Local\Programs\Blockbench")));
        assert!(o.owns_dir(Path::new(r"C:\Users\x\AppData\Local\Programs\Blockbench\resources")));
        assert!(o.name_matches("blockbench-updater"));
        assert!(o.name_matches("Blockbench"));
        assert!(o.name_matches("JannisX11"));
        assert!(!o.name_matches("NexoMaker"), "dead apps must not own folders");
        assert!(!o.name_matches("Sideloadly"));
        assert!(o.families.contains("microsoft.paint_8wekyb3d8bbwe"));
    }

    /// In danh sách thư mục thừa máy này nhìn thấy, để soi bằng mắt khi chỉnh heuristics:
    /// `cargo test print_this_machine_leftovers -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn print_this_machine_leftovers() {
        let r = Roots::detect().unwrap();
        let usage = Usage::collect();
        let desktop = crate::installed_apps::list();
        let store = crate::store_apps::list();
        let owners = Owners::build(&desktop, &store, crate::store_apps::installed_families(), &usage);
        let list = scan(&r, &owners, &usage);
        println!("leftovers={}", list.len());
        for l in &list {
            println!("  [{}] {}{}", l.area, l.path.display(), if l.has_exe { "  (exe)" } else { "" });
        }
    }

    #[test]
    fn norm_and_keywords() {
        assert_eq!(norm("Discord PTB"), "discordptb");
        assert_eq!(keywords("Visual Studio Code"), vec!["visualstudiocode".to_string(), "visual".to_string()]);
        assert_eq!(keywords("Microsoft Edge"), vec!["microsoftedge".to_string()]);
    }
}
