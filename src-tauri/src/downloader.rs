// downloader.rs — 多线程下载（简化版，async reqwest）
// 对应 Python: downloader.py

use serde_json::{json, Value};
use tauri::{AppHandle, Emitter};

pub async fn start(app: &AppHandle, tasks: Value, _opts: Value) -> Value {
    let task_list = match tasks.as_array() {
        Some(a) => a.clone(),
        None => return json!({"ok": [], "fail": []}),
    };

    let mut ok = Vec::new();
    let mut fail = Vec::new();

    for (i, t) in task_list.iter().enumerate() {
        let url = t.get("url").and_then(|v| v.as_str()).unwrap_or("");
        let name = t.get("name").and_then(|v| v.as_str()).unwrap_or("download");
        let dst_dir = t.get("dstDir").and_then(|v| v.as_str())
            .map(std::path::PathBuf::from);

        let _ = app.emit("dl:progress", json!({
            "index": i, "total": task_list.len(),
            "name": name, "url": url, "status": "downloading"
        }));

        if url.is_empty() || dst_dir.is_none() {
            fail.push(json!({"name": name, "url": url, "error": "missing url or dstDir"}));
            continue;
        }

        let dst_dir = dst_dir.unwrap();
        let _ = std::fs::create_dir_all(&dst_dir);
        let dst = dst_dir.join(name);

        match reqwest::get(url).await {
            Ok(resp) => {
                if !resp.status().is_success() {
                    fail.push(json!({
                        "name": name, "url": url,
                        "error": format!("HTTP {}", resp.status())
                    }));
                    continue;
                }
                let bytes = match resp.bytes().await {
                    Ok(b) => b,
                    Err(e) => {
                        fail.push(json!({"name": name, "url": url, "error": e.to_string()}));
                        continue;
                    }
                };
                if std::fs::write(&dst, &bytes).is_err() {
                    fail.push(json!({"name": name, "url": url, "error": "write failed"}));
                    continue;
                }
                ok.push(json!({
                    "path": dst.to_string_lossy(),
                    "size": bytes.len(),
                    "name": name,
                }));

                let _ = app.emit("dl:progress", json!({
                    "index": i, "total": task_list.len(),
                    "name": name, "status": "done", "size": bytes.len()
                }));
            }
            Err(e) => {
                fail.push(json!({"name": name, "url": url, "error": e.to_string()}));
                let _ = app.emit("dl:log", json!({
                    "level": "error", "msg": format!("{}: {}", name, e)
                }));
            }
        }
    }

    let result = json!({"ok": ok, "fail": fail});
    let _ = app.emit("dl:done", &result);
    result
}
