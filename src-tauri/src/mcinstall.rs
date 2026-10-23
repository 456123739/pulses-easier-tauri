// mcinstall.rs — 把整合包变成 PCL 能直接扫到的实例
//
// 目标（用户原话）：「拿进去个 CF/MR 整合包能加速下载并且 PCL 能识别」
//
// 产出布局（就是 PCL 版本隔离的标准布局，PCL 一扫描就能看到）：
//
//   <游戏目录>/
//     versions/<实例名>/<实例名>.json     ← 版本 JSON（id 与文件夹同名）
//     versions/<实例名>/<实例名>.jar      ← client.jar（加载器版本里可能是继承）
//     versions/<实例名>/mods/ config/ …   ← 整合包内容
//     versions/<mc>/<mc>.json + <mc>.jar  ← 原版
//     libraries/…                         ← 库
//     assets/indexes/<id>.json
//     assets/objects/<xx>/<hash>          ← 资源
//
// 加载器三种情况：
//   · Fabric / Quilt —— 官方给的是完整 profile JSON，落盘即可，**不需要 Java**
//   · Forge / NeoForge —— 只有 installer.jar，必须本机 Java 跑一遍
//   · 原版 —— 只下 client.jar + libraries + assets，不需要 Java
//
// 所有下载都走 RideX（多源回退 + 分片 + 哈希校验）。
// 镜像策略：官方地址为准，BMCLAPI 作为**首选**地址排在前面，
// 官方作为兜底 —— 这样国内快、国外也不会挂。

use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use crate::ridex::{self, DownloadTask};

// ═══════════════════════════════════════════════════════════════
// 常量与类型
// ═══════════════════════════════════════════════════════════════

const BMCL: &str = "https://bmclapi2.bangbang93.com";
const MOJANG_MANIFEST: &str = "https://launchermeta.mojang.com/mc/game/version_manifest_v2.json";
const FABRIC_META: &str = "https://meta.fabricmc.net/v2";
const QUILT_META: &str = "https://meta.quiltmc.org/v3";
const FORGE_MAVEN: &str = "https://maven.minecraftforge.net";
const NEOFORGE_MAVEN: &str = "https://maven.neoforged.net/releases";

/// 进度回调：(阶段, 已完成, 总数, 已下载字节, 总字节)
pub type Progress = std::sync::Arc<dyn Fn(&str, usize, usize, u64, u64) + Send + Sync>;

#[derive(Clone, Debug, Default)]
pub struct Target {
    /// 游戏目录（相当于 .minecraft）
    pub root: PathBuf,
    /// 原版版本号，如 1.20.1
    pub mc: String,
    /// vanilla / fabric / quilt / forge / neoforge
    pub loader: String,
    /// 加载器版本；空表示用最新的
    pub loader_ver: String,
    /// 实例名（versions/ 下的文件夹名）
    pub instance: String,
    /// 用国内镜像（BMCLAPI）优先
    pub mirror: bool,
    /// Java 可执行文件（装 Forge 用）
    pub java: String,
}

impl Target {
    pub fn from_json(v: &Value) -> Self {
        let s = |k: &str| v.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
        let mut t = Self {
            root: PathBuf::from(s("root")),
            mc: s("mc"),
            loader: s("loader"),
            loader_ver: s("loaderVer"),
            instance: s("instance"),
            mirror: v.get("mirror").and_then(|x| x.as_bool()).unwrap_or(true),
            java: s("java"),
        };
        if t.loader.is_empty() {
            t.loader = "vanilla".into();
        }
        t.loader = t.loader.to_ascii_lowercase();
        if t.instance.is_empty() {
            t.instance = format!("{}-{}", t.mc, t.loader);
        }
        t
    }
}

// ═══════════════════════════════════════════════════════════════
// HTTP
// ═══════════════════════════════════════════════════════════════

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 \
(KHTML, like Gecko) Chrome/131.0.0.0 Safari/537.36";

fn client(timeout: u64) -> Result<reqwest::blocking::Client, String> {
    reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(timeout))
        .user_agent(UA)
        .build()
        .map_err(|e| format!("HTTP 初始化失败：{e}"))
}

/// 按顺序试每个地址，第一个成功的返回。
fn get_json(urls: &[String]) -> Result<Value, String> {
    let c = client(30)?;
    let mut last = String::new();
    for u in urls {
        match c.get(u).send() {
            Ok(r) if r.status().is_success() => match r.json::<Value>() {
                Ok(v) => return Ok(v),
                Err(e) => last = format!("{u} 解析失败：{e}"),
            },
            Ok(r) => last = format!("{u} 返回 {}", r.status().as_u16()),
            Err(e) => last = format!("{u} 连不上：{e}"),
        }
    }
    Err(if last.is_empty() {
        "没有可用的下载源".to_string()
    } else {
        last
    })
}

fn get_text(urls: &[String]) -> Result<String, String> {
    let c = client(30)?;
    let mut last = String::new();
    for u in urls {
        match c.get(u).send() {
            Ok(r) if r.status().is_success() => match r.text() {
                Ok(t) => return Ok(t),
                Err(e) => last = format!("{u} 读取失败：{e}"),
            },
            Ok(r) => last = format!("{u} 返回 {}", r.status().as_u16()),
            Err(e) => last = format!("{u} 连不上：{e}"),
        }
    }
    Err(if last.is_empty() {
        "没有可用的下载源".to_string()
    } else {
        last
    })
}

/// 官方地址 → BMCLAPI 对应地址（没有对应就 None）
pub fn mirror_of(url: &str) -> Option<String> {
    let rest = url
        .strip_prefix("https://libraries.minecraft.net/")
        .or_else(|| url.strip_prefix("http://libraries.minecraft.net/"));
    if let Some(p) = rest {
        return Some(format!("{BMCL}/maven/{p}"));
    }
    let rest = url
        .strip_prefix("https://resources.download.minecraft.net/")
        .or_else(|| url.strip_prefix("http://resources.download.minecraft.net/"));
    if let Some(p) = rest {
        return Some(format!("{BMCL}/assets/{p}"));
    }
    None
}

/// 组装候选地址：开了镜像就镜像优先、官方兜底。
fn urls_for(official: &str, mirror: bool) -> Vec<String> {
    let mut v = Vec::new();
    if mirror {
        if let Some(m) = mirror_of(official) {
            v.push(m);
        }
    }
    if !v.iter().any(|x| x == official) {
        v.push(official.to_string());
    }
    v
}

// ═══════════════════════════════════════════════════════════════
// 平台规则
// ═══════════════════════════════════════════════════════════════

pub fn os_name() -> &'static str {
    if cfg!(windows) {
        "windows"
    } else if cfg!(target_os = "macos") {
        "osx"
    } else {
        "linux"
    }
}

pub fn os_arch() -> &'static str {
    match std::env::consts::ARCH {
        "x86" => "x86",
        "aarch64" => "arm64",
        _ => "x86_64",
    }
}

/// Mojang 的 rules 求值。
///
/// 规则按顺序匹配，命中的最后一条决定 allow/disallow。
/// 没有 rules 字段 = 默认允许。
/// 带 `features` 的规则一律当作不匹配 —— 我们不开 demo / 自定义分辨率。
pub fn is_allowed(rules: &Value) -> bool {
    let arr = match rules.as_array() {
        Some(a) => a,
        None => return true,
    };
    if arr.is_empty() {
        return true;
    }
    let mut allowed = false;
    for r in arr {
        if !rule_matches(r) {
            continue;
        }
        allowed = r.get("action").and_then(|x| x.as_str()) == Some("allow");
    }
    allowed
}

fn rule_matches(rule: &Value) -> bool {
    // 带 features 的规则：我们一个都不满足
    if rule.get("features").map(|f| !f.is_null()).unwrap_or(false) {
        return false;
    }
    let os = match rule.get("os") {
        Some(o) if !o.is_null() => o,
        _ => return true, // 没写 os 就是全平台匹配
    };
    if let Some(n) = os.get("name").and_then(|x| x.as_str()) {
        if n != os_name() {
            return false;
        }
    }
    if let Some(a) = os.get("arch").and_then(|x| x.as_str()) {
        // Mojang 用 "x86" 表示 32 位；其余按原样比
        if a != os_arch() {
            return false;
        }
    }
    true
}

// ═══════════════════════════════════════════════════════════════
// 版本元数据
// ═══════════════════════════════════════════════════════════════

fn manifest_urls(mirror: bool) -> Vec<String> {
    if mirror {
        vec![
            format!("{BMCL}/mc/game/version_manifest_v2.json"),
            MOJANG_MANIFEST.to_string(),
        ]
    } else {
        vec![MOJANG_MANIFEST.to_string()]
    }
}

/// 版本清单里所有 release（给前端选版本用）
pub fn list_versions(mirror: bool) -> Result<Value, String> {
    let m = get_json(&manifest_urls(mirror))?;
    let mut out = Vec::new();
    if let Some(arr) = m.get("versions").and_then(|x| x.as_array()) {
        for v in arr {
            if v.get("type").and_then(|x| x.as_str()) != Some("release") {
                continue;
            }
            out.push(json!({
                "id": v.get("id").cloned().unwrap_or(Value::Null),
                "time": v.get("releaseTime").cloned().unwrap_or(Value::Null),
            }));
        }
    }
    Ok(json!({ "ok": true, "versions": out }))
}

/// 拿某个版本的版本 JSON（含 libraries / assets / downloads）
pub fn version_json(mc: &str, mirror: bool) -> Result<Value, String> {
    // 镜像站有 /version/<v>/json 的短地址，先试它
    if mirror {
        let short = format!("{BMCL}/version/{mc}/json");
        if let Ok(v) = get_json(&[short]) {
            if v.get("libraries").is_some() {
                return Ok(v);
            }
        }
    }
    let m = get_json(&manifest_urls(mirror))?;
    let arr = m
        .get("versions")
        .and_then(|x| x.as_array())
        .ok_or_else(|| "版本清单格式不对".to_string())?;
    let hit = arr
        .iter()
        .find(|v| v.get("id").and_then(|x| x.as_str()) == Some(mc))
        .ok_or_else(|| format!("找不到 Minecraft {mc}"))?;
    let url = hit
        .get("url")
        .and_then(|x| x.as_str())
        .ok_or_else(|| "版本条目没有 url".to_string())?;
    let mut urls = urls_for(url, mirror);
    // 官方 piston-meta 没有 BMCLAPI 等价物，保留原样即可
    if urls.len() > 1 && !url.contains("libraries.minecraft.net") {
        urls.truncate(1);
        urls.push(url.to_string());
    }
    get_json(&urls)
}

// ═══════════════════════════════════════════════════════════════
// 任务构建
// ═══════════════════════════════════════════════════════════════

/// 一个待下载项：落到 `dir/name`，候选地址 urls。
fn task(dir: &Path, name: &str, urls: Vec<String>, size: u64, hash: &str) -> DownloadTask {
    DownloadTask {
        name: name.to_string(),
        urls,
        target_dir: dir.to_path_buf(),
        size,
        hash: hash.to_string(),
    }
}

/// 已经下好的（大小对得上）就不再下。资源有 3000 多个，重复下会很慢。
fn prune(tasks: Vec<DownloadTask>) -> Vec<DownloadTask> {
    tasks
        .into_iter()
        .filter(|t| {
            let p = t.target_dir.join(&t.name);
            match std::fs::metadata(&p) {
                Ok(m) if m.is_file() => {
                    if t.size > 0 {
                        m.len() != t.size
                    } else {
                        m.len() == 0
                    }
                }
                _ => true,
            }
        })
        .collect()
}

/// 原版：client.jar + libraries + assets + 版本 JSON
pub fn vanilla_tasks(
    root: &Path,
    ver: &Value,
    mirror: bool,
) -> (Vec<DownloadTask>, Vec<(PathBuf, Value)>) {
    let mut tasks = Vec::new();
    let mut write = Vec::new();

    let mc = ver
        .get("id")
        .and_then(|x| x.as_str())
        .unwrap_or("unknown")
        .to_string();
    let vdir = root.join("versions").join(&mc);

    // 版本 JSON 自己也要落盘
    write.push((vdir.join(format!("{mc}.json")), ver.clone()));

    // client.jar
    if let Some(c) = ver.get("downloads").and_then(|d| d.get("client")) {
        let url = c.get("url").and_then(|x| x.as_str()).unwrap_or("");
        if !url.is_empty() {
            let mut urls = urls_for(url, mirror);
            if mirror {
                // 镜像站有直链 /version/<mc>/client
                urls.insert(0, format!("{BMCL}/version/{mc}/client"));
            }
            tasks.push(task(
                &vdir,
                &format!("{mc}.jar"),
                urls,
                c.get("size").and_then(|x| x.as_u64()).unwrap_or(0),
                c.get("sha1").and_then(|x| x.as_str()).unwrap_or(""),
            ));
        }
    }

    // libraries
    if let Some(arr) = ver.get("libraries").and_then(|x| x.as_array()) {
        for lib in arr {
            if !is_allowed(&lib.get("rules").cloned().unwrap_or(Value::Null)) {
                continue;
            }
            let art = match lib.get("downloads").and_then(|d| d.get("artifact")) {
                Some(a) if !a.is_null() => a,
                _ => continue,
            };
            let path = match art.get("path").and_then(|x| x.as_str()) {
                Some(p) if !p.is_empty() => p,
                _ => continue,
            };
            let url = art.get("url").and_then(|x| x.as_str()).unwrap_or("");
            if url.is_empty() {
                continue;
            }
            let full = root.join("libraries").join(path);
            let dir = full.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| root.to_path_buf());
            let name = full
                .file_name()
                .map(|s| s.to_string_lossy().to_string())
                .unwrap_or_default();
            tasks.push(task(
                &dir,
                &name,
                urls_for(url, mirror),
                art.get("size").and_then(|x| x.as_u64()).unwrap_or(0),
                art.get("sha1").and_then(|x| x.as_str()).unwrap_or(""),
            ));
        }
    }

    (tasks, write)
}

/// 资源索引里的 3598 个对象
pub fn asset_tasks(root: &Path, index: &Value, mirror: bool) -> (Vec<DownloadTask>, Vec<(PathBuf, Value)>) {
    let mut tasks = Vec::new();
    let mut write = Vec::new();
    let id = index
        .get("id")
        .and_then(|x| x.as_str())
        .unwrap_or("legacy")
        .to_string();

    write.push((root.join("assets/indexes").join(format!("{id}.json")), index.clone()));

    let objs = match index.get("objects").and_then(|x| x.as_object()) {
        Some(o) => o,
        None => return (tasks, write),
    };

    let mut seen: Vec<&str> = Vec::new();
    for (_name, o) in objs {
        let hash = match o.get("hash").and_then(|x| x.as_str()) {
            Some(h) if h.len() >= 2 => h,
            _ => continue,
        };
        if seen.contains(&hash) {
            continue;
        }
        seen.push(hash);
        let sub = &hash[..2];
        let dir = root.join("assets/objects").join(sub);
        let official = format!("https://resources.download.minecraft.net/{sub}/{hash}");
        tasks.push(task(
            &dir,
            hash,
            urls_for(&official, mirror),
            o.get("size").and_then(|x| x.as_u64()).unwrap_or(0),
            hash,
        ));
    }

    (tasks, write)
}

// ═══════════════════════════════════════════════════════════════
// 加载器：Fabric / Quilt（免 Java）
// ═══════════════════════════════════════════════════════════════

/// 挑一个加载器版本（没指定就用最新的稳定版）
pub fn fabric_loader_version(mc: &str, loader: &str, want: &str) -> Result<String, String> {
    if !want.is_empty() {
        return Ok(want.to_string());
    }
    let base = if loader == "quilt" { QUILT_META } else { FABRIC_META };
    let txt = get_text(&[format!("{base}/versions/loader/{mc}")])?;
    let v: Value = serde_json::from_str(&txt).map_err(|e| format!("加载器版本列表解析失败：{e}"))?;
    let arr = v.as_array().ok_or_else(|| "加载器版本列表格式不对".to_string())?;
    // 优先 stable，其次任意
    let stable = arr.iter().find(|x| {
        x.get("loader")
            .and_then(|l| l.get("stable"))
            .and_then(|s| s.as_bool())
            .unwrap_or(false)
    });
    let pick = stable.or_else(|| arr.first()).ok_or_else(|| format!("{mc} 没有可用的加载器"))?;
    pick.get("loader")
        .and_then(|l| l.get("version"))
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
        .ok_or_else(|| "加载器版本字段缺失".to_string())
}

/// Fabric / Quilt 的 profile JSON（就是一份能直接用的版本 JSON）
pub fn fabric_profile(mc: &str, loader: &str, loader_ver: &str) -> Result<Value, String> {
    let base = if loader == "quilt" { QUILT_META } else { FABRIC_META };
    let txt = get_text(&[format!(
        "{base}/versions/loader/{mc}/{loader_ver}/profile/json"
    )])?;
    let v: Value =
        serde_json::from_str(&txt).map_err(|e| format!("加载器 profile 解析失败：{e}"))?;
    if v.get("libraries").is_none() {
        return Err("加载器 profile 里没有 libraries".to_string());
    }
    Ok(v)
}

/// profile JSON 里的库
pub fn loader_tasks(root: &Path, profile: &Value, mirror: bool) -> Vec<DownloadTask> {
    let mut tasks = Vec::new();
    let arr = match profile.get("libraries").and_then(|x| x.as_array()) {
        Some(a) => a,
        None => return tasks,
    };
    for lib in arr {
        if !is_allowed(&lib.get("rules").cloned().unwrap_or(Value::Null)) {
            continue;
        }
        // Fabric 的库只有 name + url（maven 根），没有 downloads 段
        let name = lib.get("name").and_then(|x| x.as_str()).unwrap_or("");
        let base = lib
            .get("url")
            .and_then(|x| x.as_str())
            .unwrap_or("https://maven.fabricmc.net/");
        let path = match maven_path(name) {
            Some(p) => p,
            None => continue,
        };
        let full = root.join("libraries").join(&path);
        let dir = full.parent().map(|p| p.to_path_buf()).unwrap_or_else(|| root.to_path_buf());
        let file = full
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        let official = format!("{}/{}", base.trim_end_matches('/'), path);
        let mut urls = Vec::new();
        if mirror {
            // BMCLAPI 也镜像了 fabric 的 maven（quilt 没有，会 404 后自动落到官方）
            urls.push(format!("{BMCL}/maven/{path}"));
        }
        urls.push(official);
        tasks.push(task(&dir, &file, urls, 0, ""));
    }
    tasks
}

/// `group:artifact:version[:classifier]` → maven 相对路径
pub fn maven_path(coords: &str) -> Option<String> {
    let parts: Vec<&str> = coords.split(':').collect();
    if parts.len() < 3 {
        return None;
    }
    let (g, a, v) = (parts[0], parts[1], parts[2]);
    if g.is_empty() || a.is_empty() || v.is_empty() {
        return None;
    }
    let file = if parts.len() >= 4 && !parts[3].is_empty() {
        format!("{a}-{v}-{}.jar", parts[3])
    } else {
        format!("{a}-{v}.jar")
    };
    Some(format!(
        "{}/{}/{}/{file}",
        g.replace('.', "/"),
        a,
        v
    ))
}

// ═══════════════════════════════════════════════════════════════
// 加载器：Forge / NeoForge（要 Java）
// ═══════════════════════════════════════════════════════════════

/// 整合包没写加载器版本时，去查推荐版本。
///
/// 之前这里直接拿空版本拼文件名，拼出 `forge-1.20.1-installer.jar`，
/// 两个源都 404，玩家看到的就是「安装失败：HTTP 404」。
pub fn resolve_forge_version(mc: &str) -> Result<String, String> {
    let txt = get_text(&[
        "https://files.minecraftforge.net/net/minecraftforge/forge/promotions_slim.json".to_string(),
    ])?;
    let v: Value =
        serde_json::from_str(&txt).map_err(|e| format!("Forge 版本列表解析失败：{e}"))?;
    let promos = v
        .get("promos")
        .and_then(|x| x.as_object())
        .ok_or_else(|| "Forge 版本列表格式不对".to_string())?;
    for key in [format!("{mc}-recommended"), format!("{mc}-latest")] {
        if let Some(s) = promos.get(&key).and_then(|x| x.as_str()) {
            return Ok(s.to_string());
        }
    }
    Err(format!("没有找到 Minecraft {mc} 对应的 Forge 版本"))
}

/// NeoForge 同理。NeoForge 的版本号和 MC 版本不是一一对应，得问它自己的列表。
pub fn resolve_neoforge_version(mc: &str) -> Result<String, String> {
    let txt = get_text(&[
        format!("{BMCL}/neoforge/list/{mc}"),
        format!("https://maven.neoforged.net/api/maven/versions/releases/net/neoforged/neoforge"),
    ])?;
    let arr: Value =
        serde_json::from_str(&txt).map_err(|e| format!("NeoForge 版本列表解析失败：{e}"))?;
    let list = arr.as_array().ok_or_else(|| "NeoForge 版本列表格式不对".to_string())?;
    // 列表按时间排，最后一个是新的
    for it in list.iter().rev() {
        if let Some(raw) = it.get("rawVersion").and_then(|x| x.as_str()) {
            let ver = raw.trim_start_matches("neoforge-");
            if !ver.is_empty() {
                return Ok(ver.to_string());
            }
        }
    }
    Err(format!("没有找到 Minecraft {mc} 对应的 NeoForge 版本"))
}

/// Forge 的 installer 直链。版本号形如 `1.20.1-47.2.0`。
pub fn forge_installer_url(mc: &str, ver: &str, mirror: bool) -> (String, String) {
    let full = if ver.starts_with(&format!("{mc}-")) {
        ver.to_string()
    } else {
        format!("{mc}-{ver}")
    };
    let file = format!("forge-{full}-installer.jar");
    let official = format!(
        "{FORGE_MAVEN}/net/minecraftforge/forge/{full}/{file}"
    );
    let m = format!("{BMCL}/maven/net/minecraftforge/forge/{full}/{file}");
    if mirror {
        (m, official)
    } else {
        (official.clone(), official)
    }
}

/// NeoForge 的 installer 直链。版本号形如 `21.1.1`（1.21.1）或 `20.4.237`（1.20.4）。
pub fn neoforge_installer_url(ver: &str, mirror: bool) -> (String, String) {
    let file = format!("neoforge-{ver}-installer.jar");
    let path = format!("net/neoforged/neoforge/{ver}/{file}");
    let official = format!("{NEOFORGE_MAVEN}/{path}");
    let m = format!("{BMCL}/maven/{path}");
    if mirror {
        (m, official)
    } else {
        (official.clone(), official)
    }
}

/// 跑 `java -jar installer.jar --installClient <root>`
pub fn run_forge_installer(java: &str, jar: &Path, root: &Path) -> Result<String, String> {
    let mut cmd = std::process::Command::new(java);
    cmd.arg("-jar")
        .arg(jar)
        .arg("--installClient")
        .arg(root)
        .current_dir(root);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd
        .output()
        .map_err(|e| format!("跑不起来 Java：{e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout).to_string();
    let stderr = String::from_utf8_lossy(&out.stderr).to_string();
    if !out.status.success() {
        let tail: String = stdout
            .lines()
            .chain(stderr.lines())
            .rev()
            .take(8)
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect::<Vec<_>>()
            .join("\n");
        return Err(format!("安装器失败（退出码 {:?}）：\n{tail}", out.status.code()));
    }
    Ok(stdout)
}

// ═══════════════════════════════════════════════════════════════
// 整合包元信息识别
// ═══════════════════════════════════════════════════════════════

/// 从整合包目录里认出 MC 版本 + 加载器。
///
/// 支持：
///   · CurseForge  manifest.json
///   · Prism/MultiMC mmc-pack.json
///   · Modrinth    modrinth.index.json
pub fn pack_meta(dir: &Path) -> Value {
    // CurseForge
    if let Ok(t) = std::fs::read_to_string(dir.join("manifest.json")) {
        if let Ok(v) = serde_json::from_str::<Value>(&t) {
            let mc = v
                .get("minecraft")
                .and_then(|m| m.get("version"))
                .and_then(|x| x.as_str())
                .unwrap_or("")
                .to_string();
            let loaders = v
                .get("minecraft")
                .and_then(|m| m.get("modLoaders"))
                .and_then(|x| x.as_array())
                .cloned()
                .unwrap_or_default();
            // primary 优先
            let primary = loaders
                .iter()
                .find(|l| l.get("primary").and_then(|p| p.as_bool()).unwrap_or(false))
                .or_else(|| loaders.first());
            let (loader, ver) = primary
                .and_then(|l| l.get("id").and_then(|x| x.as_str()))
                .map(split_loader_id)
                .unwrap_or(("vanilla".to_string(), String::new()));
            if !mc.is_empty() {
                return json!({
                    "ok": true, "kind": "curseforge",
                    "name": v.get("name").and_then(|x| x.as_str()).unwrap_or(""),
                    "version": v.get("version").and_then(|x| x.as_str()).unwrap_or(""),
                    "mc": mc, "loader": loader, "loaderVer": ver,
                });
            }
        }
    }

    // Prism / MultiMC
    if let Ok(t) = std::fs::read_to_string(dir.join("mmc-pack.json")) {
        if let Ok(v) = serde_json::from_str::<Value>(&t) {
            let comps = v
                .get("components")
                .and_then(|x| x.as_array())
                .cloned()
                .unwrap_or_default();
            let find = |uid: &str| -> String {
                comps
                    .iter()
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
            if !mc.is_empty() {
                return json!({
                    "ok": true, "kind": "prism",
                    "name": dir.file_name().map(|s| s.to_string_lossy().to_string()).unwrap_or_default(),
                    "version": "", "mc": mc, "loader": loader, "loaderVer": ver,
                });
            }
        }
    }

    // Modrinth
    if let Ok(t) = std::fs::read_to_string(dir.join("modrinth.index.json")) {
        if let Ok(v) = serde_json::from_str::<Value>(&t) {
            let deps = v.get("dependencies").cloned().unwrap_or(Value::Null);
            let g = |k: &str| deps.get(k).and_then(|x| x.as_str()).unwrap_or("").to_string();
            let mc = g("minecraft");
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
            if !mc.is_empty() {
                return json!({
                    "ok": true, "kind": "modrinth",
                    "name": v.get("name").and_then(|x| x.as_str()).unwrap_or(""),
                    "version": v.get("versionId").and_then(|x| x.as_str()).unwrap_or(""),
                    "mc": mc, "loader": loader, "loaderVer": ver,
                });
            }
        }
    }

    json!({ "ok": false })
}

/// "forge-47.2.0" / "neoforge-21.1.1" / "fabric-0.15.11" → (加载器, 版本)
pub fn split_loader_id(id: &str) -> (String, String) {
    let low = id.to_ascii_lowercase();
    for l in ["neoforge", "forge", "fabric", "quilt"] {
        if let Some(rest) = low.strip_prefix(l) {
            let rest = rest.trim_start_matches('-');
            // "forge-1.20.1-47.2.0" 里的 1.20.1 是 MC 版本，要剥掉
            let ver = if l == "forge" || l == "neoforge" {
                strip_mc_prefix(rest)
            } else {
                rest.to_string()
            };
            return (l.to_string(), ver);
        }
    }
    ("vanilla".to_string(), String::new())
}

/// "1.20.1-47.2.0" → "47.2.0"；"21.1.1" 原样返回
fn strip_mc_prefix(s: &str) -> String {
    let mut it = s.splitn(2, '-');
    let first = it.next().unwrap_or("");
    match it.next() {
        Some(rest) if first.starts_with("1.") => rest.to_string(),
        _ => s.to_string(),
    }
}

// ═══════════════════════════════════════════════════════════════
// 落地
// ═══════════════════════════════════════════════════════════════

fn write_json(path: &Path, v: &Value) -> Result<(), String> {
    if let Some(p) = path.parent() {
        let _ = std::fs::create_dir_all(p);
    }
    let t = serde_json::to_string_pretty(v).map_err(|e| format!("序列化失败：{e}"))?;
    std::fs::write(path, t).map_err(|e| format!("写 {} 失败：{e}", path.display()))
}

/// 跑一批下载，返回失败项
fn run(tasks: Vec<DownloadTask>, stage: &str, prog: &Option<Progress>, cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>) -> (usize, Vec<String>) {
    run_with(tasks, stage, prog, cancel, 16)
}

/// 同上，可指定并发槽位。
///
/// 资源文件有三千多个、每个几 KB，靠并发压延迟才快得起来；
/// 游戏本体/库都是大文件，并发高了只会互相抢带宽。
fn run_with(
    tasks: Vec<DownloadTask>,
    stage: &str,
    prog: &Option<Progress>,
    cancel: &std::sync::Arc<std::sync::atomic::AtomicBool>,
    slots: usize,
) -> (usize, Vec<String>) {
    let tasks = prune(tasks);
    if tasks.is_empty() {
        if let Some(p) = prog {
            p(stage, 1, 1, 0, 0);
        }
        return (0, Vec::new());
    }
    let total = tasks.len();
    let stage_owned = stage.to_string();
    let p2 = prog.clone();
    let cb: ridex::ProgressCb = std::sync::Arc::new(move |done, tot, _name, bytes, tb| {
        if let Some(p) = &p2 {
            p(&stage_owned, done, tot, bytes, tb);
        }
    });
    let mut opts = ridex::Options::default();
    opts.multi_slots = slots.max(1);
    let batch = crate::engine::run(tasks, &opts, cancel.clone(), Some(cb), None);
    let fails = batch
        .results
        .iter()
        .filter(|r| !r.ok)
        .map(|r| format!("{}：{}", r.name, r.error))
        .collect();
    let _ = total;
    (batch.ok, fails)
}

/// 把版本 JSON 里的 `id` 改成实例名（PCL 用 id 匹配文件夹）
pub fn retag(mut v: Value, id: &str) -> Value {
    if let Some(o) = v.as_object_mut() {
        o.insert("id".to_string(), json!(id));
    }
    v
}

/// 递归搬目录（rename 跨卷会失败，所以要有退路）
fn move_dir(from: &Path, to: &Path) -> Result<(), String> {
    if std::fs::rename(from, to).is_ok() {
        return Ok(());
    }
    let mut stack = vec![(from.to_path_buf(), to.to_path_buf())];
    while let Some((src, dst)) = stack.pop() {
        std::fs::create_dir_all(&dst).map_err(|e| format!("建目录失败：{e}"))?;
        let rd = std::fs::read_dir(&src).map_err(|e| format!("读目录失败：{e}"))?;
        for e in rd.flatten() {
            let p = e.path();
            let d = dst.join(e.file_name());
            if p.is_dir() {
                stack.push((p, d));
            } else {
                std::fs::rename(&p, &d).or_else(|_| {
                    std::fs::copy(&p, &d).map(|_| ()).and_then(|_| std::fs::remove_file(&p))
                })
                .map_err(|e| format!("搬 {} 失败：{e}", p.display()))?;
            }
        }
    }
    let _ = std::fs::remove_dir_all(from);
    Ok(())
}

/// 整合包自己的元数据文件，不该进实例目录（PCL 用不上，留着只会让人困惑）
const PACK_META: [&str; 6] = [
    "manifest.json",
    "modrinth.index.json",
    "mmc-pack.json",
    "instance.cfg",
    "pack.json",
    "meta.json",
];

/// 整合包根下这几个目录装的是「游戏目录里的东西」，要摊平到实例根。
///
///   CurseForge / Modrinth → overrides/
///   Prism / MultiMC       → .minecraft/
///
/// 不摊平的话 mods 会被塞进 versions/<实例>/overrides/mods，游戏根本读不到。
const FLATTEN: [&str; 3] = ["overrides", ".minecraft", "minecraft"];

/// 把整合包内容并进实例目录（同名覆盖）
fn merge_into(src: &Path, dst: &Path) -> Result<usize, String> {
    merge_at(src, dst, true)
}

fn merge_at(src: &Path, dst: &Path, top: bool) -> Result<usize, String> {
    let mut n = 0usize;
    std::fs::create_dir_all(dst).map_err(|e| format!("建目录失败：{e}"))?;
    let rd = std::fs::read_dir(src).map_err(|e| format!("读目录失败：{e}"))?;
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if top && PACK_META.iter().any(|m| *m == name) {
            continue;
        }
        let p = e.path();
        // overrides/ 和 .minecraft/ 的内容直接摊到实例根，不保留这一层
        if top && p.is_dir() && FLATTEN.iter().any(|f| f.eq_ignore_ascii_case(&name)) {
            n += merge_at(&p, dst, false)?;
            continue;
        }
        let d = dst.join(&name);
        if p.is_dir() {
            n += merge_at(&p, &d, false)?;
        } else {
            if let Some(par) = d.parent() {
                let _ = std::fs::create_dir_all(par);
            }
            std::fs::rename(&p, &d)
                .or_else(|_| std::fs::copy(&p, &d).map(|_| ()))
                .map_err(|e| format!("搬 {} 失败：{e}", p.display()))?;
            n += 1;
        }
    }
    Ok(n)
}

/// 主流程：装原版 → 装加载器 → 产出 `versions/<实例名>/`
///
/// `content` 给了的话，装完把里面的东西并进实例目录（整合包的 overrides）。
pub fn install(
    t: &Target,
    content: Option<&Path>,
    prog: Option<Progress>,
    cancel: std::sync::Arc<std::sync::atomic::AtomicBool>,
) -> Value {
    let say = |stage: &str| {
        if let Some(p) = &prog {
            p(stage, 0, 0, 0, 0);
        }
    };

    let fail = |msg: String| json!({ "ok": false, "error": msg });
    let root = t.root.clone();

    if t.mc.is_empty() {
        return fail("没有拿到 Minecraft 版本号".into());
    }
    if let Err(e) = std::fs::create_dir_all(&root) {
        return fail(format!("建游戏目录失败：{e}"));
    }

    // ── 1. 原版 ──
    say("读取版本信息…");
    let ver = match version_json(&t.mc, t.mirror) {
        Ok(v) => v,
        Err(e) => return fail(e),
    };

    say("下载游戏本体…");
    let (tasks, writes) = vanilla_tasks(&root, &ver, t.mirror);
    let (ok, mut fails) = run(tasks, "下载游戏本体…", &prog, &cancel);
    let mut files = ok;

    // 版本 JSON 落盘（原样，id 就是 mc 版本号）
    for (path, v) in writes {
        if let Err(e) = write_json(&path, &v) {
            return fail(e);
        }
    }

    // ── 2. 资源 ──
    let asset_url = ver
        .get("assetIndex")
        .and_then(|a| a.get("url"))
        .and_then(|x| x.as_str())
        .unwrap_or("")
        .to_string();
    if !asset_url.is_empty() {
        say("读取资源索引…");
        match get_json(&[asset_url.clone()]) {
            Ok(index) => {
                let (atasks, awrites) = asset_tasks(&root, &index, t.mirror);
                let (n, f2) = run_with(atasks, "下载资源文件…", &prog, &cancel, 32);
                files += n;
                fails.extend(f2);
                for (path, v) in awrites {
                    if let Err(e) = write_json(&path, &v) {
                        return fail(e);
                    }
                }
            }
            Err(e) => fails.push(format!("资源索引：{e}")),
        }
    }

    // ── 3. 加载器 ──
    let mut loader_id = t.mc.clone();
    match t.loader.as_str() {
        "vanilla" => {}

        "fabric" | "quilt" => {
            say("读取加载器信息…");
            let lv = match fabric_loader_version(&t.mc, &t.loader, &t.loader_ver) {
                Ok(v) => v,
                Err(e) => return fail(e),
            };
            let profile = match fabric_profile(&t.mc, &t.loader, &lv) {
                Ok(v) => v,
                Err(e) => return fail(e),
            };
            loader_id = profile
                .get("id")
                .and_then(|x| x.as_str())
                .unwrap_or(&format!("{}-{}-{}", t.loader, lv, t.mc))
                .to_string();

            let tasks = loader_tasks(&root, &profile, t.mirror);
            let (n, f2) = run(tasks, "下载加载器…", &prog, &cancel);
            files += n;
            fails.extend(f2);

            let vdir = root.join("versions").join(&loader_id);
            if let Err(e) = write_json(&vdir.join(format!("{loader_id}.json")), &profile) {
                return fail(e);
            }
        }

        "forge" | "neoforge" => {
            say("准备安装器…");
            // 整合包没写加载器版本就自己查一个，别拿空版本去拼文件名
            let mut lver = t.loader_ver.clone();
            if lver.is_empty() {
                let r = if t.loader == "neoforge" {
                    resolve_neoforge_version(&t.mc)
                } else {
                    resolve_forge_version(&t.mc)
                };
                match r {
                    Ok(v) => {
                        lver = v;
                        if let Some(p) = &prog {
                            p(&format!("{} 版本 {}", t.loader, lver), 0, 0, 0, 0);
                        }
                    }
                    Err(e) => return fail(e),
                }
            }

            let (m_url, o_url) = if t.loader == "neoforge" {
                neoforge_installer_url(&lver, t.mirror)
            } else {
                forge_installer_url(&t.mc, &lver, t.mirror)
            };

            let cache = crate::db::app_dir().join("cache");
            let _ = std::fs::create_dir_all(&cache);
            let jar_name = m_url.rsplit('/').next().unwrap_or("installer.jar").to_string();
            let itask = task(&cache, &jar_name, vec![m_url, o_url], 0, "");
            let (n, f2) = run(vec![itask], "下载安装器…", &prog, &cancel);
            files += n;
            if !f2.is_empty() {
                fails.extend(f2);
                return json!({ "ok": false, "error": fails.join("\n"), "failed": fails });
            }
            let jar = cache.join(&jar_name);

            // 老安装器要求游戏目录里有 launcher_profiles.json
            let lp = root.join("launcher_profiles.json");
            if !lp.exists() {
                let _ = std::fs::write(&lp, "{\"profiles\":{}}");
            }

            say("运行安装器…");
            if let Err(e) = run_forge_installer(&t.java, &jar, &root) {
                return json!({ "ok": false, "error": e, "failed": fails });
            }
            // 安装器会留一份自己的日志在游戏根目录，用户用不上
            let _ = std::fs::remove_file(root.join(format!("{jar_name}.log")));

            // 安装器自己会建 versions/<完整id>/，找出来
            let want = if t.loader == "neoforge" {
                format!("neoforge-{lver}")
            } else if lver.starts_with(&format!("{}-", t.mc)) {
                lver.clone()
            } else {
                format!("{}-{lver}", t.mc)
            };
            let mut found = String::new();
            if root.join("versions").join(&want).join(format!("{want}.json")).exists() {
                found = want;
            } else if let Ok(rd) = std::fs::read_dir(root.join("versions")) {
                // 兜底：按前缀找最新建的
                let mut cands: Vec<String> = rd
                    .flatten()
                    .filter_map(|e| {
                        let n = e.file_name().to_string_lossy().to_string();
                        let hit = if t.loader == "neoforge" {
                            n.starts_with("neoforge")
                        } else {
                            n.contains("forge") && n.starts_with(&t.mc)
                        };
                        if hit {
                            Some(n)
                        } else {
                            None
                        }
                    })
                    .collect();
                cands.sort();
                found = cands.pop().unwrap_or_default();
            }
            if found.is_empty() {
                return json!({
                    "ok": false,
                    "error": "安装器跑完了但没找到生成的版本目录",
                    "failed": fails
                });
            }
            loader_id = found;
        }

        other => return fail(format!("不认识的加载器：{other}")),
    }

    // ── 4. 改成实例名 ──
    let inst = sanitize_name(&t.instance);
    let inst_dir = root.join("versions").join(&inst);

    if loader_id != inst {
        if t.loader == "vanilla" {
            // 原版版本**不能搬**：别的实例都继承它。
            // 改成写一个只继承原版的空版本，实例名放这里。
            let _ = std::fs::create_dir_all(&inst_dir);
            let stub = json!({
                "id": inst,
                "inheritsFrom": loader_id,
                "type": "release",
                "jar": loader_id,
            });
            if let Err(e) = write_json(&inst_dir.join(format!("{inst}.json")), &stub) {
                return fail(e);
            }
        } else {
            // 加载器版本是叶子节点，直接改名成实例名（PCL 自己也是这么干的）
            let src = root.join("versions").join(&loader_id);
            if src.exists() {
                if inst_dir.exists() {
                    // 同名实例已经在了：并进去，**不能整个删掉** ——
                    // 那会把玩家自己加的模组、存档配置一起抹了
                    if let Err(e) = merge_into(&src, &inst_dir) {
                        return fail(e);
                    }
                    let _ = std::fs::remove_dir_all(&src);
                } else if let Err(e) = move_dir(&src, &inst_dir) {
                    return fail(e);
                }
            } else if !inst_dir.exists() {
                let _ = std::fs::create_dir_all(&inst_dir);
            }

            // 改 id，并把版本 JSON 改名为 <实例名>.json
            let src_json = inst_dir.join(format!("{loader_id}.json"));
            if src_json.exists() {
                if let Ok(txt) = std::fs::read_to_string(&src_json) {
                    if let Ok(v) = serde_json::from_str::<Value>(&txt) {
                        let _ = write_json(&inst_dir.join(format!("{inst}.json")), &retag(v, &inst));
                    }
                }
                let _ = std::fs::remove_file(&src_json);
            }
            let src_jar = inst_dir.join(format!("{loader_id}.jar"));
            if src_jar.exists() {
                let _ = std::fs::rename(&src_jar, inst_dir.join(format!("{inst}.jar")));
            }
        }
    }

    // ── 5. 并进整合包内容 ──
    let mut moved = 0usize;
    if let Some(c) = content {
        // 防呆：内容目录就是实例目录（或它的祖先）时绝对不能搬
        let same = c == inst_dir
            || inst_dir.starts_with(c)
            || c.starts_with(&inst_dir);
        if same {
            fails.push(format!(
                "整合包内容和实例目录重叠，已跳过搬移：{}",
                c.display()
            ));
        } else if c.is_dir() {
            say("写入整合包文件…");
            let expect = count_files(c);
            match merge_into(c, &inst_dir) {
                Ok(n) => {
                    moved = n;
                    // 源里明明有文件却一个都没搬过去 —— 多半是路径没对上，
                    // 这种「装完了但没东西」最坑，必须报出来
                    if expect > 0 && n == 0 {
                        fails.push(format!(
                            "整合包内容没有被写入：{}（源目录里有 {expect} 个文件）",
                            c.display()
                        ));
                    }
                }
                Err(e) => fails.push(format!("写入整合包内容：{e}")),
            }
        }
    }

    // ── 6. Modrinth：模组不在包里，按 files[] 一个个下 ──
    if let Some(c) = content {
        let idx_path = c.join("modrinth.index.json");
        if let Ok(txt) = std::fs::read_to_string(&idx_path) {
            if let Ok(idx) = serde_json::from_str::<Value>(&txt) {
                let tasks = modrinth_tasks(&inst_dir, &idx);
                if !tasks.is_empty() {
                    say("下载整合包里的模组…");
                    let (n, f2) = run_with(tasks, "下载整合包里的模组…", &prog, &cancel, 16);
                    files += n;
                    fails.extend(f2);
                }
            }
        }
    }

    if !inst_dir.exists() {
        let _ = std::fs::create_dir_all(&inst_dir);
    }

    json!({
        "ok": fails.is_empty(),
        "instance": inst,
        "dir": inst_dir.to_string_lossy(),
        "version": loader_id,
        "files": files,
        "moved": moved,
        "content": content.map(|c| c.to_string_lossy().to_string()).unwrap_or_default(),
        "mods": count_mods(&inst_dir),
        "failed": fails,
    })
}

/// 数一数整合包内容里有多少个文件（跳过顶层元信息文件）
pub fn count_files(src: &Path) -> usize {
    let mut n = 0usize;
    let mut stack = vec![(src.to_path_buf(), true)];
    while let Some((dir, top)) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&dir) else { continue };
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if top && PACK_META.iter().any(|m| *m == name) {
                continue;
            }
            let p = e.path();
            if p.is_dir() {
                stack.push((p, false));
            } else {
                n += 1;
            }
        }
    }
    n
}

/// 安全拼接相对路径：绝对路径、盘符、`..` 一律拒绝
fn safe_rel(p: &str) -> Option<PathBuf> {
    let n = p.replace('\\', "/");
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

/// Modrinth 的 .mrpack 里**没有模组文件**，只有一份清单和下载地址。
/// 不处理 files[] 的话，装完 mods 是空的。
pub fn modrinth_tasks(inst_dir: &Path, index: &Value) -> Vec<DownloadTask> {
    let mut out = Vec::new();
    let Some(arr) = index.get("files").and_then(|x| x.as_array()) else {
        return out;
    };
    for f in arr {
        // 只装客户端需要的
        if let Some(c) = f
            .get("env")
            .and_then(|e| e.get("client"))
            .and_then(|x| x.as_str())
        {
            if c == "unsupported" {
                continue;
            }
        }
        let path = match f.get("path").and_then(|x| x.as_str()) {
            Some(p) if !p.is_empty() => p,
            _ => continue,
        };
        let rel = match safe_rel(path) {
            Some(r) => r,
            None => continue,
        };
        let urls: Vec<String> = f
            .get("downloads")
            .and_then(|x| x.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|u| u.as_str())
                    .map(|s| s.to_string())
                    .collect()
            })
            .unwrap_or_default();
        if urls.is_empty() {
            continue;
        }
        let sha1 = f
            .get("hashes")
            .and_then(|h| h.get("sha1"))
            .and_then(|x| x.as_str())
            .unwrap_or("");
        let size = f.get("fileSize").and_then(|x| x.as_u64()).unwrap_or(0);

        let full = inst_dir.join(&rel);
        let dir = full
            .parent()
            .map(|p| p.to_path_buf())
            .unwrap_or_else(|| inst_dir.to_path_buf());
        let name = full
            .file_name()
            .map(|s| s.to_string_lossy().to_string())
            .unwrap_or_default();
        if name.is_empty() {
            continue;
        }
        let hash = if sha1.is_empty() {
            String::new()
        } else {
            format!("sha1:{sha1}")
        };
        out.push(task(&dir, &name, urls, size, &hash));
    }
    out
}

/// 数一数实例里到底有几个模组（.jar / .jar.disabled）
pub fn count_mods(inst_dir: &Path) -> usize {
    let mods = inst_dir.join("mods");
    let Ok(rd) = std::fs::read_dir(&mods) else { return 0 };
    rd.flatten()
        .filter(|e| {
            let n = e.file_name().to_string_lossy().to_ascii_lowercase();
            n.ends_with(".jar") || n.ends_with(".jar.disabled")
        })
        .count()
}

/// 实例名清洗：去掉 Windows 不允许的字符
pub fn sanitize_name(s: &str) -> String {
    let mut out = String::new();
    for ch in s.chars() {
        if "\\/:*?\"<>|".contains(ch) || ch.is_control() {
            out.push('_');
        } else {
            out.push(ch);
        }
    }
    let t = out.trim().trim_end_matches('.').to_string();
    if t.is_empty() {
        "modpack".to_string()
    } else {
        t
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rules_platform() {
        let win = json!([{"action":"allow","os":{"name":"windows"}}]);
        let linux = json!([{"action":"allow","os":{"name":"linux"}}]);
        assert_eq!(is_allowed(&win), cfg!(windows));
        assert_eq!(is_allowed(&linux), cfg!(target_os = "linux"));
        assert!(is_allowed(&Value::Null));
        assert!(is_allowed(&json!([])));
    }

    #[test]
    fn rules_disallow_wins() {
        // 先允许全平台，再禁掉 32 位 —— 在 64 位上应该仍然允许
        let r = json!([
            {"action":"allow"},
            {"action":"disallow","os":{"arch":"x86"}}
        ]);
        assert!(is_allowed(&r));
    }

    #[test]
    fn rules_features_never_match() {
        let r = json!([{"action":"allow","features":{"has_custom_resolution":true}}]);
        assert!(!is_allowed(&r));
    }

    #[test]
    fn maven_paths() {
        assert_eq!(
            maven_path("org.ow2.asm:asm:9.5").unwrap(),
            "org/ow2/asm/asm/9.5/asm-9.5.jar"
        );
        assert_eq!(
            maven_path("net.fabricmc:sponge-mixin:0.12.5+mixin.0.8.5").unwrap(),
            "net/fabricmc/sponge-mixin/0.12.5+mixin.0.8.5/sponge-mixin-0.12.5+mixin.0.8.5.jar"
        );
        assert_eq!(
            maven_path("org.lwjgl:lwjgl:3.3.1:natives-windows").unwrap(),
            "org/lwjgl/lwjgl/3.3.1/lwjgl-3.3.1-natives-windows.jar"
        );
        assert!(maven_path("bad").is_none());
    }

    #[test]
    fn loader_ids() {
        assert_eq!(split_loader_id("forge-47.2.0"), ("forge".into(), "47.2.0".into()));
        assert_eq!(
            split_loader_id("forge-1.20.1-47.2.0"),
            ("forge".into(), "47.2.0".into())
        );
        assert_eq!(split_loader_id("fabric-0.15.11"), ("fabric".into(), "0.15.11".into()));
        assert_eq!(
            split_loader_id("neoforge-21.1.1"),
            ("neoforge".into(), "21.1.1".into())
        );
        assert_eq!(split_loader_id("weird"), ("vanilla".into(), "".into()));
    }

    #[test]
    fn mirror_rewrites() {
        assert_eq!(
            mirror_of("https://libraries.minecraft.net/a/b/c.jar").unwrap(),
            format!("{BMCL}/maven/a/b/c.jar")
        );
        assert_eq!(
            mirror_of("https://resources.download.minecraft.net/ab/abcd").unwrap(),
            format!("{BMCL}/assets/ab/abcd")
        );
        assert!(mirror_of("https://piston-data.mojang.com/x").is_none());
    }

    #[test]
    fn mirror_order() {
        let u = urls_for("https://libraries.minecraft.net/a.jar", true);
        assert_eq!(u.len(), 2);
        assert!(u[0].contains("bmclapi"));
        assert!(u[1].contains("libraries.minecraft.net"));

        let u = urls_for("https://libraries.minecraft.net/a.jar", false);
        assert_eq!(u.len(), 1);
        assert!(u[0].contains("libraries.minecraft.net"));
    }

    #[test]
    fn merge_skips_pack_meta() {
        let base = std::env::temp_dir().join("pulses_merge_meta");
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(base.join("src/mods")).unwrap();
        std::fs::create_dir_all(base.join("src/config")).unwrap();
        std::fs::write(base.join("src/manifest.json"), "{}").unwrap();
        std::fs::write(base.join("src/mods/a.jar"), "x").unwrap();
        // 嵌套的 manifest.json 是 mod 自己的东西，不能误删
        std::fs::write(base.join("src/config/manifest.json"), "{}").unwrap();

        let n = merge_into(&base.join("src"), &base.join("dst")).unwrap();
        assert_eq!(n, 2);
        assert!(base.join("dst/mods/a.jar").exists());
        assert!(base.join("dst/config/manifest.json").exists());
        assert!(!base.join("dst/manifest.json").exists());

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn sanitize_instance_names() {
        assert_eq!(sanitize_name("a/b:c*?"), "a_b_c__");
        assert_eq!(sanitize_name("  红石生电优化  "), "红石生电优化");
        assert_eq!(sanitize_name(""), "modpack");
        assert_eq!(sanitize_name(".."), "modpack");
        assert_eq!(sanitize_name("名字."), "名字");
    }

    #[test]
    fn retag_changes_id() {
        let v = retag(json!({"id":"1.20.1","x":1}), "我的整合包");
        assert_eq!(v["id"], "我的整合包");
        assert_eq!(v["x"], 1);
    }
}
