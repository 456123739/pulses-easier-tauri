// updater.rs — 更新计划构建与执行
// 对应 Python: updater.py

use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use tauri::{AppHandle, Emitter};

/// 合并 `options.txt`：
///   · 玩家原有的**所有键位行**（以 `key_` 开头）原样保留
///   · 其余选项（FOV / 渲染距离 / GUI 缩放 / 音量…）全部用新包的
///
/// 原版键位和绝大多数模组键位都写在这一个文件里，行格式
/// `key_key.forward:key.keyboard.w`；只要 `key_` 行不动，玩家的热键就一个不变。
fn merge_options(old_text: &str, new_text: &str) -> String {
    let mut out: Vec<String> = new_text
        .lines()
        .filter(|l| !l.starts_with("key_"))
        .map(|l| l.to_string())
        .collect();
    for l in old_text.lines().filter(|l| l.starts_with("key_")) {
        out.push(l.to_string());
    }
    let mut s = out.join("\n");
    s.push('\n');
    s
}

/// 顶层项是否被勾选。
///   · `{"all": true}` → 全选（前端默认就发这个）
///   · 数组 → 数组里的顶层名算勾选
///   · 对象 → 按 key 取布尔
///   · 空 / 其它 → **一律不勾选**（与界面默认一致：没勾就不应用）
fn is_checked(checked: &Value, top: &str) -> bool {
    if let Some(all) = checked.get("all").and_then(|v| v.as_bool()) {
        if all {
            return true;
        }
    }
    match checked {
        Value::Array(a) => a.iter().any(|v| v.as_str() == Some(top)),
        Value::Object(o) => o.get(top).and_then(|v| v.as_bool()).unwrap_or(false),
        _ => false,
    }
}

/// 某个顶层项的策略。未知/非法一律回退「完全匹配」（最保守）。
fn strategy_of(strategies: &Value, top: &str) -> &'static str {
    let raw = strategies
        .get(top)
        .and_then(|v| v.as_str())
        .unwrap_or("full");
    match raw {
        "skip" | "skip_same" | "跳过重名" => "skip",
        "replace" | "replace_same" | "替换重名" => "replace",
        _ => "full",
    }
}

fn top_of(rel: &str) -> &str {
    rel.split('/').next().unwrap_or(rel)
}

/// 生成执行计划。
///
/// 判定规则（与 Python 版 apply_rules 一致）：
///   文件级  ADDED                → 应用
///           MODIFIED + 跳过重名   → 跳过
///           MODIFIED + 其它策略   → 应用
///           DELETED  + 完全匹配   → 删除
///           DELETED  + 其它策略   → 跳过
///   未勾选的顶层项 → 整项跳过
pub fn build_plan(diff: Value, checked: Value, strategies: Value,
                  old_root: &Path, new_root: &Path) -> Value {
    let mut tasks = Vec::new();
    let mut skipped = 0usize;

    let mut push_copy = |tasks: &mut Vec<Value>, rel: &str, size: u64| {
        tasks.push(json!({
            "type": "copy",
            "rel": rel,
            "src": new_root.join(rel).to_string_lossy().to_string(),
            "dst": old_root.join(rel).to_string_lossy().to_string(),
            "size": size,
        }));
    };

    // 新增：一律应用
    if let Some(added) = diff.get("added").and_then(|v| v.as_array()) {
        for f in added {
            let Some(rel) = f.get("rel").and_then(|v| v.as_str()) else { continue };
            if !is_checked(&checked, top_of(rel)) {
                skipped += 1;
                continue;
            }
            push_copy(&mut tasks, rel, f.get("size").and_then(|v| v.as_u64()).unwrap_or(0));
        }
    }

    // 修改：跳过重名策略下不覆盖
    if let Some(modified) = diff.get("modified").and_then(|v| v.as_array()) {
        for f in modified {
            let Some(rel) = f.get("rel").and_then(|v| v.as_str()) else { continue };
            let top = top_of(rel);
            if !is_checked(&checked, top) {
                skipped += 1;
                continue;
            }
            if strategy_of(&strategies, top) == "skip" {
                skipped += 1;
                continue;
            }
            push_copy(&mut tasks, rel, f.get("size").and_then(|v| v.as_u64()).unwrap_or(0));
        }
    }

    // 删除：只有「完全匹配」才删；options.txt 永不删
    if let Some(deleted) = diff.get("deleted").and_then(|v| v.as_array()) {
        for f in deleted {
            let Some(rel) = f.get("rel").and_then(|v| v.as_str()) else { continue };
            if rel == "options.txt" {
                continue;
            }
            let top = top_of(rel);
            if !is_checked(&checked, top) {
                skipped += 1;
                continue;
            }
            if strategy_of(&strategies, top) != "full" {
                skipped += 1;
                continue;
            }
            tasks.push(json!({
                "type": "delete",
                "rel": rel,
                "dst": old_root.join(rel).to_string_lossy().to_string(),
            }));
        }
    }

    json!({
        "tasks": tasks,
        "skipped": skipped,
        "oldRoot": old_root.to_string_lossy(),
        "newRoot": new_root.to_string_lossy(),
    })
}

pub async fn execute_plan(app: &AppHandle, plan: Value,
                          _old_root: &Path, _new_root: &Path) -> Value {
    let tasks = match plan.get("tasks").and_then(|v| v.as_array()) {
        Some(t) => t.clone(),
        None => return json!({"ok": false, "msg": "no tasks"}),
    };

    let total = tasks.len();
    let mut done = 0usize;
    let mut total_bytes = 0u64;
    let mut moved_bytes = 0u64;
    let mut failed: Vec<Value> = Vec::new();

    for t in &tasks {
        let task_type = t.get("type").and_then(|v| v.as_str()).unwrap_or("");
        if task_type == "copy" {
            let src = t.get("src").and_then(|v| v.as_str()).unwrap_or("");
            if let Ok(st) = fs::metadata(src) {
                total_bytes += st.len();
            }
        }
    }

    for t in &tasks {
        let task_type = t.get("type").and_then(|v| v.as_str()).unwrap_or("");
        match task_type {
            "copy" => {
                let src = t.get("src").and_then(|v| v.as_str()).unwrap_or("");
                let dst = t.get("dst").and_then(|v| v.as_str()).unwrap_or("");
                if let Some(parent) = Path::new(dst).parent() {
                    let _ = fs::create_dir_all(parent);
                }
                if let Ok(st) = fs::metadata(src) {
                    let size = st.len();
                    let ok = if t.get("rel").and_then(|v| v.as_str()) == Some("options.txt") {
                        // 合并而不是覆盖：键位留玩家的，画质用新包的
                        let new_text = fs::read_to_string(src).unwrap_or_default();
                        let old_text = fs::read_to_string(dst).unwrap_or_default();
                        let body = if old_text.trim().is_empty() {
                            new_text
                        } else {
                            merge_options(&old_text, &new_text)
                        };
                        fs::write(dst, body).is_ok()
                    } else {
                        fs::copy(src, dst).is_ok()
                    };
                    if ok {
                        moved_bytes += size;
                    } else if let Some(rel) = t.get("rel").and_then(|v| v.as_str()) {
                        failed.push(json!({"rel": rel, "error": "写入失败"}));
                    }
                } else if let Some(rel) = t.get("rel").and_then(|v| v.as_str()) {
                    failed.push(json!({"rel": rel, "error": "源文件不存在"}));
                }
            }
            "delete" => {
                let dst = t.get("dst").and_then(|v| v.as_str()).unwrap_or("");
                if fs::remove_file(dst).is_err() && Path::new(dst).exists() {
                    if let Some(rel) = t.get("rel").and_then(|v| v.as_str()) {
                        failed.push(json!({"rel": rel, "error": "删除失败"}));
                    }
                }
            }
            _ => {}
        }

        done += 1;
        if done % 10 == 0 || done == total {
            let _ = app.emit("update:progress", json!({
                "done": done,
                "total": total,
                "moved": moved_bytes,
                "totalBytes": total_bytes,
                "percent": if total_bytes > 0 {
                    moved_bytes as f64 / total_bytes as f64
                } else if total > 0 {
                    done as f64 / total as f64
                } else { 0.0 },
            }));
        }
    }

    // ok 必须反映真实结果：全部失败也报 true 的话，界面会显示「更新完成 ✓」
    json!({"ok": failed.is_empty(), "done": done, "total": total, "failed": failed})
}

pub fn verify(plan: &Value, _old_root: &Path, _new_root: &Path) -> Value {
    let mut errors = Vec::new();
    if let Some(tasks) = plan.get("tasks").and_then(|v| v.as_array()) {
        for t in tasks {
            let kind = t.get("type").and_then(|v| v.as_str()).unwrap_or("");
            // 删除项也要校验：删没删掉必须能看出来
            if kind == "delete" {
                if let Some(dst) = t.get("dst").and_then(|v| v.as_str()) {
                    if Path::new(dst).exists() {
                        if let Some(rel) = t.get("rel").and_then(|v| v.as_str()) {
                            errors.push(json!({"rel": rel, "msg": "删除未生效"}));
                        }
                    }
                }
                continue;
            }
            if kind != "copy" {
                continue;
            }
            if let Some(dst) = t.get("dst").and_then(|v| v.as_str()) {
                if !Path::new(dst).exists() {
                    if let Some(rel) = t.get("rel").and_then(|v| v.as_str()) {
                        errors.push(json!({"rel": rel, "msg": "目标文件不存在"}));
                    }
                    continue;
                }
                if let (Some(exp_size), Ok(meta)) =
                    (t.get("size").and_then(|v| v.as_u64()), fs::metadata(dst)) {
                    if meta.len() != exp_size {
                        if let Some(rel) = t.get("rel").and_then(|v| v.as_str()) {
                            errors.push(json!({
                                "rel": rel,
                                "msg": format!("大小不匹配: {} vs {}", meta.len(), exp_size)
                            }));
                        }
                    }
                }
            }
        }
    }
    json!({"ok": errors.is_empty(), "errors": errors})
}

pub fn estimate(plan: &Value, _old_root: &Path) -> Value {
    let mut bytes = 0u64;
    if let Some(tasks) = plan.get("tasks").and_then(|v| v.as_array()) {
        for t in tasks {
            if t.get("type").and_then(|v| v.as_str()) == Some("copy") {
                if let Some(size) = t.get("size").and_then(|v| v.as_u64()) {
                    bytes += size;
                }
            }
        }
    }
    json!({"copyBytes": bytes})
}
