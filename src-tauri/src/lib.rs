//! Lệnh Tauri cho giao diện: liệt kê ổ đĩa, quét danh mục cache, tìm artifact build, dọn,
//! Thùng rác, cài đặt, chọn thư mục. Việc nặng chạy trên thread riêng; kết quả từng mục
//! được phát qua event để UI cập nhật dần thay vì chờ toàn bộ.

mod artifacts;
mod catalog;
mod catalog_dynamic;
mod catalog_specs;
mod cleaner;
mod drives;
mod elevation;
mod parallel;
mod project_roots;
mod settings;
mod single_instance;
mod sizer;

use serde::Serialize;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_dialog::DialogExt;

pub struct ScanState {
    cancel: Arc<AtomicBool>,
}

#[derive(Clone, Serialize)]
pub struct CatalogItem {
    #[serde(flatten)]
    pub entry: catalog::CatalogEntry,
    pub bytes: u64,
    pub files: u64,
    pub denied: u64,
}

/// Số liệu tạm trong lúc đo một mục, phát định kỳ để UI hiện số đang tăng.
#[derive(Clone, Serialize)]
pub struct CatalogProgress {
    pub id: String,
    pub bytes: u64,
    pub files: u64,
}

#[derive(Clone, Serialize)]
pub struct RecycleBin {
    pub items: usize,
    pub bytes: u64,
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

#[tauri::command]
fn recycle_bin_info() -> RecycleBin {
    let items = trash::os_limited::list().unwrap_or_default();
    let bytes = items
        .iter()
        .filter_map(|i| trash::os_limited::metadata(i).ok())
        .map(|m| match m.size {
            trash::TrashItemSize::Bytes(b) => b,
            trash::TrashItemSize::Entries(_) => 0,
        })
        .sum();
    RecycleBin { items: items.len(), bytes }
}

#[tauri::command]
fn empty_recycle_bin() -> Result<RecycleBin, String> {
    let before = recycle_bin_info();
    let items = trash::os_limited::list().map_err(|e| e.to_string())?;
    trash::os_limited::purge_all(items).map_err(|e| e.to_string())?;
    Ok(before)
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
            pick_folder
        ])
        .run(tauri::generate_context!())
        .expect("không khởi động được SLClean");
}
