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

    for i in 0..archive.len() {
        let mut entry = match archive.by_index(i) {
            Ok(e) => e,
            Err(_) => continue,
        };
        let name = entry.name().to_string();
        let outpath = dest_dir.join(&name);

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

    json!({"ok": true, "files": count})
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
    for f in &["mmc-pack.json", "manifest.json"] {
        let p = root.join(f);
        if let Ok(text) = fs::read_to_string(&p) {
            if let Ok(data) = serde_json::from_str::<Value>(&text) {
                if let Some(n) = data.get("name").and_then(|v| v.as_str()) {
                    if !n.is_empty() {
                        info["name"] = Value::String(n.to_string());
                    }
                }
                if let Some(v) = data.get("version").and_then(|v| v.as_str()) {
                    info["version"] = Value::String(v.to_string());
                }
            }
        }
    }

    // instance.cfg (MMC)
    if let Ok(text) = fs::read_to_string(root.join("instance.cfg")) {
        for line in text.lines() {
            if let Some(eq) = line.find('=') {
                let key = &line[..eq];
                let val = &line[eq + 1..];
                if key == "name" && info["name"].as_str().unwrap_or("").is_empty() {
                    info["name"] = Value::String(val.to_string());
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

    // 打包文件内容
    if let Some(files) = opts.get("files").and_then(|v| v.as_array()) {
        for f in files {
            if let Some(rel) = f.as_str() {
                let fp = root.join(rel);
                if fp.is_file() {
                    if let Ok(data) = fs::read(&fp) {
                        let _ = zip.start_file(rel, zopt);
                        let _ = zip.write_all(&data);
                    }
                }
            }
        }
    }

    if let Err(e) = zip.finish() {
        return json!({"ok": false, "msg": e.to_string()});
    }

    let size = fs::metadata(&output).map(|m| m.len()).unwrap_or(0);
    let _ = app.emit("pack:progress", json!({"done": true, "path": &output}));
    json!({"ok": true, "path": output, "size": size})
}
