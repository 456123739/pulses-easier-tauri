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
