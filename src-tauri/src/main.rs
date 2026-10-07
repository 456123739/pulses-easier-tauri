// main.rs — Tauri 主进程入口
// 替代 Node index.js：所有文件操作/下载/哈希/解压/比对 都在 Rust 里

#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod commands;
mod db;
mod differ;
mod downloader;
mod eapack;
mod updater;

use std::sync::atomic::AtomicBool;

fn main() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_shell::init())
        .manage(commands::CancelFlag(AtomicBool::new(false)))
        .invoke_handler(tauri::generate_handler![
            commands::app_version,
            commands::db_get_path,
            commands::db_load_config,
            commands::db_save_config,
            commands::db_create,
            commands::db_is_valid,
            commands::db_describe,
            commands::db_migrate,
            commands::diff_packs,
            commands::update_build_plan,
            commands::update_execute,
            commands::update_verify,
            commands::update_estimate,
            commands::dl_start,
            commands::dl_cancel,
            commands::pack_read_entry,
            commands::pack_export,
            commands::fs_read_dir,
            commands::fs_stat,
            commands::fs_exists,
            commands::fs_modpack_info,
            commands::fs_hash,
            commands::fs_unzip,
            commands::fs_space,
        ])
        .run(tauri::generate_context!())
        .expect("error while running Pulses Easier");
}
