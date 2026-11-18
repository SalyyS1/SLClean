//! Nhật ký dung lượng: sau mỗi lần quét trọn vẹn, ghi lại một "mốc" gồm dung lượng trống của
//! từng ổ và cỡ của từng mục quét được. So hai mốc cho biết cái gì đã ăn ổ đĩa từ lần trước.
//! Ý này không mới (DiskBrain, SpaceRover, SizeTrend và `dfp.exe` của Windows đều so được hai
//! lần chụp); điểm khác của SLClean là mốc được ghi theo *mục đã phân loại* của catalog, nên
//! phần chênh lệch đọc ra là "cache của X phình 2 GB" thay vì một cây thư mục thô.
//! Dữ liệu nằm trong `%APPDATA%\<identifier>\history.json`, không gửi đi đâu.

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
#[cfg(windows)]
use std::path::PathBuf;
#[cfg(windows)]
use tauri::{AppHandle, Manager};

/// Số mốc giữ lại; cũ hơn thì bỏ. Mỗi mốc vài chục KB nên 60 mốc vẫn nhỏ.
const MAX_SNAPSHOTS: usize = 60;
/// Hai lần quét gần nhau hơn khoảng này thì lần sau thay lần trước, thay vì thêm mốc mới.
///
/// Ban đầu gộp theo *ngày*, nhưng như thế người mới cài quét bao nhiêu lần trong ngày đầu cũng
/// chỉ có một mốc, và tab Nhật ký cứ báo "cần thêm một lần quét" suốt cả ngày — trông như hỏng.
/// Gộp theo một giờ vừa chặn được việc bấm quét liên tục làm loãng nhật ký, vừa cho người dùng
/// thấy tính năng hoạt động ngay trong ngày đầu.
const MIN_GAP: u64 = 3_600;
#[cfg(test)]
const SECS_PER_DAY: u64 = 86_400;

/// Một mục trong mốc. Không lưu đường dẫn con hay số file để file không phình.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct SnapItem {
    pub id: String,
    /// Nhãn hai ngôn ngữ đã dựng sẵn; mốc cũ vẫn đọc được kể cả khi danh mục đổi tên mục.
    pub vi: String,
    pub en: String,
    pub group: String,
    pub bytes: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct DriveFree {
    pub mount: String,
    pub free: u64,
    pub total: u64,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Snapshot {
    /// Giây epoch lúc quét xong.
    pub at: u64,
    pub drives: Vec<DriveFree>,
    pub items: Vec<SnapItem>,
}

impl Snapshot {
    pub fn total_bytes(&self) -> u64 {
        self.items.iter().map(|i| i.bytes).sum()
    }
}

/// Mốc rút gọn cho danh sách chọn: không kèm `items`.
#[derive(Clone, Debug, Serialize)]
pub struct SnapshotInfo {
    pub at: u64,
    pub items: usize,
    pub bytes: u64,
    pub drives: Vec<DriveFree>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct History {
    /// Cũ trước, mới sau.
    pub snapshots: Vec<Snapshot>,
}

#[cfg(windows)]
fn file(app: &AppHandle) -> Option<PathBuf> {
    app.path().app_config_dir().ok().map(|d| d.join("history.json"))
}

/// Thiếu file hoặc file hỏng thì coi như chưa có nhật ký, không báo lỗi.
#[cfg(windows)]
pub fn load(app: &AppHandle) -> History {
    file(app)
        .and_then(|p| std::fs::read_to_string(p).ok())
        .and_then(|s| serde_json::from_str(&s).ok())
        .unwrap_or_default()
}

#[cfg(windows)]
pub fn save(app: &AppHandle, history: &History) -> Result<(), String> {
    let path = file(app).ok_or("Không tìm được thư mục cấu hình.")?;
    if let Some(dir) = path.parent() {
        std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
    }
    let text = serde_json::to_string(history).map_err(|e| e.to_string())?;
    std::fs::write(&path, text).map_err(|e| e.to_string())
}

/// Thêm một mốc: quét lại trong vòng `MIN_GAP` thì thay mốc cuối (bấm quét mấy lần liên tiếp
/// không làm loãng nhật ký), rồi cắt còn `MAX_SNAPSHOTS` mốc gần nhất.
pub fn push(history: &mut History, snap: Snapshot) {
    match history.snapshots.last() {
        Some(last) if snap.at.saturating_sub(last.at) < MIN_GAP => {
            history.snapshots.pop();
        }
        _ => {}
    }
    history.snapshots.push(snap);
    let n = history.snapshots.len();
    if n > MAX_SNAPSHOTS {
        history.snapshots.drain(..n - MAX_SNAPSHOTS);
    }
}

pub fn list(history: &History) -> Vec<SnapshotInfo> {
    history
        .snapshots
        .iter()
        .map(|s| SnapshotInfo { at: s.at, items: s.items.len(), bytes: s.total_bytes(), drives: s.drives.clone() })
        .collect()
}

/// Thay đổi của một mục giữa hai mốc. `before`/`after` là byte; mục mới có `before = None`,
/// mục đã biến mất có `after = None`.
#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct Change {
    pub id: String,
    pub vi: String,
    pub en: String,
    pub group: String,
    pub before: Option<u64>,
    pub after: Option<u64>,
    /// after - before, tính bằng i64 để nêu được cả giảm.
    pub delta: i64,
}

#[derive(Clone, Debug, Serialize)]
pub struct Diff {
    pub from: u64,
    pub to: u64,
    /// Tổng thay đổi của mọi mục (byte).
    pub total_delta: i64,
    /// Dung lượng trống từng ổ đã đổi thế nào: (mount, trống lúc trước, trống lúc sau).
    pub drives: Vec<DriveChange>,
    pub changes: Vec<Change>,
}

#[derive(Clone, Debug, Serialize, PartialEq, Eq)]
pub struct DriveChange {
    pub mount: String,
    pub before: u64,
    pub after: u64,
    /// after - before: dương = trống ra, âm = đầy thêm.
    pub delta: i64,
}

fn delta(before: Option<u64>, after: Option<u64>) -> i64 {
    after.unwrap_or(0) as i64 - before.unwrap_or(0) as i64
}

/// So hai mốc. Mục chỉ có ở một bên vẫn được nêu (mới xuất hiện / đã biến mất) nhưng UI tách
/// riêng, vì chúng thường do người dùng đổi thư mục quét chứ không phải dung lượng thật đổi.
/// Kết quả sắp theo độ lớn thay đổi giảm dần; mục không đổi bị bỏ.
pub fn diff(from: &Snapshot, to: &Snapshot) -> Diff {
    let before: HashMap<&str, &SnapItem> = from.items.iter().map(|i| (i.id.as_str(), i)).collect();
    let after: HashMap<&str, &SnapItem> = to.items.iter().map(|i| (i.id.as_str(), i)).collect();
    let mut changes: Vec<Change> = Vec::new();
    for (id, b) in &before {
        let a = after.get(id);
        let (bb, aa) = (Some(b.bytes), a.map(|x| x.bytes));
        if bb == aa {
            continue;
        }
        let src = a.unwrap_or(b);
        changes.push(Change { id: (*id).to_string(), vi: src.vi.clone(), en: src.en.clone(), group: src.group.clone(), before: bb, after: aa, delta: delta(bb, aa) });
    }
    for (id, a) in &after {
        if before.contains_key(id) {
            continue;
        }
        changes.push(Change { id: (*id).to_string(), vi: a.vi.clone(), en: a.en.clone(), group: a.group.clone(), before: None, after: Some(a.bytes), delta: a.bytes as i64 });
    }
    changes.sort_by_key(|c| std::cmp::Reverse(c.delta.abs()));

    let free_at = |snap: &Snapshot, mount: &str| snap.drives.iter().find(|d| d.mount == mount).map(|d| d.free);
    let mut drives: Vec<DriveChange> = to
        .drives
        .iter()
        .filter_map(|d| {
            let b = free_at(from, &d.mount)?;
            Some(DriveChange { mount: d.mount.clone(), before: b, after: d.free, delta: d.free as i64 - b as i64 })
        })
        .collect();
    drives.sort_by_key(|d| d.mount.clone());

    Diff { from: from.at, to: to.at, total_delta: to.total_bytes() as i64 - from.total_bytes() as i64, drives, changes }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(id: &str, bytes: u64) -> SnapItem {
        SnapItem { id: id.into(), vi: id.into(), en: id.into(), group: "temp".into(), bytes }
    }

    fn snap(at: u64, items: Vec<SnapItem>) -> Snapshot {
        Snapshot { at, drives: vec![DriveFree { mount: "C:".into(), free: 10_000, total: 100_000 }], items }
    }

    #[test]
    fn back_to_back_scans_replace_instead_of_piling_up() {
        let mut h = History::default();
        push(&mut h, snap(SECS_PER_DAY * 10, vec![item("a", 1)]));
        push(&mut h, snap(SECS_PER_DAY * 10 + 60, vec![item("a", 2)]));
        assert_eq!(h.snapshots.len(), 1, "quét lại ngay phải thay mốc cũ");
        assert_eq!(h.snapshots[0].items[0].bytes, 2, "giữ số của lần quét mới nhất");
        // Cách nhau đủ lâu thì là mốc mới — kể cả trong cùng một ngày, để người mới cài
        // vẫn so được hai mốc ngay hôm đầu.
        push(&mut h, snap(SECS_PER_DAY * 10 + MIN_GAP + 60, vec![item("a", 3)]));
        assert_eq!(h.snapshots.len(), 2, "cách nhau hơn một giờ thì thêm mốc");
        push(&mut h, snap(SECS_PER_DAY * 11, vec![item("a", 4)]));
        assert_eq!(h.snapshots.len(), 3, "hôm sau thì thêm mốc");
    }

    #[test]
    fn history_keeps_only_the_newest_snapshots() {
        let mut h = History::default();
        for d in 0..(MAX_SNAPSHOTS as u64 + 5) {
            push(&mut h, snap(SECS_PER_DAY * d, vec![item("a", d)]));
        }
        assert_eq!(h.snapshots.len(), MAX_SNAPSHOTS);
        assert_eq!(h.snapshots[0].at, SECS_PER_DAY * 5, "mốc cũ nhất bị cắt");
        assert_eq!(h.snapshots.last().unwrap().at, SECS_PER_DAY * (MAX_SNAPSHOTS as u64 + 4));
    }

    #[test]
    fn diff_reports_growth_shrink_new_and_gone() {
        let from = snap(100, vec![item("grew", 1_000), item("shrank", 5_000), item("same", 7), item("gone", 400)]);
        let mut to = snap(200, vec![item("grew", 9_000), item("shrank", 1_000), item("same", 7), item("new", 300)]);
        to.drives[0].free = 8_000;
        let d = diff(&from, &to);
        let by = |id: &str| d.changes.iter().find(|c| c.id == id).cloned();
        assert_eq!(d.changes.len(), 4, "mục không đổi không được nêu: {:?}", d.changes);
        assert_eq!(by("grew").unwrap().delta, 8_000);
        assert_eq!(by("shrank").unwrap().delta, -4_000);
        assert_eq!(by("gone").unwrap(), Change { id: "gone".into(), vi: "gone".into(), en: "gone".into(), group: "temp".into(), before: Some(400), after: None, delta: -400 });
        assert_eq!(by("new").unwrap().before, None);
        assert!(by("same").is_none());
        // Sắp theo độ lớn thay đổi, không phân biệt tăng hay giảm.
        assert_eq!(d.changes[0].id, "grew");
        assert_eq!(d.changes[1].id, "shrank");
        assert_eq!(d.total_delta, 10_307 - 6_407);
        assert_eq!(d.drives, vec![DriveChange { mount: "C:".into(), before: 10_000, after: 8_000, delta: -2_000 }]);
    }

    #[test]
    fn diff_of_identical_snapshots_is_empty() {
        let a = snap(1, vec![item("x", 5)]);
        let b = snap(2, vec![item("x", 5)]);
        let d = diff(&a, &b);
        assert!(d.changes.is_empty());
        assert_eq!(d.total_delta, 0);
    }
}
