//! Dung lượng các ổ đĩa cố định (bỏ qua ổ mạng/USB) qua sysinfo.

use serde::Serialize;
use sysinfo::Disks;

#[derive(Clone, Debug, Serialize)]
pub struct Drive {
    pub mount: String,
    pub name: String,
    pub total: u64,
    pub free: u64,
}

pub fn list_drives() -> Vec<Drive> {
    let disks = Disks::new_with_refreshed_list();
    let mut out: Vec<Drive> = disks
        .iter()
        .filter(|d| !d.is_removable() && d.total_space() > 0)
        .map(|d| Drive {
            mount: d.mount_point().to_string_lossy().trim_end_matches('\\').to_string(),
            name: d.name().to_string_lossy().to_string(),
            total: d.total_space(),
            free: d.available_space(),
        })
        .collect();
    out.sort_by(|a, b| a.mount.cmp(&b.mount));
    out.dedup_by(|a, b| a.mount == b.mount);
    out
}
