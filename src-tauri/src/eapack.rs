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

/// 读 zip 里某个条目的开头若干字节（探活用，不解压整个包）
fn read_head(ar: &mut ZipArchive<fs::File>, name: &str, cap: usize) -> Option<String> {
    let mut entry = ar.by_name(name).ok()?;
    let mut buf = vec![0u8; cap];
    let n = entry.read(&mut buf).ok()?;
    buf.truncate(n);
    Some(String::from_utf8_lossy(&buf).to_string())
}

/// 所有条目共享的顶层目录（有就返回 "X/"，没有返回 ""）。
/// 7-Zip「压缩到文件夹」会多套一层，识别时得先剥掉。
pub fn root_prefix(names: &[String]) -> String {
    let mut top: Option<&str> = None;
    for n in names {
        let first = match n.split('/').next() {
            Some(f) if !f.is_empty() => f,
            _ => continue,
        };
        // 根目录下的文件（没有子目录）说明没有统一顶层
        if !n.contains('/') {
            return String::new();
        }
        match top {
            None => top = Some(first),
            Some(t) if t != first => return String::new(),
            _ => {}
        }
    }
    match top {
        Some(t) => format!("{t}/"),
        None => String::new(),
    }
}

/// 认一下这个 zip 是「整合包」还是「更新包」。
///
/// 只看条目名和几个小 JSON，**不解压** —— 拖进来就能立刻给结论。
///
/// 判据（从强到弱）：
///   modrinth.index.json            → Modrinth 整合包
///   mmc-pack.json                  → Prism / MultiMC 整合包
///   manifest.json 里有 minecraft   → CurseForge 整合包
///   settings/hashes/strategies     → 我们自己的更新包
///   manifest.json（没有 minecraft）→ 我们自己的更新包
///   overrides/ 或 mods/            → 整合包（认不出加载器，得问）
///   其余                            → 更新包
pub fn pack_kind(zip_path: &Path) -> Value {
    let file = match fs::File::open(zip_path) {
        Ok(f) => f,
        Err(e) => return json!({ "ok": false, "error": format!("打不开文件：{e}") }),
    };
    let mut ar = match ZipArchive::new(file) {
        Ok(a) => a,
        Err(e) => return json!({ "ok": false, "error": format!("不是有效的 zip：{e}") }),
    };

    let mut names: Vec<String> = Vec::with_capacity(ar.len());
    for i in 0..ar.len() {
        if let Some(n) = ar.name_for_index(i) {
            names.push(n.to_string());
        }
    }
    let root = root_prefix(&names);
    let has = |rel: &str| names.iter().any(|n| n == &format!("{root}{rel}"));

    // ── 1. Modrinth ──
    if has("modrinth.index.json") {
        if let Some(t) = read_head(&mut ar, &format!("{root}modrinth.index.json"), 65536) {
            if let Ok(v) = serde_json::from_str::<Value>(&t) {
                let deps = v.get("dependencies").cloned().unwrap_or(Value::Null);
                let g = |k: &str| deps.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
                let (loader, ver) = if !g("neoforge").is_empty() {
                    ("neoforge".to_string(), g("neoforge"))
                } else if !g("forge").is_empty() {
                    ("forge".to_string(), g("forge"))
                } else if !g("fabric-loader").is_empty() {
                    ("fabric".to_string(), g("fabric-loader"))
                } else if !g("quilt-loader").is_empty() {
                    ("quilt".to_string(), g("quilt-loader"))
                } else {
                    ("vanilla".to_string(), String::new())
                };
                return json!({
                    "ok": true, "kind": "modpack", "format": "modrinth",
                    "name": v.get("name").and_then(|x| x.as_str()).unwrap_or(""),
                    "version": v.get("versionId").and_then(|x| x.as_str()).unwrap_or(""),
                    "mc": g("minecraft"), "loader": loader, "loaderVer": ver,
                });
            }
        }
        return json!({ "ok": true, "kind": "modpack", "format": "modrinth" });
    }

    // ── 2. Prism / MultiMC ──
    if has("mmc-pack.json") {
        if let Some(t) = read_head(&mut ar, &format!("{root}mmc-pack.json"), 65536) {
            if let Ok(v) = serde_json::from_str::<Value>(&t) {
                let comps = v.get("components").and_then(|x| x.as_array()).cloned().unwrap_or_default();
                let find = |uid: &str| -> String {
                    comps.iter()
                        .find(|c| c.get("uid").and_then(|x| x.as_str()) == Some(uid))
                        .and_then(|c| c.get("version"))
                        .and_then(|x| x.as_str())
                        .unwrap_or("")
                        .to_string()
                };
                let mc = find("net.minecraft");
                let mut loader = "vanilla".to_string();
                let mut ver = String::new();
                for (uid, l) in [
                    ("net.minecraftforge", "forge"),
                    ("net.neoforged", "neoforge"),
                    ("net.fabricmc.fabric-loader", "fabric"),
                    ("org.quiltmc.quilt-loader", "quilt"),
                ] {
                    let vv = find(uid);
                    if !vv.is_empty() {
                        loader = l.to_string();
                        ver = vv;
                        break;
                    }
                }
                return json!({
                    "ok": true, "kind": "modpack", "format": "prism",
                    "name": "", "version": "", "mc": mc,
                    "loader": loader, "loaderVer": ver,
                });
            }
        }
        return json!({ "ok": true, "kind": "modpack", "format": "prism" });
    }

    // ── 3. CurseForge ──
    if has("manifest.json") {
        if let Some(t) = read_head(&mut ar, &format!("{root}manifest.json"), 262144) {
            if let Ok(v) = serde_json::from_str::<Value>(&t) {
                if let Some(mc) = v
                    .get("minecraft")
                    .and_then(|m| m.get("version"))
                    .and_then(|x| x.as_str())
                {
                    let loaders = v
                        .get("minecraft")
                        .and_then(|m| m.get("modLoaders"))
                        .and_then(|x| x.as_array())
                        .cloned()
                        .unwrap_or_default();
                    let primary = loaders
                        .iter()
                        .find(|l| l.get("primary").and_then(|p| p.as_bool()).unwrap_or(false))
                        .or_else(|| loaders.first());
                    let (loader, ver) = primary
                        .and_then(|l| l.get("id").and_then(|x| x.as_str()))
                        .map(crate::mcinstall::split_loader_id)
                        .unwrap_or(("vanilla".to_string(), String::new()));
                    return json!({
                        "ok": true, "kind": "modpack", "format": "curseforge",
                        "name": v.get("name").and_then(|x| x.as_str()).unwrap_or(""),
                        "version": v.get("version").and_then(|x| x.as_str()).unwrap_or(""),
                        "mc": mc, "loader": loader, "loaderVer": ver,
                    });
                }
            }
        }
    }

    // ── 4. 我们自己的更新包 ──
    let pulses = ["settings.json", "hashes.json", "strategies.json", "whitelist.json"]
        .iter()
        .any(|f| has(f));
    if pulses || has("manifest.json") || has("CHANGELOG.md") {
        let mut name = String::new();
        let mut version = String::new();
        if let Some(t) = read_head(&mut ar, &format!("{root}manifest.json"), 8192) {
            if let Ok(v) = serde_json::from_str::<Value>(&t) {
                name = v.get("name").and_then(|x| x.as_str()).unwrap_or("").to_string();
                version = v.get("version").and_then(|x| x.as_str()).unwrap_or("").to_string();
            }
        }
        return json!({
            "ok": true, "kind": "update", "format": "pulses",
            "name": name, "version": version,
        });
    }

    // ── 5. 什么元信息都没有：按目录结构猜 ──
    if names.iter().any(|n| n.starts_with(&format!("{root}overrides/")))
        || names.iter().any(|n| n == &format!("{root}mods/"))
        || names.iter().any(|n| n.starts_with(&format!("{root}mods/")))
    {
        return json!({ "ok": true, "kind": "modpack", "format": "unknown" });
    }

    json!({ "ok": true, "kind": "update", "format": "unknown" })
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
