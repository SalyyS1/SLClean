//! Lệnh Tauri cho giao diện: liệt kê ổ đĩa, quét danh mục cache, tìm artifact build, dọn,
//! Thùng rác, cài đặt, chọn thư mục. Việc nặng chạy trên thread riêng; kết quả từng mục
//! được phát qua event để UI cập nhật dần thay vì chờ toàn bộ.

#[cfg(windows)]
mod apps;
mod artifacts;
mod big_files;
pub mod blob_dupes;
mod catalog;
mod catalog_dynamic;
mod catalog_specs;
mod cleaner;
pub mod cli;
mod drives;
mod duplicates;
mod elevation;
mod history;
#[cfg(windows)]
mod installed_apps;
#[cfg(windows)]
mod leftovers;
mod parallel;
mod project_roots;
#[cfg(windows)]
mod recycle_bin;
#[cfg(windows)]
mod registry;
pub mod sarif;
mod settings;
#[cfg(windows)]
mod shortcuts;
#[cfg(windows)]
mod single_instance;
mod sizer;
#[cfg(windows)]
mod startup;
#[cfg(windows)]
mod store_apps;
#[cfg(windows)]
mod user_assist;

use serde::Serialize;

#[derive(Clone, Serialize)]
pub struct CatalogItem {
    #[serde(flatten)]
    pub entry: catalog::CatalogEntry,
    pub bytes: u64,
    pub files: u64,
    pub denied: u64,
}

#[cfg(windows)]
mod gui {
    use super::*;
    use serde::Deserialize;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex};
    use std::time::{Duration, Instant};
    use tauri::{AppHandle, Emitter, Manager, State};
    use tauri_plugin_dialog::DialogExt;

    pub struct ScanState {
        cancel: Arc<AtomicBool>,
    }

/// Số liệu tạm trong lúc đo một mục, phát định kỳ để UI hiện số đang tăng.
#[derive(Clone, Serialize)]
pub struct CatalogProgress {
    pub id: String,
    pub bytes: u64,
    pub files: u64,
}

#[derive(Clone, Serialize)]
pub struct RootsInfo {
    pub discovered: Vec<PathBuf>,
    pub extra: Vec<PathBuf>,
    pub excluded: Vec<PathBuf>,
}

#[tauri::command]
fn list_drives() -> Vec<drives::Drive> {
    drives::list_drives()
}

#[tauri::command]
fn cancel_scan(state: State<'_, ScanState>) {
    state.cancel.store(true, Ordering::Relaxed);
}

/// Đo dung lượng mọi mục trong danh mục. Phát `catalog-start` (danh sách mục) ngay, rồi
/// `catalog-progress` trong lúc đo, và `catalog-item` khi mỗi mục xong.
#[tauri::command]
async fn scan_catalog(app: AppHandle, state: State<'_, ScanState>) -> Result<Vec<CatalogItem>, String> {
    state.cancel.store(false, Ordering::Relaxed);
    let cancel = state.cancel.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let entries = catalog::existing_entries();
        let _ = app.emit("catalog-start", &entries);
        let mut items = parallel::map(entries, parallel::default_workers(), |entry| {
            let mut total = sizer::DirStats::default();
            let last = Mutex::new(Instant::now());
            for p in &entry.paths {
                let st = sizer::dir_stats_with(p, &cancel, |partial| {
                    let mut t = last.lock().unwrap();
                    if t.elapsed() >= Duration::from_millis(250) {
                        *t = Instant::now();
                        let _ = app.emit("catalog-progress", CatalogProgress { id: entry.id.clone(), bytes: total.bytes + partial.bytes, files: total.files + partial.files });
                    }
                });
                total.bytes += st.bytes;
                total.files += st.files;
                total.denied += st.denied;
            }
            let item = CatalogItem { entry, bytes: total.bytes, files: total.files, denied: total.denied };
            let _ = app.emit("catalog-item", &item);
            item
        });
        items.sort_by(|a, b| b.bytes.cmp(&a.bytes));
        items
    })
    .await
    .map_err(|e| e.to_string())
}

/// Tìm artifact build theo kế hoạch từ cài đặt. Mỗi artifact xong phát event `artifact-found`.
#[tauri::command]
async fn scan_artifacts(app: AppHandle, state: State<'_, ScanState>) -> Result<Vec<artifacts::Artifact>, String> {
    state.cancel.store(false, Ordering::Relaxed);
    let cancel = state.cancel.clone();
    let settings = settings::load(&app);
    tauri::async_runtime::spawn_blocking(move || {
        let Some(roots) = catalog::Roots::detect() else { return Vec::new() };
        // Thư mục tạm ở gốc ổ (nhóm temp) vẫn được duyệt: bên trong thường có dự án bỏ dở với
        // node_modules/target riêng mà người dùng muốn thấy từng cái. Các mục cache khác thì
        // loại khỏi quét để không đếm trùng.
        let catalog_paths: Vec<PathBuf> = catalog::existing_entries().into_iter().filter(|e| e.group != "temp").flat_map(|e| e.paths).collect();
        let plan = project_roots::plan(&roots, &settings, &catalog_paths);
        artifacts::find_artifacts(&plan, &cancel, |a| {
            let _ = app.emit("artifact-found", a);
        })
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn project_roots(app: AppHandle) -> RootsInfo {
    let s = settings::load(&app);
    let discovered = catalog::Roots::detect().map(|r| project_roots::discovered_roots(&r)).unwrap_or_default();
    RootsInfo { discovered, extra: s.extra_roots, excluded: s.excluded_roots }
}

/// Dọn các mục đã chọn. Mỗi mục xong phát event `clean-progress`.
#[tauri::command]
async fn clean(app: AppHandle, items: Vec<cleaner::CleanItem>, to_trash: bool) -> Result<Vec<cleaner::CleanResult>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        items
            .iter()
            .map(|item| {
                let r = cleaner::clean_one(item, to_trash);
                let _ = app.emit("clean-progress", &r);
                r
            })
            .collect()
    })
    .await
    .map_err(|e| e.to_string())
}

#[tauri::command]
fn is_elevated() -> bool {
    elevation::is_elevated()
}

/// Mở bản mới với quyền admin qua UAC rồi đóng bản hiện tại nếu người dùng đồng ý.
/// Chờ UAC trên thread riêng để cửa sổ không bị treo trong lúc hộp thoại đang mở.
#[tauri::command]
async fn relaunch_as_admin(app: AppHandle) -> Result<(), String> {
    if elevation::is_elevated() {
        return Err("already-elevated".into());
    }
    tauri::async_runtime::spawn_blocking(elevation::relaunch_elevated).await.map_err(|e| e.to_string())??;
    app.exit(0);
    Ok(())
}

/// Số mục và dung lượng Thùng rác. Chạy ngoài main thread để cửa sổ không bao giờ treo,
/// dù shell có chậm.
#[tauri::command]
async fn recycle_bin_info() -> Result<recycle_bin::RecycleBin, String> {
    tauri::async_runtime::spawn_blocking(recycle_bin::query).await.map_err(|e| e.to_string())?
}

/// Dọn sạch Thùng rác; trả số liệu trước khi dọn để UI báo đã giải phóng bao nhiêu.
#[tauri::command]
async fn empty_recycle_bin() -> Result<recycle_bin::RecycleBin, String> {
    tauri::async_runtime::spawn_blocking(|| {
        let before = recycle_bin::query()?;
        recycle_bin::empty()?;
        Ok(before)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn get_settings(app: AppHandle) -> settings::Settings {
    settings::load(&app)
}

#[tauri::command]
fn save_settings(app: AppHandle, settings: settings::Settings) -> Result<(), String> {
    settings::save(&app, &settings)
}

/// Ngôn ngữ giao diện hiệu lực ("vi" | "en") từ cài đặt hoặc locale hệ thống.
#[tauri::command]
fn ui_language(app: AppHandle) -> String {
    settings::effective_language(&settings::load(&app))
}

/// Số liệu đo thư mục của một app / thư mục thừa, phát định kỳ và khi xong.
#[derive(Clone, Serialize)]
pub struct DirSize {
    pub id: String,
    pub bytes: u64,
    pub files: u64,
    pub denied: u64,
    pub done: bool,
}

/// Đo dung lượng từng thư mục theo id, phát `event` với số tạm mỗi 250 ms và số cuối khi xong.
/// Trả số cuối của từng id.
fn measure_dirs(app: &AppHandle, cancel: &AtomicBool, event: &str, targets: Vec<(String, PathBuf)>) -> Vec<DirSize> {
    parallel::map(targets, parallel::default_workers(), |(id, path)| {
        let last = Mutex::new(Instant::now());
        let st = sizer::dir_stats_with(&path, cancel, |partial| {
            let mut t = last.lock().unwrap();
            if t.elapsed() >= Duration::from_millis(250) {
                *t = Instant::now();
                let _ = app.emit(event, DirSize { id: id.clone(), bytes: partial.bytes, files: partial.files, denied: partial.denied, done: false });
            }
        });
        let d = DirSize { id, bytes: st.bytes, files: st.files, denied: st.denied, done: true };
        let _ = app.emit(event, &d);
        d
    })
}

/// Tab Ứng dụng: phát `apps-list` ngay (chỉ đọc registry, nhanh) rồi đo thư mục cài từng app,
/// phát `app-size`. Trả danh sách đã gắn số đo cuối.
#[tauri::command]
async fn scan_apps(app: AppHandle, state: State<'_, ScanState>) -> Result<Vec<apps::AppInfo>, String> {
    state.cancel.store(false, Ordering::Relaxed);
    let cancel = state.cancel.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let usage = apps::Usage::collect();
        let mut list = apps::discover(&usage);
        let _ = app.emit("apps-list", &list);
        let targets: Vec<(String, PathBuf)> = list.iter().filter(|a| a.folder_exists).filter_map(|a| a.install_dir.clone().map(|p| (a.id.clone(), p))).collect();
        for d in measure_dirs(&app, &cancel, "app-size", targets) {
            if let Some(a) = list.iter_mut().find(|a| a.id == d.id) {
                a.bytes = d.bytes;
                a.files = d.files;
                a.denied = d.denied;
                a.measured = true;
            }
        }
        list
    })
    .await
    .map_err(|e| e.to_string())
}

/// Gỡ một app qua trình gỡ của hãng hoặc Store, chờ xong, báo mục còn không và thư mục còn sót.
#[tauri::command]
async fn uninstall_app(id: String) -> Result<apps::ActionOutcome, String> {
    tauri::async_runtime::spawn_blocking(move || apps::uninstall(&id)).await.map_err(|e| e.to_string())?
}

/// Xoá mục đăng ký chết (trình gỡ đã mất), tuỳ chọn xoá luôn thư mục còn sót.
#[tauri::command]
async fn remove_dead_app(id: String, delete_folder: bool) -> Result<apps::ActionOutcome, String> {
    tauri::async_runtime::spawn_blocking(move || apps::remove_dead(&id, delete_folder)).await.map_err(|e| e.to_string())?
}

/// Tab Thư mục thừa: phát `leftovers-list` ngay rồi đo từng thư mục, phát `leftover-size`.
#[tauri::command]
async fn scan_leftovers(app: AppHandle, state: State<'_, ScanState>) -> Result<Vec<leftovers::Leftover>, String> {
    state.cancel.store(false, Ordering::Relaxed);
    let cancel = state.cancel.clone();
    tauri::async_runtime::spawn_blocking(move || {
        let Some(roots) = catalog::Roots::detect() else { return Vec::new() };
        let usage = apps::Usage::collect();
        let desktop = installed_apps::list();
        let store = store_apps::list();
        let owners = leftovers::Owners::build(&desktop, &store, store_apps::installed_families(), &usage);
        let list = leftovers::scan(&roots, &owners, &usage);
        let _ = app.emit("leftovers-list", &list);
        let targets = list.iter().map(|l| (l.id.clone(), l.path.clone())).collect();
        measure_dirs(&app, &cancel, "leftover-size", targets);
        list
    })
    .await
    .map_err(|e| e.to_string())
}

/// Tab File lớn: quét file đơn lẻ ≥ ngưỡng trên mọi ổ cố định, kèm bản đồ dung lượng theo thư
/// mục. Phát `big-file` cho từng file tìm thấy và `big-progress` định kỳ để UI hiện dần; kết
/// quả cuối kèm bản đồ. Đi cả ổ nên có thể lâu — dùng chung nút Huỷ với các tab khác.
#[tauri::command]
async fn scan_big_files(app: AppHandle, state: State<'_, ScanState>, threshold: Option<u64>) -> Result<big_files::BigScan, String> {
    state.cancel.store(false, Ordering::Relaxed);
    let cancel = state.cancel.clone();
    let settings = settings::load(&app);
    tauri::async_runtime::spawn_blocking(move || {
        let roots: Vec<PathBuf> = drives::list_drives().into_iter().map(|d| PathBuf::from(format!("{}\\", d.mount))).collect();
        let threshold = threshold.unwrap_or(big_files::DEFAULT_THRESHOLD);
        let last = Mutex::new(Instant::now());
        big_files::scan(
            &roots,
            &settings.excluded_roots,
            threshold,
            &cancel,
            |partial| {
                let mut t = last.lock().unwrap();
                if t.elapsed() >= Duration::from_millis(250) {
                    *t = Instant::now();
                    let _ = app.emit("big-progress", BigProgress { files: partial.scanned_files, bytes: partial.scanned_bytes, found: partial.files.len() as u64 });
                }
            },
            |f| {
                let _ = app.emit("big-file", f);
            },
        )
    })
    .await
    .map_err(|e| e.to_string())
}

/// Tab File trùng: nhóm file giống hệt nhau trên mọi ổ cố định. Phát `dup-group` cho từng nhóm
/// tìm được để UI hiện dần; kết quả cuối kèm tổng lãng phí. Dùng chung nút Huỷ với các tab khác.
#[tauri::command]
async fn scan_duplicates(app: AppHandle, state: State<'_, ScanState>, min_size: Option<u64>) -> Result<duplicates::DupScan, String> {
    state.cancel.store(false, Ordering::Relaxed);
    let cancel = state.cancel.clone();
    let settings = settings::load(&app);
    tauri::async_runtime::spawn_blocking(move || {
        let roots: Vec<PathBuf> = drives::list_drives().into_iter().map(|d| PathBuf::from(format!("{}\\", d.mount))).collect();
        // Bản trùng trong Program Files / AppData / Windows là do trình cài dựng ra, không phải
        // người dùng chép hai lần: nêu ra chỉ tổ dụ người ta xoá hỏng app. Các thư mục cache
        // trong danh mục cũng loại luôn — tab Dọn dẹp đã dọn cả cụm, tốt hơn là xoá lẻ từng file.
        let mut excluded = settings.excluded_roots.clone();
        excluded.extend(duplicates::program_areas());
        excluded.extend(catalog::existing_entries().into_iter().flat_map(|e| e.paths));
        duplicates::scan(&roots, &excluded, min_size.unwrap_or(duplicates::MIN_SIZE), &cancel, |g| {
            let _ = app.emit("dup-group", g);
        })
    })
    .await
    .map_err(|e| e.to_string())
}

/// Số liệu tạm trong lúc quét file lớn.
#[derive(Clone, Serialize)]
pub struct BigProgress {
    pub files: u64,
    pub bytes: u64,
    pub found: u64,
}

/// Tab Khởi động: mục tự chạy cùng Windows từ Run, thư mục Startup và scheduled task.
/// Chỉ đọc registry + một lần gọi `schtasks` nên nhanh, không cần phát event tiến độ.
#[tauri::command]
async fn scan_startup() -> Result<Vec<startup::StartupEntry>, String> {
    tauri::async_runtime::spawn_blocking(|| startup::discover(&apps::Usage::collect())).await.map_err(|e| e.to_string())
}

/// Bật/tắt một mục khởi động. Không xoá gì; bật lại được, kể cả bằng Task Manager.
#[tauri::command]
async fn set_startup_enabled(id: String, enabled: bool) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || startup::set_enabled(&id, enabled)).await.map_err(|e| e.to_string())?
}

/// Xoá hẳn một mục khởi động hỏng (lệnh trỏ tới file không còn).
#[tauri::command]
async fn remove_startup_entry(id: String) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || startup::remove(&id)).await.map_err(|e| e.to_string())?
}

/// Một mục UI gửi lên để ghi vào mốc nhật ký.
#[derive(Deserialize)]
pub struct SnapInput {
    pub id: String,
    pub vi: String,
    pub en: String,
    pub group: String,
    pub bytes: u64,
}

/// Ghi một mốc nhật ký sau khi quét xong trọn vẹn. UI gọi lệnh này chứ không phải backend tự
/// ghi, vì chỉ UI biết cả hai pha quét (danh mục + artifact) đã xong và không bị huỷ giữa chừng.
#[tauri::command]
async fn record_snapshot(app: AppHandle, items: Vec<SnapInput>) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let at = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
        let drives = drives::list_drives().into_iter().map(|d| history::DriveFree { mount: d.mount, free: d.free, total: d.total }).collect();
        let items = items.into_iter().map(|i| history::SnapItem { id: i.id, vi: i.vi, en: i.en, group: i.group, bytes: i.bytes }).collect();
        let mut h = history::load(&app);
        history::push(&mut h, history::Snapshot { at, drives, items });
        history::save(&app, &h)
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Danh sách mốc đã ghi (không kèm chi tiết từng mục) để UI cho chọn hai mốc so sánh.
#[tauri::command]
async fn history_list(app: AppHandle) -> Result<Vec<history::SnapshotInfo>, String> {
    tauri::async_runtime::spawn_blocking(move || history::list(&history::load(&app))).await.map_err(|e| e.to_string())
}

/// So hai mốc theo thời điểm (`at`). Thiếu một trong hai mốc → lỗi ngắn cho UI dịch.
#[tauri::command]
async fn history_diff(app: AppHandle, from: u64, to: u64) -> Result<history::Diff, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let h = history::load(&app);
        let find = |at: u64| h.snapshots.iter().find(|s| s.at == at).cloned().ok_or_else(|| "snapshot-missing".to_string());
        Ok(history::diff(&find(from)?, &find(to)?))
    })
    .await
    .map_err(|e| e.to_string())?
}

/// Hộp thoại chọn thư mục của hệ điều hành; None nếu người dùng huỷ.
#[tauri::command]
async fn pick_folder(app: AppHandle, title: String) -> Result<Option<PathBuf>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        app.dialog().file().set_title(title).blocking_pick_folder().and_then(|f| f.into_path().ok())
    })
    .await
    .map_err(|e| e.to_string())
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    if let Some(code) = cli::try_run_from_env() {
        std::process::exit(code);
    }
    // Bản được mở lại với quyền admin phải chờ bản cũ thoát; nếu vẫn còn bản khác thì
    // đưa cửa sổ của nó ra trước và thoát, thay vì hiện thêm một cửa sổ không có webview.
    if let Some(pid) = single_instance::after_pid_arg() {
        single_instance::wait_for_exit(pid, std::time::Duration::from_secs(15));
    }
    if single_instance::focus_existing_and_should_exit() {
        return;
    }
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_dialog::init())
        .setup(|app| {
            app.manage(ScanState { cancel: Arc::new(AtomicBool::new(false)) });
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            list_drives,
            cancel_scan,
            scan_catalog,
            scan_artifacts,
            project_roots,
            clean,
            is_elevated,
            relaunch_as_admin,
            recycle_bin_info,
            empty_recycle_bin,
            get_settings,
            save_settings,
            ui_language,
            pick_folder,
            scan_apps,
            uninstall_app,
            remove_dead_app,
            scan_leftovers,
            scan_big_files,
            scan_duplicates,
            scan_startup,
            set_startup_enabled,
            remove_startup_entry,
            record_snapshot,
            history_list,
            history_diff
        ])
        .run(tauri::generate_context!())
        .expect("không khởi động được SLClean");
    }
}

#[cfg(windows)]
pub use gui::run;

#[cfg(not(windows))]
pub fn run() {
    if let Some(code) = cli::try_run_from_env() {
        std::process::exit(code);
    }
    eprintln!("SLClean GUI requires Windows. Use --help for headless CLI commands.");
    std::process::exit(1);
}
