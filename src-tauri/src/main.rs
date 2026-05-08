#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod audio;
mod config;
mod injector;
mod models;
mod shortcut;
mod transcriber;

fn main() {
    tauri::Builder::default()
        .run(tauri::generate_context!())
        .expect("error running tauri application");
}
