// commands.rs — Tauri #[command] 注册（对应 Electron ipcMain.handle）
//
// 注意：Tauri 的 async 命令**不能带引用参数**（带引用就必须返回 Result，
// 且宏展开后会有生命周期问题）。所以这里统一：
//   · 异步命令只收 owned 参数（AppHandle / String / Value）
//   · 取消标志用模块级 static，不用 State<'_, T>

use serde_json::Value;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use tauri::Emitter;
use tauri::AppHandle;

use crate::db;
use crate::differ;
use crate::ridex;
use crate::eapack;
use crate::recent;
use crate::updater;

/// 下载取消标志。RideX 要 Arc 才能跨线程共享，所以这里存一份。
static CANCEL: Mutex<Option<std::sync::Arc<AtomicBool>>> = Mutex::new(None);

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
pub fn db_clear_cache() -> Value {
    db::clear_cache()
}

#[tauri::command]
pub fn db_migrate(src: String, dst: String, skip: Option<Vec<String>>) -> Value {
    db::migrate(
        &PathBuf::from(&src),
        &PathBuf::from(&dst),
        skip.unwrap_or_else(|| vec!["cache".to_string()]),
    )
}

// ── 最近打开 ──
#[tauri::command]
pub fn recent_load() -> Vec<String> {
    recent::load()
}

#[tauri::command]
pub fn recent_add(path: String) -> Vec<String> {
    recent::add(&path)
}

#[tauri::command]
pub fn recent_remove(path: String) -> Vec<String> {
    recent::remove(&path)
}

#[tauri::command]
pub fn recent_clear() -> Vec<String> {
    recent::clear()
}

// ── 比对 ──
#[tauri::command]
pub fn diff_packs(old_root: String, new_root: String, update_pack: Option<bool>) -> Value {
    // update_pack=true：new_root 是增量更新包，旧包里没带到的文件不能判删除
    differ::diff_packs(
        &PathBuf::from(&old_root),
        &PathBuf::from(&new_root),
        update_pack.unwrap_or(false),
    )
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
pub async fn dl_start(app: AppHandle, tasks: Value, opts: Value) -> Result<Value, String> {
    let flag = std::sync::Arc::new(AtomicBool::new(false));
    if let Ok(mut g) = CANCEL.lock() {
        *g = Some(flag.clone());
    }

    let default_dir = std::env::temp_dir();
    let list = ridex::tasks_from_json(&tasks, &default_dir);
    let o = ridex::Options::from_json(&opts);

    // 进度事件（前端目前没监听，但引擎该发就发）
    let app2 = app.clone();
    let prog: ridex::ProgressCb = std::sync::Arc::new(
        move |done, total, name, bytes, total_bytes| {
            let _ = app2.emit(
                "dl:progress",
                serde_json::json!({
                    "done": done, "total": total, "name": name,
                    "bytes": bytes, "totalBytes": total_bytes,
                }),
            );
        },
    );

    let batch = ridex::run_batch(list, &o, flag, Some(prog), None);
    let out = ridex::batch_to_json(&batch);
    let _ = app.emit("dl:done", &out);
    Ok(out)
}

#[tauri::command]
pub fn dl_cancel() -> bool {
    if let Ok(g) = CANCEL.lock() {
        if let Some(f) = g.as_ref() {
            f.store(true, Ordering::SeqCst);
        }
    }
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
pub fn pack_clean_extract(path: String) -> Value {
    eapack::clean_extract(&PathBuf::from(&path))
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

/// 目标路径所在磁盘的 (可用, 总量)。路径不存在时往上找到第一个存在的祖先。
#[cfg(unix)]
fn disk_space(p: &std::path::Path) -> Option<(u64, u64)> {
    use std::ffi::CString;
    use std::os::unix::ffi::OsStrExt;
    let c = CString::new(p.as_os_str().as_bytes()).ok()?;
    let mut st: libc::statvfs = unsafe { std::mem::zeroed() };
    if unsafe { libc::statvfs(c.as_ptr(), &mut st) } != 0 {
        return None;
    }
    let bs = st.f_frsize as u64;
    Some((st.f_bavail as u64 * bs, st.f_blocks as u64 * bs))
}

#[cfg(windows)]
fn disk_space(p: &std::path::Path) -> Option<(u64, u64)> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::GetDiskFreeSpaceExW;
    let mut w: Vec<u16> = p.as_os_str().encode_wide().collect();
    w.push(0);
    let (mut free, mut total) = (0u64, 0u64);
    let ok = unsafe {
        GetDiskFreeSpaceExW(w.as_ptr(), &mut free, &mut total, std::ptr::null_mut())
    };
    if ok == 0 {
        None
    } else {
        Some((free, total))
    }
}

#[cfg(not(any(unix, windows)))]
fn disk_space(_p: &std::path::Path) -> Option<(u64, u64)> {
    None
}

#[tauri::command]
pub fn fs_space(path: String) -> Value {
    let mut p = PathBuf::from(&path);
    while !p.exists() {
        match p.parent() {
            Some(parent) => p = parent.to_path_buf(),
            None => break,
        }
    }
    match disk_space(&p) {
        Some((free, total)) => serde_json::json!({"free": free, "total": total}),
        None => serde_json::json!({"free": 0u64, "total": 0u64}),
    }
}
