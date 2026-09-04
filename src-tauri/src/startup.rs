//! Mục tự chạy cùng Windows, từ ba nguồn: khoá `Run` (HKCU, HKLM, HKLM WOW64), thư mục Startup
//! (của bạn và của mọi người dùng), và scheduled task có trigger lúc đăng nhập/khởi động.
//!
//! Bật/tắt (không xoá) đúng cách Task Manager làm: ghi cờ vào `Explorer\StartupApproved\*` cho
//! Run và thư mục Startup, `schtasks /Change` cho task. Mục đã tắt vẫn nằm nguyên trong registry
//! nên bật lại được, kể cả bằng Task Manager.
//!
//! Cờ `StartupApproved` là 12 byte: byte 0 là cờ trạng thái, 8 byte từ offset 4 là FILETIME lúc
//! đổi. Đo trên máy thật thấy byte 0 nhận 1, 2, 3 và 6 — **bit 0 bật nghĩa là đã tắt** (1 và 3 =
//! tắt, 2 và 6 = bật), chứ không phải chỉ hai giá trị 2/3 như phần lớn tài liệu viết. Không có
//! giá trị nào cho mục = đang bật.

use crate::apps::Usage;
use crate::installed_apps::split_command;
use crate::registry::{self, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
use crate::user_assist::known_folder;
use serde::Serialize;
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};

const RUN: &str = r"Software\Microsoft\Windows\CurrentVersion\Run";
const RUN_WOW: &str = r"Software\WOW6432Node\Microsoft\Windows\CurrentVersion\Run";
const APPROVED: &str = r"Software\Microsoft\Windows\CurrentVersion\Explorer\StartupApproved";
/// FOLDERID_Startup và FOLDERID_CommonStartup.
const FOLDERID_STARTUP: &str = "B97D20BB-F46A-4C97-BA10-5E3608430854";
const FOLDERID_COMMON_STARTUP: &str = "82A5EA35-D9CD-47C5-9629-E15D2F714E6E";
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Nguồn của một mục khởi động. Quyết định cách bật/tắt và khoá cờ nào phải ghi.
#[derive(Clone, Copy, Debug, Serialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum Source {
    /// Khoá Run của người dùng hiện tại.
    Run,
    /// Khoá Run của máy (mọi người dùng); sửa cần admin.
    RunMachine,
    /// Thư mục Startup của người dùng hiện tại.
    Folder,
    /// Thư mục Startup dùng chung; sửa cần admin.
    FolderMachine,
    Task,
}

impl Source {
    fn needs_admin(self) -> bool {
        matches!(self, Source::RunMachine | Source::FolderMachine)
    }

    /// Nguồn nào ghi cờ vào khoá con nào của `StartupApproved`.
    fn approved_key(self, wow64: bool) -> Option<(registry::Hkey, String)> {
        match self {
            // Mục 32-bit dưới WOW6432Node có khoá cờ riêng tên "Run32".
            Source::Run => Some((HKEY_CURRENT_USER, format!("{APPROVED}\\Run"))),
            Source::RunMachine => Some((HKEY_LOCAL_MACHINE, format!("{APPROVED}\\{}", if wow64 { "Run32" } else { "Run" }))),
            Source::Folder => Some((HKEY_CURRENT_USER, format!("{APPROVED}\\StartupFolder"))),
            Source::FolderMachine => Some((HKEY_LOCAL_MACHINE, format!("{APPROVED}\\StartupFolder"))),
            Source::Task => None,
        }
    }
}

#[derive(Clone, Debug, Serialize)]
pub struct StartupEntry {
    /// "run:hkcu:0:Tên" | "folder:0:tên.lnk" | "task:\Đường\Dẫn".
    pub id: String,
    pub name: String,
    /// Lệnh đầy đủ như đã đăng ký, để người dùng thấy đúng thứ sẽ chạy.
    pub command: String,
    /// Exe tách từ lệnh, nếu chỉ ra được một file có thật.
    pub target: Option<PathBuf>,
    pub source: Source,
    pub enabled: bool,
    /// Lần cuối mở exe đó theo UserAssist; 0 = không có bản ghi.
    pub last_used: u64,
    /// Có đối chiếu được lần mở không. False = không kết luận "lâu không dùng".
    pub usage_known: bool,
    pub running: bool,
    /// Exe nằm dưới %SystemRoot% hoặc thư mục Microsoft — không gợi ý tắt.
    pub system: bool,
    /// Lệnh trỏ tới file không còn: tắt hay không cũng chẳng chạy được.
    pub broken: bool,
    pub needs_admin: bool,
}

/// Byte 0 của cờ `StartupApproved`. Bit 0 bật = mục đã bị tắt.
fn flag_says_enabled(flag: &[u8]) -> bool {
    flag.first().map(|b| b & 1 == 0).unwrap_or(true)
}

/// 12 byte cờ: byte 0 là trạng thái, 8 byte từ offset 4 là FILETIME lúc đổi. Ghi giá trị Task
/// Manager dùng (2 = bật, 3 = tắt) để công cụ khác đọc ra đúng thứ nó chờ.
fn make_flag(enabled: bool, now_filetime: u64) -> [u8; 12] {
    let mut out = [0u8; 12];
    out[0] = if enabled { 2 } else { 3 };
    out[4..12].copy_from_slice(&now_filetime.to_le_bytes());
    out
}

fn filetime_now() -> u64 {
    let secs = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    (secs + 11_644_473_600) * 10_000_000
}

/// Exe của lệnh Run: bỏ tham số, thay biến môi trường, chỉ nhận file có thật.
fn target_of(command: &str) -> Option<PathBuf> {
    let (file, _) = split_command(&registry::expand_env(command))?;
    let p = PathBuf::from(file.trim());
    p.is_file().then_some(p)
}

/// Lần mở cuối của exe mục này chạy. Ưu tiên bản ghi của đúng file đó; chỉ khi không có mới
/// mở rộng ra cả thư mục cài, và cũng chỉ cho exe **ngoài** thư mục Windows.
///
/// Lý do: nhiều launcher (Discord `Update.exe`, OneDrive `OneDriveLauncher.exe`) không phải cái
/// người dùng bấm, nên chỉ tra đúng file sẽ luôn ra "chưa từng" — mở rộng ra thư mục là đúng.
/// Nhưng làm thế trong `System32` thì mọi mục đều thừa hưởng lịch sử của hàng xóm và trông như
/// vừa mở hôm qua, nên vùng đó phải tra chính xác.
fn last_used_of(exe: &Path, usage: &Usage) -> u64 {
    let (exact, _) = usage.for_exe(exe);
    if exact > 0 || is_system_path(exe) {
        return exact;
    }
    usage.under_dir(exe.parent().unwrap_or(exe)).0
}

/// Chương trình chạy được, tức thứ Windows có thể ghi lần mở. Script và file dữ liệu thì không.
fn is_program(p: &Path) -> bool {
    p.extension().map(|e| e.eq_ignore_ascii_case("exe") || e.eq_ignore_ascii_case("com")).unwrap_or(false)
}

/// Chỉ tính là "của Windows" khi exe nằm dưới %SystemRoot%. Cố tình hẹp: bắt theo chữ
/// "Microsoft" ở bất kỳ đâu sẽ gán nhãn cho cả OneDrive, Edge auto-launch và mọi shortcut trong
/// Start Menu (đường dẫn của nó có `\Microsoft\Windows\`) — toàn thứ người dùng tắt được vô hại.
fn is_system_path(p: &Path) -> bool {
    let lower = p.to_string_lossy().to_ascii_lowercase();
    let win = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into()).to_ascii_lowercase();
    lower.starts_with(&format!("{win}\\"))
}

/// Cờ bật/tắt của một mục theo tên giá trị, đọc từ khoá `StartupApproved` tương ứng.
fn approved_flag(source: Source, wow64: bool, name: &str) -> Option<Vec<u8>> {
    let (root, sub) = source.approved_key(wow64)?;
    registry::open(root, &sub)?.binary(name)
}

fn run_entries(source: Source, wow64: bool, usage: &Usage) -> Vec<StartupEntry> {
    let (root, sub) = match source {
        Source::Run => (HKEY_CURRENT_USER, RUN),
        Source::RunMachine => (HKEY_LOCAL_MACHINE, if wow64 { RUN_WOW } else { RUN }),
        _ => return Vec::new(),
    };
    let Some(key) = registry::open(root, sub) else { return Vec::new() };
    key.value_names()
        .into_iter()
        .filter_map(|name| {
            let command = key.string(&name)?;
            let target = target_of(&command);
            let hive = if source == Source::Run { "hkcu" } else { "hklm" };
            Some(entry(
                format!("run:{hive}:{}:{name}", if wow64 { 1 } else { 0 }),
                name.clone(),
                command,
                target,
                source,
                flag_says_enabled(approved_flag(source, wow64, &name).as_deref().unwrap_or(&[])),
                usage,
            ))
        })
        .collect()
}

fn folder_entries(source: Source, usage: &Usage) -> Vec<StartupEntry> {
    let guid = if source == Source::Folder { FOLDERID_STARTUP } else { FOLDERID_COMMON_STARTUP };
    let Some(dir) = known_folder(guid) else { return Vec::new() };
    let Ok(read) = std::fs::read_dir(&dir) else { return Vec::new() };
    read.filter_map(|e| e.ok())
        .filter(|e| e.path().is_file())
        .filter_map(|e| {
            let path = e.path();
            let file = path.file_name()?.to_string_lossy().into_owned();
            // desktop.ini chỉ là cấu hình hiển thị của thư mục, không phải mục khởi động.
            if file.eq_ignore_ascii_case("desktop.ini") {
                return None;
            }
            // Shortcut thì thứ sẽ chạy là đích của nó, không phải file .lnk.
            let is_lnk = path.extension().map(|e| e.eq_ignore_ascii_case("lnk")).unwrap_or(false);
            let target = if is_lnk {
                std::fs::read(&path).ok().and_then(|b| crate::shortcuts::parse(&b)).and_then(|sc| sc.target).map(PathBuf::from).filter(|p| p.is_file())
            } else {
                Some(path.clone())
            };
            let name = path.file_stem()?.to_string_lossy().into_owned();
            Some(entry(
                format!("folder:{}:{file}", if source == Source::Folder { 0 } else { 1 }),
                name,
                path.to_string_lossy().into_owned(),
                target,
                source,
                flag_says_enabled(approved_flag(source, false, &file).as_deref().unwrap_or(&[])),
                usage,
            ))
        })
        .collect()
}

fn entry(id: String, name: String, command: String, target: Option<PathBuf>, source: Source, enabled: bool, usage: &Usage) -> StartupEntry {
    let (last_used, running, system) = match target.as_deref() {
        Some(t) => (last_used_of(t, usage), usage.is_exe_running(t) || (!is_system_path(t) && usage.is_running_under(t.parent().unwrap_or(t))), is_system_path(t)),
        None => (0, false, false),
    };
    // Windows chỉ ghi lần mở cho chương trình. Mục trỏ tới script (.vbs, .bat, .cmd) hay file
    // dữ liệu thì không có gì để đối chiếu, và báo "chưa từng mở" cho một script khởi động là
    // vô nghĩa — không ai "mở" nó bao giờ.
    let usage_known = target.as_deref().map(is_program).unwrap_or(false);
    StartupEntry {
        id,
        name,
        command,
        broken: target.is_none(),
        usage_known,
        target,
        needs_admin: source.needs_admin(),
        source,
        enabled,
        last_used,
        running,
        system,
    }
}

/// `schtasks /Query /XML` cho mọi task một lần; trả các tài liệu XML đã tách.
fn query_tasks_xml() -> Vec<String> {
    let out = match std::process::Command::new("schtasks").args(["/Query", "/XML"]).creation_flags(CREATE_NO_WINDOW).output() {
        Ok(o) if o.status.success() => o.stdout,
        _ => return Vec::new(),
    };
    // schtasks xuất UTF-16LE có BOM khi ghi ra pipe.
    let text = if out.len() >= 2 && out[0] == 0xFF && out[1] == 0xFE {
        let u16s: Vec<u16> = out[2..].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
        String::from_utf16_lossy(&u16s)
    } else {
        String::from_utf8_lossy(&out).into_owned()
    };
    split_task_docs(&text)
}

/// Nhiều tài liệu XML nối nhau trong một luồng: cắt tại mỗi khai báo `<?xml`.
pub(crate) fn split_task_docs(text: &str) -> Vec<String> {
    let mut docs = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find("<?xml") {
        rest = &rest[start..];
        let next = rest[5..].find("<?xml").map(|i| i + 5);
        match next {
            Some(i) => {
                docs.push(rest[..i].to_string());
                rest = &rest[i..];
            }
            None => {
                docs.push(rest.to_string());
                break;
            }
        }
    }
    docs
}

/// Nội dung giữa thẻ mở/đóng đầu tiên. XML của schtasks không có namespace lồng nên đủ dùng.
pub(crate) fn tag(doc: &str, name: &str) -> Option<String> {
    let open = format!("<{name}>");
    let close = format!("</{name}>");
    let s = doc.find(&open)? + open.len();
    let e = doc[s..].find(&close)? + s;
    Some(doc[s..e].trim().to_string())
}

/// Task đáng gọi là "khởi động": có trigger lúc đăng nhập hoặc lúc máy khởi động.
pub(crate) fn runs_at_startup(doc: &str) -> bool {
    doc.contains("<LogonTrigger") || doc.contains("<BootTrigger")
}

/// Task của Windows nằm dưới `\Microsoft\`; ẩn khỏi danh sách vì tắt chúng dễ hỏng máy và
/// người dùng thường không có quyền sửa.
pub(crate) fn is_microsoft_task(uri: &str) -> bool {
    uri.to_ascii_lowercase().starts_with("\\microsoft\\")
}

/// Một tài liệu XML task → mục khởi động, nếu nó thật sự chạy lúc đăng nhập/khởi động.
fn task_entry(doc: &str, usage: &Usage) -> Option<StartupEntry> {
    let uri = tag(doc, "URI")?;
    if is_microsoft_task(&uri) || !runs_at_startup(doc) {
        return None;
    }
    let command = tag(doc, "Command")?;
    let args = tag(doc, "Arguments").unwrap_or_default();
    // XML bọc đường dẫn trong nháy kép khi có dấu cách; bỏ để tách exe cho đúng.
    let exe = registry::expand_env(command.trim_matches('"'));
    let target = PathBuf::from(&exe);
    let target = target.is_file().then_some(target);
    let full = if args.is_empty() { exe.clone() } else { format!("{exe} {args}") };
    let name = uri.rsplit('\\').next().unwrap_or(&uri).to_string();
    // `<Enabled>` xuất hiện cả trong Triggers lẫn Settings; chỉ thẻ đầu là trạng thái task khi
    // task không có thẻ trigger riêng, nên đọc thẻ cuối cùng cho chắc.
    let enabled = doc.rmatch_indices("<Enabled>").next().map(|(i, _)| doc[i..].starts_with("<Enabled>true")).unwrap_or(true);
    Some(entry(format!("task:{uri}"), name, full, target, Source::Task, enabled, usage))
}

pub fn discover(usage: &Usage) -> Vec<StartupEntry> {
    let mut out = Vec::new();
    out.extend(run_entries(Source::Run, false, usage));
    out.extend(run_entries(Source::RunMachine, false, usage));
    out.extend(run_entries(Source::RunMachine, true, usage));
    out.extend(folder_entries(Source::Folder, usage));
    out.extend(folder_entries(Source::FolderMachine, usage));
    for doc in query_tasks_xml() {
        if let Some(e) = task_entry(&doc, usage) {
            out.push(e);
        }
    }
    // Đang bật trước, rồi mục lâu không dùng lên đầu (0 = không rõ, xếp cuối trong nhóm).
    out.sort_by(|a, b| b.enabled.cmp(&a.enabled).then_with(|| key_of(a).cmp(&key_of(b))).then_with(|| a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    out
}

/// Khoá sắp xếp trong nhóm: mục có bản ghi mở xếp theo lần mở tăng dần (lâu nhất trước), mục
/// không rõ xuống sau cùng.
fn key_of(e: &StartupEntry) -> (u8, u64) {
    match e.last_used {
        0 => (1, 0),
        t => (0, t),
    }
}

/// Tách id thành các phần đã mã hoá trong `discover`.
fn parse_id(id: &str) -> Option<(Source, bool, String)> {
    if let Some(rest) = id.strip_prefix("run:") {
        let mut it = rest.splitn(3, ':');
        let hive = it.next()?;
        let wow64 = it.next()? == "1";
        let name = it.next()?.to_string();
        return Some((if hive == "hkcu" { Source::Run } else { Source::RunMachine }, wow64, name));
    }
    if let Some(rest) = id.strip_prefix("folder:") {
        let (scope, file) = rest.split_once(':')?;
        return Some((if scope == "0" { Source::Folder } else { Source::FolderMachine }, false, file.to_string()));
    }
    id.strip_prefix("task:").map(|uri| (Source::Task, false, uri.to_string()))
}

/// Bật/tắt một mục. Không xoá gì: mục vẫn nằm nguyên, bật lại được bằng app hay Task Manager.
/// Lỗi trả về là chuỗi ngắn cho UI dịch.
pub fn set_enabled(id: &str, enabled: bool) -> Result<(), String> {
    let (source, wow64, name) = parse_id(id).ok_or("bad-id")?;
    if source == Source::Task {
        let flag = if enabled { "/ENABLE" } else { "/DISABLE" };
        let out = std::process::Command::new("schtasks").args(["/Change", "/TN", &name, flag]).creation_flags(CREATE_NO_WINDOW).output().map_err(|_| "task-failed".to_string())?;
        // schtasks đổi chữ theo ngôn ngữ máy nên chỉ tin exit code.
        return if out.status.success() { Ok(()) } else { Err("needs-admin".into()) };
    }
    let (root, sub) = source.approved_key(wow64).ok_or("bad-id")?;
    let key = registry::create_writable(root, &sub).map_err(|_| "needs-admin".to_string())?;
    key.set_binary(&name, &make_flag(enabled, filetime_now())).map_err(|_| "needs-admin".into())
}

/// Xoá hẳn một mục khởi động hỏng (lệnh trỏ tới file không còn). Chỉ dùng cho mục `broken`:
/// tắt một mục đã hỏng chẳng giải quyết gì, mà nó vẫn nằm đó gây rối. Mục còn chạy được thì
/// dùng `set_enabled` để tắt, không xoá — người dùng đổi ý là bật lại được.
pub fn remove(id: &str) -> Result<(), String> {
    let (source, wow64, name) = parse_id(id).ok_or("bad-id")?;
    match source {
        Source::Task => {
            let out = std::process::Command::new("schtasks").args(["/Delete", "/TN", &name, "/F"]).creation_flags(CREATE_NO_WINDOW).output().map_err(|_| "task-failed".to_string())?;
            if out.status.success() { Ok(()) } else { Err("needs-admin".into()) }
        }
        Source::Folder | Source::FolderMachine => {
            let guid = if source == Source::Folder { FOLDERID_STARTUP } else { FOLDERID_COMMON_STARTUP };
            let dir = known_folder(guid).ok_or("bad-id")?;
            let path = Path::new(&dir).join(&name);
            // Chỉ được xoá file nằm ngay trong thư mục Startup, không nhận id kèm "..".
            if path.parent() != Some(Path::new(&dir)) {
                return Err("bad-id".into());
            }
            std::fs::remove_file(&path).map_err(|e| if e.kind() == std::io::ErrorKind::PermissionDenied { "needs-admin".to_string() } else { e.to_string() })
        }
        Source::Run | Source::RunMachine => {
            let (root, sub) = match source {
                Source::Run => (HKEY_CURRENT_USER, RUN.to_string()),
                _ => (HKEY_LOCAL_MACHINE, if wow64 { RUN_WOW.to_string() } else { RUN.to_string() }),
            };
            registry::create_writable(root, &sub).map_err(|_| "needs-admin".to_string())?.delete_value(&name).map_err(|_| "needs-admin".to_string())?;
            // Cờ bật/tắt của mục đã xoá không còn nghĩa gì; bỏ luôn cho sạch, lỗi thì kệ.
            if let Some((aroot, asub)) = source.approved_key(wow64) {
                if let Ok(k) = registry::create_writable(aroot, &asub) {
                    let _ = k.delete_value(&name);
                }
            }
            Ok(())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn flag_bit_zero_means_disabled() {
        // Giá trị đo được trên máy thật: 2 và 6 là đang bật, 1 và 3 là đã tắt.
        assert!(flag_says_enabled(&[2, 0, 0, 0]));
        assert!(flag_says_enabled(&[6, 0, 0, 0]));
        assert!(!flag_says_enabled(&[3, 0, 0, 0]));
        assert!(!flag_says_enabled(&[1, 0, 0, 0]));
        // Không có cờ = Windows chưa từng tắt mục này = đang bật.
        assert!(flag_says_enabled(&[]));
    }

    #[test]
    fn written_flag_round_trips_and_matches_task_manager_values() {
        let on = make_flag(true, 0);
        let off = make_flag(false, 133_000_000_000_000_000);
        assert_eq!(on[0], 2, "Task Manager ghi 2 khi bật");
        assert_eq!(off[0], 3, "Task Manager ghi 3 khi tắt");
        assert_eq!(on.len(), 12);
        assert!(flag_says_enabled(&on) && !flag_says_enabled(&off));
        assert_eq!(u64::from_le_bytes(off[4..12].try_into().unwrap()), 133_000_000_000_000_000);
    }

    #[test]
    fn id_round_trips_for_every_source() {
        let cases = [
            ("run:hkcu:0:Steam", (Source::Run, false, "Steam")),
            ("run:hklm:1:RadminVPN", (Source::RunMachine, true, "RadminVPN")),
            ("run:hklm:0:SecurityHealth", (Source::RunMachine, false, "SecurityHealth")),
            ("folder:0:EVKey64.lnk", (Source::Folder, false, "EVKey64.lnk")),
            ("folder:1:Tailscale.lnk", (Source::FolderMachine, false, "Tailscale.lnk")),
            ("task:\\gxx speed launcher", (Source::Task, false, "\\gxx speed launcher")),
        ];
        for (id, want) in cases {
            let (s, w, n) = parse_id(id).unwrap_or_else(|| panic!("không tách được {id}"));
            assert_eq!((s, w, n.as_str()), want, "id {id}");
        }
        // Tên mục Run có thể chứa dấu hai chấm; phần tên phải giữ nguyên.
        assert_eq!(parse_id("run:hkcu:0:A:B").unwrap().2, "A:B");
        assert!(parse_id("nonsense").is_none());
    }

    #[test]
    fn approved_key_differs_for_wow64_and_hive() {
        assert_eq!(Source::Run.approved_key(false).unwrap().1, format!("{APPROVED}\\Run"));
        assert_eq!(Source::RunMachine.approved_key(true).unwrap().1, format!("{APPROVED}\\Run32"));
        assert_eq!(Source::RunMachine.approved_key(false).unwrap().1, format!("{APPROVED}\\Run"));
        assert_eq!(Source::Folder.approved_key(false).unwrap().1, format!("{APPROVED}\\StartupFolder"));
        assert!(Source::Task.approved_key(false).is_none(), "task không dùng cờ registry");
        assert!(Source::RunMachine.needs_admin() && !Source::Run.needs_admin());
    }

    #[test]
    fn splits_and_reads_task_xml() {
        let xml = concat!(
            "<?xml version=\"1.0\" encoding=\"UTF-16\"?><Task><RegistrationInfo><URI>\\Zz One</URI></RegistrationInfo>",
            "<Triggers><LogonTrigger><Enabled>true</Enabled></LogonTrigger></Triggers>",
            "<Actions><Exec><Command>C:\\x\\a.exe</Command><Arguments>--q</Arguments></Exec></Actions>",
            "<Settings><Enabled>false</Enabled></Settings></Task>",
            "<?xml version=\"1.0\"?><Task><RegistrationInfo><URI>\\Microsoft\\Windows\\Zz</URI></RegistrationInfo>",
            "<Triggers><LogonTrigger/></Triggers></Task>",
            "<?xml version=\"1.0\"?><Task><RegistrationInfo><URI>\\Zz Daily</URI></RegistrationInfo>",
            "<Triggers><TimeTrigger/></Triggers></Task>",
        );
        let docs = split_task_docs(xml);
        assert_eq!(docs.len(), 3);
        assert_eq!(tag(&docs[0], "URI").as_deref(), Some("\\Zz One"));
        assert_eq!(tag(&docs[0], "Command").as_deref(), Some("C:\\x\\a.exe"));
        assert!(runs_at_startup(&docs[0]) && !runs_at_startup(&docs[2]), "chỉ trigger đăng nhập/khởi động mới tính");
        assert!(is_microsoft_task(&tag(&docs[1], "URI").unwrap()));

        let usage = Usage::empty();
        let e = task_entry(&docs[0], &usage).expect("task người dùng phải thành mục");
        assert_eq!(e.name, "Zz One");
        assert_eq!(e.command, "C:\\x\\a.exe --q");
        assert!(!e.enabled, "thẻ Enabled cuối (Settings) mới là trạng thái task");
        assert!(e.broken, "exe không có thật → hỏng");
        assert!(task_entry(&docs[1], &usage).is_none(), "task Microsoft bị ẩn");
        assert!(task_entry(&docs[2], &usage).is_none(), "task chạy theo giờ không phải mục khởi động");
    }

    #[test]
    fn only_systemroot_counts_as_windows() {
        let win = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        assert!(is_system_path(Path::new(&format!("{win}\\system32\\SecurityHealthSystray.exe"))));
        assert!(!is_system_path(Path::new(r"D:\Game\Steam\steam.exe")));
        // Tắt được vô hại, không được gán nhãn "của Windows".
        assert!(!is_system_path(Path::new(r"C:\Program Files (x86)\Microsoft\Edge\Application\msedge.exe")));
        assert!(!is_system_path(Path::new(r"C:\Users\x\AppData\Local\Microsoft\OneDrive\OneDrive.exe")));
        assert!(!is_system_path(Path::new(r"C:\Users\x\AppData\Roaming\Microsoft\Windows\Start Menu\Programs\Startup\a.vbs")));
    }

    #[test]
    fn only_programs_can_have_a_last_opened_time() {
        assert!(is_program(Path::new(r"C:\x\a.exe")) && is_program(Path::new(r"C:\x\A.EXE")));
        // Script khởi động không phải thứ ai đó "mở", nên không được gắn nhãn "chưa từng mở".
        assert!(!is_program(Path::new(r"C:\x\9router.vbs")));
        assert!(!is_program(Path::new(r"C:\x\go.bat")));
        assert!(!is_program(Path::new(r"C:\x\uk_process_state.txt")));

        let usage = Usage::empty();
        let script = entry("folder:0:s.vbs".into(), "s".into(), r"C:\x\s.vbs".into(), Some(PathBuf::from(r"C:\x\s.vbs")), Source::Folder, true, &usage);
        assert!(!script.usage_known, "script không đối chiếu được với nhật ký mở app");
        assert!(!script.broken, "file có thật (đường dẫn) vẫn không phải là hỏng");
    }

    #[test]
    fn windows_folder_entries_do_not_inherit_neighbours_history() {
        let win = std::env::var("SystemRoot").unwrap_or_else(|_| r"C:\Windows".into());
        let usage = Usage::collect();
        // notepad.exe hầu như không ai mở qua Explorer, nhưng System32 thì đầy bản ghi của thứ
        // khác. Gom theo thư mục sẽ cho ra một mốc thời gian không phải của nó.
        let notepad = PathBuf::from(format!("{win}\\System32\\notepad.exe"));
        assert_eq!(last_used_of(&notepad, &usage), usage.for_exe(&notepad).0, "mục trong thư mục Windows phải tra đúng file");
    }

    #[test]
    fn this_machine_lists_run_entries_with_flags() {
        let usage = Usage::empty();
        let all = discover(&usage);
        assert!(!all.is_empty(), "máy nào cũng có ít nhất một mục khởi động");
        // Mọi mục phải có id tách lại được và lệnh không rỗng.
        for e in &all {
            assert!(parse_id(&e.id).is_some(), "id lạ: {}", e.id);
            assert!(!e.command.trim().is_empty(), "lệnh rỗng: {}", e.id);
        }
        // Mục HKLM phải được đánh dấu cần admin, mục HKCU thì không.
        for e in all.iter().filter(|e| e.id.starts_with("run:hklm:")) {
            assert!(e.needs_admin, "{} dưới HKLM phải cần admin", e.id);
        }
        for e in all.iter().filter(|e| e.id.starts_with("run:hkcu:")) {
            assert!(!e.needs_admin, "{} dưới HKCU không cần admin", e.id);
        }
    }

    /// `cargo test -- --ignored --nocapture print_this_machine_startup` để xem máy thật có gì.
    #[test]
    #[ignore]
    fn print_this_machine_startup() {
        for e in discover(&Usage::collect()) {
            println!("{:?} enabled={} broken={} sys={} last={} {} :: {}", e.source, e.enabled, e.broken, e.system, e.last_used, e.name, e.command);
        }
    }
}
