// differ.rs — 文件差异比对
// 对应 Python: differ.py

use serde_json::{json, Value};
use std::collections::HashMap;
use std::path::Path;
use walkdir::WalkDir;

pub struct Entry {
    pub rel: String,
    pub size: u64,
    pub mtime: u64,
}

/// 玩家本地文件：更新时一律不碰（不加、不覆盖、不删除）。
///
/// 注意 `options.txt` **不在此列** —— 它要参与更新（作者调的 FOV / 渲染距离 /
/// GUI 缩放必须能下来），但写入时走 updater.rs 的 `merge_options`：
/// 保留玩家所有 `key_` 开头的键位行，其余用新包的。
///
/// `saves/` 也不在此列：整合包每版都自带存档，要跟着更新。
const LOCAL_ONLY_FILES: &[&str] = &[
    "optionsof.txt",          // OptiFine 画质
    "servers.dat",            // 服务器列表
    "usercache.json",
    "launcher_profiles.json",
];

const LOCAL_ONLY_DIRS: &[&str] = &[
    "logs",
    "crash-reports",
    "screenshots",
];

/// rel 是包根相对的 posix 路径。
fn is_local_only(rel: &str) -> bool {
    let top = rel.split('/').next().unwrap_or("");
    if LOCAL_ONLY_DIRS.contains(&top) {
        return true;
    }
    // 只对根目录散文件按名字排除（config/options.txt 之类不算）
    !rel.contains('/') && LOCAL_ONLY_FILES.contains(&rel)
}

fn folder_manifest(dir: &Path) -> Vec<Entry> {
    if !dir.exists() {
        return Vec::new();
    }
    let mut entries = Vec::new();
    for entry in WalkDir::new(dir).into_iter().flatten() {
        if !entry.file_type().is_file() {
            continue;
        }
        let rel = entry.path()
            .strip_prefix(dir)
            .unwrap_or(entry.path())
            .to_string_lossy()
            .replace('\\', "/")
            .to_string();
        if rel.is_empty() {
            continue;
        }
        // 玩家本地文件永不参与比对 —— 既不会被复制，也不会被判删除
        if is_local_only(&rel) {
            continue;
        }
        let md = entry.metadata().ok();
        let size = md.as_ref().map(|m| m.len()).unwrap_or(0);
        let mtime = md
            .and_then(|m| m.modified().ok())
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_millis() as u64)
            .unwrap_or(0);
        entries.push(Entry { rel, size, mtime });
    }
    entries
}

pub fn diff_packs(old_root: &Path, new_root: &Path) -> Value {
    let old_entries = folder_manifest(old_root);
    let new_entries = folder_manifest(new_root);

    let mut old_map: HashMap<String, &Entry> = HashMap::new();
    for e in &old_entries {
        old_map.insert(e.rel.clone(), e);
    }
    let mut new_map: HashMap<String, &Entry> = HashMap::new();
    for e in &new_entries {
        new_map.insert(e.rel.clone(), e);
    }

    let mut added = Vec::new();
    let mut modified = Vec::new();
    let mut deleted = Vec::new();
    let mut unchanged = Vec::new();

    for (rel, ne) in &new_map {
        match old_map.get(rel) {
            None => added.push(json!({"rel": rel, "size": ne.size})),
            Some(oe) => {
                if oe.size != ne.size || oe.mtime != ne.mtime {
                    modified.push(json!({
                        "rel": rel, "size": ne.size, "oldSize": oe.size
                    }));
                } else {
                    unchanged.push(json!({"rel": rel, "size": ne.size}));
                }
            }
        }
    }
    for (rel, oe) in &old_map {
        if !new_map.contains_key(rel) {
            deleted.push(json!({"rel": rel, "size": oe.size}));
        }
    }

    json!({
        "added": added,
        "modified": modified,
        "deleted": deleted,
        "unchanged": unchanged,
    })
}
