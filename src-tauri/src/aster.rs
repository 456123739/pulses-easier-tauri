// aster.rs — Aster 引擎（自适应分片下载）
// 对应 Python: app/core/engines/aster.py
//
// 名字取自拉丁语「星」，中文别称星驰，与 RideX 锐驰对应。
// 策略移植自 Plain Craft Launcher 2 的 NetTask，但不共享任何代码：
// Aster 有自己的调度器和失败判定，与 RideX 只共用任务/结果的数据结构。
//
// 与 RideX 的根本差别：
//   RideX  一开始就按 part_threads 铺开分片，多源并发
//   Aster  单线程起步，盯着「剩余量最大的那块」，够大就从尾巴切 40% 开新线程；
//          快文件迅速铺满带宽，慢文件自然只跑一两个线程
//
// 移植过来的策略：
//   1. 单线程起步、自适应增开（PIECE_LIMIT / TAIL_RATIO）
//   2. 全局线程预算（aster_threads），所有文件共享
//   3. 三种情况不分割：大小未知、小于 1 MB、源属于「多线程容易抽风」名单
//   4. 下载源逐个降级，404/416/范围不符直接判该源不可用
//   5. 全源失败 → 整文件单线程重试一次
//   6. 慢速断链：连续 5 秒没新数据且不足 1 KB/s 就换源
//
// 与 Python 实现的差异（不影响策略）：
//   · 分片失败后不做「块级续传」，直接退到整文件单线程重试。
//     PCL 会保留块内进度，这里为了简单先退一步；整文件重试能保证正确性。
//   · 并发模型：文件级仍按 multi_slots 并发，每个文件再按自适应分片，
//     预算在文件之间均分。

use std::fs::{File, OpenOptions};
use std::io::Read;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::ridex::{self, Batch, DownloadResult, DownloadTask, Options};

// ═══════════════════════════════════════════════════════════════
// 从 PCL NetTask 移植的常量
// ═══════════════════════════════════════════════════════════════

/// FilePieceLimit：剩余量超过它才值得再开一个线程
const PIECE_LIMIT: u64 = 256 * 1024;
/// 从最大碎片尾部切走的比例
const TAIL_RATIO: f64 = 0.4;
/// 小于这个大小不分割
const MIN_SPLIT_SIZE: u64 = 1024 * 1024;
/// PCL 里明确禁用多线程的源
const NO_SPLIT_HOSTS: &[&str] = &[
    "pcl2-server",
    "meloong.com",
    "bmclapi",
    "github.com",
    "optifine.net",
    "momot.rs",
];
/// 连续这么久没新数据 → 判定卡死
const SLOW_GAP: Duration = Duration::from_millis(5000);
/// 且这段时间只收到不到这么多字节
const SLOW_BYTES: u64 = 1024;
const READ_BLOCK: usize = 16 * 1024;
/// 全局线程预算的默认值
pub const DEFAULT_BUDGET: usize = 32;
/// 引擎版本（日志/关于页显示，跟 Python 侧对齐）
pub const BUILD: u32 = 1;
/// 调度轮询间隔
const TICK: Duration = Duration::from_millis(20);

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
(KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

// ═══════════════════════════════════════════════════════════════
// 小工具
// ═══════════════════════════════════════════════════════════════

fn client(opts: &Options) -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(opts.connect_timeout.max(1)))
        .timeout(Duration::from_secs(opts.per_url_timeout.max(1)))
        .user_agent(UA)
        .build()
        .map_err(|e| e.to_string())
}

fn host_of(url: &str) -> String {
    let rest = match url.find("://") {
        Some(i) => &url[i + 3..],
        None => url,
    };
    match rest.find('/') {
        Some(j) => rest[..j].to_ascii_lowercase(),
        None => rest.to_ascii_lowercase(),
    }
}

fn no_split(url: &str) -> bool {
    let h = host_of(url);
    NO_SPLIT_HOSTS.iter().any(|s| h.contains(s))
}

/// 写到文件的绝对偏移。
#[cfg(unix)]
fn write_at(f: &File, buf: &[u8], off: u64) -> std::io::Result<()> {
    use std::os::unix::fs::FileExt;
    f.write_all_at(buf, off)
}

#[cfg(windows)]
fn write_at(f: &File, buf: &[u8], off: u64) -> std::io::Result<()> {
    use std::os::windows::fs::FileExt;
    let mut n = 0usize;
    while n < buf.len() {
        let w = f.seek_write(&buf[n..], off + n as u64)?;
        if w == 0 {
            return Err(std::io::Error::new(
                std::io::ErrorKind::WriteZero,
                "写入 0 字节",
            ));
        }
        n += w;
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn write_at(f: &File, buf: &[u8], off: u64) -> std::io::Result<()> {
    use std::io::{Seek, SeekFrom, Write};
    let mut f = f.try_clone()?;
    f.seek(SeekFrom::Start(off))?;
    f.write_all(buf)
}

fn part_path(target: &Path) -> PathBuf {
    let mut s = target.as_os_str().to_os_string();
    s.push(".aster-tmp");
    PathBuf::from(s)
}

// ═══════════════════════════════════════════════════════════════
// 探测与单流
// ═══════════════════════════════════════════════════════════════

/// 用 Range 0-0 探总大小
fn probe_size(c: &reqwest::blocking::Client, url: &str) -> Option<u64> {
    let r = c
        .get(url)
        .header(reqwest::header::RANGE, "bytes=0-0")
        .send()
        .ok()?;
    if r.status().as_u16() == 206 {
        let cr = r.headers().get("content-range")?.to_str().ok()?;
        let total = cr.rsplit('/').next()?.trim().parse::<u64>().ok()?;
        return Some(total);
    }
    if r.status().is_success() {
        return r.content_length();
    }
    None
}

/// 单连接把 [start, end) 写进文件，返回读到的字节数。
/// `on_tick` 每读一块调一次，用来报进度和检查取消。
fn stream_range<F>(
    c: &reqwest::blocking::Client,
    url: &str,
    f: &File,
    start: u64,
    end: u64,
    mut on_tick: F,
) -> Result<u64, String>
where
    F: FnMut(u64) -> bool,
{
    let mut req = c.get(url);
    if end > start {
        req = req.header(
            reqwest::header::RANGE,
            format!("bytes={}-{}", start, end - 1),
        );
    }
    let mut resp = req.send().map_err(|e| format!("连不上：{e}"))?;
    let code = resp.status().as_u16();
    if !(code == 200 || code == 206) {
        return Err(format!("HTTP {code}"));
    }
    // 206 要校验起点对不对
    if code == 206 {
        if let Some(cr) = resp
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
        {
            let want = format!("bytes {}-", start);
            if !cr.starts_with(&want) {
                return Err(format!("Content-Range 不符：{cr}"));
            }
        }
    } else if start > 0 {
        // 要了续传却给了整文件：只能从头来，这里直接判失败让上层换源
        return Err("服务器忽略了 Range".to_string());
    }

    let mut off = start;
    let mut buf = vec![0u8; READ_BLOCK];
    let mut gap_bytes = 0u64;
    let mut got = 0u64;

    loop {
        if off >= end {
            break;
        }
        let want = ((end - off) as usize).min(READ_BLOCK);
        // 读是阻塞的，用它本身的耗时判断「停了多久」：
        // 一次读超过 5 秒却只拿到不到 1 KB，就是卡死了
        let t0 = Instant::now();
        let n = match resp.read(&mut buf[..want]) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => return Err(format!("读取出错：{e}")),
        };
        let dt = t0.elapsed();

        write_at(f, &buf[..n], off).map_err(|e| format!("写盘出错：{e}"))?;
        off += n as u64;
        got += n as u64;
        gap_bytes += n as u64;

        if dt >= SLOW_GAP {
            if gap_bytes < SLOW_BYTES {
                return Err("停滞".to_string());
            }
            gap_bytes = 0;
        }

        if !on_tick(n as u64) {
            return Err("已取消".to_string());
        }
    }
    Ok(got)
}

// ═══════════════════════════════════════════════════════════════
// 自适应分片
// ═══════════════════════════════════════════════════════════════

#[derive(Clone, Copy, PartialEq, Eq)]
enum PieceState {
    Idle,
    Active,
}

struct Piece {
    end: u64,
    cursor: u64,
    state: PieceState,
}

impl Piece {
    fn remain(&self) -> u64 {
        self.end.saturating_sub(self.cursor)
    }
}

struct Shared {
    pieces: Mutex<Vec<Piece>>,
    error: Mutex<Option<String>>,
    running: AtomicUsize,
    received: Arc<AtomicU64>,
}

/// 单文件自适应分片下载。
///
/// 起步只有 1 片；调度器每 TICK 看一眼，找到剩余量最大的一片，
/// 只要它还剩 > PIECE_LIMIT 就从尾巴切 40% 出去开新线程。
fn adaptive(
    c: &reqwest::blocking::Client,
    urls: &[String],
    f: &File,
    total: u64,
    budget: usize,
    cancel: &Arc<AtomicBool>,
    received: &Arc<AtomicU64>,
) -> Result<(), String> {
    let sh = Arc::new(Shared {
        pieces: Mutex::new(vec![Piece {
            end: total,
            cursor: 0,
            state: PieceState::Idle,
        }]),
        error: Mutex::new(None),
        running: AtomicUsize::new(0),
        received: received.clone(),
    });

    let mut handles: Vec<std::thread::JoinHandle<()>> = Vec::new();
    let budget = budget.max(1);

    loop {
        // 出错就收工
        if let Ok(g) = sh.error.lock() {
            if g.is_some() {
                break;
            }
        }
        if cancel.load(Ordering::SeqCst) {
            break;
        }

        // 1) 启动所有 idle 片
        let mut to_spawn: Vec<usize> = Vec::new();
        {
            let mut ps = match sh.pieces.lock() {
                Ok(g) => g,
                Err(_) => break,
            };
            for i in 0..ps.len() {
                if ps[i].state == PieceState::Idle && ps[i].remain() > 0 {
                    ps[i].state = PieceState::Active;
                    to_spawn.push(i);
                }
            }
        }
        for idx in to_spawn {
            let sh2 = sh.clone();
            let cancel2 = cancel.clone();
            let fh = match f.try_clone() {
                Ok(x) => x,
                Err(e) => return Err(format!("复制文件句柄失败：{e}")),
            };
            let urls2 = urls.to_vec();
            let cc = c.clone();
            sh.running.fetch_add(1, Ordering::SeqCst);
            handles.push(std::thread::spawn(move || {
                run_piece(&cc, &urls2, &fh, &sh2, idx, &cancel2);
                sh2.running.fetch_sub(1, Ordering::SeqCst);
            }));
        }

        // 2) 看是否全部完成
        let (all_done, active_n) = {
            let ps = match sh.pieces.lock() {
                Ok(g) => g,
                Err(_) => break,
            };
            (
                ps.iter().all(|p| p.remain() == 0),
                ps.iter().filter(|p| p.state == PieceState::Active).count(),
            )
        };
        if all_done {
            break;
        }

        // 3) 自适应增开：找剩余最大的片，从尾巴切 40%
        if sh.running.load(Ordering::SeqCst) < budget && active_n > 0 {
            let mut ps = match sh.pieces.lock() {
                Ok(g) => g,
                Err(_) => break,
            };
            let mut best: Option<usize> = None;
            let mut best_remain = 0u64;
            for (i, p) in ps.iter().enumerate() {
                if p.state == PieceState::Active && p.remain() > best_remain {
                    best = Some(i);
                    best_remain = p.remain();
                }
            }
            if let Some(i) = best {
                if best_remain > PIECE_LIMIT {
                    let new_start = ps[i].end - (best_remain as f64 * TAIL_RATIO) as u64;
                    let new_end = ps[i].end;
                    ps[i].end = new_start;
                    ps.push(Piece {
                        end: new_end,
                        cursor: new_start,
                        state: PieceState::Idle,
                    });
                }
            }
        }

        std::thread::sleep(TICK);
    }

    // 等所有线程收尾
    for h in handles {
        let _ = h.join();
    }

    if let Ok(g) = sh.error.lock() {
        if let Some(e) = g.as_ref() {
            return Err(e.clone());
        }
    }
    if cancel.load(Ordering::SeqCst) {
        return Err("已取消".to_string());
    }

    let ps = match sh.pieces.lock() {
        Ok(g) => g,
        Err(_) => return Err("调度器损坏".to_string()),
    };
    if ps.iter().any(|p| p.remain() > 0) {
        return Err("有分片没下完".to_string());
    }
    Ok(())
}

/// 一个分片线程：按 cursor 往前推，每次写绝对偏移。
fn run_piece(
    c: &reqwest::blocking::Client,
    urls: &[String],
    f: &File,
    sh: &Arc<Shared>,
    idx: usize,
    cancel: &Arc<AtomicBool>,
) {
    let mut src = 0usize;
    let mut tries = 0usize;

    loop {
        if cancel.load(Ordering::SeqCst) {
            return;
        }
        if let Ok(g) = sh.error.lock() {
            if g.is_some() {
                return;
            }
        }

        // 取当前片的可下区间
        let (cursor, end) = {
            let ps = match sh.pieces.lock() {
                Ok(g) => g,
                Err(_) => return,
            };
            let p = &ps[idx];
            if p.remain() == 0 {
                return;
            }
            (p.cursor, p.end)
        };

        if src >= urls.len() {
            mark_error(sh, "所有下载源都失败了".to_string());
            return;
        }

        // 分段读，边读边推进 cursor
        let res = stream_piece(c, &urls[src], f, sh, idx, cursor, end, cancel);
        match res {
            Ok(()) => return, // 本片下完
            Err(e) => {
                tries += 1;
                // 同一个源再试一次，仍然失败就换源
                if tries > 1 {
                    src += 1;
                    tries = 0;
                }
                if src >= urls.len() {
                    mark_error(sh, format!("分片下载失败：{e}"));
                    return;
                }
            }
        }
    }
}

fn mark_error(sh: &Arc<Shared>, msg: String) {
    if let Ok(mut g) = sh.error.lock() {
        if g.is_none() {
            *g = Some(msg);
        }
    }
}

/// 从 cursor 往 end 推一段。每读一块就更新共享 cursor，供调度器切分。
#[allow(clippy::too_many_arguments)]
fn stream_piece(
    c: &reqwest::blocking::Client,
    url: &str,
    f: &File,
    sh: &Arc<Shared>,
    idx: usize,
    from: u64,
    end: u64,
    cancel: &Arc<AtomicBool>,
) -> Result<(), String> {
    let mut req = c.get(url);
    req = req.header(reqwest::header::RANGE, format!("bytes={from}-{}", end - 1));
    let mut resp = req.send().map_err(|e| format!("连不上：{e}"))?;
    let code = resp.status().as_u16();
    if !(code == 200 || code == 206) {
        return Err(format!("HTTP {code}"));
    }
    if code == 206 {
        if let Some(cr) = resp
            .headers()
            .get("content-range")
            .and_then(|v| v.to_str().ok())
        {
            if !cr.starts_with(&format!("bytes {from}-")) {
                return Err(format!("Content-Range 不符：{cr}"));
            }
        }
    } else if from > 0 {
        return Err("服务器忽略了 Range".to_string());
    }

    let mut off = from;
    let mut buf = vec![0u8; READ_BLOCK];
    let mut gap_bytes = 0u64;

    loop {
        // 每次循环都重读 end：调度器可能已经把这一片的尾巴切走了
        let cur_end = {
            let ps = match sh.pieces.lock() {
                Ok(g) => g,
                Err(_) => return Err("调度器损坏".to_string()),
            };
            ps[idx].end
        };
        if off >= cur_end {
            return Ok(());
        }
        if cancel.load(Ordering::SeqCst) {
            return Err("已取消".to_string());
        }

        let want = ((cur_end - off) as usize).min(READ_BLOCK);
        let t0 = Instant::now();
        let n = match resp.read(&mut buf[..want]) {
            Ok(0) => return Err("提前结束".to_string()),
            Ok(n) => n,
            Err(e) => return Err(format!("读取出错：{e}")),
        };
        let dt = t0.elapsed();

        write_at(f, &buf[..n], off).map_err(|e| format!("写盘出错：{e}"))?;
        off += n as u64;
        sh.received.fetch_add(n as u64, Ordering::SeqCst);

        if let Ok(mut ps) = sh.pieces.lock() {
            ps[idx].cursor = off;
        }

        gap_bytes += n as u64;
        if dt >= SLOW_GAP {
            if gap_bytes < SLOW_BYTES {
                return Err("停滞".to_string());
            }
            gap_bytes = 0;
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// 单文件：先自适应，失败退整文件单线程
// ═══════════════════════════════════════════════════════════════

fn download_one(
    task: &DownloadTask,
    opts: &Options,
    cancel: &Arc<AtomicBool>,
    budget: usize,
) -> DownloadResult {
    let target = task.target_dir.join(&task.name);
    if let Some(p) = target.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    let tmp = part_path(&target);
    let _ = std::fs::remove_file(&tmp);

    let fail = |msg: String| DownloadResult {
        name: task.name.clone(),
        path: target.to_string_lossy().to_string(),
        size: 0,
        ok: false,
        error: msg,
        bytes: 0,
    };

    let c = match client(opts) {
        Ok(x) => x,
        Err(e) => return fail(format!("建不了 HTTP 客户端：{e}")),
    };

    // 镜像优先：官方地址永远留最后兜底
    let urls = crate::mirrors::ordered(task.urls.clone());
    if urls.is_empty() {
        return fail("没有下载源".to_string());
    }

    // 探大小
    let total = if task.size > 0 {
        Some(task.size)
    } else {
        urls.iter().find_map(|u| probe_size(&c, u))
    };

    let received = Arc::new(AtomicU64::new(0));
    let file = match OpenOptions::new()
        .create(true)
        .truncate(true)
        .read(true)
        .write(true)
        .open(&tmp)
    {
        Ok(f) => f,
        Err(e) => return fail(format!("建文件失败：{e}")),
    };

    let mut last_error = String::new();

    if let Some(t) = total {
        if t > 0 {
            let _ = file.set_len(t);
        }
        let splittable = t >= MIN_SPLIT_SIZE
            && budget > 1
            && !urls.iter().any(|u| no_split(u));

        if splittable {
            match adaptive(&c, &urls, &file, t, budget, cancel, &received) {
                Ok(()) => {
                    drop(file);
                    return finish(target, tmp, task, received.load(Ordering::SeqCst));
                }
                Err(e) => {
                    if e == "已取消" {
                        let _ = std::fs::remove_file(&tmp);
                        return fail("已取消".to_string());
                    }
                    last_error = e;
                    // 分片不可靠：清掉重来，走整文件单线程
                    let _ = file.set_len(0);
                }
            }
        }
    }

    // 整文件单线程，逐个源试，全源失败再来一轮
    for round in 0..2 {
        if cancel.load(Ordering::SeqCst) {
            let _ = std::fs::remove_file(&tmp);
            return fail("已取消".to_string());
        }
        for u in &urls {
            match stream_whole(&c, u, &file, total, cancel, &received) {
                Ok(()) => {
                    drop(file);
                    return finish(target, tmp, task, received.load(Ordering::SeqCst));
                }
                Err(e) => {
                    last_error = format!("{u}：{e}");
                    let _ = file.set_len(0);
                }
            }
        }
        let _ = round;
    }

    let _ = std::fs::remove_file(&tmp);
    fail(if last_error.is_empty() {
        "所有下载源都失败了".to_string()
    } else {
        format!("所有下载源都失败了（最后一个错误：{last_error}）")
    })
}

/// 整文件单连接下载
fn stream_whole(
    c: &reqwest::blocking::Client,
    url: &str,
    f: &File,
    total: Option<u64>,
    cancel: &Arc<AtomicBool>,
    received: &Arc<AtomicU64>,
) -> Result<(), String> {
    let end = match total {
        Some(t) => t,
        None => u64::MAX,
    };
    stream_range(c, url, f, 0, end, |n| {
        received.fetch_add(n, Ordering::SeqCst);
        if cancel.load(Ordering::SeqCst) {
            return false;
        }
        true
    })
    .map(|_| ())
}

fn finish(target: PathBuf, tmp: PathBuf, task: &DownloadTask, bytes: u64) -> DownloadResult {
    let (ok, why) = ridex::verify(&tmp, task);
    if !ok {
        let _ = std::fs::remove_file(&tmp);
        return DownloadResult {
            name: task.name.clone(),
            path: target.to_string_lossy().to_string(),
            size: 0,
            ok: false,
            error: why,
            bytes,
        };
    }
    if std::fs::rename(&tmp, &target).is_err() {
        let _ = std::fs::copy(&tmp, &target);
        let _ = std::fs::remove_file(&tmp);
    }
    let size = std::fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
    DownloadResult {
        name: task.name.clone(),
        path: target.to_string_lossy().to_string(),
        size,
        ok: true,
        error: String::new(),
        bytes,
    }
}

// ═══════════════════════════════════════════════════════════════
// 对外：与 ridex::run_batch 同签名，可直接切换
// ═══════════════════════════════════════════════════════════════

pub fn run_batch(
    tasks: Vec<DownloadTask>,
    opts: &Options,
    cancel: Arc<AtomicBool>,
    on_progress: Option<ridex::ProgressCb>,
    on_file: Option<ridex::FileCb>,
) -> Batch {
    crate::mirrors::configure(opts.mirror_first, &opts.mirror_hosts);

    let total = tasks.len();
    let total_bytes: u64 = tasks.iter().map(|t| t.size).sum();
    let done = Arc::new(AtomicU64::new(0));
    let moved = Arc::new(AtomicU64::new(0));

    let slots = opts.multi_slots.max(1);
    // 全局线程预算在各文件之间均分，至少给 1
    let budget_all = if opts.aster_threads > 0 {
        opts.aster_threads
    } else {
        DEFAULT_BUDGET
    };
    let per_file = (budget_all / slots).max(1);

    let mut results: Vec<DownloadResult> = Vec::with_capacity(total);
    let mut idx = 0usize;

    while idx < total {
        let end = (idx + slots).min(total);
        let chunk: Vec<DownloadTask> = tasks[idx..end].to_vec();
        let mut handles = Vec::new();

        for t in chunk {
            let cancel_ref = cancel.clone();
            let opts_ref = opts.clone();
            let done_c = done.clone();
            let moved_c = moved.clone();
            let prog = on_progress.clone();
            let file_cb = on_file.clone();
            let recv = Arc::new(AtomicU64::new(0));
            let recv2 = recv.clone();
            handles.push(std::thread::spawn(move || {
                let r = download_one(&t, &opts_ref, &cancel_ref, per_file);
                let _ = recv2;
                let n = done_c.fetch_add(1, Ordering::SeqCst) + 1;
                moved_c.fetch_add(r.bytes, Ordering::SeqCst);
                if let Some(cb) = &prog {
                    cb(
                        n as usize,
                        total,
                        &r.name,
                        moved_c.load(Ordering::SeqCst),
                        total_bytes,
                    );
                }
                if let Some(cb) = &file_cb {
                    cb(&r, r.ok);
                }
                r
            }));
        }

        for h in handles {
            match h.join() {
                Ok(r) => results.push(r),
                Err(_) => results.push(DownloadResult {
                    name: "?".into(),
                    path: String::new(),
                    size: 0,
                    ok: false,
                    error: "工作线程崩溃".into(),
                    bytes: 0,
                }),
            }
        }

        if cancel.load(Ordering::SeqCst) {
            break;
        }
        idx = end;
    }

    let ok = results.iter().filter(|r| r.ok).count();
    let fail_n = results.len() - ok;
    Batch {
        results,
        ok,
        fail: fail_n,
    }
}

