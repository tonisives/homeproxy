#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if let Err(error) = homeproxy_lib::run() {
        eprintln!("HomeProxy failed: {error}");
    }
}
