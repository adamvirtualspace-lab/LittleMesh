//! `littlemesh-app`: the LittleMesh desktop app, a Tauri shell over lm-core
//! (docs/milestones.md, M3). The frontend lives in `app/src`.

// No console window behind the app in release builds on Windows.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod transfer;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .manage(commands::AppState::default())
        .invoke_handler(tauri::generate_handler![
            commands::open_file,
            commands::input_stats,
            commands::scene_buffer,
            commands::process,
            commands::export,
            commands::startup_file,
        ])
        .run(tauri::generate_context!())
        .expect("failed to start LittleMesh");
}
