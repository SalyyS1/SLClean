//! Gộp app desktop (registry Uninstall) và app Store thành một danh sách cho tab Ứng dụng,
//! kèm lần chạy cuối (UserAssist), có đang chạy không (tiến trình), và các hành động: gỡ,
//! xoá mục đăng ký chết.

use crate::installed_apps::{self, DesktopApp, Hive};
use crate::shortcuts::ShortcutIndex;
use crate::store_apps::{self, StoreApp};
use crate::user_assist::{self, Run};
use serde::Serialize;
use std::path::{Path, PathBuf};
use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};

#[derive(Clone, Debug, Serialize)]
pub struct AppInfo {
    pub id: String,
    /// "desktop" | "store"
    pub kind: String,
    pub name: String,
    pub publisher: String,
    pub version: String,
    pub install_dir: Option<PathBuf>,
    /// Giây epoch; 0 = không rõ.
    pub installed: u64,
    /// Giây epoch của lần mở cuối theo Windows ghi nhận; 0 = chưa thấy chạy.
    pub last_used: u64,
    /// false = registry không ghi thư mục cài nên không đối chiếu được nhật ký; khi đó
    /// `last_used == 0` nghĩa là "không rõ" chứ không phải "chưa từng mở".
    pub usage_known: bool,
    pub run_count: u32,
    pub running: bool,
    pub bytes: u64,
    pub files: u64,
    pub denied: u64,
    /// false = `bytes` là ước tính của trình cài, chưa đo thư mục.
    pub measured: bool,
    /// Mục đăng ký còn nhưng trình gỡ đã mất: chỉ còn cách xoá mục.
    pub dead: bool,
    /// Thành phần chạy nền (redistributable, runtime, SDK, driver, service…): người dùng không
    /// bao giờ tự mở nên "chưa từng mở" không có nghĩa gì; không phải ứng viên để gỡ.
    pub system_component: bool,
    pub folder_exists: bool,
    /// Xoá mục chết dưới HKLM cần quyền admin.
    pub needs_admin: bool,
    pub msi: bool,
}

/// Lịch sử chạy và tiến trình đang chạy, thu một lần cho cả danh sách.
pub struct Usage {
    runs: Vec<Run>,
    /// Đường dẫn exe (chữ thường) của mọi tiến trình đang chạy.
    running: Vec<String>,
    shortcuts: ShortcutIndex,
}

/// Chữ thường, dấu `/` đổi thành `\` (Riot ghi InstallLocation với `/`), bỏ `\` cuối.
fn lower(p: &Path) -> String {
    p.to_string_lossy().to_ascii_lowercase().replace('/', "\\").trim_end_matches('\\').to_string()
}

impl Usage {
    /// Không lịch sử, không tiến trình; cho test.
    #[cfg(test)]
    pub fn empty() -> Usage {
        Usage { runs: Vec::new(), running: Vec::new(), shortcuts: ShortcutIndex::default() }
    }

    pub fn collect() -> Usage {
        let mut sys = System::new();
        sys.refresh_processes_specifics(ProcessesToUpdate::All, true, ProcessRefreshKind::nothing().with_exe(UpdateKind::OnlyIfNotSet));
        let running = sys.processes().values().filter_map(|p| p.exe().map(lower)).collect();
        let shortcuts = ShortcutIndex::build();
        Usage { runs: Self::resolve_runs(user_assist::runs(), &shortcuts), running, shortcuts }
    }

    /// Bản ghi theo shortcut (.lnk) hay AppUserModelID được nhân đôi thành bản ghi theo exe đích,
    /// để lần mở Chrome/VS Code/Discord… (Windows ghi theo ID) đối chiếu được với thư mục cài.
    fn resolve_runs(mut runs: Vec<Run>, shortcuts: &ShortcutIndex) -> Vec<Run> {
        let extra: Vec<Run> = runs
            .iter()
            .filter_map(|r| shortcuts.resolve(&r.target).map(|exe| Run { target: exe.to_string_lossy().into_owned(), count: r.count, last: r.last }))
            .collect();
        runs.extend(extra);
        runs
    }

    /// Thư mục chứa exe của shortcut Start Menu/Desktop trùng tên app, khi registry không ghi
    /// thư mục cài (MSI thường bỏ trống InstallLocation).
    pub fn dir_by_shortcut_name(&self, name: &str) -> Option<PathBuf> {
        self.shortcuts.dir_for_name(name).filter(|p| !crate::cleaner::is_root_like(p) && p.is_dir())
    }

    /// Có lối mở app này từ Start Menu/Taskbar/Desktop không.
    pub fn has_shortcut_into(&self, dir: &Path) -> bool {
        self.shortcuts.points_into(dir)
    }

    /// (lần cuối, tổng số lần) của mọi bản ghi có đường dẫn nằm dưới `dir`.
    pub fn under_dir(&self, dir: &Path) -> (u64, u32) {
        let prefix = format!("{}\\", lower(dir));
        self.runs.iter().filter(|r| r.target.to_ascii_lowercase().starts_with(&prefix)).fold((0, 0), |(l, c), r| (l.max(r.last), c + r.count))
    }

    pub fn is_running_under(&self, dir: &Path) -> bool {
        let prefix = format!("{}\\", lower(dir));
        self.running.iter().any(|p| p.starts_with(&prefix))
    }

    /// Tên (không đuôi, chữ thường) của mọi exe đang chạy hoặc đã chạy từ `since` (giây epoch),
    /// để nhận ra thư mục dữ liệu của app portable/không đăng ký mà người dùng vẫn dùng.
    pub fn exe_stems(&self, since: u64) -> Vec<String> {
        let stem = |p: &str| Path::new(p).file_stem().map(|s| s.to_string_lossy().to_ascii_lowercase());
        let mut v: Vec<String> = self.running.iter().filter_map(|p| stem(p)).collect();
        v.extend(self.runs.iter().filter(|r| r.last >= since && r.target.contains('\\')).filter_map(|r| stem(&r.target)));
        v.sort();
        v.dedup();
        v
    }

    /// App Store được ghi theo AUMID `Family!App`.
    pub fn by_family(&self, family: &str) -> (u64, u32) {
        let prefix = format!("{}!", family.to_ascii_lowercase());
        self.runs.iter().filter(|r| r.target.to_ascii_lowercase().starts_with(&prefix)).fold((0, 0), |(l, c), r| (l.max(r.last), c + r.count))
    }
}

/// Từ trong tên nói rằng đây là thành phần chạy nền chứ không phải app để mở.
const COMPONENT_WORDS: &[&str] = &[
    "redistributable", "redist", "runtime", "sdk", "driver", "drivers", "service", "services", "framework",
    "extension", "extensions", "codec", "codecs", "addon", "updater", "installer", "support", "libraries", "runtimes",
];

/// Tên có từ khoá thành phần (so theo từ, không so chuỗi con: "Discord" không chứa "cord").
fn name_says_component(name: &str) -> bool {
    let lower = name.to_ascii_lowercase();
    lower.split(|c: char| !c.is_alphanumeric() && c != '+').any(|w| COMPONENT_WORDS.contains(&w)) || lower.contains("software development kit")
}

/// Thành phần chạy nền: tên nói vậy **và** Windows không cho người dùng lối mở nào (không
/// shortcut nào ở Start Menu/Taskbar/Desktop trỏ vào thư mục cài). Điều kiện thứ hai giữ lại
/// những app tên nghe như thành phần nhưng vẫn mở được, ví dụ "Visual Studio Installer".
fn is_system_component(name: &str, install_dir: Option<&Path>, usage: &Usage) -> bool {
    name_says_component(name) && !install_dir.map(|p| usage.has_shortcut_into(p)).unwrap_or(false)
}

fn from_desktop(d: DesktopApp, usage: &Usage, elevated: bool) -> AppInfo {
    let install_dir = d.install_dir.clone().or_else(|| if d.dead { None } else { usage.dir_by_shortcut_name(&d.name) });
    let (last_used, run_count) = install_dir.as_ref().map(|p| usage.under_dir(p)).unwrap_or((0, 0));
    let folder_exists = install_dir.as_ref().map(|p| p.is_dir()).unwrap_or(false);
    let running = folder_exists && install_dir.as_ref().map(|p| usage.is_running_under(p)).unwrap_or(false);
    // Có đường dẫn thư mục cài là đối chiếu được, kể cả khi thư mục đã bị xoá tay: nhật ký
    // UserAssist vẫn giữ bản ghi cũ nên mục chết vẫn cho biết lần mở cuối trước khi xoá.
    let usage_known = install_dir.is_some();
    let system_component = !d.dead && is_system_component(&d.name, install_dir.as_deref(), usage);
    AppInfo {
        id: d.id(),
        kind: "desktop".into(),
        name: d.name.clone(),
        publisher: d.publisher.clone(),
        version: d.version.clone(),
        install_dir,
        installed: d.installed,
        last_used,
        usage_known,
        run_count,
        running,
        bytes: d.est_bytes,
        files: 0,
        denied: 0,
        measured: false,
        dead: d.dead,
        system_component,
        folder_exists,
        needs_admin: d.dead && d.hive == Hive::Hklm && !elevated,
        msi: d.msi,
    }
}

fn from_store(s: StoreApp, usage: &Usage) -> AppInfo {
    let (last_used, run_count) = usage.by_family(&s.family);
    let folder_exists = s.root.is_dir();
    AppInfo {
        id: s.id(),
        kind: "store".into(),
        name: s.display.clone(),
        publisher: s.publisher.clone(),
        version: s.version.clone(),
        install_dir: Some(s.root.clone()),
        installed: 0,
        last_used,
        usage_known: true,
        run_count,
        running: folder_exists && usage.is_running_under(&s.root),
        bytes: 0,
        files: 0,
        denied: 0,
        measured: false,
        dead: false,
        system_component: is_system_component(&s.display, Some(&s.root), usage),
        folder_exists,
        needs_admin: false,
        msi: false,
    }
}

/// Toàn bộ app, sắp theo tên. Nhanh (chỉ registry), chưa đo thư mục.
pub fn discover(usage: &Usage) -> Vec<AppInfo> {
    let elevated = crate::elevation::is_elevated();
    let mut out: Vec<AppInfo> = installed_apps::list().into_iter().map(|d| from_desktop(d, usage, elevated)).collect();
    out.extend(store_apps::list().into_iter().map(|s| from_store(s, usage)));
    out.sort_by_cached_key(|a| a.name.to_lowercase());
    out
}

#[derive(Clone, Debug, Serialize)]
pub struct ActionOutcome {
    /// Mục không còn trong registry sau hành động.
    pub gone: bool,
    /// Thư mục cài còn sót sau khi gỡ (để đề nghị dọn tiếp).
    pub leftover_dir: Option<PathBuf>,
    pub freed: u64,
}

/// Gỡ app: chạy trình gỡ của hãng (desktop) hoặc Remove-AppxPackage (Store), rồi kiểm tra lại.
pub fn uninstall(id: &str) -> Result<ActionOutcome, String> {
    if let Some(s) = store_apps::find(id) {
        store_apps::remove(&s.full_name)?;
        let gone = !store_apps::exists(&s.full_name);
        return Ok(ActionOutcome { gone, leftover_dir: None, freed: 0 });
    }
    let d = installed_apps::find(id).ok_or("missing")?;
    if d.dead {
        return Err("uninstaller-missing".into());
    }
    installed_apps::run_uninstaller(&d)?;
    let gone = !installed_apps::exists(&d);
    let leftover_dir = if gone { d.install_dir.filter(|p| p.is_dir()) } else { None };
    Ok(ActionOutcome { gone, leftover_dir, freed: 0 })
}

/// Xoá mục đăng ký chết; `delete_folder` xoá luôn thư mục còn sót (không qua Thùng rác).
pub fn remove_dead(id: &str, delete_folder: bool) -> Result<ActionOutcome, String> {
    let d = installed_apps::find(id).ok_or("missing")?;
    if !d.dead {
        return Err("not-dead".into());
    }
    installed_apps::remove_entry(&d)?;
    let mut freed = 0;
    let mut leftover_dir = d.install_dir.clone().filter(|p| p.is_dir());
    if delete_folder {
        if let Some(dir) = leftover_dir.take() {
            let item = crate::cleaner::CleanItem { id: id.to_string(), paths: vec![dir.clone()], keep_root: false };
            let r = crate::cleaner::clean_one(&item, false);
            freed = r.freed;
            if dir.is_dir() {
                leftover_dir = Some(dir);
            }
        }
    }
    Ok(ActionOutcome { gone: !installed_apps::exists(&d), leftover_dir, freed })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn usage_joins_runs_to_folders() {
        let usage = Usage {
            runs: vec![
                Run { target: r"C:\Apps\Foo\foo.exe".into(), count: 3, last: 100 },
                Run { target: r"C:\Apps\Foo\bin\helper.exe".into(), count: 1, last: 250 },
                Run { target: r"C:\Apps\Foobar\x.exe".into(), count: 9, last: 999 },
                Run { target: "Microsoft.Paint_8wekyb3d8bbwe!App".into(), count: 2, last: 42 },
            ],
            running: vec![r"c:\apps\foo\foo.exe".into()],
            shortcuts: ShortcutIndex::default(),
        };
        assert_eq!(usage.under_dir(Path::new(r"C:\Apps\Foo")), (250, 4));
        assert_eq!(usage.under_dir(Path::new(r"C:\Apps\Foo\")), (250, 4));
        assert_eq!(usage.under_dir(Path::new(r"C:\Apps\Nope")), (0, 0));
        assert!(usage.is_running_under(Path::new(r"C:\Apps\FOO")));
        assert!(!usage.is_running_under(Path::new(r"C:\Apps\Foobar")));
        assert_eq!(usage.by_family("Microsoft.Paint_8wekyb3d8bbwe"), (42, 2));
    }

    #[test]
    fn component_names_are_recognised_by_word() {
        for n in [
            "Microsoft Visual C++ 2013 Redistributable (x64) - 12.0.40664",
            "Microsoft Windows Desktop Runtime - 8.0.24 (x64)",
            "Windows Software Development Kit - Windows 10.0.26100.7175",
            "Mozilla Maintenance Service",
            "TAP-Windows 9.24.2 (driver)",
            "AV1 Video Extension",
            "Apple Mobile Device Support",
        ] {
            assert!(name_says_component(n), "{n}");
        }
        for n in ["Discord", "Blockbench 4.12", "Zoom Workplace", "Steam", "Riot Client", "FL Studio 2025", "Slay the Spire 2"] {
            assert!(!name_says_component(n), "{n}");
        }
        // Có shortcut để mở thì không phải thành phần nền, dù tên nghe như vậy.
        let usage = Usage::empty();
        assert!(is_system_component("Microsoft Visual Studio Installer", Some(Path::new(r"C:\Nope")), &usage));
        assert!(!is_system_component("Discord", Some(Path::new(r"C:\Nope")), &usage));
    }

    /// Máy nào cũng có vài app mà Windows ghi lần mở theo AppUserModelID/shortcut; sau khi đổi
    /// qua exe đích, số bản ghi có đường dẫn phải tăng lên.
    #[test]
    fn shortcut_records_become_exe_records() {
        let raw = user_assist::runs();
        let idx = ShortcutIndex::build();
        let resolved = Usage::resolve_runs(raw.clone(), &idx);
        let paths = |v: &[Run]| v.iter().filter(|r| r.target.to_ascii_lowercase().ends_with(".exe")).count();
        assert!(resolved.len() >= raw.len());
        assert!(paths(&resolved) >= paths(&raw));
    }

    /// In app máy này nhìn thấy (mục chết, lần dùng cuối) để soi bằng mắt:
    /// `cargo test print_this_machine_apps -- --ignored --nocapture`
    #[test]
    #[ignore]
    fn print_this_machine_apps() {
        let usage = Usage::collect();
        let apps = discover(&usage);
        let dead: Vec<_> = apps.iter().filter(|a| a.dead).collect();
        println!("apps={} desktop={} store={} dead={} with-last-used={} running={}", apps.len(), apps.iter().filter(|a| a.kind == "desktop").count(), apps.iter().filter(|a| a.kind == "store").count(), dead.len(), apps.iter().filter(|a| a.last_used > 0).count(), apps.iter().filter(|a| a.running).count());
        for a in &dead {
            println!("  DEAD {} | dir={:?} exists={} admin={}", a.name, a.install_dir, a.folder_exists, a.needs_admin);
        }
        println!("components={}", apps.iter().filter(|a| a.system_component).count());
        for a in apps.iter().filter(|a| a.system_component) {
            println!("  COMP {}", a.name);
        }
        let mut used: Vec<_> = apps.iter().filter(|a| a.last_used > 0).collect();
        used.sort_by_key(|a| std::cmp::Reverse(a.last_used));
        for a in used.iter().take(12) {
            println!("  USED {} | last={} x{} running={} | {:?}", a.name, a.last_used, a.run_count, a.running, a.install_dir);
        }
        for a in apps.iter().filter(|a| a.kind == "store").take(8) {
            println!("  STORE {} | {} | v{} | {:?}", a.name, a.publisher, a.version, a.install_dir);
        }
    }

    #[test]
    fn discover_lists_desktop_and_store() {
        let usage = Usage::collect();
        let apps = discover(&usage);
        assert!(apps.iter().any(|a| a.kind == "desktop"));
        assert!(apps.iter().any(|a| a.kind == "store"));
        assert!(apps.windows(2).all(|w| w[0].name.to_lowercase() <= w[1].name.to_lowercase()));
    }
}
