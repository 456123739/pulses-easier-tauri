// db.rs — 数据库路径与配置管理
// 对应 Python: database.py + dbmigrate.py

use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// 配置目录。
///
/// 绿色版优先：exe 同级能写就用 `<exe 同级>/data`，
/// 这样解压到哪就在哪存配置，不碰用户目录、不写注册表。
/// 装在 Program Files（不可写）时自动退回 ~/.pulses_easier。
pub fn app_dir() -> PathBuf {
    if let Some(d) = portable_dir() {
        return d;
    }
    dirs::home_dir().unwrap_or_default().join(".pulses_easier")
}

fn portable_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let base = exe.parent()?;

    // 显式标记优先
    if base.join("portable.txt").exists() {
        let d = base.join("data");
        if std::fs::create_dir_all(&d).is_ok() {
            return Some(d);
        }
        return None;
    }

    // 没有标记：同级能建 data 就当绿色版（安装到 Program Files 会失败）
    let d = base.join("data");
    if d.is_dir() {
        return Some(d);
    }
    if std::fs::create_dir_all(&d).is_ok() {
        return Some(d);
    }
    None
}

fn app_cfg_path() -> PathBuf {
    app_dir().join("config.json")
}

fn read_json(path: &Path) -> Option<Value> {
    let text = fs::read_to_string(path).ok()?;
    serde_json::from_str(&text).ok()
}

fn atomic_write_json(path: &Path, val: &Value) -> bool {
    let _ = fs::create_dir_all(path.parent().unwrap_or(Path::new(".")));
    let text = serde_json::to_string_pretty(val).unwrap_or_default();
    let tmp = path.with_extension("json.tmp");
    if fs::write(&tmp, &text).is_err() { return false; }
    fs::rename(&tmp, path).is_ok()
}

fn default_config() -> Value {
    json!({
        "download": {
            "multi_slots": 16, "single_slots": 4,
            "part_threads": 4, "max_connections": 128,
            "connect_timeout": 10, "read_idle_timeout": 20,
        },
        "compare": { "hash_algo": "sha1", "hash_threads": 4 },
        "ui": { "reduce_motion": false, "theme": "dark" },
        "whitelist": ["mods", "resourcepacks", "shaderpacks"],
        "scan_list": ["mods", "resourcepacks", "shaderpacks"],
        "blacklist": [],
        "export_options": {
            "include_hashes": true, "tamper_proof": true,
            "compress": true,
        },
    })
}

pub fn get_db_path() -> Option<String> {
    let cfg = read_json(&app_cfg_path())?;
    cfg.get("db_path")?.as_str().map(|s| s.to_string())
}

pub fn load_config() -> Option<Value> {
    let app_cfg = read_json(&app_cfg_path())?;
    let db_path = app_cfg.get("db_path")?.as_str()?;
    let db_cfg = read_json(&PathBuf::from(db_path).join("config.json"))
        .unwrap_or_else(default_config);
    let mut merged = db_cfg;
    if let Some(obj) = merged.as_object_mut() {
        obj.insert("db_path".to_string(), Value::String(db_path.to_string()));
    }
    Some(merged)
}

pub fn save_config(cfg: Value) -> bool {
    let app_cfg = match read_json(&app_cfg_path()) {
        Some(c) => c,
        None => return false,
    };
    let db_path = match app_cfg.get("db_path").and_then(|v| v.as_str()) {
        Some(p) => p,
        None => return false,
    };
    atomic_write_json(&PathBuf::from(db_path).join("config.json"), &cfg)
}

pub fn create_database(db_path: &Path) -> bool {
    let _ = fs::create_dir_all(db_path.join("projects"));
    let cfg_file = db_path.join("config.json");
    if !cfg_file.exists() {
        let mut init = default_config();
        if let Some(obj) = init.as_object_mut() {
            obj.insert("format_version".to_string(), json!(1));
            obj.insert("created".to_string(), json!(now_iso()));
        }
        let _ = atomic_write_json(&cfg_file, &init);
    }
    let mut app_cfg = read_json(&app_cfg_path()).unwrap_or(json!({}));
    if let Some(obj) = app_cfg.as_object_mut() {
        obj.insert("db_path".to_string(),
            Value::String(db_path.to_string_lossy().to_string()));
    }
    atomic_write_json(&app_cfg_path(), &app_cfg)
}

/// 当前时间（UTC，秒级 ISO 风格，避免引入 chrono）
fn now_iso() -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("unix:{}", secs)
}

pub fn is_valid_database(db_path: &Path) -> bool {
    db_path.join("config.json").exists() && db_path.join("projects").exists()
}

pub fn describe(db_path: &Path) -> Value {
    let mut files = 0u64;
    let mut bytes = 0u64;
    let mut projects = 0u64;
    let mut cache_bytes = 0u64;

    if let Ok(entries) = fs::read_dir(db_path.join("projects")) {
        for e in entries.flatten() {
            if e.metadata().map(|m| m.is_dir()).unwrap_or(false) {
                projects += 1;
            }
        }
    }

    for entry in WalkDir::new(db_path).into_iter().flatten() {
        if entry.file_type().is_file() {
            let size = entry.metadata().map(|m| m.len()).unwrap_or(0);
            files += 1;
            bytes += size;
            // 判断是否在 cache 目录下
            let path_str = entry.path().to_string_lossy();
            if path_str.contains("/cache/") || path_str.contains("\\cache\\") {
                cache_bytes += size;
            }
        }
    }

    json!({
        "files": files,
        "bytes": bytes,
        "projects": projects,
        "cache_bytes": cache_bytes,
    })
}

pub fn migrate(src: &Path, dst: &Path, skip: Vec<String>) -> Value {
    let _ = fs::create_dir_all(dst.join("projects"));
    fn copy_dir(s: &Path, d: &Path, skip: &[String], depth: usize) -> Result<(), String> {
        fs::create_dir_all(d).map_err(|e| e.to_string())?;
        for entry in fs::read_dir(s).map_err(|e| e.to_string())?.flatten() {
            let name = entry.file_name().to_string_lossy().to_string();
            if depth == 0 && skip.contains(&name) { continue; }
            let sp = entry.path();
            let dp = d.join(&name);
            let ft = entry.file_type().map_err(|e| e.to_string())?;
            if ft.is_dir() {
                copy_dir(&sp, &dp, skip, depth + 1)?;
            } else {
                fs::copy(&sp, &dp).map_err(|e| e.to_string())?;
                // 校验大小
                let ss = fs::metadata(&sp).map_err(|e| e.to_string())?.len();
                let ds = fs::metadata(&dp).map_err(|e| e.to_string())?.len();
                if ss != ds { return Err(format!("size mismatch: {}", name)); }
            }
        }
        Ok(())
    }
    match copy_dir(src, dst, &skip, 0) {
        Ok(()) => json!({"ok": true, "msg": "迁移完成"}),
        Err(e) => json!({"ok": false, "msg": e}),
    }
}
