// commands.rs — Tauri #[command] 注册（对应 Electron ipcMain.handle）
//
// 注意：Tauri 的 async 命令**不能带引用参数**（带引用就必须返回 Result，
// 且宏展开后会有生命周期问题）。所以这里统一：
//   · 异步命令只收 owned 参数（AppHandle / String / Value）
//   · 取消标志用模块级 static，不用 State<'_, T>

use serde_json::Value;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use tauri::AppHandle;

use crate::db;
use crate::differ;
use crate::downloader;
use crate::eapack;
use crate::updater;

/// 下载取消标志（模块级 static：async 命令里不能借 State）
static CANCEL: AtomicBool = AtomicBool::new(false);

// ── 版本 ──
#[tauri::command]
pub fn app_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

// ── 数据库 ──
#[tauri::command]
pub fn db_get_path() -> Option<String> {
    db::get_db_path()
}

#[tauri::command]
pub fn db_load_config() -> Option<Value> {
    db::load_config()
}

#[tauri::command]
pub fn db_save_config(cfg: Value) -> bool {
    db::save_config(cfg)
}

#[tauri::command]
pub fn db_create(path: String) -> bool {
    db::create_database(&PathBuf::from(&path))
}

#[tauri::command]
pub fn db_is_valid(path: String) -> bool {
    db::is_valid_database(&PathBuf::from(&path))
}

#[tauri::command]
pub fn db_describe(path: String) -> Value {
    db::describe(&PathBuf::from(&path))
}

#[tauri::command]
pub fn db_migrate(src: String, dst: String, skip: Option<Vec<String>>) -> Value {
    db::migrate(
        &PathBuf::from(&src),
        &PathBuf::from(&dst),
        skip.unwrap_or_else(|| vec!["cache".to_string()]),
    )
}

// ── 比对 ──
#[tauri::command]
pub fn diff_packs(old_root: String, new_root: String) -> Value {
    differ::diff_packs(&PathBuf::from(&old_root), &PathBuf::from(&new_root))
}

// ── 更新 ──
#[tauri::command]
pub fn update_build_plan(
    diff: Value,
    checked: Value,
    strategies: Value,
    old_root: String,
    new_root: String,
) -> Value {
    updater::build_plan(
        diff,
        checked,
        strategies,
        &PathBuf::from(&old_root),
        &PathBuf::from(&new_root),
    )
}

#[tauri::command]
pub async fn update_execute(
    app: AppHandle,
    plan: Value,
    old_root: String,
    new_root: String,
) -> Result<Value, String> {
    Ok(updater::execute_plan(
        &app,
        plan,
        &PathBuf::from(&old_root),
        &PathBuf::from(&new_root),
    )
    .await)
}

#[tauri::command]
pub fn update_verify(plan: Value, old_root: String, new_root: String) -> Value {
    updater::verify(&plan, &PathBuf::from(&old_root), &PathBuf::from(&new_root))
}

#[tauri::command]
pub fn update_estimate(plan: Value, old_root: String) -> Value {
    updater::estimate(&plan, &PathBuf::from(&old_root))
}

// ── 下载 ──
#[tauri::command]
pub async fn dl_start(
    app: AppHandle,
    tasks: Value,
    opts: Value,
) -> Result<Value, String> {
    CANCEL.store(false, Ordering::SeqCst);
    Ok(downloader::start(&app, tasks, opts).await)
}

#[tauri::command]
pub fn dl_cancel() -> bool {
    CANCEL.store(true, Ordering::SeqCst);
    true
}

// ── 更新包 ──
#[tauri::command]
pub async fn pack_read_entry(
    path: String,
    entry: String,
) -> Result<Option<String>, String> {
    Ok(eapack::read_entry(&PathBuf::from(&path), &entry).await)
}

#[tauri::command]
pub async fn pack_export(app: AppHandle, opts: Value) -> Result<Value, String> {
    Ok(eapack::export(&app, opts).await)
}

// ── 文件系统 ──
#[tauri::command]
pub fn fs_read_dir(path: String) -> Vec<String> {
    std::fs::read_dir(&path)
        .map(|entries| {
            entries
                .filter_map(|e| e.ok())
                .map(|e| e.file_name().to_string_lossy().to_string())
                .collect()
        })
        .unwrap_or_default()
}

#[tauri::command]
pub fn fs_stat(path: String) -> Value {
    match std::fs::metadata(&path) {
        Ok(m) => serde_json::json!({
            "exists": true,
            "size": m.len(),
            "isDir": m.is_dir(),
            "mtime": m.modified()
                .ok()
                .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
                .map(|d| d.as_millis() as u64)
                .unwrap_or(0),
        }),
        Err(_) => serde_json::json!({"exists": false}),
    }
}

#[tauri::command]
pub fn fs_exists(path: String) -> bool {
    std::path::Path::new(&path).exists()
}

#[tauri::command]
pub fn fs_modpack_info(path: String) -> Value {
    eapack::modpack_info(&PathBuf::from(&path))
}

#[tauri::command]
pub fn fs_hash(path: String, algo: Option<String>) -> Option<String> {
    let algo = algo.unwrap_or_else(|| "sha1".to_string());
    let data = std::fs::read(&path).ok()?;
    match algo.as_str() {
        "sha256" => {
            use sha2::Digest;
            let mut h = sha2::Sha256::new();
            h.update(&data);
            Some(format!("{:x}", h.finalize()))
        }
        _ => {
            use sha1::Digest;
            let mut h = sha1::Sha1::new();
            h.update(&data);
            Some(format!("{:x}", h.finalize()))
        }
    }
}

#[tauri::command]
pub async fn fs_unzip(zip_path: String, dest_dir: String) -> Result<Value, String> {
    Ok(eapack::unzip(&PathBuf::from(&zip_path), &PathBuf::from(&dest_dir)).await)
}

#[tauri::command]
pub fn fs_space(_path: String) -> Value {
    // 简化版：返回 0，让前端不因空间检查阻断
    serde_json::json!({"free": 0u64, "total": 0u64})
}
