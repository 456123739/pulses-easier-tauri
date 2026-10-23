// mirrors.rs — 下载镜像站表（Modrinth / CurseForge）
// 对应 Python: app/core/mirrors.py
//
// 官方 CDN（`cdn.modrinth.com`）在国内经常偏慢，所以下载时把镜像地址排在前面：
// 镜像通了就用，挂了就退回官方，对上层完全透明。
//
// **这张表是数据，不是逻辑**：新增/停用一个镜像只改 `MIRROR_SITES`，不用动引擎。
//
// 现状（2026-10 核对，与 Python 侧一致）：
//   · MCIM（mod.mcimirror.top / api.mcimirror.top）已停服，条目保留但 enabled=false
//   · BMCLAPI 只镜像 MC 原版资源，没有模组文件，不进表
//   · 公开通用反代扛不住二进制大文件，不进表
//   · 官方 CDN 永远是最后兜底，不依赖表
//
// 所以默认表里**没有启用中的镜像**，下载行为与旧版一致。有可用镜像时改一个字段就行。

use std::collections::{HashMap, HashSet};
use std::sync::{Mutex, OnceLock};

// ═══════════════════════════════════════════════════════════════
// 表
// ═══════════════════════════════════════════════════════════════

pub struct MirrorSite {
    pub key: &'static str,
    pub name: &'static str,
    /// 前缀式：base + 官方 path
    pub base: &'static str,
    /// 模板式：含 `{qurl}`（已编码）或 `{url}`（原样）
    pub template: &'static str,
    /// 这个镜像服务哪几类内容
    pub kinds: &'static [&'static str],
    pub enabled: bool,
    pub note: &'static str,
    /// 最后一次人工核对日期
    pub checked: &'static str,
}

impl MirrorSite {
    fn base_of(&self) -> String {
        if !self.base.is_empty() {
            return self.base.trim_end_matches('/').to_string();
        }
        if !self.template.is_empty() {
            let stripped = self.template.replace("{qurl}", "").replace("{url}", "");
            return base_of(&stripped);
        }
        String::new()
    }

    fn build(&self, official: &str, path: &str) -> String {
        if !self.template.is_empty() {
            let q = percent_encode(official);
            return self
                .template
                .replace("{qurl}", &q)
                .replace("{url}", official);
        }
        format!("{}{}", self.base.trim_end_matches('/'), path)
    }
}

/// 主机 → 内容类型。只有表里认得的类型才会派生镜像。
const HOST_KIND: &[(&str, &str)] = &[
    ("cdn.modrinth.com", "modrinth"),
    ("cdn-raw.modrinth.com", "modrinth"),
    ("edge.forgecdn.net", "curseforge"),
    ("mediafilez.forgecdn.net", "curseforge"),
];

pub const MIRROR_SITES: &[MirrorSite] = &[
    MirrorSite {
        key: "mcim",
        name: "MCIM（Modrinth 反代）",
        base: "https://mod.mcimirror.top",
        template: "",
        kinds: &["modrinth"],
        enabled: false,
        note: "2026-10 起全站 503，DNS 仅返回 IPv6；服务恢复后可重新启用",
        checked: "2026-10-22",
    },
    MirrorSite {
        key: "mcim-api",
        name: "MCIM（API 反代）",
        base: "https://api.mcimirror.top",
        template: "",
        kinds: &["modrinth"],
        enabled: false,
        note: "DNS 不可达（已随主站下线）",
        checked: "2026-10-22",
    },
    MirrorSite {
        key: "bmclapi",
        name: "BMCLAPI",
        base: "https://bmclapi2.bangbang93.com",
        template: "",
        kinds: &["minecraft"],
        enabled: false,
        note: "只镜像 MC 原版资源（版本/库/资源），没有模组文件；本工具用不上",
        checked: "2026-10-22",
    },
];

/// 失败几次后本次运行不再用这个镜像
const FAIL_LIMIT: usize = 3;

// ═══════════════════════════════════════════════════════════════
// 状态（进程内）
// ═══════════════════════════════════════════════════════════════

struct State {
    enabled: bool,
    /// 设置里临时追加的前缀，排在表前面
    hosts: Vec<String>,
    fail: HashMap<String, usize>,
    dead: HashSet<String>,
}

fn state() -> &'static Mutex<State> {
    static S: OnceLock<Mutex<State>> = OnceLock::new();
    S.get_or_init(|| {
        Mutex::new(State {
            enabled: true,
            hosts: Vec::new(),
            fail: HashMap::new(),
            dead: HashSet::new(),
        })
    })
}

/// 每批下载开始前调一次（不要在每文件里调，会反复抢锁）。
pub fn configure(enabled: bool, hosts: &[String]) {
    let mut s = match state().lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    s.enabled = enabled;
    let mut out: Vec<String> = Vec::new();
    for h in hosts {
        let t = h.trim().trim_end_matches('/');
        if !t.is_empty() && t.contains("://") && !out.iter().any(|x| x == t) {
            out.push(t.to_string());
        }
    }
    s.hosts = out;
}

/// 解析 `https://host/path` → (host 小写, path)
fn split_host_path(url: &str) -> Option<(String, String)> {
    let i = url.find("://")?;
    let rest = &url[i + 3..];
    match rest.find('/') {
        Some(j) => Some((rest[..j].to_ascii_lowercase(), rest[j..].to_string())),
        None => Some((rest.to_ascii_lowercase(), "/".to_string())),
    }
}

/// `https://host/a` → `https://host`
fn base_of(url: &str) -> String {
    let i = match url.find("://") {
        Some(i) => i,
        None => return String::new(),
    };
    let scheme = &url[..i];
    let rest = &url[i + 3..];
    let host = match rest.find('/') {
        Some(j) => &rest[..j],
        None => rest,
    };
    if host.is_empty() {
        return String::new();
    }
    format!("{scheme}://{host}")
}

/// 尽量贴近 Python 的 `urllib.parse.quote(s, safe="")`
fn percent_encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.as_bytes() {
        let c = *b as char;
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '~') {
            out.push(c);
        } else {
            out.push('%');
            out.push_str(&format!("{:02X}", b));
        }
    }
    out
}

/// 表里启用的 + 设置里临时追加的（后者排前面，方便覆盖）
fn active() -> Vec<(&'static str, String)> {
    let s = match state().lock() {
        Ok(g) => g,
        Err(_) => return Vec::new(),
    };
    if !s.enabled {
        return Vec::new();
    }
    let mut out: Vec<(&'static str, String)> = Vec::new();
    for h in &s.hosts {
        out.push(("", format!("{h}/")));
    }
    for site in MIRROR_SITES {
        if !site.enabled {
            continue;
        }
        let b = site.base_of();
        if b.is_empty() {
            continue;
        }
        out.push((site.key, b));
    }
    out
}

fn site_by_key(key: &str) -> Option<&'static MirrorSite> {
    MIRROR_SITES.iter().find(|s| s.key == key)
}

/// 给一个官方地址返回它的镜像候选（按表顺序，已去掉不可用的）
pub fn candidates(url: &str) -> Vec<String> {
    let (enabled, dead) = match state().lock() {
        Ok(g) => (g.enabled, g.dead.clone()),
        Err(_) => return Vec::new(),
    };
    if !enabled {
        return Vec::new();
    }
    let (host, path) = match split_host_path(url) {
        Some(v) => v,
        None => return Vec::new(),
    };
    let kind = match HOST_KIND.iter().find(|(h, _)| *h == host) {
        Some((_, k)) => *k,
        None => return Vec::new(),
    };

    let mut out: Vec<String> = Vec::new();

    // 用户临时前缀：不区分内容类型，直接前缀拼
    for (key, base) in active() {
        if !key.is_empty() {
            continue;
        }
        let b = base.trim_end_matches('/').to_string();
        if b.is_empty() || dead.contains(&b) {
            continue;
        }
        let cand = format!("{b}{path}");
        if !out.contains(&cand) {
            out.push(cand);
        }
    }

    // 表里的站点
    let s = match state().lock() {
        Ok(g) => g,
        Err(_) => return out,
    };
    if !s.enabled {
        return out;
    }
    for site in MIRROR_SITES {
        if !site.enabled || !site.kinds.contains(&kind) {
            continue;
        }
        let b = site.base_of();
        if b.is_empty() || dead.contains(&b) {
            continue;
        }
        let cand = site.build(url, &path);
        if !cand.is_empty() && !out.contains(&cand) {
            out.push(cand);
        }
    }
    out
}

/// 镜像在前、官方在后。没有可用镜像时原样返回。
pub fn ordered(urls: Vec<String>) -> Vec<String> {
    if urls.is_empty() {
        return urls;
    }
    let on = match state().lock() {
        Ok(g) => g.enabled,
        Err(_) => false,
    };
    if !on {
        return urls;
    }

    let mut head: Vec<String> = Vec::new();
    let mut tail: Vec<String> = Vec::new();
    for u in &urls {
        let c = candidates(u);
        if c.is_empty() {
            tail.push(u.clone());
        } else {
            head.extend(c);
            tail.push(u.clone());
        }
    }
    if head.is_empty() {
        return urls;
    }

    let mut seen: HashSet<String> = HashSet::new();
    let mut out: Vec<String> = Vec::new();
    for u in head.into_iter().chain(tail) {
        if seen.insert(u.clone()) {
            out.push(u);
        }
    }
    out
}

// ═══════════════════════════════════════════════════════════════
// 健康度
// ═══════════════════════════════════════════════════════════════

fn known_bases() -> HashSet<String> {
    let mut out = HashSet::new();
    for (_, b) in active() {
        let t = b.trim_end_matches('/');
        if !t.is_empty() {
            out.insert(t.to_string());
        }
    }
    for site in MIRROR_SITES {
        if site.enabled {
            let b = site.base_of();
            if !b.is_empty() {
                out.insert(b);
            }
        }
    }
    out
}

pub fn note_failure(url: &str) {
    let base = base_of(url);
    if base.is_empty() || !known_bases().contains(&base) {
        return;
    }
    let mut s = match state().lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    let n = s.fail.get(&base).copied().unwrap_or(0) + 1;
    s.fail.insert(base.clone(), n);
    if n >= FAIL_LIMIT {
        s.dead.insert(base);
    }
}

pub fn note_success(url: &str) {
    let base = base_of(url);
    if base.is_empty() {
        return;
    }
    let mut s = match state().lock() {
        Ok(g) => g,
        Err(_) => return,
    };
    s.fail.remove(&base);
    s.dead.remove(&base);
}

/// 测试用：清空健康度
pub fn reset() {
    if let Ok(mut s) = state().lock() {
        s.fail.clear();
        s.dead.clear();
    }
}

pub fn dead_hosts() -> HashSet<String> {
    match state().lock() {
        Ok(g) => g.dead.clone(),
        Err(_) => HashSet::new(),
    }
}

pub fn is_mirror(url: &str) -> bool {
    known_bases().contains(&base_of(url))
}

/// 当前生效的镜像 key 列表（自检用）
pub fn enabled_keys() -> Vec<&'static str> {
    MIRROR_SITES
        .iter()
        .filter(|s| s.enabled)
        .map(|s| s.key)
        .collect()
}

// 让 site_by_key 在没人用时也不报警
#[allow(dead_code)]
fn _unused(key: &str) -> Option<&'static MirrorSite> {
    site_by_key(key)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 镜像配置是进程级全局状态，测试并行跑会互相踩，统一串行
    fn guard() -> std::sync::MutexGuard<'static, ()> {
        static M: OnceLock<Mutex<()>> = OnceLock::new();
        M.get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner())
    }

    fn setup(enabled: bool, hosts: &[&str]) {
        let hs: Vec<String> = hosts.iter().map(|s| s.to_string()).collect();
        configure(enabled, &hs);
        reset();
        // 表里默认没有启用的站点，测试里临时打开一个前缀式镜像
        for site in MIRROR_SITES {
            let _ = site;
        }
    }

    #[test]
    fn percent_encoding_matches_python() {
        let _g = guard();
        assert_eq!(percent_encode("https://a.com/b c"), "https%3A%2F%2Fa.com%2Fb%20c");
        assert_eq!(percent_encode("a-b_c.d~e"), "a-b_c.d~e");
    }

    #[test]
    fn splits_host_and_path() {
        let _g = guard();
        let (h, p) = split_host_path("https://cdn.modrinth.com/data/x/y.jar").unwrap();
        assert_eq!(h, "cdn.modrinth.com");
        assert_eq!(p, "/data/x/y.jar");
        assert_eq!(base_of("https://cdn.modrinth.com/data/x/y.jar"), "https://cdn.modrinth.com");
    }

    #[test]
    fn no_mirror_configured_keeps_order() {
        let _g = guard();
        setup(true, &[]);
        let urls = vec!["https://cdn.modrinth.com/data/a.jar".to_string()];
        assert_eq!(ordered(urls.clone()), urls);
    }

    #[test]
    fn user_prefix_goes_first_official_last() {
        let _g = guard();
        setup(true, &["https://mirror.example.com"]);
        let urls = vec!["https://cdn.modrinth.com/data/a.jar".to_string()];
        let got = ordered(urls);
        assert_eq!(got.len(), 2);
        assert!(got[0].starts_with("https://mirror.example.com"));
        assert!(got[0].ends_with("/data/a.jar"));
        assert_eq!(got[1], "https://cdn.modrinth.com/data/a.jar");
    }

    #[test]
    fn disabled_means_untouched() {
        let _g = guard();
        setup(false, &["https://mirror.example.com"]);
        let urls = vec!["https://cdn.modrinth.com/data/a.jar".to_string()];
        assert_eq!(ordered(urls.clone()), urls);
        assert!(candidates("https://cdn.modrinth.com/data/a.jar").is_empty());
    }

    #[test]
    fn non_modrinth_untouched() {
        let _g = guard();
        setup(true, &["https://mirror.example.com"]);
        let urls = vec!["https://libraries.minecraft.net/a.jar".to_string()];
        assert_eq!(ordered(urls.clone()), urls);
    }

    #[test]
    fn three_failures_marks_dead_then_success_clears() {
        let _g = guard();
        setup(true, &["https://mirror.example.com"]);
        let m = "https://mirror.example.com/data/a.jar";
        for _ in 0..3 {
            note_failure(m);
        }
        assert!(dead_hosts().contains("https://mirror.example.com"));

        // 死了以后不再出现在候选里
        let got = ordered(vec!["https://cdn.modrinth.com/data/b.jar".to_string()]);
        assert_eq!(got.len(), 1);

        note_success(m);
        assert!(!dead_hosts().contains("https://mirror.example.com"));
        let got = ordered(vec!["https://cdn.modrinth.com/data/c.jar".to_string()]);
        assert_eq!(got.len(), 2);
    }

    #[test]
    fn default_table_has_nothing_enabled() {
        let _g = guard();
        // 与 Python 侧保持一致：默认不改变下载行为
        assert!(enabled_keys().is_empty());
    }
}
