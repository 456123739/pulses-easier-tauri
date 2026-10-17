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

/// 更新包**自身**的元数据（导出时写进去的），不是整合包内容。
/// 只在「这一侧是增量更新包」时才排除 —— 玩家的完整整合包可能自带
/// CurseForge 的 manifest.json，那是真内容，不能动。
const PACK_META_FILES: &[&str] = &[
    "manifest.json",
    "settings.json",
    "hashes.json",
    "strategies.json",
    "whitelist.json",
    "CHANGELOG.md",
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

fn folder_manifest(dir: &Path, is_update_pack: bool) -> Vec<Entry> {
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
        // 更新包自己的元数据也不参与，免得被当成内容复制进玩家目录
        if is_update_pack && !rel.contains('/') && PACK_META_FILES.contains(&rel.as_str()) {
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

/// 文件内容的 sha1（只用于判等，不做安全用途）
fn file_sha1(p: &Path) -> Option<String> {
    use sha1::{Digest, Sha1};
    use std::io::Read;
    let mut f = std::fs::File::open(p).ok()?;
    let mut h = Sha1::new();
    let mut buf = vec![0u8; 1 << 20];
    loop {
        let n = f.read(&mut buf).ok()?;
        if n == 0 {
            break;
        }
        h.update(&buf[..n]);
    }
    Some(format!("{:x}", h.finalize()))
}

/// 两边内容是否一致。
///
/// **不能只比 mtime** —— 会双向误判：
///   · 内容一样但 mtime 变了（解压/复制）→ 误判「修改」，每次都全量覆盖
///   · 大小和 mtime 都一样但内容变了     → 漏判，永远不更新
/// 先比大小（最便宜），大小相同才读内容算 sha1。
fn same_content(a: &Path, b: &Path) -> bool {
    match (file_sha1(a), file_sha1(b)) {
        (Some(x), Some(y)) => x == y,
        _ => false,
    }
}

/// 差异比对。
///
/// `update_pack = true` 表示 new_root 是**增量更新包**（只带变更文件）。
/// 这种情况下旧包里没被带到的文件**绝不能判删除** —— 否则玩家的整合包会被删空。
pub fn diff_packs(old_root: &Path, new_root: &Path, update_pack: bool) -> Value {
    let old_entries = folder_manifest(old_root, false);
    let new_entries = folder_manifest(new_root, update_pack);

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

    // 排序后再遍历：HashMap 的迭代顺序每次都不一样，
    // 不排序的话变更列表每次刷新都在乱跳。
    let mut new_keys: Vec<&String> = new_map.keys().collect();
    new_keys.sort();

    for rel in new_keys {
        let ne = new_map[rel];
        match old_map.get(rel) {
            None => added.push(json!({"rel": rel, "size": ne.size})),
            Some(oe) => {
                if oe.size != ne.size {
                    modified.push(json!({
                        "rel": rel, "size": ne.size, "oldSize": oe.size
                    }));
                } else if same_content(&old_root.join(rel), &new_root.join(rel)) {
                    unchanged.push(json!({"rel": rel, "size": ne.size}));
                } else {
                    modified.push(json!({
                        "rel": rel, "size": ne.size, "oldSize": oe.size
                    }));
                }
            }
        }
    }

    if !update_pack {
        // 根目录散文件（options.txt / servers.dat 之类）永不判删除 ——
        // 那是玩家的本地配置，更新包里没有不代表该删。
        let mut old_keys: Vec<&String> = old_map
            .keys()
            .filter(|k| !new_map.contains_key(*k))
            .filter(|k| k.contains('/'))
            .collect();
        old_keys.sort();
        for rel in old_keys {
            deleted.push(json!({"rel": rel, "size": old_map[rel].size}));
        }
    }

    json!({
        "added": added,
        "modified": modified,
        "deleted": deleted,
        "unchanged": unchanged,
        "updatePack": update_pack,
    })
}
