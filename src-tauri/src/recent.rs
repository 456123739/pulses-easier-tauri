// recent.rs — 最近打开的整合包记录
// 对应 Python: app/core/recent.py
// 存储：~/.pulses_easier/recent.json（上限 8 条，去重，最新在前）

use std::fs;
use std::path::PathBuf;

const MAX_ITEMS: usize = 8;

fn config_dir() -> PathBuf {
    dirs::home_dir().unwrap_or_default().join(".pulses_easier")
}

fn recent_file() -> PathBuf {
    config_dir().join("recent.json")
}

fn save(items: &[String]) -> bool {
    let _ = fs::create_dir_all(config_dir());
    let text = serde_json::to_string_pretty(items).unwrap_or_else(|_| "[]".into());
    fs::write(recent_file(), text).is_ok()
}

pub fn load() -> Vec<String> {
    let text = match fs::read_to_string(recent_file()) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    serde_json::from_str::<Vec<String>>(&text).unwrap_or_default()
}

pub fn add(path: &str) -> Vec<String> {
    let mut items = load();
    items.retain(|p| p != path);
    items.insert(0, path.to_string());
    items.truncate(MAX_ITEMS);
    let _ = save(&items);
    items
}

pub fn remove(path: &str) -> Vec<String> {
    let mut items = load();
    items.retain(|p| p != path);
    let _ = save(&items);
    items
}

pub fn clear() -> Vec<String> {
    let _ = save(&[]);
    Vec::new()
}
