// Windows 的 release 构建不要弹出控制台窗口。
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    xq_client_lib::run()
}
