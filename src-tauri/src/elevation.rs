//! Kiểm tra và xin quyền admin. Một số mục hệ thống (Windows Update Download,
//! Windows\Temp, $WinREAgent…) chỉ dọn trọn vẹn được khi tiến trình chạy elevated.

use std::os::windows::process::CommandExt;

#[link(name = "shell32")]
extern "system" {
    fn IsUserAnAdmin() -> i32;
}

pub fn is_elevated() -> bool {
    unsafe { IsUserAnAdmin() != 0 }
}

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

/// Mở một bản mới của app qua hộp thoại UAC. Ok = người dùng đồng ý (bản mới đã chạy,
/// bản này nên thoát); Err = người dùng huỷ, bản hiện tại chạy tiếp bình thường.
/// Bản mới nhận `--after-pid` để chờ bản này thoát trước khi khởi tạo webview: hai bản
/// cùng lúc thì bản sau không tạo được WebView2 và chỉ hiện cửa sổ đen.
pub fn relaunch_elevated() -> Result<(), String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    // PowerShell nhân đôi nháy đơn bên trong chuỗi nháy đơn.
    let exe = exe.to_string_lossy().replace('\'', "''");
    let flag = crate::single_instance::AFTER_PID_FLAG;
    let pid = std::process::id();
    let status = std::process::Command::new("powershell")
        .args([
            "-NoProfile",
            "-Command",
            &format!("Start-Process -FilePath '{exe}' -ArgumentList '{flag}','{pid}' -Verb RunAs"),
        ])
        .creation_flags(CREATE_NO_WINDOW)
        .status()
        .map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err("uac-cancelled".into())
    }
}
