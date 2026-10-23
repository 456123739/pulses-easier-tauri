// config_exclude.rs — 「智能剔除」：哪些 config 文件属于优化 / 渲染类模组
// 对应 Python: app/core/config_exclude.py
//
// 「智能剔除」是 config 的第四档策略：更新照常合并 config，但**属于客户端
// 优化模组和渲染器的配置文件保持玩家本地版本**（本地没有的才写入包里的默认值）。
//
// 为什么需要：这些模组的配置项是按玩家的机器调的（GPU、显存、渲染距离、
// 分配内存、mixin 开关）。整合包一更新就把作者机器上的默认值推下来，
// 轻则画面变糊掉帧，重则直接崩。
//
// 判定方式：config 里的文件几乎都以模组 id 命名，按路径反查归属。
//   config/sodium-options.json          → sodium
//   config/DistantHorizons.toml         → distanthorizons
//   config/modernfix-mixins.properties  → modernfix
//   config/sodium/xxx.json              → sodium
//   config/particle_core_config_v1.json → particlecore
// 比对前统一规范化（小写、去掉分隔符），`Dynamic_FPS` / `dynamic-fps` 都算同一个。

use std::collections::HashMap;
use std::path::Path;
use std::sync::OnceLock;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Render,
    Perf,
}

impl Kind {
    pub fn label(&self) -> &'static str {
        match self {
            Kind::Render => "渲染器",
            Kind::Perf => "优化",
        }
    }
}

pub struct Owner {
    pub key: &'static str,
    pub name: &'static str,
    pub kind: Kind,
    pub aliases: &'static [&'static str],
}

impl Owner {
    /// 日志里用的写法：`Sodium（钠）（渲染器）`
    pub fn label(&self) -> String {
        format!("{}（{}）", self.name, self.kind.label())
    }
}

use Kind::{Perf as P, Render as R};

/// 表里的人名（别名一并写在这里，索引建立时统一规范化）
///
/// 数据来源：Modrinth optimization 分类按下载量排序的前 200 项，
/// 逐个核对 client_side / server_side 与 jar 内的 mod id、配置文件名。
/// 收录标准：渲染器 / 光影加载器，以及直接决定画质帧数内存的客户端优化。
/// 不收录：纯服务端优化、网络封包、启动器界面工具、纯修复、装饰模型材质。
pub const TABLE: &[Owner] = &[
    // ---- 渲染器 / 光影加载器 ----
    Owner { key: "sodium", name: "Sodium（钠）", kind: R, aliases: &[] },
    Owner { key: "iris", name: "Iris Shaders（光影）", kind: R, aliases: &["irisshaders"] },
    Owner { key: "oculus", name: "Oculus（光影）", kind: R, aliases: &[] },
    Owner { key: "optifine", name: "OptiFine", kind: R, aliases: &[] },
    Owner { key: "embeddium", name: "Embeddium（铷）", kind: R, aliases: &[] },
    Owner { key: "rubidium", name: "Rubidium（铷·旧版）", kind: R, aliases: &[] },
    Owner { key: "magnesium", name: "Magnesium（镁）", kind: R, aliases: &[] },
    Owner { key: "xenon", name: "Xenon（氙）", kind: R, aliases: &["xenonforge"] },
    Owner { key: "vulkanmod", name: "VulkanMod", kind: R, aliases: &[] },
    Owner { key: "chloride", name: "Chloride", kind: R, aliases: &[] },
    Owner { key: "flerovium", name: "Flerovium", kind: R, aliases: &[] },
    Owner { key: "canvas", name: "Canvas 渲染器", kind: R, aliases: &["canvasrenderer"] },
    Owner { key: "nothirium", name: "Nothirium", kind: R, aliases: &[] },
    Owner { key: "nvidium", name: "Nvidium", kind: R, aliases: &[] },
    Owner { key: "sodium-extra", name: "Sodium Extra", kind: R, aliases: &[] },
    Owner { key: "sodium-extras", name: "Sodium Extras", kind: R, aliases: &[] },
    Owner { key: "sodium-options-api", name: "Sodium Options API", kind: R, aliases: &[] },
    Owner { key: "sodium-dynamic-lights", name: "Sodium Dynamic Lights", kind: R, aliases: &[] },
    Owner { key: "sodiumleafculling", name: "Sodium Leaf Culling", kind: R, aliases: &[] },
    Owner { key: "embeddium-plus", name: "Embeddium Plus", kind: R, aliases: &[] },
    Owner { key: "rubidium-extra", name: "Embeddium（Rubidium）Extra", kind: R, aliases: &["embeddiumextra"] },
    Owner { key: "distanthorizons", name: "Distant Horizons（远处地平线）", kind: R, aliases: &[] },
    Owner { key: "voxy", name: "Voxy（LOD 渲染）", kind: R, aliases: &[] },
    Owner { key: "better-clouds", name: "Better Clouds", kind: R, aliases: &[] },
    Owner { key: "bettergrassify", name: "BetterGrassify", kind: R, aliases: &["bettergrass"] },
    Owner { key: "fism", name: "Faster Iris Shadow Mapper（FISM）", kind: R, aliases: &["fastershadowmapper"] },

    // ---- 客户端优化 ----
    Owner { key: "immediatelyfast", name: "ImmediatelyFast", kind: P, aliases: &[] },
    Owner { key: "entityculling", name: "Entity Culling", kind: P, aliases: &[] },
    Owner { key: "moreculling", name: "More Culling", kind: P, aliases: &[] },
    Owner { key: "dynamic-fps", name: "Dynamic FPS", kind: P, aliases: &[] },
    Owner { key: "badoptimizations", name: "BadOptimizations", kind: P, aliases: &[] },
    Owner { key: "ferrite-core", name: "FerriteCore", kind: P, aliases: &[] },
    Owner { key: "krypton", name: "Krypton", kind: P, aliases: &["kryptonfnp"] },
    Owner { key: "lithium", name: "Lithium", kind: P, aliases: &[] },
    Owner { key: "modernfix", name: "ModernFix", kind: P, aliases: &["modernfixmvus"] },
    Owner { key: "memoryleakfix", name: "Memory Leak Fix", kind: P, aliases: &[] },
    Owner { key: "debugify", name: "Debugify", kind: P, aliases: &[] },
    Owner { key: "c2me", name: "C2ME（并发区块管理）", kind: P, aliases: &["c2mefabric", "c2meneoforge"] },
    Owner { key: "modelfix", name: "Model Gap Fix", kind: P, aliases: &["modelgapfix"] },
    Owner { key: "ebe", name: "Enhanced Block Entities", kind: P, aliases: &["enhancedblockentities"] },
    Owner { key: "obe", name: "Optimized Block Entities", kind: P, aliases: &["optimizedblockentities"] },
    Owner { key: "better-block-entities", name: "Better Block Entities", kind: P, aliases: &["bbeconfig", "bbe"] },
    Owner { key: "particle-core", name: "Particle Core", kind: P, aliases: &[] },
    Owner { key: "scalablelux", name: "ScalableLux", kind: P, aliases: &[] },
    Owner { key: "starlight", name: "Starlight", kind: P, aliases: &["starlightforge"] },
    Owner { key: "lazydfu", name: "LazyDFU", kind: P, aliases: &[] },
    Owner { key: "fastload", name: "Fastload", kind: P, aliases: &[] },
    Owner { key: "threadtweak", name: "ThreadTweak", kind: P, aliases: &[] },
    Owner { key: "cull-leaves", name: "Cull Leaves", kind: P, aliases: &[] },
    Owner { key: "cull-less-leaves", name: "Cull Less Leaves", kind: P, aliases: &["culllessleavesreforged"] },
    Owner { key: "fast-paintings", name: "Fast Paintings", kind: P, aliases: &[] },
    Owner { key: "better-beds", name: "Better Beds", kind: P, aliases: &[] },
    Owner { key: "asyncparticles", name: "AsyncParticles", kind: P, aliases: &[] },
    Owner { key: "fps-reducer", name: "FPS Reducer", kind: P, aliases: &[] },
    Owner { key: "exordium", name: "Exordium", kind: P, aliases: &[] },
    Owner { key: "redirected", name: "Redirected", kind: P, aliases: &[] },
    Owner { key: "saturn", name: "Saturn", kind: P, aliases: &[] },
    Owner { key: "pluto", name: "Pluto", kind: P, aliases: &[] },
    Owner { key: "radium", name: "Radium", kind: P, aliases: &[] },
    Owner { key: "canary", name: "Canary", kind: P, aliases: &[] },
    Owner { key: "gputape", name: "GPUBooster", kind: P, aliases: &["gpubooster"] },
    Owner { key: "smooth-boot", name: "Smooth Boot", kind: P, aliases: &["smoothbootfabric", "smoothbootreloaded"] },
    Owner { key: "dashloader", name: "DashLoader", kind: P, aliases: &[] },
    Owner { key: "fastanim", name: "FastAnim", kind: P, aliases: &["fasterentityanimations"] },
    Owner { key: "renderscale", name: "RenderScale", kind: P, aliases: &[] },
    Owner { key: "entity-view-distance", name: "Entity View Distance", kind: P, aliases: &[] },
    Owner { key: "fastchest", name: "FastChest", kind: P, aliases: &[] },
    Owner { key: "foamfix", name: "FoamFix", kind: P, aliases: &[] },
    Owner { key: "phosphor", name: "Phosphor", kind: P, aliases: &[] },
    Owner { key: "clumps", name: "Clumps", kind: P, aliases: &["lyclumps"] },
    Owner { key: "stutterfix", name: "StutterFix", kind: P, aliases: &[] },
    Owner { key: "videotape", name: "Video Tape", kind: P, aliases: &[] },
    Owner { key: "better-biome-blend", name: "Better Biome Blend", kind: P, aliases: &[] },
    Owner { key: "bbrb", name: "Better Biome Reblend", kind: P, aliases: &["betterbiomereblend"] },
    // decce6 的一批，出自 NRG 与「超现代优化」两个整合包
    Owner { key: "ixeris", name: "Ixeris（事件轮询优化）", kind: P, aliases: &[] },
    Owner { key: "gnetum", name: "Gnetum（HUD 分批更新）", kind: P, aliases: &[] },
    Owner { key: "asynclogger", name: "Async Logger（异步日志）", kind: P, aliases: &[] },
    Owner { key: "kerria", name: "Kerria（纹理动画优化）", kind: P, aliases: &["kerriaopt"] },
    Owner { key: "jasione", name: "Jasione（减少内存分配）", kind: P, aliases: &[] },
    Owner { key: "dypsis", name: "Dypsis（客户端渲染优化）", kind: P, aliases: &[] },
    Owner { key: "efficient-hashing", name: "EfficientHashing", kind: P, aliases: &[] },
    Owner { key: "random-optimization", name: "Random Optimization", kind: P, aliases: &[] },
    Owner { key: "mixinbooster", name: "Mixin Booster", kind: P, aliases: &[] },
    Owner { key: "fastevent", name: "FastEvent", kind: P, aliases: &[] },
    Owner { key: "lightspeedre", name: "LightSpeedRe（资源包缓存）", kind: P, aliases: &["lightspeed"] },
    Owner { key: "0pack2reload", name: "0Pack2Reload", kind: P, aliases: &["opack2reload"] },
    Owner { key: "threatengl", name: "ThreatenGL", kind: P, aliases: &[] },
    Owner { key: "performant", name: "Performant", kind: P, aliases: &[] },
    Owner { key: "betterfps", name: "BetterFps", kind: P, aliases: &[] },
    Owner { key: "vanillafix", name: "VanillaFix", kind: P, aliases: &[] },
];

/// 配置文件名的通用尾巴：`sodium-options.json`、`clumps-common.toml` 之类。
/// 剥掉这些词之后再比对模组 id（只认下面这几个，避免误伤别的模组）。
const GENERIC_SUFFIXES: &[&str] = &[
    "options", "option", "config", "configs", "settings", "setting",
    "client", "common", "server", "mixins", "mixin", "tweak", "tweaks",
    "optimization", "optimizations",
    "extra", "extras",
    "v1", "v2", "v3", "v4", "version",
];

/// 按非字母数字切词（`sodium-options` → ["sodium", "options"]）
fn split_words(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut buf = String::new();
    for ch in text.chars() {
        if ch.is_ascii_alphanumeric() {
            buf.push(ch.to_ascii_lowercase());
        } else if !buf.is_empty() {
            out.push(std::mem::take(&mut buf));
        }
    }
    if !buf.is_empty() {
        out.push(buf);
    }
    out
}

/// 小写 + 去掉全部分隔符：`Dynamic_FPS` → `dynamicfps`
pub fn normalize(text: &str) -> String {
    split_words(text).concat()
}

fn index() -> &'static HashMap<String, usize> {
    static IDX: OnceLock<HashMap<String, usize>> = OnceLock::new();
    IDX.get_or_init(|| {
        let mut m = HashMap::new();
        for (i, o) in TABLE.iter().enumerate() {
            m.insert(normalize(o.key), i);
            for a in o.aliases {
                m.insert(normalize(a), i);
            }
        }
        m
    })
}

/// 从一条 config 相对路径推出可能的「模组 id」候选（按可信度排序）
pub fn candidates(rel: &str) -> Vec<String> {
    let norm = rel.replace('\\', "/");
    let parts: Vec<&str> = norm.split('/').filter(|s| !s.is_empty()).collect();
    let mut out: Vec<String> = Vec::new();

    if parts.len() >= 3 && parts[0].eq_ignore_ascii_case("config") {
        let c = normalize(parts[1]);
        if !c.is_empty() {
            out.push(c);
        }
    }
    if let Some(last) = parts.last() {
        // 去掉扩展名
        let stem = last.rsplit_once('.').map(|(a, _)| a).unwrap_or(last);
        let mut tokens = split_words(stem);
        let joined = tokens.concat();
        if !joined.is_empty() {
            out.push(joined);
        }
        while tokens.len() > 1
            && GENERIC_SUFFIXES.contains(&tokens[tokens.len() - 1].as_str())
        {
            tokens.pop();
            let j = tokens.concat();
            if !j.is_empty() {
                out.push(j);
            }
        }
    }

    let mut seen: Vec<String> = Vec::new();
    for c in out {
        if !c.is_empty() && !seen.contains(&c) {
            seen.push(c);
        }
    }
    seen
}

/// 这条 config 文件属于哪个「该保护」的模组；不属于返回 None
pub fn owner_of(rel: &str) -> Option<&'static Owner> {
    let idx = index();
    for cand in candidates(rel) {
        if let Some(&i) = idx.get(&cand) {
            return Some(&TABLE[i]);
        }
    }
    None
}

/// 这条 config 文件是否要「更新时不动玩家本地版本」
pub fn is_protected(rel: &str) -> bool {
    owner_of(rel).is_some()
}

/// 把一批被保护的文件汇总成一句人话
///
///   "Sodium（钠）、Iris Shaders（光影）等 6 个模组的 12 个配置"
pub fn summarize<'a, I>(rels: I, limit: usize) -> String
where
    I: IntoIterator<Item = &'a str>,
{
    let mut names: Vec<&'static str> = Vec::new();
    let mut count = 0usize;
    for rel in rels {
        let Some(o) = owner_of(rel) else { continue };
        count += 1;
        if !names.contains(&o.name) {
            names.push(o.name);
        }
    }
    if count == 0 {
        return String::new();
    }
    let head = names
        .iter()
        .take(limit)
        .copied()
        .collect::<Vec<_>>()
        .join("、");
    let head = if names.len() > limit {
        format!("{head} 等 {} 个模组", names.len())
    } else {
        head
    };
    format!("{head} 的 {count} 个配置")
}

/// 把某个目录下的文件分成「照常写入」和「智能剔除、保持本地版本」两堆
///
/// 只剔除**玩家本地已经有**的文件：本地没有说明是新装/新加的配置，
/// 写包里的默认值是安全的（也避免了「新装的优化模组没有配置」）。
/// `old_root` 为 None 时一律剔除：宁可少写，不要覆盖。
pub fn split_smart_excluded(
    files: Vec<String>,
    old_root: Option<&Path>,
) -> (Vec<String>, Vec<String>) {
    let mut kept = Vec::new();
    let mut skipped = Vec::new();
    for rel in files {
        if !is_protected(&rel) {
            kept.push(rel);
            continue;
        }
        match old_root {
            None => skipped.push(rel),
            Some(root) => {
                if root.join(&rel).is_file() {
                    skipped.push(rel);
                } else {
                    kept.push(rel);
                }
            }
        }
    }
    (kept, skipped)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_separators() {
        assert_eq!(normalize("Dynamic_FPS"), "dynamicfps");
        assert_eq!(normalize("dynamic-fps"), "dynamicfps");
        assert_eq!(normalize("Dynamic.FPS"), "dynamicfps");
        assert_eq!(normalize("Dynamic FPS"), "dynamicfps");
    }

    #[test]
    fn recognises_common_config_names() {
        assert!(is_protected("config/sodium-options.json"));
        assert!(is_protected("config/embeddium-options.json"));
        assert!(is_protected("config/DistantHorizons.toml"));
        assert!(is_protected("config/modernfix-mixins.properties"));
        assert!(is_protected("config/sodium/quality.json"));
        assert!(is_protected("config/lithium.properties"));
        assert!(is_protected("config/particle_core_config_v1.json"));
        assert!(is_protected("config/BetterClouds-v1.json"));
        assert!(is_protected("config/BBEConfig.json"));
    }

    #[test]
    fn leaves_other_mods_alone() {
        assert!(!is_protected("config/jei/jei.toml"));
        assert!(!is_protected("config/farmersdelight.toml"));
        assert!(!is_protected("config/create-common.toml"));
        assert!(!is_protected("config/quark.json"));
        // 服务端优化不在表里
        assert!(!is_protected("config/servercore.toml"));
    }

    #[test]
    fn candidates_are_ordered() {
        // 先给完整名，再给剥掉尾巴的
        let c = candidates("config/sodium-options.json");
        assert_eq!(c[0], "sodiumoptions");
        assert!(c.contains(&"sodium".to_string()));
        // 子目录优先
        let c = candidates("config/sodium/quality.json");
        assert_eq!(c[0], "sodium");
    }

    #[test]
    fn table_is_sane() {
        let mut seen = std::collections::HashSet::new();
        for o in TABLE {
            assert!(!o.key.is_empty(), "空 key");
            assert!(
                o.key.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-' || c == '_' || c == '.'),
                "key 没规范化：{}", o.key
            );
            assert!(seen.insert(normalize(o.key)), "重复 key：{}", o.key);
            for a in o.aliases {
                assert!(seen.insert(normalize(a)), "别名重复：{}", a);
            }
        }
        assert!(TABLE.len() >= 85, "表太短了，是不是漏了：{}", TABLE.len());
    }

    #[test]
    fn key_renderers_present() {
        for k in ["sodium", "iris", "embeddium", "optifine", "distanthorizons", "voxy"] {
            assert!(owner_of(&format!("config/{k}.json")).is_some(), "缺 {k}");
        }
    }

    #[test]
    fn split_protects_only_existing() {
        let dir = std::env::temp_dir().join("pulses_ce_test");
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("config")).unwrap();
        std::fs::write(dir.join("config/sodium-options.json"), "{}").unwrap();

        let files = vec![
            "config/sodium-options.json".to_string(),  // 本地有 → 剔除
            "config/iris.properties".to_string(),      // 本地没有 → 写入
            "config/jei.toml".to_string(),             // 不保护 → 写入
        ];
        let (kept, skipped) = split_smart_excluded(files, Some(&dir));
        assert_eq!(skipped, vec!["config/sodium-options.json".to_string()]);
        assert_eq!(kept.len(), 2);

        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn summarize_reads_like_a_sentence() {
        let s = summarize(
            ["config/sodium-options.json", "config/iris.properties"],
            3,
        );
        assert!(s.contains("Sodium"), "{s}");
        assert!(s.contains("Iris"), "{s}");
        assert!(s.contains("2 个配置"), "{s}");
    }
}
