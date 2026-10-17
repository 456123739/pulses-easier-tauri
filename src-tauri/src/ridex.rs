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
    /// 小于这个大小就不分片。几千个几 KB 的资源文件分片纯属浪费
    pub min_part_size: u64,
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
            min_part_size: 1_048_576,
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
        if let Some(b) = v.get("min_part_size").and_then(|x| x.as_u64()) {
            o.min_part_size = b;
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

    /// 已完成（不含在途分片）
    pub fn done_value(&self) -> u64 {
        self.done.load(Ordering::SeqCst)
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
// 速度窗口：连续一段时间低于阈值就判「这条连接没戏」
// （Python 版 B2：连续 3 秒 < 20 KB/s 就移出多线程槽位）
// ═══════════════════════════════════════════════════════════════

pub struct SpeedWindow {
    window: Duration,
    min_bytes_per_sec: f64,
    start: Instant,
    last_bytes: u64,
    last_ts: Instant,
}

impl SpeedWindow {
    pub fn new(window_secs: u64, min_bytes_per_sec: f64) -> Self {
        let now = Instant::now();
        Self {
            window: Duration::from_secs(window_secs.max(1)),
            min_bytes_per_sec,
            start: now,
            last_bytes: 0,
            last_ts: now,
        }
    }

    /// 喂进累计字节数；到窗口期就结算一次。
    /// 返回 true 表示「太慢，该放弃了」。
    pub fn feed(&mut self, total_bytes: u64) -> bool {
        let now = Instant::now();
        if now.duration_since(self.start) < self.window {
            return false;
        }
        let dt = now.duration_since(self.last_ts).as_secs_f64().max(0.001);
        let db = total_bytes.saturating_sub(self.last_bytes) as f64;
        let speed = db / dt;
        self.start = now;
        self.last_bytes = total_bytes;
        self.last_ts = now;
        speed < self.min_bytes_per_sec
    }
}

// ═══════════════════════════════════════════════════════════════
// 磁盘类型策略：并发只降不升
// ═══════════════════════════════════════════════════════════════

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DiskKind {
    Ssd,
    Hdd,
    Network,
    Removable,
    Unknown,
}

impl DiskKind {
    /// 这类盘最多允许几个分片。机械盘/网络盘随机读代价高，分片反而更慢。
    pub fn part_threads_cap(self) -> usize {
        match self {
            DiskKind::Ssd => usize::MAX,
            DiskKind::Hdd | DiskKind::Network | DiskKind::Removable => 1,
            DiskKind::Unknown => usize::MAX,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            DiskKind::Ssd => "ssd",
            DiskKind::Hdd => "hdd",
            DiskKind::Network => "network",
            DiskKind::Removable => "removable",
            DiskKind::Unknown => "unknown",
        }
    }
}

#[cfg(windows)]
pub fn detect_disk_kind(path: &Path) -> DiskKind {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{
        GetDriveTypeW, DRIVE_FIXED, DRIVE_REMOTE, DRIVE_REMOVABLE,
    };
    // 取盘符：C:\...
    let s = path.to_string_lossy().to_string();
    let mut drive: Vec<u16> = s.chars().take(3).collect::<String>().encode_utf16().collect();
    while drive.len() < 4 {
        drive.push('\\' as u16);
    }
    drive.push(0);
    let t = unsafe { GetDriveTypeW(drive.as_ptr()) };
    if t == DRIVE_REMOTE {
        DiskKind::Network
    } else if t == DRIVE_REMOVABLE {
        DiskKind::Removable
    } else if t == DRIVE_FIXED {
        DiskKind::Ssd // 固定盘默认按 SSD（不额外限制）
    } else {
        DiskKind::Unknown
    }
}

#[cfg(target_os = "linux")]
pub fn detect_disk_kind(path: &Path) -> DiskKind {
    // 找挂载点 → 设备 → /sys/block/<dev>/queue/rotational
    let Ok(mounts) = fs::read_to_string("/proc/mounts") else {
        return DiskKind::Unknown;
    };
    let p = fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
    let ps = p.to_string_lossy().to_string();
    let mut best: Option<(usize, String)> = None;
    for line in mounts.lines() {
        let mut it = line.split_whitespace();
        let (Some(dev), Some(mnt)) = (it.next(), it.next()) else { continue };
        if mnt.starts_with("/dev") && ps.starts_with(mnt) && mnt != "/" {
            continue;
        }
        if ps.starts_with(mnt) {
            let n = mnt.len();
            if best.as_ref().map(|(l, _)| n > *l).unwrap_or(true) {
                best = Some((n, dev.to_string()));
            }
        }
    }
    let Some((_, dev)) = best else { return DiskKind::Unknown };
    if dev.starts_with("/dev/nfs") || dev.contains("cifs") || dev.contains("nfs") {
        return DiskKind::Network;
    }
    let base = dev.rsplit('/').next().unwrap_or("");
    let name: String = base.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    let rot = fs::read_to_string(format!("/sys/block/{name}/queue/rotational"))
        .ok()
        .and_then(|s| s.trim().parse::<u8>().ok());
    match rot {
        Some(0) => DiskKind::Ssd,
        Some(_) => DiskKind::Hdd,
        None => DiskKind::Unknown,
    }
}

#[cfg(not(any(windows, target_os = "linux")))]
pub fn detect_disk_kind(_path: &Path) -> DiskKind {
    DiskKind::Unknown
}

/// 目标目录实际允许的分片数（只降不升）
pub fn part_threads_for(dir: &Path, want: usize) -> usize {
    let cap = detect_disk_kind(dir).part_threads_cap();
    want.min(cap).max(1)
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

/// 请求用的 User-Agent。
///
/// 必须发：reqwest 默认不带 UA，清华镜像站（以及不少国内 CDN 的 WAF）
/// 会直接回 403 —— 表现是「明明浏览器/curl 下得飞快，程序里一个字节都下不来」。
/// 用浏览器串而不是自定义串，是为了过那种只放行常见 UA 的站点。
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
(KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

fn build_client(opts: &Options) -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .connect_timeout(Duration::from_secs(opts.connect_timeout.max(1)))
        .timeout(Duration::from_secs(opts.per_url_timeout.max(1)))
        .user_agent(UA)
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
    // 单连接阶段用宽松阈值：1 KB/s 持续 5 秒才放弃
    let mut speed = SpeedWindow::new(5, 1024.0);

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
                if speed.feed(got) {
                    return Attempt::Retry("连接太慢（低于 1 KB/s）".to_string());
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
///
/// 自己建 HTTP 客户端。批量下载请走 [`run_batch`]，它会让所有文件
/// 复用同一个连接池 —— 资源文件有三千多个，每个都重新握手会慢十倍。
pub fn download_one(
    task: &DownloadTask,
    opts: &Options,
    cancel: std::sync::Arc<AtomicBool>,
) -> DownloadResult {
    match build_client(opts) {
        Ok(c) => download_one_with(&c, task, opts, cancel),
        Err(e) => DownloadResult {
            name: task.name.clone(),
            path: task.target_dir.join(&task.name).to_string_lossy().to_string(),
            size: 0,
            ok: false,
            error: format!("建不了 HTTP 客户端：{e}"),
            bytes: 0,
        },
    }
}

/// 同 [`download_one`]，但用外面给的客户端（连接池共享）。
pub fn download_one_with(
    client: &reqwest::blocking::Client,
    task: &DownloadTask,
    opts: &Options,
    cancel: std::sync::Arc<AtomicBool>,
) -> DownloadResult {
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

    let client = client.clone();

    let total_known = task.size;
    let mut network_bytes = 0u64;
    let mut last_error = String::new();

    // 收尾：校验 → rename → 返回结果
    let finish = |part: &Path, network_bytes: u64| -> Result<DownloadResult, String> {
        let (ok, why) = verify(part, task);
        if !ok {
            let _ = fs::remove_file(part);
            drop_ranges(part);
            return Err(why);
        }
        if fs::rename(part, &target).is_err() {
            let _ = fs::copy(part, &target);
            let _ = fs::remove_file(part);
        }
        drop_ranges(part);
        let size = fs::metadata(&target).map(|m| m.len()).unwrap_or(0);
        Ok(DownloadResult {
            name: task.name.clone(),
            path: target.to_string_lossy().to_string(),
            size,
            ok: true,
            error: String::new(),
            bytes: network_bytes,
        })
    };

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
            ranges.clear();
            let _ = fs::remove_file(&part);
            drop_ranges(&part);
        }

        // ── 第一阶段：单连接 ──
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

        if let Ok(mut file) = ensure_part_size(&part, total_known) {
            let attempts = opts.retry_same_url + 1;
            for attempt in 0..attempts {
                if cancel.load(Ordering::SeqCst) {
                    return fail("已取消".to_string());
                }
                match stream_once(&client, url, &mut file, start, None, None, &cancel) {
                    Attempt::Done(n) => {
                        network_bytes += n;
                        let now_end = start + n;
                        if total_known > 0 && now_end != total_known {
                            if opts.part_meta_enabled {
                                ranges = merge_ranges(vec![(0, now_end)]);
                                save_ranges(&part, &ranges, total_known);
                            }
                            start = now_end;
                            let _ = file.seek(SeekFrom::Start(start));
                            continue;
                        }
                        let _ = file.sync_all();
                        drop(file);
                        match finish(&part, network_bytes) {
                            Ok(r) => return r,
                            Err(why) => {
                                last_error = why;
                                break;
                            }
                        }
                    }
                    Attempt::Retry(msg) => {
                        last_error = msg;
                        if attempt + 1 >= attempts {
                            break;
                        }
                        backoff(opts.retry_backoff_ms, attempt);
                    }
                    Attempt::Dead(msg) => {
                        last_error = msg;
                        break;
                    }
                }
            }
        }

        if cancel.load(Ordering::SeqCst) {
            return fail("已取消".to_string());
        }

        // 404/403 是「这个源彻底没戏」，别再浪费请求试分片
        if last_error.starts_with("HTTP 4") {
            continue;
        }

        // ── 第二阶段：分片（B1：单连接不行才分） ──
        let total = if total_known > 0 {
            Some(total_known)
        } else {
            probe_size(&client, url)
        };
        if let Some(total) = total {
            if total > 0 && opts.part_threads > 1 && total >= opts.min_part_size {
                let (done_ranges, _) = if opts.part_meta_enabled {
                    load_ranges(&part)
                } else {
                    (Vec::new(), 0)
                };
                match download_multi(
                    &client, url, &part, total, done_ranges, opts, cancel.clone(),
                ) {
                    Ok(n) => {
                        network_bytes += n;
                        if opts.part_meta_enabled {
                            save_ranges(&part, &[(0, total)], total);
                        }
                        match finish(&part, network_bytes) {
                            Ok(r) => return r,
                            Err(why) => last_error = why,
                        }
                    }
                    Err(e) => {
                        last_error = e;
                        let _ = fs::remove_file(&part);
                        drop_ranges(&part);
                    }
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
// 分片下载
// ═══════════════════════════════════════════════════════════════

/// 把 [0, total) 切成 n 段（半开区间）。太小的文件不分片。
pub fn plan_parts(total: u64, threads: usize) -> Vec<(u64, u64)> {
    if total == 0 {
        return vec![(0, 0)];
    }
    let min_part = 256 * 1024u64; // 小于这个不分，分片开销不划算
    let max_n = ((total / min_part).max(1) as usize).min(threads.max(1));
    if max_n <= 1 {
        return vec![(0, total)];
    }
    let chunk = total / max_n as u64;
    let mut out = Vec::with_capacity(max_n);
    let mut s = 0u64;
    for i in 0..max_n {
        let e = if i + 1 == max_n { total } else { s + chunk };
        out.push((s, e));
        s = e;
    }
    out
}

/// 探一下文件总大小：发 `Range: bytes=0-0`，从 `Content-Range` 的 `/N` 取。
fn probe_size(client: &reqwest::blocking::Client, url: &str) -> Option<u64> {
    let resp = client
        .get(url)
        .header(reqwest::header::RANGE, "bytes=0-0")
        .send()
        .ok()?;
    if let Some(cr) = resp
        .headers()
        .get(reqwest::header::CONTENT_RANGE)
        .and_then(|v| v.to_str().ok())
    {
        if let Some(total) = cr.rsplit('/').next().and_then(|x| x.trim().parse::<u64>().ok()) {
            return Some(total);
        }
    }
    // 服务器不支持 Range：退回到 Content-Length
    resp.headers()
        .get(reqwest::header::CONTENT_LENGTH)
        .and_then(|v| v.to_str().ok())
        .and_then(|s| s.parse::<u64>().ok())
}

/// 一个分片：从 url 拉 [s, e)，写到 part 的对应偏移。
#[allow(unused_assignments)]
fn fetch_part(
    client: &reqwest::blocking::Client,
    url: &str,
    part: &Path,
    r: (u64, u64),
    validate: bool,
    cancel: &AtomicBool,
    progress: Option<(&Progress, usize)>,
) -> Attempt {
    let (s, e) = r;
    let mut req = client
        .get(url)
        .header(reqwest::header::RANGE, format!("bytes={}-{}", s, e.saturating_sub(1)));

    // 每次重试都新建连接头，避免复用陈旧 keep-alive
    req = req.header(reqwest::header::CONNECTION, "close");

    let resp = match req.send() {
        Ok(r) => r,
        Err(err) => return Attempt::Retry(format!("分片请求失败：{err}")),
    };

    let status = resp.status();
    if status.as_u16() != 206 {
        // 服务器没按 Range 回，说明不支持分段 —— 交给上层换单连接
        return Attempt::Retry(format!("分片要求 206，实际 {}", status.as_u16()));
    }
    if validate {
        let cr = resp
            .headers()
            .get(reqwest::header::CONTENT_RANGE)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");
        // 形如 "bytes 0-199999/200000"：先去掉 /total 再拆 -
        let ok = cr
            .split_whitespace()
            .nth(1)
            .and_then(|x| x.split('/').next())
            .and_then(|x| x.split_once('-'))
            .map(|(a, b)| {
                a.trim().parse::<u64>().ok() == Some(s)
                    && b.trim().parse::<u64>().ok() == Some(e.saturating_sub(1))
            })
            .unwrap_or(false);
        if !ok {
            return Attempt::Retry(format!("Content-Range 不符：{cr}"));
        }
    }

    let mut f = match OpenOptions::new().write(true).open(part) {
        Ok(f) => f,
        Err(err) => return Attempt::Retry(format!("打开 part 失败：{err}")),
    };
    if let Err(err) = f.seek(SeekFrom::Start(s)) {
        return Attempt::Retry(format!("定位失败：{err}"));
    }

    let mut reader = resp;
    let mut buf = vec![0u8; 256 * 1024];
    let mut got = 0u64;
    let mut last = Instant::now();
    // 分片阶段严格一些：20 KB/s 持续 3 秒就换
    let mut speed = SpeedWindow::new(3, 20.0 * 1024.0);

    loop {
        if cancel.load(Ordering::SeqCst) {
            return Attempt::Retry("已取消".to_string());
        }
        match reader.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => {
                if let Err(err) = f.write_all(&buf[..n]) {
                    return Attempt::Retry(format!("写入失败：{err}"));
                }
                got += n as u64;
                last = Instant::now();
                if let Some((p, id)) = progress {
                    p.add_part(id, n as u64);
                }
                if speed.feed(got) {
                    return Attempt::Retry("分片太慢".to_string());
                }
            }
            Err(err) => return Attempt::Retry(format!("读取出错：{err}")),
        }
        if last.elapsed().as_secs() > 30 {
            return Attempt::Retry("分片读空闲超时".to_string());
        }
    }

    if got != e - s {
        return Attempt::Retry(format!("分片长度不符：{} != {}", got, e - s));
    }
    Attempt::Done(got)
}

/// 多分片把整个文件拉下来。返回读到的总字节数。
#[allow(clippy::too_many_arguments)]
fn download_multi(
    client: &reqwest::blocking::Client,
    url: &str,
    part: &Path,
    total: u64,
    done_ranges: Vec<(u64, u64)>,
    opts: &Options,
    cancel: std::sync::Arc<AtomicBool>,
) -> Result<u64, String> {
    // 机械盘/网络盘分片反而更慢：只降不升
    let threads = part_threads_for(
        part.parent().unwrap_or(Path::new(".")), opts.part_threads);
    let all = plan_parts(total, threads);
    let todo: Vec<(u64, u64)> = all
        .iter()
        .copied()
        .filter(|r| !range_covered_any(*r, &done_ranges))
        .collect();
    if todo.is_empty() {
        return Ok(0);
    }

    // 预分配：分片直接写偏移，不产生中间文件
    ensure_part_size(part, total).map_err(|e| e.to_string())?;

    let progress = Arc::new(Progress::new(total));
    let done = Arc::new(Mutex::new(done_ranges));
    let bytes = Arc::new(AtomicU64::new(0));
    let err = Arc::new(Mutex::new(String::new()));

    let mut handles = Vec::new();
    for (id, r) in todo.into_iter().enumerate() {
        let client = client.clone();
        let url = url.to_string();
        let part = part.to_path_buf();
        let progress = progress.clone();
        let done = done.clone();
        let bytes = bytes.clone();
        let err = err.clone();
        let cancel_flag = cancel.clone();
        let validate = opts.range_validate;
        let attempts = opts.part_retry + 1;

        handles.push(std::thread::spawn(move || {
            let cancel = cancel_flag;
            for a in 0..attempts {
                match fetch_part(&client, &url, &part, r, validate, &cancel, Some((&progress, id))) {
                    Attempt::Done(n) => {
                        bytes.fetch_add(n, Ordering::SeqCst);
                        let moved = progress.take_part(id);
                        progress.set_done(progress.done_value() + moved);
                        if let Ok(mut d) = done.lock() {
                            *d = merge_ranges({
                                let mut v = d.clone();
                                v.push(r);
                                v
                            });
                        }
                        return;
                    }
                    Attempt::Retry(m) => {
                        if a + 1 >= attempts {
                            if let Ok(mut e) = err.lock() {
                                if e.is_empty() {
                                    *e = m;
                                }
                            }
                            return;
                        }
                    }
                    Attempt::Dead(m) => {
                        if let Ok(mut e) = err.lock() {
                            if e.is_empty() {
                                *e = m;
                            }
                        }
                        return;
                    }
                }
            }
        }));
    }

    for h in handles {
        let _ = h.join();
    }

    if cancel.load(Ordering::SeqCst) {
        return Err("已取消".to_string());
    }
    let e = err.lock().map(|g| g.clone()).unwrap_or_default();
    if !e.is_empty() {
        return Err(e);
    }

    // 校验：所有分片都覆盖到了
    let d = done.lock().map(|g| g.clone()).unwrap_or_default();
    let covered = covered_upto(&d);
    if covered < total {
        return Err(format!("分片没下全：{covered}/{total}"));
    }

    Ok(bytes.load(Ordering::SeqCst))
}

/// 某个区间是否被已有区间完整覆盖
fn range_covered_any(r: (u64, u64), ranges: &[(u64, u64)]) -> bool {
    ranges.iter().any(|(s, e)| *s <= r.0 && *e >= r.1)
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

    // 连接池只建一次：三千多个小文件，每个都重新 TLS 握手会慢一个数量级
    let shared = build_client(opts).ok();

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
            let cl = shared.clone();
            handles.push(std::thread::spawn(move || {
                let r = match cl {
                    Some(c) => download_one_with(&c, &t, &opts_ref, cancel_ref.clone()),
                    None => download_one(&t, &opts_ref, cancel_ref.clone()),
                };
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
