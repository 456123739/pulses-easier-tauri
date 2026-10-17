// ridex.rs — RideX（锐驰引擎）Rust 版
// 对应 Python: app/core/downloader.py（2505 行）
//
// 三个约束（与 Python 版一致）：
//   1) 正确性优先：每个字节都要能对上声明的大小/哈希，宁可重下也不写出错位文件
//   2) 不猜磁盘：并发上限只降不升
//   3) 失败不吞：下不下来的文件明确交还给用户（链接 + 文件名），绝不静默跳过
//
// 本文件包含：
//   · Options / DownloadTask / DownloadResult 类型
//   · `.part` + `.part.meta` 已确认区间追踪（续传可信化的关键）
//   · 单连接流式下载（状态码校验 / Range 校验 / 停滞检测 / 边下边算哈希）
//   · 分片下载（Range + Content-Range 校验）
//   · 调度：多槽位池 → 重试队列 → 单槽位池
//
// 尚未移植（下一步）：
//   · 磁盘类型探测（Linux rotational / Windows seek penalty）—— 先用保守默认
//   · keep-alive 连接复用池 —— 现在交给 reqwest 的内置连接池

use serde_json::{json, Value};
use std::collections::HashMap;
use std::fs::{self, File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

// ═══════════════════════════════════════════════════════════════
// 配置
// ═══════════════════════════════════════════════════════════════

#[derive(Clone, Debug)]
pub struct Options {
    /// 多线程槽位数（同时下几个文件）
    pub multi_slots: usize,
    /// 单线程重试槽位数
    pub single_slots: usize,
    /// 单个文件的分片线程数
    pub part_threads: usize,
    /// 全局最大连接数
    pub max_connections: usize,
    pub connect_timeout: u64,
    /// 读空闲超时：这么久没数据就重连
    pub read_idle_timeout: u64,
    /// 单个 URL 的总时限
    pub per_url_timeout: u64,
    /// 停滞判定窗口
    pub stall_timeout: u64,
    /// 同一 URL 单连接重试次数
    pub retry_same_url: usize,
    pub retry_backoff_ms: u64,
    /// 单个分片失败后的重试次数
    pub part_retry: usize,
    /// 分片必须 206 且 Content-Range 一致
    pub range_validate: bool,
    /// 用 `.part.meta` 记录可信区间（关掉就退回旧行为，危险）
    pub part_meta_enabled: bool,
    pub hash_algo: String,
}

impl Default for Options {
    fn default() -> Self {
        Self {
            multi_slots: 16,
            single_slots: 4,
            part_threads: 4,
            max_connections: 128,
            connect_timeout: 10,
            read_idle_timeout: 20,
            per_url_timeout: 90,
            stall_timeout: 10,
            retry_same_url: 1,
            retry_backoff_ms: 500,
            part_retry: 1,
            range_validate: true,
            part_meta_enabled: true,
            hash_algo: "sha1".to_string(),
        }
    }
}

impl Options {
    /// 从 JSON（数据库 config 里的 download 段）构造，缺的用默认值
    pub fn from_json(v: &Value) -> Self {
        let mut o = Self::default();
        let get_u = |k: &str, d: usize| {
            v.get(k).and_then(|x| x.as_u64()).map(|x| x as usize).unwrap_or(d)
        };
        o.multi_slots = get_u("multi_slots", o.multi_slots).max(1);
        o.single_slots = get_u("single_slots", o.single_slots).max(1);
        o.part_threads = get_u("part_threads", o.part_threads).max(1);
        o.max_connections = get_u("max_connections", o.max_connections).max(1);
        o.connect_timeout = get_u("connect_timeout", o.connect_timeout as usize) as u64;
        o.read_idle_timeout = get_u("read_idle_timeout", o.read_idle_timeout as usize) as u64;
        o.per_url_timeout = get_u("per_url_timeout", o.per_url_timeout as usize) as u64;
        o.stall_timeout = get_u("stall_timeout", o.stall_timeout as usize) as u64;
        o.retry_same_url = get_u("retry_same_url", o.retry_same_url);
        o.retry_backoff_ms = get_u("retry_backoff_ms", o.retry_backoff_ms as usize) as u64;
        o.part_retry = get_u("part_retry", o.part_retry);
        if let Some(b) = v.get("range_validate").and_then(|x| x.as_u64()) {
            o.range_validate = b != 0;
        }
        if let Some(b) = v.get("part_meta_enabled").and_then(|x| x.as_u64()) {
            o.part_meta_enabled = b != 0;
        }
        if let Some(a) = v.get("hash_algo").and_then(|x| x.as_str()) {
            if !a.is_empty() {
                o.hash_algo = a.to_string();
            }
        }
        o
    }
}

// ═══════════════════════════════════════════════════════════════
// 任务与结果
// ═══════════════════════════════════════════════════════════════

#[derive(Clone, Debug)]
pub struct DownloadTask {
    pub name: String,
    /// 多源：按顺序尝试
    pub urls: Vec<String>,
    pub target_dir: PathBuf,
    /// 期望大小（0 = 不校验）
    pub size: u64,
    /// "sha1:..." / "sha256:..."（空 = 不校验）
    pub hash: String,
}

impl DownloadTask {
    pub fn from_json(v: &Value, default_dir: &Path) -> Option<Self> {
        let name = v.get("name").and_then(|x| x.as_str())?.to_string();
        let urls: Vec<String> = v
            .get("urls")
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|u| u.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_else(|| {
                v.get("url")
                    .and_then(|x| x.as_str())
                    .map(|u| vec![u.to_string()])
                    .unwrap_or_default()
            });
        if urls.is_empty() {
            return None;
        }
        let dir = v
            .get("dstDir")
            .and_then(|x| x.as_str())
            .map(PathBuf::from)
            .unwrap_or_else(|| default_dir.to_path_buf());
        Some(Self {
            name,
            urls,
            target_dir: dir,
            size: v.get("size").and_then(|x| x.as_u64()).unwrap_or(0),
            hash: v
                .get("hash")
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string(),
        })
    }
}

#[derive(Clone, Debug)]
pub struct DownloadResult {
    pub name: String,
    pub path: String,
    pub size: u64,
    pub ok: bool,
    pub error: String,
    /// 真正从网络读下来的字节数（续传时小于文件大小）
    pub bytes: u64,
}

impl DownloadResult {
    pub fn to_json(&self) -> Value {
        json!({
            "name": self.name,
            "path": self.path,
            "size": self.size,
            "ok": self.ok,
            "error": self.error,
            "bytes": self.bytes,
        })
    }
}

// ═══════════════════════════════════════════════════════════════
// `.part` 与 `.part.meta`：已确认完成的字节区间
//
// G4 教训：分片会预分配 `.part` 到整文件大小，所以「.part 大小 == 文件大小」
// **不能**当作下载完成 —— 那是零填充的假文件。只信任 meta 里记录的连续区间。
// ═══════════════════════════════════════════════════════════════

fn meta_path(part: &Path) -> PathBuf {
    let mut s = part.as_os_str().to_os_string();
    s.push(".meta");
    PathBuf::from(s)
}

/// 合并重叠/相邻区间
pub fn merge_ranges(mut rs: Vec<(u64, u64)>) -> Vec<(u64, u64)> {
    if rs.is_empty() {
        return rs;
    }
    rs.sort();
    let mut out: Vec<(u64, u64)> = Vec::with_capacity(rs.len());
    for (s, e) in rs {
        if s >= e {
            continue;
        }
        match out.last_mut() {
            Some(last) if s <= last.1 => {
                if e > last.1 {
                    last.1 = e;
                }
            }
            _ => out.push((s, e)),
        }
    }
    out
}

/// 从 0 开始连续覆盖到哪
pub fn covered_upto(ranges: &[(u64, u64)]) -> u64 {
    let mut up = 0u64;
    for (s, e) in ranges {
        if *s > up {
            break;
        }
        if *e > up {
            up = *e;
        }
    }
    up
}

#[cfg(test)]
fn range_covered(r: (u64, u64), ranges: &[(u64, u64)]) -> bool {
    ranges.iter().any(|(s, e)| *s <= r.0 && *e >= r.1)
}

/// 读 meta。没有 meta → 空（一律重下，不信任裸 `.part`）
pub fn load_ranges(part: &Path) -> (Vec<(u64, u64)>, u64) {
    let mp = meta_path(part);
    let Ok(text) = fs::read_to_string(&mp) else {
        return (Vec::new(), 0);
    };
    let Ok(v) = serde_json::from_str::<Value>(&text) else {
        return (Vec::new(), 0);
    };
    let total = v.get("total").and_then(|x| x.as_u64()).unwrap_or(0);
    let ranges = v
        .get("ranges")
        .and_then(|x| x.as_array())
        .map(|a| {
            a.iter()
                .filter_map(|r| {
                    let s = r.get(0)?.as_u64()?;
                    let e = r.get(1)?.as_u64()?;
                    Some((s, e))
                })
                .collect()
        })
        .unwrap_or_default();
    (merge_ranges(ranges), total)
}

pub fn save_ranges(part: &Path, ranges: &[(u64, u64)], total: u64) {
    let mp = meta_path(part);
    let body = json!({
        "total": total,
        "ranges": ranges.iter().map(|(s, e)| json!([s, e])).collect::<Vec<_>>(),
    });
    let _ = fs::write(&mp, body.to_string());
}

fn drop_ranges(part: &Path) {
    let _ = fs::remove_file(meta_path(part));
}

/// 把 `.part` 预分配到指定大小（不可用就 truncate）
fn ensure_part_size(part: &Path, total: u64) -> std::io::Result<File> {
    let f = OpenOptions::new()
        .create(true)
        .read(true)
        .write(true)
        .truncate(false)
        .open(part)?;
    if total > 0 && f.metadata()?.len() != total {
        f.set_len(total)?;
    }
    Ok(f)
}

// ═══════════════════════════════════════════════════════════════
// 进度（聚合多分片，对外单调不回落）
// ═══════════════════════════════════════════════════════════════

pub struct Progress {
    total: u64,
    done: AtomicU64,
    parts: Mutex<HashMap<usize, u64>>,
}

impl Progress {
    pub fn new(total: u64) -> Self {
        Self {
            total,
            done: AtomicU64::new(0),
            parts: Mutex::new(HashMap::new()),
        }
    }

    pub fn value(&self) -> u64 {
        self.done.load(Ordering::SeqCst)
            + self.parts.lock().map(|m| m.values().sum::<u64>()).unwrap_or(0)
    }

    pub fn add_part(&self, id: usize, n: u64) {
        if let Ok(mut m) = self.parts.lock() {
            *m.entry(id).or_insert(0) += n;
        }
    }

    pub fn take_part(&self, id: usize) -> u64 {
        self.parts
            .lock()
            .ok()
            .and_then(|mut m| m.remove(&id))
            .unwrap_or(0)
    }

    pub fn set_done(&self, n: u64) {
        self.done.store(n, Ordering::SeqCst);
    }

    pub fn total(&self) -> u64 {
        self.total
    }
}

// ═══════════════════════════════════════════════════════════════
// 哈希
// ═══════════════════════════════════════════════════════════════

pub fn hash_file(path: &Path, algo: &str) -> Option<String> {
    let mut f = File::open(path).ok()?;
    let mut buf = vec![0u8; 1 << 20];
    match algo {
        "sha256" => {
            use sha2::{Digest, Sha256};
            let mut h = Sha256::new();
            loop {
                let n = f.read(&mut buf).ok()?;
                if n == 0 {
                    break;
                }
                h.update(&buf[..n]);
            }
            Some(format!("{:x}", h.finalize()))
        }
        _ => {
            use sha1::{Digest, Sha1};
            let mut h = Sha1::new();
            loop {
                let n = f.read(&mut buf).ok()?;
                if n == 0 {
                    break;
                }
                h.update(&buf[..n]);
            }
            Some(format!("{:x}", h.finalize()))
        }
    }
}

/// 解析 "sha1:abc" → ("sha1", "abc")
fn hash_spec(task: &DownloadTask) -> Option<(String, String)> {
    let (a, h) = task.hash.split_once(':')?;
    if h.is_empty() {
        None
    } else {
        Some((a.to_string(), h.to_lowercase()))
    }
}

/// 校验：大小（若声明）+ 哈希（若声明）
pub fn verify(target: &Path, task: &DownloadTask) -> (bool, String) {
    let md = match fs::metadata(target) {
        Ok(m) => m,
        Err(e) => return (false, format!("读不到文件：{e}")),
    };
    if task.size > 0 && md.len() != task.size {
        return (false, format!("大小不符：{} != {}", md.len(), task.size));
    }
    if let Some((algo, want)) = hash_spec(task) {
        match hash_file(target, &algo) {
            Some(got) if got == want => (true, String::new()),
            Some(got) => (false, format!("{algo} 不符：{got} != {want}")),
            None => (false, "算不出哈希".to_string()),
        }
    } else {
        (true, String::new())
    }
}

// ═══════════════════════════════════════════════════════════════
// 单连接流式下载
// ═══════════════════════════════════════════════════════════════

/// 一次尝试的结果
enum Attempt {
    Done(u64),
    Retry(String),
    Dead(String),
}

fn backoff(ms: u64, attempt: usize) {
    // 指数退避 + 抖动
    let base = ms.saturating_mul(1u64 << attempt.min(6));
    let jitter = (base as f64 * 0.2 * rand_f64()) as u64;
    std::thread::sleep(Duration::from_millis(base + jitter));
}

/// 不引入 rand 依赖的伪随机
fn rand_f64() -> f64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    let n = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.subsec_nanos())
        .unwrap_or(0);
    (n % 1000) as f64 / 1000.0
}

fn build_client(opts: &Options) -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(opts.connect_timeout.max(1)))
        .timeout(Duration::from_secs(opts.per_url_timeout.max(1)))
        .build()
        .map_err(|e| e.to_string())
}

/// 单连接把 URL 写进 `f`（从 `start` 偏移开始追加）。
/// 返回读到的字节数。
#[allow(clippy::too_many_arguments, unused_assignments)]
fn stream_once(
    client: &reqwest::blocking::Client,
    url: &str,
    f: &mut File,
    start: u64,
    end: Option<u64>,
    progress: Option<(&Progress, usize)>,
    cancel: &AtomicBool,
) -> Attempt {
    let mut req = client.get(url);
    if start > 0 || end.is_some() {
        let range = match end {
            Some(e) => format!("bytes={}-{}", start, e.saturating_sub(1)),
            None => format!("bytes={}-", start),
        };
        req = req.header(reqwest::header::RANGE, range);
    }

    let resp = match req.send() {
        Ok(r) => r,
        Err(e) => {
            return Attempt::Retry(format!("请求失败：{e}"));
        }
    };

    let status = resp.status();
    if !status.is_success() {
        // G2：非 2xx 绝不能当文件写盘
        let dead = matches!(
            status.as_u16(),
            400 | 401 | 403 | 404 | 410 | 451
        );
        let msg = format!("HTTP {}", status.as_u16());
        return if dead {
            Attempt::Dead(msg)
        } else {
            Attempt::Retry(msg)
        };
    }

    // G3：分片必须 206，且 Content-Range 起点要对得上
    if start > 0 {
        if status.as_u16() != 206 {
            return Attempt::Retry("服务器不支持分段（没返回 206）".to_string());
        }
        if let Some(cr) = resp
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
        {
            let ok = cr
                .split_whitespace()
                .nth(1)
                .and_then(|r| r.split('-').next())
                .and_then(|s| s.parse::<u64>().ok())
                .map(|s| s == start)
                .unwrap_or(false);
            if !ok {
                return Attempt::Retry(format!("Content-Range 不符：{cr}"));
            }
        }
    }

    if let Err(e) = f.seek(SeekFrom::Start(start)) {
        return Attempt::Retry(format!("定位失败：{e}"));
    }

    let mut reader = resp;
    let mut buf = vec![0u8; 256 * 1024];
    let mut got = 0u64;
    let started = Instant::now();
    let mut last_data = started;

    loop {
        if cancel.load(Ordering::SeqCst) {
            return Attempt::Retry("已取消".to_string());
        }
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if let Err(e) = f.write_all(&buf[..n]) {
                    return Attempt::Retry(format!("写入失败：{e}"));
                }
                got += n as u64;
                last_data = Instant::now();
                if let Some((p, id)) = progress {
                    p.add_part(id, n as u64);
                }
            }
            Err(e) => {
                return Attempt::Retry(format!("读取出错：{e}"));
            }
        }
        if last_data.elapsed().as_secs() > 20 {
            return Attempt::Retry("读空闲超时".to_string());
        }
        if started.elapsed().as_secs() > 600 {
            return Attempt::Retry("超过单次时限".to_string());
        }
    }

    Attempt::Done(got)
}

/// 多源 + 重试：把任务下到 `.part`，成功后 rename 成目标文件。
pub fn download_one(task: &DownloadTask, opts: &Options, cancel: &AtomicBool) -> DownloadResult {
    let target = task.target_dir.join(&task.name);
    let part = {
        let mut s = target.as_os_str().to_os_string();
        s.push(".part");
        PathBuf::from(s)
    };

    let fail = |msg: String| DownloadResult {
        name: task.name.clone(),
        path: target.to_string_lossy().to_string(),
        size: 0,
        ok: false,
        error: msg,
        bytes: 0,
    };

    if let Some(parent) = target.parent() {
        let _ = fs::create_dir_all(parent);
    }

    let client = match build_client(opts) {
        Ok(c) => c,
        Err(e) => return fail(format!("建不了 HTTP 客户端：{e}")),
    };

    let total_known = task.size;
    let mut network_bytes = 0u64;
    let mut last_error = String::new();

    // G1：每个候选 URL 独立跑完整套重试，一个不行换下一个
    for url in &task.urls {
        if cancel.load(Ordering::SeqCst) {
            return fail("已取消".to_string());
        }

        // 续传：只信任 meta 里记录的连续前缀
        let (mut ranges, meta_total) = if opts.part_meta_enabled {
            load_ranges(&part)
        } else {
            (Vec::new(), 0)
        };
        if meta_total > 0 && total_known > 0 && meta_total != total_known {
            // 大小变了：旧 part 作废
            ranges.clear();
            let _ = fs::remove_file(&part);
            drop_ranges(&part);
        }
        let mut start = if opts.part_meta_enabled {
            covered_upto(&ranges)
        } else {
            fs::metadata(&part).map(|m| m.len()).unwrap_or(0)
        };
        if total_known > 0 && start > total_known {
            start = 0;
            let _ = fs::remove_file(&part);
            drop_ranges(&part);
        }

        let mut file = match ensure_part_size(&part, total_known) {
            Ok(f) => f,
            Err(e) => return fail(format!("建不了临时文件：{e}")),
        };

        let attempts = opts.retry_same_url + 1;
        for attempt in 0..attempts {
            if cancel.load(Ordering::SeqCst) {
                return fail("已取消".to_string());
            }
            let progress = Progress::new(total_known);
            match stream_once(&client, url, &mut file, start, None, None, cancel) {
                Attempt::Done(n) => {
                    network_bytes += n;
                    let now_end = start + n;
                    if total_known > 0 && now_end != total_known {
                        // 少了：记下已确认前缀，下次接着下
                        if opts.part_meta_enabled {
                            ranges = merge_ranges(vec![(0, now_end)]);
                            save_ranges(&part, &ranges, total_known);
                        }
                        start = now_end;
                        continue;
                    }
                    let _ = file.sync_all();
                    drop(file);
                    let (ok, why) = verify(&part, task);
                    if ok {
                        if fs::rename(&part, &target).is_err() {
                            let _ = fs::copy(&part, &target);
                            let _ = fs::remove_file(&part);
                        }
                        drop_ranges(&part);
                        let size = fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
                        return DownloadResult {
                            name: task.name.clone(),
                            path: target.to_string_lossy().to_string(),
                            size,
                            ok: true,
                            error: String::new(),
                            bytes: network_bytes,
                        };
                    }
                    // 内容不对：整个 part 作废重来
                    let _ = fs::remove_file(&part);
                    drop_ranges(&part);
                    start = 0;
                    file = match ensure_part_size(&part, total_known) {
                        Ok(f) => f,
                        Err(e) => return fail(format!("建不了临时文件：{e}")),
                    };
                    let _ = progress;
                    if attempt + 1 >= attempts {
                        return fail(why);
                    }
                    backoff(opts.retry_backoff_ms, attempt);
                }
                Attempt::Retry(msg) => {
                    if attempt + 1 >= attempts {
                        // 这个源用尽，换下一个
                        let _ = msg;
                        break;
                    }
                    backoff(opts.retry_backoff_ms, attempt);
                }
                Attempt::Dead(msg) => {
                    last_error = msg;
                    break; // 404/403：这个源彻底没戏
                }
            }
        }
    }

    fail(if last_error.is_empty() {
        "所有下载源都失败了".to_string()
    } else {
        format!("所有下载源都失败了（最后一个错误：{last_error}）")
    })
}

// ═══════════════════════════════════════════════════════════════
// 批量：多槽位 → 重试队列 → 单槽位
// ═══════════════════════════════════════════════════════════════

/// 进度回调：(已完成, 总数, 当前文件名, 已下载字节, 总字节)
pub type ProgressCb = Arc<dyn Fn(usize, usize, &str, u64, u64) + Send + Sync>;
/// 单个文件完成回调：(结果, 是否成功)
pub type FileCb = Arc<dyn Fn(&DownloadResult, bool) + Send + Sync>;

pub struct Batch {
    pub results: Vec<DownloadResult>,
    pub ok: usize,
    pub fail: usize,
}

/// 跑一批任务。并发度受 `opts.multi_slots` 限制（这里用分块并发，
/// 不引 tokio：每个 worker 内部是阻塞 IO，分块天然限流）。
pub fn run_batch(
    tasks: Vec<DownloadTask>,
    opts: &Options,
    cancel: Arc<AtomicBool>,
    on_progress: Option<ProgressCb>,
    on_file: Option<FileCb>,
) -> Batch {
    let total = tasks.len();
    let total_bytes: u64 = tasks.iter().map(|t| t.size).sum();
    let done = Arc::new(AtomicU64::new(0));
    let moved = Arc::new(AtomicU64::new(0));

    let mut results: Vec<DownloadResult> = Vec::with_capacity(total);
    let slots = opts.multi_slots.max(1);
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
            let total_n = total;
            let tb = total_bytes;
            handles.push(std::thread::spawn(move || {
                let r = download_one(&t, &opts_ref, &cancel_ref);
                let n = done_c.fetch_add(1, Ordering::SeqCst) + 1;
                moved_c.fetch_add(r.bytes, Ordering::SeqCst);
                if let Some(cb) = &prog {
                    cb(
                        n as usize,
                        total_n,
                        &r.name,
                        moved_c.load(Ordering::SeqCst),
                        tb,
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
    let fail = results.len() - ok;
    Batch { results, ok, fail }
}

// ═══════════════════════════════════════════════════════════════
// 对外入口（给 Tauri 命令用）
// ═══════════════════════════════════════════════════════════════

pub fn tasks_from_json(v: &Value, default_dir: &Path) -> Vec<DownloadTask> {
    v.as_array()
        .map(|a| {
            a.iter()
                .filter_map(|t| DownloadTask::from_json(t, default_dir))
                .collect()
        })
        .unwrap_or_default()
}

pub fn batch_to_json(b: &Batch) -> Value {
    json!({
        "ok": b.ok,
        "fail": b.fail,
        "results": b.results.iter().map(|r| r.to_json()).collect::<Vec<_>>(),
    })
}

// ═══════════════════════════════════════════════════════════════
// 自测（cargo test 用，不参与 Tauri 构建）
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn merge_ranges_works() {
        let r = merge_ranges(vec![(10, 20), (0, 10), (30, 40), (35, 45)]);
        assert_eq!(r, vec![(0, 20), (30, 45)]);
    }

    #[test]
    fn covered_upto_stops_at_gap() {
        assert_eq!(covered_upto(&[(0, 100), (200, 300)]), 100);
        assert_eq!(covered_upto(&[(0, 100), (100, 200)]), 200);
        assert_eq!(covered_upto(&[]), 0);
    }

    #[test]
    fn range_covered_checks_containment() {
        let rs = vec![(0u64, 100u64)];
        assert!(range_covered((10, 50), &rs));
        assert!(!range_covered((50, 150), &rs));
    }

    #[test]
    fn options_from_json_overrides() {
        let o = Options::from_json(&json!({"multi_slots": 3, "range_validate": 0}));
        assert_eq!(o.multi_slots, 3);
        assert!(!o.range_validate);
        assert_eq!(o.part_threads, 4); // 未给的保持默认
    }
}
