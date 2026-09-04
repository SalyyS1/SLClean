// Ẩn cửa sổ console khi chạy bản release trên Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    slclean_lib::run()
}
