// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if std::env::args().nth(1).as_deref() == Some("--remote-client-bridge") {
        std::process::exit(kanban_lib::remote_bridge_main());
    }
    if std::env::args().nth(1).as_deref() == Some("--remote-helper") {
        std::process::exit(kanban_lib::remote_helper_main());
    }
    kanban_lib::run()
}
