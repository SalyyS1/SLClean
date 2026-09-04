//! Mục danh mục sinh ra theo cấu trúc thật của máy, không phải bảng cố định: từng profile
//! trình duyệt (Chromium và Firefox), mọi app Electron có thư mục cache, họ VS Code, sản phẩm
//! JetBrains, thư viện Steam, app Microsoft Store, và thư mục tạm ở gốc ổ đĩa / thư mục home.

use crate::catalog::{lower, make_entry, CatalogEntry, Roots, Safety, Text};
use std::collections::HashMap;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};

pub fn entries(r: &Roots) -> Vec<CatalogEntry> {
    let mut out = Vec::new();
    chromium_browsers(r, &mut out);
    gecko_browsers(r, &mut out);
    electron_apps(r, &mut out);
    jetbrains(r, &mut out);
    steam(r, &mut out);
    store_apps(r, &mut out);
    root_temp_dirs(r, &mut out);
    out
}

fn slug(s: &str) -> String {
    let raw: String = s.chars().map(|c| if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '-' }).collect();
    raw.split('-').filter(|p| !p.is_empty()).collect::<Vec<_>>().join("-")
}

fn existing(base: &Path, subs: &[&str]) -> Vec<PathBuf> {
    subs.iter().map(|s| base.join(s)).filter(|p| p.is_dir()).collect()
}

fn subdirs(dir: &Path) -> Vec<PathBuf> {
    let mut v: Vec<PathBuf> = std::fs::read_dir(dir)
        .map(|it| it.flatten().filter(|e| e.file_type().map(|t| t.is_dir() && !t.is_symlink()).unwrap_or(false)).map(|e| e.path()).collect())
        .unwrap_or_default();
    v.sort();
    v
}

fn name_of(p: &Path) -> String {
    p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default()
}

// ---------- Trình duyệt nhân Chromium ----------

/// `single`: thư mục gốc chính là profile (Opera). Ngược lại gốc là "User Data" chứa nhiều profile.
struct Chromium {
    root: &'static str,
    name: &'static str,
    single: bool,
}

const CHROMIUM: &[Chromium] = &[
    Chromium { root: "Google/Chrome/User Data", name: "Chrome", single: false },
    Chromium { root: "Google/Chrome Beta/User Data", name: "Chrome Beta", single: false },
    Chromium { root: "Google/Chrome SxS/User Data", name: "Chrome Canary", single: false },
    Chromium { root: "Microsoft/Edge/User Data", name: "Edge", single: false },
    Chromium { root: "Microsoft/Edge Beta/User Data", name: "Edge Beta", single: false },
    Chromium { root: "BraveSoftware/Brave-Browser/User Data", name: "Brave", single: false },
    Chromium { root: "Vivaldi/User Data", name: "Vivaldi", single: false },
    Chromium { root: "Chromium/User Data", name: "Chromium", single: false },
    Chromium { root: "Arc/User Data", name: "Arc", single: false },
    Chromium { root: "CocCoc/Browser/User Data", name: "Cốc Cốc", single: false },
    Chromium { root: "Opera Software/Opera Stable", name: "Opera", single: true },
    Chromium { root: "Opera Software/Opera GX Stable", name: "Opera GX", single: true },
];

const PROFILE_CACHE: &[&str] = &[
    "Cache", "Code Cache", "GPUCache", "DawnGraphiteCache", "DawnWebGPUCache",
    "Service Worker/CacheStorage", "Service Worker/ScriptCache", "ShaderCache", "GrShaderCache",
];
const ROOT_CACHE: &[&str] = &["ShaderCache", "GrShaderCache", "GraphiteDawnCache"];

/// Tên hiển thị từng profile từ `Local State` (profile.info_cache.<dir>.name).
fn profile_names(root: &Path) -> HashMap<String, String> {
    let Ok(text) = std::fs::read_to_string(root.join("Local State")) else { return HashMap::new() };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&text) else { return HashMap::new() };
    v["profile"]["info_cache"]
        .as_object()
        .map(|m| m.iter().filter_map(|(k, v)| v["name"].as_str().map(|n| (k.clone(), n.to_string()))).collect())
        .unwrap_or_default()
}

fn is_profile_dir(name: &str) -> bool {
    name == "Default" || name.starts_with("Profile ") || name == "Guest Profile"
}

fn chromium_browsers(r: &Roots, out: &mut Vec<CatalogEntry>) {
    for b in CHROMIUM {
        let root = r.local.join(b.root);
        if !root.is_dir() {
            continue;
        }
        let note = Text::new(
            "Cache web, JS đã biên dịch, GPU và service worker của profile này. Đăng nhập, mật khẩu, bookmark không bị đụng.",
            "Web, compiled-JS, GPU and service-worker caches of this profile. Logins, passwords and bookmarks are untouched.",
        );
        if b.single {
            let paths = existing(&root, PROFILE_CACHE);
            if let Some(e) = make_entry(r, format!("{}-cache", slug(b.name)), "browser", Text::same(format!("{} — cache", b.name)), note, root.clone(), paths, Safety::Safe, true) {
                out.push(e);
            }
            continue;
        }
        let names = profile_names(&root);
        for dir in subdirs(&root) {
            let dn = name_of(&dir);
            if !is_profile_dir(&dn) {
                continue;
            }
            let shown = names.get(&dn).filter(|n| !n.is_empty()).cloned().unwrap_or_else(|| dn.clone());
            let paths = existing(&dir, PROFILE_CACHE);
            let label = if shown == dn { format!("{} — {} — cache", b.name, dn) } else { format!("{} — {} ({}) — cache", b.name, shown, dn) };
            if let Some(e) = make_entry(r, format!("{}-{}", slug(b.name), slug(&dn)), "browser", Text::same(label), note.clone(), dir.clone(), paths, Safety::Safe, true) {
                out.push(e);
            }
        }
        let paths = existing(&root, ROOT_CACHE);
        if let Some(e) = make_entry(
            r,
            format!("{}-shader", slug(b.name)),
            "browser",
            Text::new(format!("{} — shader cache", b.name), format!("{} — shader cache", b.name)),
            Text::new("Cache shader GPU dùng chung các profile; trình duyệt tự tạo lại.", "GPU shader cache shared by all profiles; the browser rebuilds it."),
            root.clone(),
            paths,
            Safety::Safe,
            true,
        ) {
            out.push(e);
        }
    }
}

// ---------- Trình duyệt nhân Gecko (Firefox, Zen, Floorp…) ----------

const GECKO: &[(&str, &str)] = &[
    ("Mozilla/Firefox/Profiles", "Firefox"),
    ("zen/Profiles", "Zen Browser"),
    ("Floorp/Profiles", "Floorp"),
    ("LibreWolf/Profiles", "LibreWolf"),
    ("Waterfox/Profiles", "Waterfox"),
];
const GECKO_CACHE: &[&str] = &["cache2", "startupCache", "jumpListCache", "thumbnails", "OfflineCache"];

fn gecko_browsers(r: &Roots, out: &mut Vec<CatalogEntry>) {
    for (rel, name) in GECKO {
        let root = r.local.join(rel);
        for dir in subdirs(&root) {
            let dn = name_of(&dir);
            // "kbo3d5q4.Default (release)" → "Default (release)"
            let shown = dn.split_once('.').map(|(_, s)| s.to_string()).unwrap_or_else(|| dn.clone());
            let paths = existing(&dir, GECKO_CACHE);
            if let Some(e) = make_entry(
                r,
                format!("{}-{}", slug(name), slug(&dn)),
                "browser",
                Text::same(format!("{name} — {shown} — cache")),
                Text::new(
                    "Cache web, startupCache, thumbnails (nhánh Local). Profile, bookmark, mật khẩu nằm ở Roaming, không bị đụng.",
                    "Web cache, startupCache and thumbnails (Local branch). Profile, bookmarks and passwords live in Roaming and are untouched.",
                ),
                dir.clone(),
                paths,
                Safety::Safe,
                true,
            ) {
                out.push(e);
            }
        }
    }
}

// ---------- App Electron / họ VS Code ----------

const ELECTRON_CACHE: &[&str] = &["Cache", "Code Cache", "GPUCache", "DawnGraphiteCache", "DawnWebGPUCache"];
const VSCODE_CACHE: &[&str] = &["Cache", "CachedData", "Code Cache", "GPUCache", "DawnGraphiteCache", "DawnWebGPUCache"];

/// Tên đẹp cho thư mục app đã biết; app lạ hiện đúng tên thư mục.
fn app_label(dir_name: &str) -> String {
    match dir_name.to_ascii_lowercase().as_str() {
        "code" => "VS Code".into(),
        "code - insiders" => "VS Code Insiders".into(),
        "discord" => "Discord".into(),
        "discordptb" => "Discord PTB".into(),
        "discordcanary" => "Discord Canary".into(),
        "claude" => "Claude Desktop".into(),
        "orca" => "Orca (Codex desktop)".into(),
        "slack" => "Slack".into(),
        "figma" => "Figma".into(),
        "notion" => "Notion".into(),
        "obsidian" => "Obsidian".into(),
        "postman" => "Postman".into(),
        "blockbench" => "Blockbench".into(),
        "termius" => "Termius".into(),
        "pgadmin4" => "pgAdmin 4".into(),
        "riot client" => "Riot Client".into(),
        "riot-client-ux" => "Riot Client UX".into(),
        "microsoft teams" => "Microsoft Teams".into(),
        "whatsapp" => "WhatsApp".into(),
        "signal" => "Signal".into(),
        "zalopc" => "Zalo".into(),
        _ => dir_name.to_string(),
    }
}

fn electron_apps(r: &Roots, out: &mut Vec<CatalogEntry>) {
    let skip_local = ["Programs", "Packages", "Temp", "Microsoft", "Google"];
    let mut dirs = subdirs(&r.roaming);
    dirs.extend(subdirs(&r.local).into_iter().filter(|d| !skip_local.iter().any(|s| name_of(d).eq_ignore_ascii_case(s))));
    for dir in dirs {
        let dn = name_of(&dir);
        let label = app_label(&dn);
        let id = format!("app-{}", slug(&dn));
        if dir.join("User/workspaceStorage").is_dir() {
            vscode_family(r, out, &dir, &label, &id);
            continue;
        }
        if !(dir.join("Cache").is_dir() && dir.join("Code Cache").is_dir()) {
            continue;
        }
        let paths = existing(&dir, ELECTRON_CACHE);
        if let Some(e) = make_entry(
            r,
            id,
            "app",
            Text::same(format!("{label} — cache")),
            Text::new("Cache web/JS/GPU của app (nền Electron). App tự tạo lại; đăng nhập và dữ liệu không bị đụng.", "The app's web/JS/GPU caches (Electron-based). Recreated by the app; logins and data are untouched."),
            dir.clone(),
            paths,
            Safety::Safe,
            true,
        ) {
            out.push(e);
        }
    }
}

fn vscode_family(r: &Roots, out: &mut Vec<CatalogEntry>, dir: &Path, label: &str, id: &str) {
    let mut push = |suffix: &str, group_label: (String, String), note: (&str, &str), rep: PathBuf, paths: Vec<PathBuf>, safety: Safety| {
        if let Some(e) = make_entry(r, format!("{id}-{suffix}"), "editor", Text::new(group_label.0, group_label.1), Text::new(note.0, note.1), rep, paths, safety, true) {
            out.push(e);
        }
    };
    push("cache", (format!("{label} — cache"), format!("{label} — cache")), ("Cache trình duyệt nhúng và mã V8 đã biên dịch.", "Embedded-browser cache and compiled V8 code."), dir.to_path_buf(), existing(dir, VSCODE_CACHE), Safety::Safe);
    push("vsix", (format!("{label} — extension đã tải"), format!("{label} — downloaded extensions")), ("Gói extension đã tải để cài; cài xong không cần nữa.", "Extension packages downloaded for install; not needed afterwards."), dir.join("CachedExtensionVSIXs"), vec![], Safety::Safe);
    push("logs", (format!("{label} — logs"), format!("{label} — logs")), ("Log các phiên.", "Session logs."), dir.join("logs"), vec![], Safety::Safe);
    push("workspace", (format!("{label} — workspaceStorage"), format!("{label} — workspaceStorage")), ("Trạng thái từng workspace (chat AI, undo, layout). Xoá thì mất lịch sử chat Copilot/AI trong editor.", "Per-workspace state (AI chat, undo, layout). Deleting loses Copilot/AI chat history in the editor."), dir.join("User/workspaceStorage"), vec![], Safety::Review);
    push("history", (format!("{label} — lịch sử file cục bộ"), format!("{label} — local file history")), ("Bản sao mỗi lần lưu file (Timeline). Xoá thì không xem lại phiên bản cũ được.", "Copies of every file save (Timeline). Deleting removes old-version browsing."), dir.join("User/History"), vec![], Safety::Review);
}

// ---------- JetBrains ----------

fn jetbrains(r: &Roots, out: &mut Vec<CatalogEntry>) {
    for dir in subdirs(&r.local.join("JetBrains")) {
        let paths = existing(&dir, &["caches", "index", "log", "tmp"]);
        if !dir.join("caches").is_dir() && !dir.join("index").is_dir() {
            continue;
        }
        let dn = name_of(&dir);
        if let Some(e) = make_entry(
            r,
            format!("jb-{}", slug(&dn)),
            "editor",
            Text::new(format!("JetBrains {dn} — caches/index"), format!("JetBrains {dn} — caches/index")),
            Text::new("Index và cache của IDE; mở lại dự án sẽ index lại. Cài đặt nằm ở Roaming, không bị đụng.", "IDE index and caches; reopening a project re-indexes. Settings live in Roaming and are untouched."),
            dir.clone(),
            paths,
            Safety::Rebuild,
            true,
        ) {
            out.push(e);
        }
    }
}

// ---------- Steam ----------

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn steam_install_dir(r: &Roots) -> Option<PathBuf> {
    if let Ok(out) = std::process::Command::new("reg")
        .args(["query", r"HKCU\Software\Valve\Steam", "/v", "SteamPath"])
        .creation_flags(CREATE_NO_WINDOW)
        .output()
    {
        for line in String::from_utf8_lossy(&out.stdout).lines() {
            if let Some(i) = line.find("REG_SZ") {
                let p = PathBuf::from(line[i + 6..].trim());
                if p.join("steamapps").is_dir() {
                    return Some(p);
                }
            }
        }
    }
    let mut cands = Vec::new();
    if let Some(pf) = std::env::var_os("ProgramFiles(x86)") {
        cands.push(PathBuf::from(pf).join("Steam"));
    }
    for d in &r.drives {
        for s in ["Steam", "Game\\Steam", "Games\\Steam", "SteamLibrary"] {
            cands.push(d.join(s));
        }
    }
    cands.into_iter().find(|p| p.join("steamapps").is_dir())
}

/// Các thư viện game từ steamapps/libraryfolders.vdf (dòng `"path"  "D:\\Games"`), cộng thư mục cài.
fn steam_libraries(install: &Path) -> Vec<PathBuf> {
    let mut libs = vec![install.to_path_buf()];
    if let Ok(text) = std::fs::read_to_string(install.join("steamapps/libraryfolders.vdf")) {
        for line in text.lines() {
            let t = line.trim();
            if !t.starts_with("\"path\"") {
                continue;
            }
            let parts: Vec<&str> = t.split('"').collect();
            if let Some(raw) = parts.get(3) {
                let p = PathBuf::from(raw.replace("\\\\", "\\"));
                if p.is_dir() && !libs.iter().any(|l| lower(l) == lower(&p)) {
                    libs.push(p);
                }
            }
        }
    }
    libs
}

fn steam(r: &Roots, out: &mut Vec<CatalogEntry>) {
    let Some(install) = steam_install_dir(r) else { return };
    let libs = steam_libraries(&install);
    let shader: Vec<PathBuf> = libs.iter().map(|l| l.join("steamapps/shadercache")).filter(|p| p.is_dir()).collect();
    if let Some(e) = make_entry(r, "steam-shadercache", "game", Text::same("Steam — shader cache"), Text::new("Shader đã biên dịch của từng game; game tự tạo lại lúc chạy (lần đầu có thể giật nhẹ).", "Per-game compiled shaders; games rebuild them on launch (the first run may stutter briefly)."), install.join("steamapps/shadercache"), shader, Safety::Safe, true) {
        out.push(e);
    }
    let dl: Vec<PathBuf> = libs.iter().flat_map(|l| [l.join("steamapps/downloading"), l.join("steamapps/temp")]).filter(|p| p.is_dir()).collect();
    if let Some(e) = make_entry(r, "steam-downloading", "game", Text::new("Steam — tải dở / temp", "Steam — partial downloads / temp"), Text::new("Bản tải dở của game đã huỷ hoặc gián đoạn. Đừng dọn khi Steam đang tải.", "Leftovers from cancelled or interrupted game downloads. Don't clean while Steam is downloading."), install.join("steamapps/downloading"), dl, Safety::Safe, true) {
        out.push(e);
    }
    let web = existing(&install, &["appcache/httpcache", "logs", "dumps"]);
    if let Some(e) = make_entry(r, "steam-web", "game", Text::new("Steam — web cache, logs, dumps", "Steam — web cache, logs, dumps"), Text::new("Cache cửa hàng, log và crash dump của Steam.", "Steam store cache, logs and crash dumps."), install.join("appcache/httpcache"), web, Safety::Safe, true) {
        out.push(e);
    }
}

// ---------- App Microsoft Store (Local\Packages) ----------

fn store_apps(r: &Roots, out: &mut Vec<CatalogEntry>) {
    let packages = r.local.join("Packages");
    let paths: Vec<PathBuf> = subdirs(&packages)
        .into_iter()
        .flat_map(|p| [p.join("TempState"), p.join("AC/INetCache"), p.join("AC/Temp")])
        .filter(|p| p.is_dir())
        .collect();
    if let Some(e) = make_entry(r, "store-apps-temp", "app", Text::new("App Microsoft Store — temp & INetCache", "Microsoft Store apps — temp & INetCache"), Text::new("TempState và cache web của mọi app Store (Teams, WhatsApp, Terminal…). Dữ liệu app trong LocalState không bị đụng.", "TempState and web caches of every Store app (Teams, WhatsApp, Terminal…). App data in LocalState is untouched."), packages, paths, Safety::Safe, true) {
        out.push(e);
    }
}

// ---------- Thư mục tạm ở gốc ổ đĩa và thư mục home ----------

enum RootKind {
    Safe,
    Review,
    Leftover,
}

fn root_kind(name: &str) -> Option<RootKind> {
    let n = name.to_ascii_lowercase();
    if n == "$recycle.bin" || n == "system volume information" {
        return None;
    }
    if matches!(n.as_str(), "onedrivetemp" | "wudownloadcache" | "deliveryoptimization") {
        return Some(RootKind::Safe);
    }
    if matches!(n.as_str(), "$winreagent" | "$windows.~bt" | "$windows.~ws" | "$getcurrent" | "$sysreset" | "windows.old") {
        return Some(RootKind::Leftover);
    }
    if n == "temp" || n == "tmp" || n.starts_with("tmp-") || n.starts_with("temp-") || n.ends_with("temp") || n.ends_with("-tmp") || n.ends_with("_tmp") {
        return Some(RootKind::Review);
    }
    None
}

fn root_temp_dirs(r: &Roots, out: &mut Vec<CatalogEntry>) {
    let mut bases: Vec<PathBuf> = r.drives.clone();
    bases.push(r.home.clone());
    for base in bases {
        for dir in subdirs(&base) {
            let dn = name_of(&dir);
            let Some(kind) = root_kind(&dn) else { continue };
            let shown = dir.to_string_lossy().to_string();
            let (note, safety, keep_root) = match kind {
                RootKind::Safe => (Text::new("Thư mục tạm của hệ thống/OneDrive; tự tạo lại khi cần.", "System/OneDrive temp folder; recreated when needed."), Safety::Safe, true),
                RootKind::Review => (Text::new("Thư mục tạm do bạn hoặc công cụ tạo ở gốc ổ đĩa. Xem qua trước khi dọn.", "Temp folder created by you or a tool at the drive root. Look inside before cleaning."), Safety::Review, true),
                RootKind::Leftover => (Text::new("Còn lại sau khi nâng cấp Windows; an toàn nếu Windows đang ổn định và không cần quay lui.", "Left over from a Windows upgrade; safe if Windows is stable and you don't need to roll back."), Safety::Review, false),
            };
            if let Some(e) = make_entry(r, format!("root-{}", slug(&shown)), "temp", Text::same(shown), note, dir.clone(), vec![], safety, keep_root) {
                out.push(e);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn root_names_classified() {
        assert!(matches!(root_kind("tmp"), Some(RootKind::Review)));
        assert!(matches!(root_kind("codex-research-temp"), Some(RootKind::Review)));
        assert!(matches!(root_kind("OneDriveTemp"), Some(RootKind::Safe)));
        assert!(matches!(root_kind("$WinREAgent"), Some(RootKind::Leftover)));
        assert!(matches!(root_kind("Windows.old"), Some(RootKind::Leftover)));
        assert!(root_kind("Project").is_none());
        assert!(root_kind("$RECYCLE.BIN").is_none());
        assert!(root_kind("Templates").is_none());
    }

    #[test]
    fn slug_is_stable_and_ascii() {
        assert_eq!(slug("Profile 2"), "profile-2");
        assert_eq!(slug("D:\\tmp-aio"), "d-tmp-aio");
        assert_eq!(slug("Cốc Cốc"), "c-c-c-c");
    }
}
