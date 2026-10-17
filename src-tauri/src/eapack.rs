// eapack.rs — 更新包读取/导出/解压/整合包信息
// 对应 Python: eapack.py + pack_info.py

use serde_json::{json, Value};
use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use tauri::{AppHandle, Emitter};
use zip::ZipArchive;

/// 读取 zip 内某个条目的文本内容
pub async fn read_entry(zip_path: &Path, entry_name: &str) -> Option<String> {
    let file = fs::File::open(zip_path).ok()?;
    let mut archive = ZipArchive::new(file).ok()?;
    let mut entry = archive.by_name(entry_name).ok()?;
    let mut text = String::new();
    entry.read_to_string(&mut text).ok()?;
    Some(text)
}

/// 把 zip 条目名净化成「解压目录内的相对路径」。
///
/// 返回 None 表示这个条目必须跳过。拦三类：
///   · 绝对路径（`/etc/x`）和盘符（`C:\\x`）
///   · 含 `..` 路径段（zip slip：`../../x` 能写到解压目录之外）
///   · 净化后为空
fn safe_entry_name(name: &str) -> Option<PathBuf> {
    let n = name.replace('\\', "/");
    if n.starts_with('/') || n.contains(':') {
        return None;
    }
    let mut out = PathBuf::new();
    for seg in n.split('/') {
        if seg.is_empty() || seg == "." {
            continue;
        }
        if seg == ".." {
            return None;
        }
        out.push(seg);
    }
    if out.as_os_str().is_empty() {
        None
    } else {
        Some(out)
    }
}

/// 解压 zip 到目录
pub async fn unzip(zip_path: &Path, dest_dir: &Path) -> Value {
    let file = match fs::File::open(zip_path) {
        Ok(f) => f,
        Err(e) => return json!({"ok": false, "msg": e.to_string()}),
    };
    let mut archive = match ZipArchive::new(file) {
        Ok(a) => a,
        Err(e) => return json!({"ok": false, "msg": e.to_string()}),
    };

    let _ = fs::create_dir_all(dest_dir);
    let mut count = 0usize;
    let mut skipped = 0usize;

    for i in 0..archive.len() {
        let mut entry = match archive.by_index(i) {
            Ok(e) => e,
            Err(_) => continue,
        };
        let name = entry.name().to_string();
        // 路径净化：不干净的条目直接跳过，绝不写到解压目录之外
        let outpath = match safe_entry_name(&name) {
            Some(rel) => dest_dir.join(rel),
            None => {
                skipped += 1;
                continue;
            }
        };

        if entry.is_dir() {
            let _ = fs::create_dir_all(&outpath);
        } else {
            if let Some(parent) = outpath.parent() {
                let _ = fs::create_dir_all(parent);
            }
            let mut data = Vec::new();
            if entry.read_to_end(&mut data).is_ok() {
                if fs::write(&outpath, &data).is_ok() {
                    count += 1;
                }
            }
        }
    }

    json!({"ok": true, "files": count, "skipped": skipped})
}

/// 读取整合包信息
pub fn modpack_info(root: &Path) -> Value {
    let name = root.file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    let mut info = json!({
        "name": name,
        "version": "",
        "mods": 0,
        "icon": null,
    });

    // mmc-pack.json / manifest.json
    let mut named = false;
    for f in &["mmc-pack.json", "manifest.json"] {
        let p = root.join(f);
        if let Ok(text) = fs::read_to_string(&p) {
            if let Ok(data) = serde_json::from_str::<Value>(&text) {
                if let Some(n) = data.get("name").and_then(|v| v.as_str()) {
                    if !n.is_empty() {
                        info["name"] = Value::String(n.to_string());
                        named = true;
                    }
                }
                if let Some(v) = data.get("version").and_then(|v| v.as_str()) {
                    info["version"] = Value::String(v.to_string());
                }
            }
        }
    }

    // instance.cfg (MMC) —— 只有在 mmc-pack/manifest 没给名字时才用。
    // （以前写成「目录名为空才用」，而目录名永远非空，所以这段是死代码）
    if !named {
        if let Ok(text) = fs::read_to_string(root.join("instance.cfg")) {
            for line in text.lines() {
                if let Some(eq) = line.find('=') {
                    if &line[..eq] == "name" {
                        let v = line[eq + 1..].trim();
                        if !v.is_empty() {
                            info["name"] = Value::String(v.to_string());
                        }
                    }
                }
            }
        }
    }

    // mod 数量
    let mods_dir = root.join("mods");
    if mods_dir.exists() {
        if let Ok(entries) = fs::read_dir(&mods_dir) {
            let count = entries
                .filter_map(|e| e.ok())
                .filter(|e| {
                    e.file_name().to_string_lossy().ends_with(".jar")
                })
                .count();
            info["mods"] = json!(count);
        }
    }

    // icon
    for f in &["icon.png", "server-icon.png", "pack.png"] {
        if root.join(f).exists() {
            info["icon"] = Value::String(f.to_string());
            break;
        }
    }

    info
}

/// 导出更新包
pub async fn export(app: &AppHandle, opts: Value) -> Value {
    let root = match opts.get("root").and_then(|v| v.as_str()) {
        Some(r) => PathBuf::from(r),
        None => return json!({"ok": false, "msg": "missing root"}),
    };
    let output = opts.get("output")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| "update.zip".to_string());

    let file = match fs::File::create(&output) {
        Ok(f) => f,
        Err(e) => return json!({"ok": false, "msg": e.to_string()}),
    };

    let mut zip = zip::ZipWriter::new(file);
    let zopt = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated);

    // 写 JSON 条目（key → 文件名）
    for (key, name) in [
        ("manifest", "manifest.json"),
        ("settings", "settings.json"),
        ("hashes", "hashes.json"),
        ("strategies", "strategies.json"),
        ("whitelist", "whitelist.json"),
    ] {
        if let Some(v) = opts.get(key) {
            if !v.is_null() {
                let text = serde_json::to_string_pretty(v).unwrap_or_default();
                let _ = zip.start_file(name, zopt);
                let _ = zip.write_all(text.as_bytes());
            }
        }
    }

    // CHANGELOG.md
    if let Some(changelog) = opts.get("changelog").and_then(|v| v.as_str()) {
        let _ = zip.start_file("CHANGELOG.md", zopt);
        let _ = zip.write_all(changelog.as_bytes());
    }

    // 打包文件内容；找不到的必须报出来，不能静默跳过
    let mut missing: Vec<String> = Vec::new();
    if let Some(files) = opts.get("files").and_then(|v| v.as_array()) {
        for f in files {
            if let Some(rel) = f.as_str() {
                let fp = root.join(rel);
                if fp.is_file() {
                    match fs::read(&fp) {
                        Ok(data) => {
                            let _ = zip.start_file(rel, zopt);
                            let _ = zip.write_all(&data);
                        }
                        Err(_) => missing.push(rel.to_string()),
                    }
                } else {
                    missing.push(rel.to_string());
                }
            }
        }
    }

    if let Err(e) = zip.finish() {
        return json!({"ok": false, "msg": e.to_string()});
    }

    let size = fs::metadata(&output).map(|m| m.len()).unwrap_or(0);
    let _ = app.emit("pack:progress", json!({"done": true, "path": &output}));
    json!({
        "ok": missing.is_empty(),
        "path": output,
        "size": size,
        "missing": missing,
    })
}

/// 删除更新包的解压目录。
///
/// 只允许删名字以 `_pulses_extract` 结尾的目录 —— 防止被当成通用删目录接口。
/// 解压前先删一次（否则上一次的残留文件会被当成「新增」拷进玩家整合包），
/// 更新成功后再删一次（不占磁盘）。
pub fn clean_extract(path: &Path) -> Value {
    let name = path
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .unwrap_or_default();
    if !name.ends_with("_pulses_extract") {
        return json!({"ok": false, "msg": "不是解压目录"});
    }
    if !path.exists() {
        return json!({"ok": true, "removed": false});
    }
    match fs::remove_dir_all(path) {
        Ok(_) => json!({"ok": true, "removed": true}),
        Err(e) => json!({"ok": false, "msg": e.to_string()}),
    }
}
