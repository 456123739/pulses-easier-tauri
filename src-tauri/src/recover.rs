// recover.rs — 更新未完成凭证 + 就地自检
// 对应 Python: app/core/pending.py + checkpoint.py + recover.py
//
// 三块内容：
//
// 一、**更新未完成凭证**（写进整合包，下次定位时能认出来）
//     <整合包>/!!!更新未完成-请阅读!!!.txt       给人看，`!` 开头排在文件管理器最前
//     <整合包>/.pulses_easier/pending_update.json 给软件看
//     这两个名字都在 differ 的 LOCAL_ONLY 名单里，**永不参与更新的比对/删除**。
//
// 二、**检查点**（判断上一次更新是否正常结束）
//     <db>/cache/checkpoints/<pack_hash>.json
//     进程被强杀时检查点会停在 apply 阶段，下次能提示「重新拖入同一个更新包」。
//     不做自动回滚 —— 那需要假设「更新后玩家什么都没改过」，没法验证，
//     反而会把版本搞乱。
//
// 三、**就地自检**（强杀/断电后磁盘上的残渣）
//     .xxx.pulses_tmp  目录整体替换时旧内容的副本，还在就说明替换没跑完
//     .yyy.pulses_new  文件搬运的临时文件，目标端只会被原子覆盖，一律可删
//     原则：宁可多跑一次更新，也不让玩家丢东西。

use serde_json::{json, Value};
use std::fs;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use crate::transfer;

// ═══════════════════════════════════════════════════════════════
// 凭证
// ═══════════════════════════════════════════════════════════════

pub const MARKER_DIR: &str = ".pulses_easier";
pub const MARKER_FILE: &str = "pending_update.json";
pub const NOTICE_FILENAME: &str = "!!!更新未完成-请阅读!!!.txt";
pub const MARKER_FORMAT_VERSION: u32 = 1;
pub const MAX_ENTRIES: usize = 500;

pub fn marker_dir(pack_root: &Path) -> PathBuf {
    pack_root.join(MARKER_DIR)
}

pub fn marker_path(pack_root: &Path) -> PathBuf {
    marker_dir(pack_root).join(MARKER_FILE)
}

pub fn notice_path(pack_root: &Path) -> PathBuf {
    pack_root.join(NOTICE_FILENAME)
}

fn now_unix() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

/// `2026-10-25 06:51:46`（UTC，够用；不引 chrono 只为这一处）
fn now_text() -> String {
    let secs = now_unix() as u64;
    let days = secs / 86400;
    let rem = secs % 86400;
    let (h, m, s) = (rem / 3600, (rem % 3600) / 60, rem % 60);
    // 1970-01-01 起的天数转年月日
    let mut y = 1970i64;
    let mut d = days as i64;
    loop {
        let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
        let n = if leap { 366 } else { 365 };
        if d < n {
            break;
        }
        d -= n;
        y += 1;
    }
    let leap = (y % 4 == 0 && y % 100 != 0) || y % 400 == 0;
    let months = [
        31,
        if leap { 29 } else { 28 },
        31, 30, 31, 30, 31, 31, 30, 31, 30, 31,
    ];
    let mut mo = 0usize;
    while mo < 12 && d >= months[mo] {
        d -= months[mo];
        mo += 1;
    }
    format!(
        "{y:04}-{:02}-{:02} {h:02}:{m:02}:{s:02}",
        mo + 1,
        d + 1
    )
}

/// 构造凭证。`missing` 里每项形如 `{rel, size, sha1, urls}`。
pub fn build_marker(
    pack_root: &Path,
    pack_name: &str,
    pack_version: &str,
    update_zip: &str,
    update_fingerprint: &str,
    cache_root: &str,
    missing: Vec<Value>,
) -> Value {
    let mut list = missing;
    list.truncate(MAX_ENTRIES);
    json!({
        "format_version": MARKER_FORMAT_VERSION,
        "created_at": now_unix(),
        "created_at_text": now_text(),
        "pack_root": pack_root.to_string_lossy(),
        "pack_name": pack_name,
        "pack_version": pack_version,
        "update_zip": update_zip,
        "update_fingerprint": update_fingerprint,
        "cache_root": cache_root,
        "missing": list,
    })
}

fn human_size(n: u64) -> String {
    transfer::human_bytes(n)
}

/// 人类可读的说明文件内容
pub fn build_notice_text(marker: &Value) -> String {
    let missing = marker
        .get("missing")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();
    let root = marker.get("pack_root").and_then(|v| v.as_str()).unwrap_or("");
    let pack_name = marker
        .get("pack_name")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .map(|s| s.to_string())
        .unwrap_or_else(|| {
            Path::new(root)
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_else(|| "未命名".into())
        });
    let zip_name = Path::new(marker.get("update_zip").and_then(|v| v.as_str()).unwrap_or(""))
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "未知".into());

    let mut lines: Vec<String> = vec![
        "Pulses Easier —— 更新未完成说明".to_string(),
        "=".repeat(46),
        format!("整合包　：{pack_name}"),
        format!(
            "更新时间：{}",
            marker.get("created_at_text").and_then(|v| v.as_str()).unwrap_or("")
        ),
        format!(
            "更新包　：{zip_name}（版本 {}）",
            marker
                .get("pack_version")
                .and_then(|v| v.as_str())
                .unwrap_or("未知")
        ),
        String::new(),
        format!("有 {} 个文件没能下载，**本次更新不完整**：", missing.len()),
        String::new(),
    ];

    for (i, item) in missing.iter().enumerate() {
        let rel = item.get("rel").and_then(|v| v.as_str()).unwrap_or("");
        let size = item.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
        let sha1 = item.get("sha1").and_then(|v| v.as_str()).unwrap_or("");
        lines.push(format!("{}) {rel}", i + 1));
        let target = if root.is_empty() {
            rel.to_string()
        } else {
            Path::new(root).join(rel).to_string_lossy().to_string()
        };
        let mut size_text = human_size(size);
        if !sha1.is_empty() {
            size_text.push_str("　SHA1 ");
            size_text.push_str(sha1);
        }
        lines.push(format!("   应放到：{target}"));
        lines.push(format!("   大小　：{size_text}"));
        if let Some(urls) = item.get("urls").and_then(|v| v.as_array()) {
            if !urls.is_empty() {
                lines.push("   下载源：".to_string());
                for u in urls {
                    if let Some(u) = u.as_str() {
                        lines.push(format!("     {u}"));
                    }
                }
            }
        }
        lines.push(String::new());
    }

    lines.push("-".repeat(46));
    lines.push("补齐办法（任选其一）：".to_string());
    lines.push("  1) 打开 Pulses Easier → 定位本整合包 → 按提示补齐（推荐）".to_string());
    lines.push("  2) 自己下载上面的文件，放到「应放到」的位置（文件名要完全一致）".to_string());
    lines.push(String::new());
    lines.push("补齐完成后，本文件与 .pulses_easier/pending_update.json 可以删除；".to_string());
    lines.push("软件下次定位本整合包时也会自动核对并清理。".to_string());
    lines.join("\n")
}

/// 原子写文本
fn write_atomic(path: &Path, text: &str) -> bool {
    if let Some(p) = path.parent() {
        if fs::create_dir_all(p).is_err() {
            return false;
        }
    }
    let tmp = transfer::tmp_name(path);
    if fs::write(&tmp, text).is_err() {
        let _ = fs::remove_file(&tmp);
        return false;
    }
    if fs::rename(&tmp, path).is_err() {
        let _ = fs::remove_file(&tmp);
        return false;
    }
    true
}

/// 写凭证：JSON 给软件，txt 给人看
pub fn write_marker(pack_root: &Path, marker: &Value) -> bool {
    let j = serde_json::to_string_pretty(marker).unwrap_or_default();
    let a = write_atomic(&marker_path(pack_root), &j);
    let b = write_atomic(&notice_path(pack_root), &build_notice_text(marker));
    a && b
}

pub fn read_marker(pack_root: &Path) -> Option<Value> {
    let t = fs::read_to_string(marker_path(pack_root)).ok()?;
    serde_json::from_str::<Value>(&t).ok()
}

pub fn clear_marker(pack_root: &Path) -> bool {
    let a = fs::remove_file(marker_path(pack_root)).is_ok();
    let b = fs::remove_file(notice_path(pack_root)).is_ok();
    let _ = fs::remove_dir(marker_dir(pack_root));
    a || b
}

/// 核对凭证里列的文件是否已经补齐。
///
/// 判定：文件存在，且大小对得上；凭证里带 sha1 的再比一次摘要。
/// 全部补齐 → `all_done: true`，调用方可以清掉凭证。
pub fn check_marker(pack_root: &Path) -> Value {
    let Some(m) = read_marker(pack_root) else {
        return json!({"ok": false, "error": "没有凭证", "all_done": true});
    };
    let missing = m
        .get("missing")
        .and_then(|v| v.as_array())
        .cloned()
        .unwrap_or_default();

    let mut still: Vec<String> = Vec::new();
    let mut done = 0usize;
    for item in &missing {
        let rel = item.get("rel").and_then(|v| v.as_str()).unwrap_or("");
        if rel.is_empty() {
            continue;
        }
        let p = pack_root.join(rel);
        let size = item.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
        let sha1 = item.get("sha1").and_then(|v| v.as_str()).unwrap_or("");

        let mut ok = false;
        if let Ok(md) = fs::metadata(&p) {
            if md.is_file() && (size == 0 || md.len() == size) {
                if sha1.is_empty() {
                    ok = true;
                } else if let Some(got) = crate::ridex::hash_file(&p, "sha1") {
                    ok = got.eq_ignore_ascii_case(sha1);
                }
            }
        }
        if ok {
            done += 1;
        } else {
            still.push(rel.to_string());
        }
    }

    json!({
        "ok": true,
        "total": missing.len(),
        "done": done,
        "remaining": still,
        "all_done": still.is_empty(),
        "pack_name": m.get("pack_name").cloned().unwrap_or(Value::Null),
        "created_at_text": m.get("created_at_text").cloned().unwrap_or(Value::Null),
    })
}

// ═══════════════════════════════════════════════════════════════
// 就地自检
// ═══════════════════════════════════════════════════════════════

/// `.name.pulses_tmp` → `Some("name")`
fn split_tmp_dir_name(name: &str) -> Option<String> {
    let n = name.strip_prefix('.')?;
    let n = n.strip_suffix(".pulses_tmp")?;
    if n.is_empty() {
        None
    } else {
        Some(n.to_string())
    }
}

/// 找出残渣。整包只扫一层：目录替换只发生在顶层内容目录。
pub fn find_leftovers(pack_root: &Path) -> Value {
    let mut dirs: Vec<String> = Vec::new();
    let mut files: Vec<String> = Vec::new();

    let rd = match fs::read_dir(pack_root) {
        Ok(r) => r,
        Err(_) => return json!({"dirs": dirs, "files": files}),
    };
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let p = e.path();
        if p.is_dir() {
            if split_tmp_dir_name(&name).is_some() {
                dirs.push(name);
            }
        } else if transfer::is_transient_name(&name) {
            files.push(name);
        }
    }
    dirs.sort();
    files.sort();
    json!({ "dirs": dirs, "files": files })
}

pub fn has_leftovers(pack_root: &Path) -> bool {
    let v = find_leftovers(pack_root);
    let n = |k: &str| v.get(k).and_then(|x| x.as_array()).map(|a| a.len()).unwrap_or(0);
    n("dirs") + n("files") > 0
}

/// 清理残渣。
///
/// - `.xxx.pulses_tmp` 还在 → 说明整目录替换没跑完：
///   把目标目录整个删掉，把副本改名回原位 = 回滚这次没做完的替换
/// - `.yyy.pulses_new` → 直接删
pub fn recover_leftovers(pack_root: &Path) -> Value {
    let mut restored = 0usize;
    let mut removed = 0usize;
    let mut failed: Vec<Value> = Vec::new();

    let rd = match fs::read_dir(pack_root) {
        Ok(r) => r,
        Err(e) => {
            return json!({
                "ok": false, "restored": 0, "removed": 0,
                "failed": [{"name": "", "error": e.to_string()}],
            })
        }
    };

    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        let p = e.path();

        if p.is_dir() {
            if let Some(target_name) = split_tmp_dir_name(&name) {
                let target = pack_root.join(&target_name);
                // 先删残缺的目标，再把副本改名回来
                if target.exists() && fs::remove_dir_all(&target).is_err() {
                    if let Err(err) = fs::remove_file(&target) {
                        failed.push(json!({"name": name, "error": err.to_string()}));
                        continue;
                    }
                }
                match fs::rename(&p, &target) {
                    Ok(()) => restored += 1,
                    Err(err) => failed.push(json!({
                        "name": name,
                        "error": transfer::friendly_error(&err, "恢复失败"),
                    })),
                }
            }
        } else if transfer::is_transient_name(&name) {
            match fs::remove_file(&p) {
                Ok(()) => removed += 1,
                Err(err) => failed.push(json!({
                    "name": name,
                    "error": transfer::friendly_error(&err, "删除失败"),
                })),
            }
        }
    }

    json!({
        "ok": failed.is_empty(),
        "restored": restored,
        "removed": removed,
        "failed": failed,
    })
}

/// 一句人话
pub fn summary(stats: &Value) -> String {
    let r = stats.get("restored").and_then(|v| v.as_u64()).unwrap_or(0);
    let m = stats.get("removed").and_then(|v| v.as_u64()).unwrap_or(0);
    if r == 0 && m == 0 {
        return String::new();
    }
    let mut parts: Vec<String> = Vec::new();
    if r > 0 {
        parts.push(format!("回滚了 {r} 个没做完的目录替换"));
    }
    if m > 0 {
        parts.push(format!("清掉 {m} 个残留临时文件"));
    }
    parts.join("，")
}

// ═══════════════════════════════════════════════════════════════
// 检查点
// ═══════════════════════════════════════════════════════════════

fn checkpoint_dir() -> Option<PathBuf> {
    let db = crate::db::get_db_path()?;
    let d = PathBuf::from(db).join("cache").join("checkpoints");
    let _ = fs::create_dir_all(&d);
    Some(d)
}

/// 用整合包路径的哈希做文件名，避免路径里的特殊字符
fn pack_key(pack_root: &Path) -> String {
    let s = pack_root.to_string_lossy().to_lowercase();
    let mut h: u64 = 1469598103934665603;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(1099511628211);
    }
    format!("{h:016x}")
}

pub fn checkpoint_path(pack_root: &Path) -> Option<PathBuf> {
    Some(checkpoint_dir()?.join(format!("{}.json", pack_key(pack_root))))
}

/// 记一次「开始应用」，用于下次判断上次是不是正常结束
pub fn save_checkpoint(pack_root: &Path, stage: &str, done: u64, total: u64) -> bool {
    let Some(p) = checkpoint_path(pack_root) else { return false };
    let v = json!({
        "format_version": 1,
        "pack_root": pack_root.to_string_lossy(),
        "stage": stage,
        "done": done,
        "total": total,
        "updated_at": now_unix(),
        "updated_at_text": now_text(),
    });
    let t = serde_json::to_string_pretty(&v).unwrap_or_default();
    write_atomic(&p, &t)
}

pub fn load_checkpoint(pack_root: &Path) -> Option<Value> {
    let p = checkpoint_path(pack_root)?;
    let t = fs::read_to_string(p).ok()?;
    serde_json::from_str::<Value>(&t).ok()
}

pub fn clear_checkpoint(pack_root: &Path) -> bool {
    match checkpoint_path(pack_root) {
        Some(p) => fs::remove_file(p).is_ok(),
        None => false,
    }
}

/// 上次更新是不是没正常结束
pub fn is_unfinished(pack_root: &Path) -> bool {
    match load_checkpoint(pack_root) {
        Some(v) => v.get("stage").and_then(|s| s.as_str()).unwrap_or("") != "done",
        None => false,
    }
}

/// 扫描所有检查点
pub fn scan_checkpoints() -> Vec<Value> {
    let mut out = Vec::new();
    let Some(d) = checkpoint_dir() else { return out };
    let Ok(rd) = fs::read_dir(d) else { return out };
    for e in rd.flatten() {
        let p = e.path();
        if p.extension().map(|x| x == "json").unwrap_or(false) {
            if let Ok(t) = fs::read_to_string(&p) {
                if let Ok(v) = serde_json::from_str::<Value>(&t) {
                    out.push(v);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pulses_rec_{name}"));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn marker_roundtrip_and_notice() {
        let d = tmpdir("marker");
        let m = build_marker(
            &d,
            "红石生电优化",
            "26.3",
            "/x/pack.zip",
            "fp",
            "/c",
            vec![json!({"rel": "mods/a.jar", "size": 2048, "sha1": "abc", "urls": ["https://e/a.jar"]})],
        );
        assert!(write_marker(&d, &m));
        assert!(marker_path(&d).exists());
        assert!(notice_path(&d).exists());

        let back = read_marker(&d).unwrap();
        assert_eq!(back["pack_name"], "红石生电优化");
        assert_eq!(back["missing"].as_array().unwrap().len(), 1);

        let txt = fs::read_to_string(notice_path(&d)).unwrap();
        assert!(txt.contains("更新未完成"));
        assert!(txt.contains("mods/a.jar"));
        assert!(txt.contains("2 KB"));
        assert!(txt.contains("https://e/a.jar"));

        assert!(clear_marker(&d));
        assert!(read_marker(&d).is_none());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn check_marker_reports_what_is_still_missing() {
        let d = tmpdir("check");
        fs::create_dir_all(d.join("mods")).unwrap();
        fs::write(d.join("mods/done.jar"), vec![0u8; 10]).unwrap();

        // 真算一次 sha1，别用假值
        let sha = crate::ridex::hash_file(&d.join("mods/done.jar"), "sha1").unwrap();

        let m = build_marker(
            &d, "p", "1", "", "", "",
            vec![
                json!({"rel": "mods/done.jar", "size": 10, "sha1": sha}),
                // 这一条不带 sha1：只按大小判定，免得拿假摘要去验
                json!({"rel": "mods/missing.jar", "size": 10}),
            ],
        );
        write_marker(&d, &m);

        let r = check_marker(&d);
        assert_eq!(r["total"], 2);
        assert_eq!(r["done"], 1);
        assert_eq!(r["all_done"], false);
        assert_eq!(r["remaining"][0], "mods/missing.jar");

        // 补齐以后应该全绿
        fs::write(d.join("mods/missing.jar"), vec![0u8; 10]).unwrap();
        let r2 = check_marker(&d);
        assert_eq!(r2["all_done"], true);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn size_mismatch_does_not_count_as_done() {
        let d = tmpdir("size");
        fs::create_dir_all(d.join("mods")).unwrap();
        fs::write(d.join("mods/a.jar"), vec![0u8; 5]).unwrap();
        let m = build_marker(&d, "p", "1", "", "", "",
            vec![json!({"rel": "mods/a.jar", "size": 10})]);
        write_marker(&d, &m);
        let r = check_marker(&d);
        assert_eq!(r["done"], 0);
        assert_eq!(r["all_done"], false);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn leftovers_found_and_recovered() {
        let d = tmpdir("left");
        // 一个没跑完的目录替换：目标残缺 + 副本还在
        fs::create_dir_all(d.join("config")).unwrap();
        fs::write(d.join("config/broken.json"), b"half").unwrap();
        fs::create_dir_all(d.join(".config.pulses_tmp")).unwrap();
        fs::write(d.join(".config.pulses_tmp/good.json"), b"old").unwrap();
        // 一个搬运临时文件
        fs::write(d.join(".a.jar.pulses_new"), b"x").unwrap();
        // 正常文件
        fs::write(d.join("keep.txt"), b"k").unwrap();

        let f = find_leftovers(&d);
        assert_eq!(f["dirs"].as_array().unwrap().len(), 1);
        assert_eq!(f["files"].as_array().unwrap().len(), 1);
        assert!(has_leftovers(&d));

        let r = recover_leftovers(&d);
        assert!(r["ok"].as_bool().unwrap());
        assert_eq!(r["restored"], 1);
        assert_eq!(r["removed"], 1);

        // 副本被改名回原位，残缺内容不在了
        assert!(d.join("config/good.json").exists());
        assert!(!d.join("config/broken.json").exists());
        assert!(!d.join(".config.pulses_tmp").exists());
        assert!(!d.join(".a.jar.pulses_new").exists());
        assert!(d.join("keep.txt").exists());
        assert!(!has_leftovers(&d));

        let s = summary(&r);
        assert!(s.contains("回滚"), "{s}");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn clean_pack_has_no_leftovers() {
        let d = tmpdir("clean");
        fs::write(d.join("a.jar"), b"x").unwrap();
        assert!(!has_leftovers(&d));
        let r = recover_leftovers(&d);
        assert_eq!(r["restored"], 0);
        assert_eq!(r["removed"], 0);
        assert_eq!(summary(&r), "");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn tmp_dir_name_parsing() {
        assert_eq!(split_tmp_dir_name(".config.pulses_tmp").as_deref(), Some("config"));
        assert_eq!(split_tmp_dir_name("config.pulses_tmp"), None);
        assert_eq!(split_tmp_dir_name(".pulses_tmp"), None);
        assert_eq!(split_tmp_dir_name(".a.jar.pulses_new"), None);
    }

    #[test]
    fn pack_key_is_stable_and_case_insensitive() {
        let a = pack_key(Path::new("/home/x/Pack"));
        let b = pack_key(Path::new("/home/x/pack"));
        assert_eq!(a, b);
        assert_ne!(a, pack_key(Path::new("/home/x/other")));
        assert_eq!(a.len(), 16);
    }
}
