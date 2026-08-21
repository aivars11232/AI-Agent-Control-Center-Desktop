// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if std::env::args().any(|argument| argument == "--remove-credentials") {
        ai_agent_control_center_lib::remove_stored_credentials_for_uninstall();
        return;
    }

    ai_agent_control_center_lib::run();
}
