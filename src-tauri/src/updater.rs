// updater.rs — 更新计划构建与执行
// 对应 Python: updater.py

use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use tauri::{AppHandle, Emitter};

pub fn build_plan(diff: Value, _checked: Value, _strategies: Value,
                  old_root: &Path, new_root: &Path) -> Value {
    let mut tasks = Vec::new();

    // 新增
    if let Some(added) = diff.get("added").and_then(|v| v.as_array()) {
        for f in added {
            if let Some(rel) = f.get("rel").and_then(|v| v.as_str()) {
                tasks.push(json!({
                    "type": "copy",
                    "rel": rel,
                    "src": new_root.join(rel).to_string_lossy().to_string(),
                    "dst": old_root.join(rel).to_string_lossy().to_string(),
                    "size": f.get("size").and_then(|v| v.as_u64()).unwrap_or(0),
                }));
            }
        }
    }
    // 修改
    if let Some(modified) = diff.get("modified").and_then(|v| v.as_array()) {
        for f in modified {
            if let Some(rel) = f.get("rel").and_then(|v| v.as_str()) {
                tasks.push(json!({
                    "type": "copy",
                    "rel": rel,
                    "src": new_root.join(rel).to_string_lossy().to_string(),
                    "dst": old_root.join(rel).to_string_lossy().to_string(),
                    "size": f.get("size").and_then(|v| v.as_u64()).unwrap_or(0),
                }));
            }
        }
    }
    // 删除
    if let Some(deleted) = diff.get("deleted").and_then(|v| v.as_array()) {
        for f in deleted {
            if let Some(rel) = f.get("rel").and_then(|v| v.as_str()) {
                tasks.push(json!({
                    "type": "delete",
                    "rel": rel,
                    "dst": old_root.join(rel).to_string_lossy().to_string(),
                }));
            }
        }
    }

    json!({"tasks": tasks, "oldRoot": old_root.to_string_lossy(), "newRoot": new_root.to_string_lossy()})
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
                    if fs::copy(src, dst).is_ok() {
                        moved_bytes += size;
                    }
                }
            }
            "delete" => {
                let dst = t.get("dst").and_then(|v| v.as_str()).unwrap_or("");
                let _ = fs::remove_file(dst);
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

    json!({"ok": true, "done": done, "total": total})
}

pub fn verify(plan: &Value, _old_root: &Path, _new_root: &Path) -> Value {
    let mut errors = Vec::new();
    if let Some(tasks) = plan.get("tasks").and_then(|v| v.as_array()) {
        for t in tasks {
            if t.get("type").and_then(|v| v.as_str()) != Some("copy") {
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
