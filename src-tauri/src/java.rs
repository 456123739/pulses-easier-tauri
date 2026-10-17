// java.rs — Java 运行时：找 + 装
//
// 为什么需要：
//   Forge / NeoForge 的安装器本身是个 Java 程序（installer.jar），
//   装原版和 Fabric 不需要 Java，但装 Forge 一定需要。
//
// 策略（用户定的）：
//   要求本机有 Java；检测不到就提示「是否自动安装」。
//   所以这里只负责「找得到什么」和「怎么装一份」，
//   装不装由前端问用户。
//
// 自动安装来源（顺序即优先级）：
//   1) 华为云 openjdk 镜像 —— 国内直连快，支持 Range，能分片
//      只收 GA 版本（21.0.2 这种），没有 8
//   2) Azul Zulu 的 metadata API —— 全球可用，8/11/17/21/25 都有
//
// 踩过的坑：清华 TUNA 的 Adoptium 镜像会按 TLS 指纹 + UA 做一致性校验，
// reqwest 的指纹配任何 UA 都被判成「非常用软件」直接 403
// （curl 自己反而能下）。所以不用 TUNA。

use serde_json::{json, Value};
use std::path::{Path, PathBuf};
use std::process::Command;

/// 请求 UA。发浏览器串是为了过那些只放行常见 UA 的站点。
const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
(KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

// ═══════════════════════════════════════════════════════════════
// 类型
// ═══════════════════════════════════════════════════════════════

#[derive(Clone, Debug)]
pub struct Java {
    /// java 可执行文件绝对路径
    pub path: String,
    /// 运行时根目录（bin 的上一级）
    pub home: String,
    /// 主版本号：8 / 17 / 21 / 25
    pub major: u32,
    /// 完整版本串，如 "21.0.12"
    pub version: String,
    /// 从哪找到的：env / path / scan / installed
    pub source: String,
}

impl Java {
    pub fn to_json(&self) -> Value {
        json!({
            "path": self.path,
            "home": self.home,
            "major": self.major,
            "version": self.version,
            "source": self.source,
        })
    }
}

// ═══════════════════════════════════════════════════════════════
// 版本要求
// ═══════════════════════════════════════════════════════════════

/// 跑这个 MC 版本（以及它的 Forge 安装器）至少需要哪个 Java 主版本。
///
///   1.20.5+      → 21
///   1.18–1.20.4  → 17
///   1.17.x       → 17（官方要求 16，但 16 已经没人发了，17 完全兼容）
///   1.16.5 及以下 → 8
pub fn required_major(mc: &str) -> u32 {
    match parse_mc(mc) {
        Some((1, minor, patch)) => {
            if minor > 20 || (minor == 20 && patch >= 5) {
                21
            } else if minor >= 17 {
                17
            } else {
                8
            }
        }
        // 快照 / 老式命名（1.2.5 之前）一律按 8 处理，装不了会明确报错
        _ => 8,
    }
}

/// "1.20.1" → (1, 20, 1)。解析不了返回 None。
fn parse_mc(s: &str) -> Option<(u32, u32, u32)> {
    let mut it = s.split('.').map(|x| x.parse::<u32>().ok());
    let a = it.next().flatten()?;
    let b = it.next().flatten()?;
    let c = it.next().flatten().unwrap_or(0);
    Some((a, b, c))
}

// ═══════════════════════════════════════════════════════════════
// 找
// ═══════════════════════════════════════════════════════════════

fn exe_name() -> &'static str {
    if cfg!(windows) {
        "java.exe"
    } else {
        "java"
    }
}

/// 不弹黑框地跑一个命令（Windows 上 java -version 会闪一下控制台）
fn hidden_cmd(program: impl AsRef<std::ffi::OsStr>) -> Command {
    #[allow(unused_mut)]
    let mut c = Command::new(program);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        c.creation_flags(CREATE_NO_WINDOW);
    }
    c
}

/// 跑 `java -version` 解析出主版本。跑不起来 / 解析不了返回 None。
pub fn probe(path: &Path) -> Option<(u32, String)> {
    let out = hidden_cmd(path.as_os_str()).arg("-version").output().ok()?;
    if !out.status.success() {
        return None;
    }
    // java 把版本打到 stderr（历史遗留）
    let text = String::from_utf8_lossy(&out.stderr).to_string()
        + &String::from_utf8_lossy(&out.stdout);
    parse_version_line(&text)
}

/// 从 `java -version` 的输出里抠出版本号。
///
///   openjdk version "21.0.12" 2025-07-15
///   java version "1.8.0_411"
///   openjdk version "17" 2021-09-14
pub fn parse_version_line(text: &str) -> Option<(u32, String)> {
    for line in text.lines() {
        let l = line.trim();
        if !l.contains("version") {
            continue;
        }
        let start = match l.find('"') {
            Some(i) => i + 1,
            None => continue,
        };
        let rest = &l[start..];
        let end = match rest.find('"') {
            Some(i) => i,
            None => continue,
        };
        let ver = &rest[..end];
        if ver.is_empty() {
            continue;
        }
        let major = if let Some(legacy) = ver.strip_prefix("1.") {
            // 1.8.0_411 → 8
            legacy.split(['.', '_', '-']).next()?.parse::<u32>().ok()?
        } else {
            ver.split(['.', '_', '-']).next()?.parse::<u32>().ok()?
        };
        return Some((major, ver.to_string()));
    }
    None
}

fn make(path: PathBuf, source: &str) -> Option<Java> {
    if !path.is_file() {
        return None;
    }
    let (major, version) = probe(&path)?;
    let home = path.parent()?.parent()?.to_string_lossy().to_string();
    Some(Java {
        path: path.to_string_lossy().to_string(),
        home,
        major,
        version,
        source: source.to_string(),
    })
}

/// 列出所有能找到的 Java，按主版本降序。
/// 同一份 Java 只留一个（按 home 去重）。
pub fn find_all() -> Vec<Java> {
    let mut out: Vec<Java> = Vec::new();
    let mut seen: Vec<String> = Vec::new();

    let mut push = |j: Option<Java>| {
        if let Some(j) = j {
            if seen.iter().any(|h| *h == j.home) {
                return;
            }
            seen.push(j.home.clone());
            out.push(j);
        }
    };

    // 1) 我们自己装过的
    for d in installed_dirs() {
        push(make(d.join("bin").join(exe_name()), "installed"));
    }

    // 2) JAVA_HOME
    if let Ok(h) = std::env::var("JAVA_HOME") {
        if !h.is_empty() {
            push(make(PathBuf::from(h).join("bin").join(exe_name()), "env"));
        }
    }

    // 3) PATH 里的 java
    if let Some(p) = which_java() {
        push(make(p, "path"));
    }

    // 4) 各平台常见安装位置
    for base in scan_roots() {
        for d in list_dirs(&base) {
            push(make(d.join("bin").join(exe_name()), "scan"));
        }
    }

    out.sort_by(|a, b| b.major.cmp(&a.major));
    out
}

/// 挑一个能用的：优先刚好够用的最低版本，其次更高的。
///
/// 为什么不是「越高越好」：老版本的 Forge 安装器在 Java 21 上会挂
/// （1.16.5 的安装器在 17 上就报 UnsupportedClassVersionError），
/// 所以有 17 就用 17，别硬塞 21。
pub fn pick_for(min_major: u32) -> Option<Java> {
    let all = find_all();
    all.iter()
        .filter(|j| j.major >= min_major)
        .min_by_key(|j| j.major)
        .cloned()
        .or_else(|| all.into_iter().next())
}

fn which_java() -> Option<PathBuf> {
    let path = std::env::var_os("PATH")?;
    for dir in std::env::split_paths(&path) {
        let p = dir.join(exe_name());
        if p.is_file() {
            return Some(p);
        }
    }
    None
}

fn list_dirs(base: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(rd) = std::fs::read_dir(base) {
        for e in rd.flatten() {
            let p = e.path();
            if p.is_dir() {
                v.push(p);
            }
        }
    }
    v.sort();
    v
}

#[cfg(windows)]
fn scan_roots() -> Vec<PathBuf> {
    let mut v = Vec::new();
    let mut add = |p: PathBuf| {
        if p.is_dir() {
            v.push(p);
        }
    };
    for env in ["ProgramFiles", "ProgramFiles(x86)", "ProgramW6432"] {
        if let Ok(b) = std::env::var(env) {
            let b = PathBuf::from(b);
            // 各家发行版的安装根
            for sub in [
                "Java",
                "Eclipse Adoptium",
                "Eclipse Foundation",
                "Microsoft",
                "Zulu",
                "BellSoft",
                "Amazon Corretto",
                "Semeru",
                "AdoptOpenJDK",
                "RedHat",
            ] {
                add(b.join(sub));
            }
        }
    }
    if let Ok(la) = std::env::var("LOCALAPPDATA") {
        let la = PathBuf::from(la);
        add(la.join("Programs").join("Eclipse Adoptium"));
        add(la.join("Programs").join("Microsoft"));
    }
    v
}

#[cfg(target_os = "macos")]
fn scan_roots() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/Library/Java/JavaVirtualMachines"),
        PathBuf::from("/System/Library/Java/JavaVirtualMachines"),
    ]
}

#[cfg(all(unix, not(target_os = "macos")))]
fn scan_roots() -> Vec<PathBuf> {
    vec![
        PathBuf::from("/usr/lib/jvm"),
        PathBuf::from("/usr/java"),
        PathBuf::from("/opt/java"),
    ]
}

#[cfg(not(any(unix, windows)))]
fn scan_roots() -> Vec<PathBuf> {
    Vec::new()
}

/// 我们自己的运行时目录：<appdata>/runtime/jdk-*
pub fn runtime_root() -> PathBuf {
    crate::db::app_dir().join("runtime")
}

/// 在一个目录里找 java 可执行文件所在的 home。
///
/// 解压出来的 JDK 层级不固定：
///   jdk-17/jdk-17.0.2/bin/java          华为云、Zulu 的 zip
///   jdk-17/bin/java                     某些包只有一层
///   jdk-17/zulu-17.jdk/Contents/Home/bin/java   macOS
/// 所以往下找最多 3 层，不能只看第一层 —— 之前只看一层，
/// 装完下次扫描就找不到，界面上又显示「没找到 Java」。
fn find_java_home(base: &Path) -> Option<PathBuf> {
    let mut stack = vec![(base.to_path_buf(), 0usize)];
    while let Some((d, depth)) = stack.pop() {
        if d.join("bin").join(exe_name()).is_file() {
            return Some(d);
        }
        if depth >= 3 {
            continue;
        }
        for sub in list_dirs(&d) {
            // .jdk 包里面是 Contents/Home
            if sub.extension().map(|e| e == "jdk").unwrap_or(false) {
                let home = sub.join("Contents/Home");
                if home.join("bin").join(exe_name()).is_file() {
                    return Some(home);
                }
            }
            stack.push((sub, depth + 1));
        }
    }
    None
}

/// 给测试用的公开入口
pub fn find_java_home_pub(base: &Path) -> Option<PathBuf> {
    find_java_home(base)
}

fn installed_dirs() -> Vec<PathBuf> {
    let mut out = Vec::new();
    for d in list_dirs(&runtime_root()) {
        if let Some(home) = find_java_home(&d) {
            out.push(home);
        }
    }
    out
}

// ═══════════════════════════════════════════════════════════════
// 平台
// ═══════════════════════════════════════════════════════════════

/// 华为云镜像的命名：windows-x64 / linux-x64 / linux-aarch64 / macos-x64 …
fn huawei_platform() -> &'static str {
    let os = if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    };
    match (os, std::env::consts::ARCH) {
        ("windows", "aarch64") => "windows-aarch64",
        ("windows", _) => "windows-x64",
        ("macos", "aarch64") => "macos-aarch64",
        ("macos", _) => "macos-x64",
        ("linux", "aarch64") => "linux-aarch64",
        (_, _) => "linux-x64",
    }
}

/// Zulu API 的 os 参数
fn zulu_os() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "macos"
    } else {
        "linux"
    }
}

fn zulu_arch() -> &'static str {
    match std::env::consts::ARCH {
        "aarch64" => "aarch64",
        "x86" => "x86",
        _ => "x64",
    }
}

// ═══════════════════════════════════════════════════════════════
// 找下载源
// ═══════════════════════════════════════════════════════════════

fn http_get_text(url: &str, timeout_sec: u64) -> Result<String, String> {
    let c = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout_sec))
        .user_agent(UA)
        .build()
        .map_err(|e| format!("HTTP 初始化失败：{e}"))?;

    // 镜像站偶发返回空响应，重试一次能挡掉大部分毛刺
    let mut last = String::new();
    for attempt in 0..2 {
        if attempt > 0 {
            std::thread::sleep(std::time::Duration::from_millis(400));
        }
        match c.get(url).send() {
            Ok(r) if r.status().is_success() => match r.text() {
                Ok(t) if !t.trim().is_empty() => return Ok(t),
                Ok(_) => last = format!("{url} 返回了空内容"),
                Err(e) => last = format!("{url} 读取失败：{e}"),
            },
            Ok(r) => last = format!("{url} 返回 {}", r.status().as_u16()),
            Err(e) => last = format!("{url} 连不上：{e}"),
        }
    }
    Err(last)
}

/// 把 HTML 里所有 href 的路径抠出来（去掉首尾斜杠）
pub fn hrefs(html: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = html;
    while let Some(i) = rest.find("href=\"") {
        let after = &rest[i + 6..];
        if let Some(j) = after.find('"') {
            let h = after[..j].trim_end_matches('/').trim_start_matches('/');
            if !h.is_empty() && !h.contains("://") && !h.starts_with('?') {
                out.push(h.to_string());
            }
            rest = &after[j..];
        } else {
            break;
        }
    }
    out
}

/// 从镜像根目录里挑出某个主版本下最新的那个版本目录。
///
///   href="21.0.2/"  →  "21.0.2"
pub fn pick_version_dir(html: &str, feature: u32) -> Option<String> {
    let mut best: Option<(Vec<u32>, String)> = None;
    for h in hrefs(html) {
        let nums = version_numbers(&h);
        if nums.len() < 2 || nums[0] != feature {
            continue;
        }
        // 只认「纯版本号」的目录名
        if !h.chars().all(|c| c.is_ascii_digit() || c == '.') {
            continue;
        }
        let better = match &best {
            None => true,
            Some((b, _)) => nums > *b,
        };
        if better {
            best = Some((nums, h));
        }
    }
    best.map(|(_, n)| n)
}

/// 从版本目录里挑出当前平台的安装包。
///
///   openjdk-21.0.2_windows-x64_bin.zip
pub fn pick_platform_file(html: &str, platform: &str) -> Option<String> {
    let ext = if cfg!(windows) { ".zip" } else { ".tar.gz" };
    let mut best: Option<(Vec<u32>, String)> = None;
    for h in hrefs(html) {
        if !h.ends_with(ext) || !h.contains(platform) {
            continue;
        }
        // 跳过 debuginfo / 源码包
        if h.contains("debuginfo") || h.contains("source") {
            continue;
        }
        let nums = version_numbers(&h);
        let better = match &best {
            None => true,
            Some((b, _)) => nums > *b,
        };
        if better {
            best = Some((nums, h));
        }
    }
    best.map(|(_, n)| n)
}

/// 文件名里所有连续数字段（用于版本比较）
fn version_numbers(name: &str) -> Vec<u32> {
    let mut out = Vec::new();
    let mut cur = String::new();
    for ch in name.chars() {
        if ch.is_ascii_digit() {
            cur.push(ch);
        } else if !cur.is_empty() {
            out.push(cur.parse().unwrap_or(0));
            cur.clear();
        }
    }
    if !cur.is_empty() {
        out.push(cur.parse().unwrap_or(0));
    }
    out
}

const HUAWEI: &str = "https://mirrors.huaweicloud.com/openjdk/";

/// 华为云镜像的直链。镜像只收 GA 版本，Java 8 没有 → 返回 None。
pub fn huawei_url(feature: u32) -> Option<String> {
    let root = http_get_text(HUAWEI, 15).ok()?;
    let ver = pick_version_dir(&root, feature)?;
    let dir = format!("{HUAWEI}{ver}/");
    let html = http_get_text(&dir, 15).ok()?;
    let file = pick_platform_file(&html, huawei_platform())?;
    Some(format!("{dir}{file}"))
}

/// Zulu 的直链（走官方 metadata API）。
/// 任何主版本都有，包括华为云没有的 8。
pub fn zulu_url(feature: u32) -> Option<String> {
    let url = format!(
        "https://api.azul.com/metadata/v1/zulu/packages/?java_version={f}&os={os}&arch={arch}\
&java_package_type=jdk&archive_type=zip&latest=true&release_status=ga&page_size=100",
        f = feature,
        os = zulu_os(),
        arch = zulu_arch()
    );
    let txt = http_get_text(&url, 20).ok()?;
    let v: Value = serde_json::from_str(&txt).ok()?;
    let arr = v.as_array()?;

    // 优先不带 fx 的（fx 是带 JavaFX 的包，大 60 MB 且我们不需要）
    let mut plain: Option<String> = None;
    let mut fx: Option<String> = None;
    for it in arr {
        let name = it.get("name").and_then(|x| x.as_str()).unwrap_or("");
        let dl = it.get("download_url").and_then(|x| x.as_str()).unwrap_or("");
        if dl.is_empty() || !name.ends_with(".zip") {
            continue;
        }
        if name.contains("-fx-") {
            if fx.is_none() {
                fx = Some(dl.to_string());
            }
        } else if plain.is_none() {
            plain = Some(dl.to_string());
        }
    }
    plain.or(fx)
}

const BMCLAPI: &str = "https://bmclapi2.bangbang93.com";

/// BMCLAPI 的 java 列表。
///
/// 返回的是 [{"title":"Windows x64","file":"jre_x64.exe"}, ...]，
/// 文件从 `https://bmclapi.bangbang93.com/java/<file>` 下。
/// 只挑压缩包，exe/dmg/msi 是安装器，没法静默展开。
/// 这个接口按地区限制，国内能通，国外会 403，所以只当第一候选。
pub fn bmclapi_url() -> Option<String> {
    let txt = http_get_text(&format!("{BMCLAPI}/java/list"), 15).ok()?;
    let arr: Value = serde_json::from_str(&txt).ok()?;
    let list = arr.as_array()?;

    let os_hint = if cfg!(windows) {
        "win"
    } else if cfg!(target_os = "macos") {
        "mac"
    } else {
        "linux"
    };
    let arch_hint = if std::env::consts::ARCH == "aarch64" {
        "arm"
    } else {
        "64"
    };

    for it in list {
        let title = it.get("title").and_then(|x| x.as_str()).unwrap_or("");
        let file = it.get("file").and_then(|x| x.as_str()).unwrap_or("");
        if file.is_empty() {
            continue;
        }
        let low = format!("{} {}", title, file).to_ascii_lowercase();
        if !(low.ends_with(".zip") || low.ends_with(".tar.gz")) {
            continue;
        }
        let os_ok = if os_hint == "win" {
            low.contains("win")
        } else if os_hint == "mac" {
            low.contains("mac") || low.contains("osx")
        } else {
            low.contains("linux")
        };
        if !os_ok || !low.contains(arch_hint) {
            continue;
        }
        return Some(format!("https://bmclapi.bangbang93.com/java/{file}"));
    }
    None
}

/// 华为云不列目录也能拼出来的直链。
///
/// 目录页偶尔抽风返回空，光靠解析会白跑一趟。GA 目录就那么几个，
/// 直接拼出来让下载器去试，404 会立刻失败，代价很小。
fn huawei_direct(feature: u32) -> Vec<String> {
    let ext = if cfg!(windows) { "zip" } else { "tar.gz" };
    let plat = huawei_platform();
    let mut v = Vec::new();
    for ver in [
        format!("{feature}.0.2"),
        format!("{feature}.0.1"),
        format!("{feature}.0.3"),
        format!("{feature}"),
    ] {
        v.push(format!(
            "{HUAWEI}{ver}/openjdk-{ver}_{plat}_bin.{ext}"
        ));
    }
    v
}

/// 所有能拿到安装包的直链，按优先级排好。
pub fn sources(feature: u32) -> Vec<String> {
    let mut v: Vec<String> = Vec::new();
    let mut push = |u: String| {
        if !u.is_empty() && !v.contains(&u) {
            v.push(u);
        }
    };

    // BMCLAPI 只缓存最新的 jre，不区分主版本，所以只在 17 及以上用它
    if feature >= 17 {
        if let Some(u) = bmclapi_url() {
            push(u);
        }
    }
    if let Some(u) = huawei_url(feature) {
        push(u);
    }
    for u in huawei_direct(feature) {
        push(u);
    }
    if let Some(u) = zulu_url(feature) {
        push(u);
    }
    v
}

// ═══════════════════════════════════════════════════════════════
// 解压
// ═══════════════════════════════════════════════════════════════

/// 解压 zip 到 dest，返回顶层目录（JDK 包里通常就一个顶层目录）。
pub fn unzip_to(zip_path: &Path, dest: &Path) -> Result<PathBuf, String> {
    let f = std::fs::File::open(zip_path).map_err(|e| format!("打不开安装包：{e}"))?;
    let mut ar = zip::ZipArchive::new(f).map_err(|e| format!("安装包损坏：{e}"))?;
    std::fs::create_dir_all(dest).map_err(|e| format!("建目录失败：{e}"))?;

    for i in 0..ar.len() {
        let mut entry = ar.by_index(i).map_err(|e| format!("读条目失败：{e}"))?;
        let raw = entry.name().to_string();

        // 防目录穿越（和 eapack 同一套规则）
        let mut parts: Vec<&str> = Vec::new();
        for seg in raw.split(['/', '\\']) {
            if seg.is_empty() || seg == "." {
                continue;
            }
            if seg == ".." || seg.contains(':') {
                return Err(format!("安装包里有非法路径：{raw}"));
            }
            parts.push(seg);
        }
        if parts.is_empty() {
            continue;
        }
        let mut out = dest.to_path_buf();
        for p in &parts {
            out.push(p);
        }

        if entry.is_dir() {
            let _ = std::fs::create_dir_all(&out);
            continue;
        }
        if let Some(parent) = out.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        let mut f =
            std::fs::File::create(&out).map_err(|e| format!("写 {} 失败：{e}", out.display()))?;
        std::io::copy(&mut entry, &mut f)
            .map_err(|e| format!("解压 {} 失败：{e}", out.display()))?;

        // unix 下要保住可执行位，否则解出来的 java 跑不了
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if let Some(mode) = entry.unix_mode() {
                let _ = std::fs::set_permissions(&out, std::fs::Permissions::from_mode(mode));
            }
        }
    }

    // 顶层目录：只有一个就返回它，否则返回 dest 本身
    let tops = list_dirs(dest);
    let n = std::fs::read_dir(dest).map(|r| r.count()).unwrap_or(0);
    if tops.len() == 1 && n == 1 {
        return Ok(tops[0].clone());
    }
    Ok(dest.to_path_buf())
}

/// 解压安装包（zip / tar.gz），返回运行时根目录。
pub fn unpack(pkg: &Path, dest: &Path) -> Result<PathBuf, String> {
    let name = pkg
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();

    if name.ends_with(".zip") {
        return unzip_to(pkg, dest);
    }

    if name.ends_with(".tar.gz") || name.ends_with(".tgz") {
        #[cfg(unix)]
        {
            std::fs::create_dir_all(dest).map_err(|e| format!("建目录失败：{e}"))?;
            let st = hidden_cmd("tar")
                .arg("-xzf")
                .arg(pkg)
                .arg("-C")
                .arg(dest)
                .status()
                .map_err(|e| format!("调用 tar 失败：{e}"))?;
            if !st.success() {
                return Err("tar 解压失败".to_string());
            }
            let tops = list_dirs(dest);
            let n = std::fs::read_dir(dest).map(|r| r.count()).unwrap_or(0);
            if tops.len() == 1 && n == 1 {
                return Ok(tops[0].clone());
            }
            return Ok(dest.to_path_buf());
        }
        #[cfg(not(unix))]
        {
            return Err("当前平台不支持 tar.gz".to_string());
        }
    }

    Err(format!("不认识的安装包格式：{name}"))
}

// ═══════════════════════════════════════════════════════════════
// 自动安装
// ═══════════════════════════════════════════════════════════════

/// 自动装一份 JDK。已经装过同主版本的直接复用。
pub fn install(feature: u32) -> Result<Java, String> {
    install_into(feature, &runtime_root())
}

/// 同 [`install`]，但装到指定目录（测试用）。
pub fn install_into(feature: u32, root: &Path) -> Result<Java, String> {
    let root = root.to_path_buf();

    // 已经装过就不重复下，一份 200 MB 上下
    for d in list_dirs(&root) {
        let Some(home) = find_java_home(&d) else { continue };
        if let Some(j) = make(home.join("bin").join(exe_name()), "installed") {
            if j.major == feature {
                return Ok(j);
            }
        }
    }

    std::fs::create_dir_all(&root).map_err(|e| format!("建目录失败：{e}"))?;

    let urls = sources(feature);
    if urls.is_empty() {
        return Err(format!(
            "找不到 Java {feature} 的下载源，网络不通的话可以手动装一份再重试"
        ));
    }

    let mut last = String::new();
    for url in urls {
        let name = url.rsplit('/').next().unwrap_or("jdk.zip").to_string();
        let task = crate::ridex::DownloadTask {
            name: name.clone(),
            urls: vec![url.clone()],
            target_dir: root.clone(),
            size: 0,
            hash: String::new(),
        };
        let batch = crate::ridex::run_batch(
            vec![task],
            &crate::ridex::Options::default(),
            std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            None,
            None,
        );
        let got = match batch.results.into_iter().next() {
            Some(r) if r.ok => r,
            Some(r) => {
                last = format!("{url} 下载失败：{}", r.error);
                continue;
            }
            None => {
                last = format!("{url} 没有返回结果");
                continue;
            }
        };

        let pkg = PathBuf::from(&got.path);
        let dest = root.join(format!("jdk-{feature}"));
        let _ = std::fs::remove_dir_all(&dest);
        match unpack(&pkg, &dest) {
            Ok(home) => {
                let _ = std::fs::remove_file(&pkg);
                if let Some(j) = make(home.join("bin").join(exe_name()), "installed") {
                    return Ok(j);
                }
                last = format!("{url} 解压后跑不起来 java");
            }
            Err(e) => {
                let _ = std::fs::remove_file(&pkg);
                last = format!("{url} 解压失败：{e}");
            }
        }
    }

    Err(format!("Java {feature} 装不上：{last}"))
}

// ═══════════════════════════════════════════════════════════════
// 测试
// ═══════════════════════════════════════════════════════════════

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn version_parsing() {
        assert_eq!(
            parse_version_line("openjdk version \"21.0.12\" 2025-07-15")
                .unwrap()
                .0,
            21
        );
        assert_eq!(parse_version_line("java version \"1.8.0_411\"").unwrap().0, 8);
        assert_eq!(
            parse_version_line("openjdk version \"17\" 2021-09-14").unwrap().0,
            17
        );
        assert!(parse_version_line("Picked up JAVA_TOOL_OPTIONS").is_none());
    }

    #[test]
    fn mc_requires() {
        assert_eq!(required_major("1.20.1"), 17);
        assert_eq!(required_major("1.20.4"), 17);
        assert_eq!(required_major("1.20.5"), 21);
        assert_eq!(required_major("1.21.1"), 21);
        assert_eq!(required_major("1.16.5"), 8);
        assert_eq!(required_major("1.7.10"), 8);
        assert_eq!(required_major("1.17.1"), 17);
    }

    #[test]
    fn href_extraction() {
        let html = r#"<a href="21.0.2/">x</a><a href="17/">y</a><a href="https://e.com/z">z</a>"#;
        assert_eq!(hrefs(html), vec!["21.0.2", "17"]);
    }

    #[test]
    fn picks_newest_version_dir() {
        let html = r#"
        <a href="21/">x</a>
        <a href="21.0.1/">x</a>
        <a href="21.0.2/">x</a>
        <a href="17.0.2/">x</a>
        "#;
        assert_eq!(pick_version_dir(html, 21).unwrap(), "21.0.2");
        assert_eq!(pick_version_dir(html, 17).unwrap(), "17.0.2");
        assert!(pick_version_dir(html, 8).is_none());
    }

    #[test]
    fn picks_platform_file() {
        let html = r#"
        <a href="openjdk-21.0.2_linux-aarch64_bin.tar.gz">x</a>
        <a href="openjdk-21.0.2_linux-x64_bin.tar.gz">x</a>
        <a href="openjdk-21.0.2_macos-aarch64_bin.tar.gz">x</a>
        <a href="openjdk-21.0.2_windows-x64_bin.zip">x</a>
        "#;
        let want = if cfg!(windows) {
            "openjdk-21.0.2_windows-x64_bin.zip"
        } else {
            "openjdk-21.0.2_linux-x64_bin.tar.gz"
        };
        // mac 上这条会取到 macos 的包，所以只在 win/linux 断言
        if !cfg!(target_os = "macos") {
            assert_eq!(pick_platform_file(html, huawei_platform()).unwrap(), want);
        }
    }

    #[test]
    fn rejects_wrong_ext() {
        let html = r#"<a href="openjdk-21.0.2_windows-x64_bin.zip.sha256">x</a>"#;
        assert!(pick_platform_file(html, "windows-x64").is_none());
    }
}
