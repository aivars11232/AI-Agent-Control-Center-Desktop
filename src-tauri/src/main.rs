// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    if let Some(exit_code) = ai_agent_control_center_lib::run_cli(&arguments) {
        std::process::exit(exit_code);
    }

    ai_agent_control_center_lib::run();
}
