// transfer.rs — 把新内容搬进整合包的**唯一**通道
// 对应 Python: app/core/transfer.py
//
// 为什么单独一层：
//
// 1. **用移动，不用复制。** 同卷 rename 是元数据操作，瞬时完成、不产生 IO；
//    只有跨卷才真的需要搬字节。
// 2. **写入必须原子。** 直接写到目标等于打开就截断，进程被杀/断电会在整合包里
//    留下半截文件。这里一律"先写同目录临时名，再 rename 覆盖"，失败不留痕。
// 3. **不要吃满硬盘。** 大文件按块搬，块与块之间让出 CPU。
//
// Windows 注意点：
//   · rename 覆盖被占用的文件会报权限错误，这里翻译成「请先关闭游戏/启动器」
//   · 跨盘是 ERROR_NOT_SAME_DEVICE(17)，等价 POSIX 的 EXDEV
//   · 超过 260 字符的路径会失败，系统调用前统一加 `\\?\` 前缀

use std::fs;
use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

/// 分块搬运的默认块大小
pub const CHUNK_BYTES: usize = 4 * 1024 * 1024;
/// 写入中的临时后缀
pub const NEW_SUFFIX: &str = ".pulses_new";

#[derive(Debug, Clone)]
pub struct TransferResult {
    pub ok: bool,
    /// 是否走了瞬时路径（rename / 硬链接），没真搬字节
    pub moved: bool,
    pub bytes: u64,
    pub digest: String,
    pub error: String,
}

impl TransferResult {
    fn ok(moved: bool) -> Self {
        Self { ok: true, moved, bytes: 0, digest: String::new(), error: String::new() }
    }
    fn ok_bytes(bytes: u64, digest: String) -> Self {
        Self { ok: true, moved: false, bytes, digest, error: String::new() }
    }
    fn fail(msg: impl Into<String>) -> Self {
        Self { ok: false, moved: false, bytes: 0, digest: String::new(), error: msg.into() }
    }
}

/// 搬运参数
pub struct MoveOpts {
    /// true = 源必须保留（下载缓存要留着重试），用硬链接
    pub preserve: bool,
    pub chunk_bytes: usize,
    /// 每块之间暂停（毫秒），主要针对机械盘
    pub pace_ms: u64,
    pub expect_size: u64,
    pub expect_hash: String,
    /// sha1 / sha256
    pub algo: String,
    pub ensure_parent: bool,
}

impl Default for MoveOpts {
    fn default() -> Self {
        Self {
            preserve: false,
            chunk_bytes: CHUNK_BYTES,
            pace_ms: 0,
            expect_size: 0,
            expect_hash: String::new(),
            algo: "sha1".to_string(),
            ensure_parent: true,
        }
    }
}

// ═══════════════════════════════════════════════════════════════
// 平台差异
// ═══════════════════════════════════════════════════════════════

/// Windows 上给长路径加 `\\?\` 前缀，其余平台原样返回
#[cfg(windows)]
fn sys_path(p: &Path) -> PathBuf {
    let s = p.to_string_lossy().to_string();
    if s.starts_with(r"\\?\") || s.len() < 240 {
        return p.to_path_buf();
    }
    if s.starts_with(r"\\") {
        // UNC：\\server\share\... → \\?\UNC\server\share\...
        return PathBuf::from(format!(r"\\?\UNC\{}", &s[2..]));
    }
    PathBuf::from(format!(r"\\?\{}", s))
}

#[cfg(not(windows))]
fn sys_path(p: &Path) -> PathBuf {
    p.to_path_buf()
}

/// 跨设备（跨盘）错误
fn is_cross_device(e: &std::io::Error) -> bool {
    match e.raw_os_error() {
        // POSIX EXDEV
        Some(18) => true,
        // Windows ERROR_NOT_SAME_DEVICE
        Some(17) => true,
        _ => false,
    }
}

/// 把系统错误翻译成人话
pub fn friendly_error(e: &std::io::Error, what: &str) -> String {
    let code = e.raw_os_error();
    let base = if what.is_empty() { String::new() } else { format!("{what}：") };
    match code {
        Some(5) | Some(32) => format!(
            "{base}文件被占用，请先关闭游戏 / 启动器 / 资源管理器预览后重试"
        ),
        Some(28) => format!("{base}磁盘空间不足"),
        Some(112) => format!("{base}磁盘空间不足"),
        Some(206) => format!("{base}路径太长"),
        _ => {
            let s = e.to_string();
            if s.contains("Access is denied") || s.contains("拒绝访问") {
                format!("{base}文件被占用，请先关闭游戏 / 启动器后重试")
            } else {
                format!("{base}{s}")
            }
        }
    }
}

/// 同目录下的临时名
pub fn tmp_name(dst: &Path) -> PathBuf {
    let name = dst
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    dst.with_file_name(format!(".{name}{NEW_SUFFIX}"))
}

/// 搬运/替换过程留下的临时名，不能参与比对
pub fn is_transient_name(name: &str) -> bool {
    name.ends_with(".pulses_new") || name.contains(".pulses_tmp")
}

// ═══════════════════════════════════════════════════════════════
// 摘要
// ═══════════════════════════════════════════════════════════════

fn hex_of(algo: &str, data: &[u8]) -> String {
    if algo.eq_ignore_ascii_case("sha256") {
        use sha2::Digest;
        let mut h = sha2::Sha256::new();
        h.update(data);
        format!("{:x}", h.finalize())
    } else {
        use sha1::Digest;
        let mut h = sha1::Sha1::new();
        h.update(data);
        format!("{:x}", h.finalize())
    }
}

// ═══════════════════════════════════════════════════════════════
// 搬运
// ═══════════════════════════════════════════════════════════════

/// 把 src 搬到 dst。
///
/// 任何情况下目标端要么是**完整的旧文件**，要么是**完整的新文件**。
pub fn move_in(src: &Path, dst: &Path, o: &MoveOpts) -> TransferResult {
    let md = match fs::metadata(src) {
        Ok(m) => m,
        Err(_) => return TransferResult::fail("源文件不存在"),
    };
    if !md.is_file() {
        return TransferResult::fail("源不是文件");
    }
    if o.expect_size > 0 && md.len() != o.expect_size {
        return TransferResult::fail(format!(
            "源文件大小不符（期望 {}，实际 {}）",
            o.expect_size,
            md.len()
        ));
    }

    if o.ensure_parent {
        if let Some(p) = dst.parent() {
            if let Err(e) = fs::create_dir_all(p) {
                return TransferResult::fail(friendly_error(&e, "创建目录失败"));
            }
        }
    }

    // ---- 快路径 1：源可消耗 → 直接 rename ----
    //
    // 要求校验摘要时不能走这条路：rename 是元数据操作，
    // 不会读一个字节，等于跳过校验。
    if !o.preserve && o.expect_hash.is_empty() {
        match fs::rename(sys_path(src), sys_path(dst)) {
            Ok(()) => return TransferResult::ok(true),
            Err(e) => {
                if !is_cross_device(&e) {
                    return TransferResult::fail(friendly_error(&e, "移动失败"));
                }
            }
        }
    }

    let tmp = tmp_name(dst);
    let _ = fs::remove_file(sys_path(&tmp));

    // ---- 快路径 2：源要保留 → 硬链接 + rename ----
    if o.preserve {
        if fs::hard_link(sys_path(src), sys_path(&tmp)).is_ok() {
            match fs::rename(sys_path(&tmp), sys_path(dst)) {
                Ok(()) => return TransferResult::ok(true),
                Err(e) => {
                    let _ = fs::remove_file(sys_path(&tmp));
                    return TransferResult::fail(friendly_error(&e, "写入失败"));
                }
            }
        }
        // 硬链接不支持（跨卷 / FAT32 / 网络盘）→ 落到复制
    }

    // ---- 慢路径 A：不校验、不节流 → 交给系统快速复制 ----
    if o.expect_hash.is_empty() && o.pace_ms == 0 {
        if let Err(e) = fs::copy(sys_path(src), sys_path(&tmp)) {
            let _ = fs::remove_file(sys_path(&tmp));
            return TransferResult::fail(friendly_error(&e, "写入失败"));
        }
        if o.expect_size > 0 {
            let got = fs::metadata(sys_path(&tmp)).map(|m| m.len()).unwrap_or(0);
            if got != o.expect_size {
                let _ = fs::remove_file(sys_path(&tmp));
                return TransferResult::fail(format!(
                    "大小不符（期望 {}，实际 {got}）",
                    o.expect_size
                ));
            }
        }
        if let Err(e) = fs::rename(sys_path(&tmp), sys_path(dst)) {
            let _ = fs::remove_file(sys_path(&tmp));
            return TransferResult::fail(friendly_error(&e, "写入失败"));
        }
        if !o.preserve {
            let _ = fs::remove_file(sys_path(src));
        }
        return TransferResult::ok(false);
    }

    // ---- 慢路径 B：分块复制（要算摘要 / 要按块节流） ----
    let chunk = if o.chunk_bytes == 0 { CHUNK_BYTES } else { o.chunk_bytes };
    let mut fi = match fs::File::open(sys_path(src)) {
        Ok(f) => f,
        Err(e) => return TransferResult::fail(friendly_error(&e, "打开源文件失败")),
    };
    let mut fo = match fs::File::create(sys_path(&tmp)) {
        Ok(f) => f,
        Err(e) => return TransferResult::fail(friendly_error(&e, "创建临时文件失败")),
    };

    let mut buf = vec![0u8; chunk];
    let mut written = 0u64;
    let mut digest_data: Vec<u8> = Vec::new();
    let want_hash = !o.expect_hash.is_empty();

    loop {
        let n = match fi.read(&mut buf) {
            Ok(0) => break,
            Ok(n) => n,
            Err(e) => {
                let _ = fs::remove_file(sys_path(&tmp));
                return TransferResult::fail(friendly_error(&e, "读取失败"));
            }
        };
        if let Err(e) = fo.write_all(&buf[..n]) {
            let _ = fs::remove_file(sys_path(&tmp));
            return TransferResult::fail(friendly_error(&e, "写入失败"));
        }
        written += n as u64;
        if want_hash {
            digest_data.extend_from_slice(&buf[..n]);
        }
        if o.pace_ms > 0 {
            std::thread::sleep(Duration::from_millis(o.pace_ms));
        }
    }
    drop(fo);

    if o.expect_size > 0 && written != o.expect_size {
        let _ = fs::remove_file(sys_path(&tmp));
        return TransferResult::fail(format!(
            "大小不符（期望 {}，实际 {written}）",
            o.expect_size
        ));
    }

    let got_digest = if want_hash {
        let got = hex_of(&o.algo, &digest_data);
        if got.to_lowercase() != o.expect_hash.to_lowercase() {
            let _ = fs::remove_file(sys_path(&tmp));
            return TransferResult::fail("内容校验失败（SHA 不一致）");
        }
        got
    } else {
        String::new()
    };

    // 保留修改时间，尽可能贴近原文件
    if let Ok(m) = fs::metadata(sys_path(src)) {
        if let Ok(t) = m.modified() {
            if let Ok(f) = fs::OpenOptions::new().write(true).open(sys_path(&tmp)) {
                let _ = f.set_modified(t);
            }
        }
    }

    if let Err(e) = fs::rename(sys_path(&tmp), sys_path(dst)) {
        let _ = fs::remove_file(sys_path(&tmp));
        return TransferResult::fail(friendly_error(&e, "写入失败"));
    }
    if !o.preserve {
        let _ = fs::remove_file(sys_path(src));
    }
    TransferResult::ok_bytes(written, got_digest)
}

/// 清掉目录里遗留的临时文件（上次被中断留下的）
pub fn cleanup_transient(target_dir: &Path, max_age: Duration) -> usize {
    let mut n = 0usize;
    let rd = match fs::read_dir(target_dir) {
        Ok(r) => r,
        Err(_) => return 0,
    };
    let now = SystemTime::now();
    for e in rd.flatten() {
        let name = e.file_name().to_string_lossy().to_string();
        if !is_transient_name(&name) {
            continue;
        }
        let p = e.path();
        let old = fs::metadata(&p)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| now.duration_since(t).ok())
            .map(|d| d >= max_age)
            .unwrap_or(true);
        if old && fs::remove_file(&p).is_ok() {
            n += 1;
        }
    }
    n
}

/// 1.2 GB / 356 MB / 12 KB
pub fn human_bytes(n: u64) -> String {
    const G: u64 = 1024 * 1024 * 1024;
    const M: u64 = 1024 * 1024;
    const K: u64 = 1024;
    if n >= G {
        format!("{:.1} GB", n as f64 / G as f64)
    } else if n >= M {
        format!("{:.0} MB", n as f64 / M as f64)
    } else if n >= K {
        format!("{:.0} KB", n as f64 / K as f64)
    } else {
        format!("{n} B")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("pulses_tr_{name}"));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn same_disk_move_is_instant_and_consumes_source() {
        let d = tmpdir("same");
        let src = d.join("a.bin");
        let dst = d.join("sub/b.bin");
        fs::write(&src, b"hello world").unwrap();
        let r = move_in(&src, &dst, &MoveOpts::default());
        assert!(r.ok, "{}", r.error);
        assert!(r.moved, "同卷应该走 rename");
        assert!(!src.exists(), "源应该被消耗");
        assert_eq!(fs::read(&dst).unwrap(), b"hello world");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn preserve_keeps_source() {
        let d = tmpdir("preserve");
        let src = d.join("a.bin");
        let dst = d.join("b.bin");
        fs::write(&src, b"keep me").unwrap();
        let mut o = MoveOpts::default();
        o.preserve = true;
        let r = move_in(&src, &dst, &o);
        assert!(r.ok, "{}", r.error);
        assert!(src.exists(), "preserve 时源必须保留");
        assert_eq!(fs::read(&dst).unwrap(), b"keep me");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn hash_mismatch_leaves_no_file() {
        let d = tmpdir("hash");
        let src = d.join("a.bin");
        let dst = d.join("b.bin");
        fs::write(&src, b"payload").unwrap();
        let mut o = MoveOpts::default();
        o.expect_hash = "sha1:0000000000000000000000000000000000000000".to_string();
        let r = move_in(&src, &dst, &o);
        assert!(!r.ok);
        assert!(!dst.exists(), "校验失败不能留下目标文件");
        assert!(!tmp_name(&dst).exists(), "临时文件要清理掉");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn chunked_copy_with_sha1() {
        let d = tmpdir("chunk");
        let src = d.join("a.bin");
        let dst = d.join("b.bin");
        let data: Vec<u8> = (0..500_000u32).map(|i| (i % 251) as u8).collect();
        fs::write(&src, &data).unwrap();
        let want = {
            use sha1::Digest;
            let mut h = sha1::Sha1::new();
            h.update(&data);
            format!("{:x}", h.finalize())
        };
        let mut o = MoveOpts::default();
        o.chunk_bytes = 64 * 1024;
        o.expect_size = data.len() as u64;
        o.expect_hash = want.clone();
        o.algo = "sha1".into();
        let r = move_in(&src, &dst, &o);
        assert!(r.ok, "{}", r.error);
        assert_eq!(r.bytes, data.len() as u64);
        assert_eq!(r.digest, want);
        assert_eq!(fs::read(&dst).unwrap(), data);
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn size_mismatch_rejected_before_touching_target() {
        let d = tmpdir("size");
        let src = d.join("a.bin");
        let dst = d.join("b.bin");
        fs::write(&src, b"12345").unwrap();
        fs::write(&dst, b"old").unwrap();
        let mut o = MoveOpts::default();
        o.expect_size = 99;
        let r = move_in(&src, &dst, &o);
        assert!(!r.ok);
        assert_eq!(fs::read(&dst).unwrap(), b"old", "旧文件不能被破坏");
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn cleanup_removes_transient_files() {
        let d = tmpdir("cleanup");
        fs::write(d.join("keep.bin"), b"x").unwrap();
        fs::write(d.join(".a.bin.pulses_new"), b"x").unwrap();
        fs::write(d.join(".b.bin.pulses_new"), b"x").unwrap();
        let n = cleanup_transient(&d, Duration::from_secs(0));
        assert_eq!(n, 2);
        assert!(d.join("keep.bin").exists());
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn human_bytes_reads_right() {
        assert_eq!(human_bytes(512), "512 B");
        assert_eq!(human_bytes(2048), "2 KB");
        assert_eq!(human_bytes(3 * 1024 * 1024), "3 MB");
        assert_eq!(human_bytes(2 * 1024 * 1024 * 1024), "2.0 GB");
    }

    #[test]
    fn transient_names_recognised() {
        assert!(is_transient_name(".a.pulses_new"));
        assert!(is_transient_name("x.pulses_tmp.part"));
        assert!(!is_transient_name("a.jar"));
    }
}
