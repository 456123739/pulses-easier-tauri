// engine.rs — 下载引擎调度
// 对应 Python: app/core/engines/__init__.py（注册表）+ v0.6.6 的「失败文件直通 Aster」
//
// 两条引擎并存，接口一致，按 `download.engine` 切：
//
//   ridex  锐驰：多槽位池 + 文件内分片直写 + 磁盘类型感知。默认。
//   aster  星驰：单线程起步、自适应增开、逐源降级。慢网/抽风源更稳。
//
// 另有一层补漏：RideX 主池跑完后，**失败的文件整批交给 Aster 再试一轮**。
// 以前 RideX 里有个「单线程耐心池」，等于同一个引擎把同一个文件重跑一次，
// 收益低还多一层队列；现在换成另一条策略完全不同的引擎来接，尾巴很短。

use crate::ridex::{Batch, DownloadTask, FileCb, Options, ProgressCb};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;

pub const ENGINES: &[&str] = &["ridex", "aster"];
pub const DEFAULT_ENGINE: &str = "ridex";

/// 把任意输入收敛成一个已知引擎名；未知一律回默认
pub fn normalize(name: &str) -> &'static str {
    let n = name.trim().to_ascii_lowercase();
    if n == "aster" {
        "aster"
    } else {
        DEFAULT_ENGINE
    }
}

/// 设置界面用的显示名
pub fn label(name: &str) -> &'static str {
    match normalize(name) {
        "aster" => "Aster 星驰（自适应分片）",
        _ => "RideX 锐驰（默认）",
    }
}

/// 日志/关于页用的一行标识
pub fn banner(name: &str) -> String {
    match normalize(name) {
        "aster" => format!(
            "Aster 星驰（自适应分片，移植 PCL2 NetTask 策略）（engine build {}）",
            crate::aster::BUILD
        ),
        _ => "RideX 锐驰（多槽位池 + 文件内分片）".to_string(),
    }
}

/// 跑一批下载。先走选定的引擎，RideX 失败的文件再交 Aster 补一轮。
pub fn run(
    tasks: Vec<DownloadTask>,
    opts: &Options,
    cancel: Arc<AtomicBool>,
    on_progress: Option<ProgressCb>,
    on_file: Option<FileCb>,
) -> Batch {
    if tasks.is_empty() {
        return Batch {
            results: Vec::new(),
            ok: 0,
            fail: 0,
        };
    }

    let engine = normalize(&opts.engine);

    let mut primary = if engine == "aster" {
        crate::aster::run_batch(
            tasks.clone(),
            opts,
            cancel.clone(),
            on_progress.clone(),
            on_file.clone(),
        )
    } else {
        crate::ridex::run_batch(
            tasks.clone(),
            opts,
            cancel.clone(),
            on_progress.clone(),
            on_file.clone(),
        )
    };

    // 失败文件直通 Aster：只在 RideX 做主引擎时补，取消后不再补
    if engine != "aster" && primary.fail > 0 && !cancel.load(Ordering::SeqCst) {
        let mut idx: Vec<usize> = Vec::new();
        let mut again: Vec<DownloadTask> = Vec::new();
        for (i, r) in primary.results.iter().enumerate() {
            if !r.ok && tasks.get(i).is_some() {
                idx.push(i);
                again.push(tasks[i].clone());
            }
        }

        if !again.is_empty() {
            let second = crate::aster::run_batch(again, opts, cancel.clone(), None, None);
            for (k, i) in idx.iter().enumerate() {
                if let Some(r) = second.results.get(k) {
                    if r.ok {
                        primary.results[*i] = r.clone();
                    }
                }
            }
            primary.ok = primary.results.iter().filter(|r| r.ok).count();
            primary.fail = primary.results.len() - primary.ok;
        }
    }

    primary
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_engine_names() {
        assert_eq!(normalize("ridex"), "ridex");
        assert_eq!(normalize("Aster"), "aster");
        assert_eq!(normalize("  AST  "), "ridex");
        assert_eq!(normalize(""), "ridex");
    }

    #[test]
    fn labels_are_human_readable() {
        assert!(label("aster").contains("星驰"));
        assert!(label("ridex").contains("锐驰"));
    }

    #[test]
    fn empty_batch_is_noop() {
        let b = run(
            Vec::new(),
            &Options::default(),
            Arc::new(AtomicBool::new(false)),
            None,
            None,
        );
        assert_eq!(b.ok, 0);
        assert_eq!(b.fail, 0);
    }
}
