// installer.js — 安装整合包（把 CF / MR 整合包变成 PCL 能直接扫到的实例）
//
// 流程：
//   选游戏目录 → 选整合包 zip → 解压识别 → 确认信息 → 装游戏本体+加载器
//
// 产出（PCL 版本隔离的标准布局，PCL 一扫描就能看到）：
//   <游戏目录>/versions/<实例名>/<实例名>.json + mods/ config/ …
//   <游戏目录>/libraries/  assets/
//
// Java 只在装 Forge / NeoForge 时需要（安装器是 Java 程序）。
// Fabric / Quilt 官方直接给完整 profile，不需要 Java。

const Installer = {
  state: {
    root: null,
    zip: null,
    extracted: null,
    content: null,
    meta: null,
    busy: false,
  },

  // 入口：侧栏「安装整合包」
  async open() {
    if (this.state.busy) {
      Log.warn('正在安装中，请稍候');
      return;
    }

    // 1) 游戏目录
    const root = await window.pulses.dialog.openFolder('选择游戏目录（.minecraft 那一层）');
    if (!root) return;

    // 2) 整合包
    const zip = await window.pulses.dialog.openFile('选择整合包', [
      { name: '整合包', extensions: ['zip'] },
    ]);
    if (!zip) return;

    // 3) 解压
    App.setStatus('正在解压整合包…');
    const out = zip.replace(/\.[^.]+$/, '') + '_pulses_extract';
    try { await window.pulses.pack.cleanExtract(out); } catch (_) {}
    const unzip = await window.pulses.files.unzip(zip, out);
    if (!unzip || !unzip.ok) {
      await Overlay.alert('解压失败', (unzip && unzip.msg) || '未知错误', { level: 'error' });
      return;
    }
    const base = await unwrapPackRoot(out);

    // 内容目录：CF 的 overrides/、MMC 的 .minecraft/、MR 的 overrides/
    let content = base;
    for (const sub of ['overrides', '.minecraft', 'minecraft']) {
      const p = joinPath(base, sub);
      let st = null;
      try { st = await window.pulses.files.stat(p); } catch (_) {}
      if (st && st.isDir) { content = p; break; }
    }

    // 4) 识别
    let meta = { ok: false };
    try { meta = await window.pulses.mc.packMeta(base) || meta; } catch (_) {}

    this.state = {
      root, zip, extracted: base, content, meta,
      busy: false,
    };

    await this._panel();
  },

  // ── 配置面板 ──
  async _panel() {
    const st = this.state;
    const meta = st.meta || {};

    // 版本清单（认不出来才要用户自己选）
    let releases = [];
    if (!meta.ok) {
      App.setStatus('正在获取版本列表…');
      try {
        const r = await window.pulses.mc.versions(true);
        releases = (r && r.versions) || [];
      } catch (_) {}
    }

    const instName = sanitizeInstance(meta.name || baseName(st.extracted || st.zip));

    await Overlay.panel('安装整合包', (body, done) => {
      const card = body.closest('.ov-card');
      const footBtn = card && card.querySelector('.ov-foot .ov-btn');
      this._done = done;

      // 概要
      const sum = el('div', 'ins-sum');
      sum.appendChild(el('div', 'ins-title', instName));
      sum.appendChild(el('div', 'ins-sub', baseName(st.zip)));
      body.appendChild(sum);

      // 版本
      let mcVal = meta.mc || '';
      if (meta.ok) {
        body.appendChild(infoRow('版本', mcVal || '未知'));
      } else {
        const sel = document.createElement('select');
        sel.className = 'ins-select';
        for (const v of releases) {
          const o = document.createElement('option');
          o.value = v.id;
          o.textContent = v.id;
          sel.appendChild(o);
        }
        if (releases.length) mcVal = releases[0].id;
        body.appendChild(fieldRow('版本', sel, '没认出整合包格式，请手动选'));
        sel.addEventListener('change', () => { mcVal = sel.value; refreshJava(); });
      }

      // 加载器
      let loader = meta.loader || (meta.ok ? 'vanilla' : 'forge');
      let loaderVer = meta.loaderVer || '';

      if (meta.ok) {
        body.appendChild(infoRow('加载器', loaderLabel(loader, loaderVer)));
      } else {
        const sel = document.createElement('select');
        sel.className = 'ins-select';
        for (const [v, t] of [
          ['forge', 'Forge'], ['neoforge', 'NeoForge'],
          ['fabric', 'Fabric'], ['quilt', 'Quilt'], ['vanilla', '原版'],
        ]) {
          const o = document.createElement('option');
          o.value = v; o.textContent = t;
          if (v === loader) o.selected = true;
          sel.appendChild(o);
        }
        const inp = document.createElement('input');
        inp.className = 'ov-input ins-ver';
        inp.placeholder = '加载器版本（留空取最新）';
        inp.value = loaderVer;

        const wrap = el('div', 'ins-inline');
        wrap.appendChild(sel);
        wrap.appendChild(inp);
        body.appendChild(fieldRow('加载器', wrap, ''));

        sel.addEventListener('change', () => { loader = sel.value; refreshJava(); });
        inp.addEventListener('input', () => { loaderVer = inp.value.trim(); });
      }

      // 实例名
      const nameInp = document.createElement('input');
      nameInp.className = 'ov-input';
      nameInp.value = instName;
      nameInp.placeholder = '实例名（PCL 里显示的名字）';
      body.appendChild(fieldRow('实例名', nameInp, ''));

      // Java 状态
      const javaRow = el('div', 'pref-row');
      const javaMain = el('div', 'pref-main');
      javaMain.appendChild(el('div', 'pref-label', 'Java'));
      javaRow.appendChild(javaMain);
      const javaVal = el('div', 'pref-value', '检查中…');
      javaRow.appendChild(javaVal);
      const javaBtn = document.createElement('button');
      javaBtn.className = 'pref-btn is-hidden';
      javaBtn.textContent = '自动安装';
      javaRow.appendChild(javaBtn);
      body.appendChild(javaRow);

      // 进度 + 开始
      const prog = el('div', 'progress is-hidden');
      const track = el('div', 'progress-track');
      const fill = el('i', 'progress-fill');
      track.appendChild(fill);
      const ptext = el('div', 'progress-text');
      prog.appendChild(track);
      prog.appendChild(ptext);
      body.appendChild(prog);

      const start = document.createElement('button');
      start.className = 'cta ins-go';
      start.textContent = '开始安装';
      body.appendChild(start);

      const err = el('div', 'ov-err');
      body.appendChild(err);

      // ── 状态 ──
      let javaPath = '';
      let needMajor = 0;
      // 找 Java 要起子进程（每份 100~300ms），本机装了七八份时很慢，
      // 所以整块面板只查一次，装完新 Java 再往缓存里塞
      let javaList = null;
      const getJavaList = async () => {
        if (javaList) return javaList;
        try { javaList = (await window.pulses.mc.javaList()).list || []; }
        catch (_) { javaList = []; }
        return javaList;
      };

      const refreshJava = async () => {
        javaBtn.classList.add('is-hidden');
        javaPath = '';
        if (loader === 'vanilla' || loader === 'fabric' || loader === 'quilt') {
          javaVal.textContent = '不需要（这个加载器免安装 Java）';
          javaVal.classList.remove('is-bad');
          return;
        }
        if (!mcVal) {
          javaVal.textContent = '先选版本';
          return;
        }
        try {
          needMajor = (await window.pulses.mc.javaNeed(mcVal)).major || 17;
        } catch (_) { needMajor = 17; }

        const list = await getJavaList();

        // 刚好够用的最低版本优先：老 Forge 安装器在 Java 21 上会挂
        const ok = list.filter(j => j.major >= needMajor).sort((a, b) => a.major - b.major)[0];
        if (ok) {
          javaPath = ok.path;
          javaVal.textContent = `需要 ${needMajor} · 已找到 Java ${ok.major}`;
          javaVal.classList.remove('is-bad');
        } else if (list.length) {
          javaVal.textContent = `需要 ${needMajor} · 本机只有 Java ${list[0].major}`;
          javaVal.classList.add('is-bad');
          javaBtn.classList.remove('is-hidden');
          javaBtn.textContent = `安装 Java ${needMajor}`;
        } else {
          javaVal.textContent = `需要 ${needMajor} · 没找到 Java`;
          javaVal.classList.add('is-bad');
          javaBtn.classList.remove('is-hidden');
          javaBtn.textContent = `安装 Java ${needMajor}`;
        }
      };

      javaBtn.addEventListener('click', async () => {
        javaBtn.disabled = true;
        javaBtn.textContent = '下载中…';
        javaVal.textContent = `正在安装 Java ${needMajor}（约 200 MB，请稍候）`;
        javaVal.classList.remove('is-bad');
        try {
          const r = await window.pulses.mc.javaInstall(needMajor);
          if (r && r.ok) {
            javaPath = r.java.path;
            if (javaList) javaList.push(r.java);
            javaVal.textContent = `Java ${r.java.major} 已就绪`;
            javaBtn.classList.add('is-hidden');
            Log.ok(`已安装 Java ${r.java.major}：${r.java.path}`);
          } else {
            javaVal.textContent = '自动安装失败';
            javaVal.classList.add('is-bad');
            err.textContent = (r && r.error) || '未知错误';
          }
        } catch (e) {
          javaVal.textContent = '自动安装失败';
          javaVal.classList.add('is-bad');
          err.textContent = String(e);
        }
        javaBtn.disabled = false;
      });

      // 初次检查
      refreshJava();

      // ── 开始 ──
      start.addEventListener('click', async () => {
        if (this.state.busy) return;
        err.textContent = '';

        const instance = sanitizeInstance(nameInp.value.trim() || instName);
        if (!mcVal) { err.textContent = '请先选版本'; return; }
        if ((loader === 'forge' || loader === 'neoforge') && !javaPath) {
          err.textContent = `装 ${loaderLabel(loader, '')} 需要 Java ${needMajor}，请先安装`;
          return;
        }

        this.state.busy = true;
        start.disabled = true;
        start.textContent = '安装中…';
        if (footBtn) footBtn.disabled = true;
        prog.classList.remove('is-hidden');

        window.pulses.mc.onProgress(d => {
          if (!d || !d.stage) return;
          if (d.total > 0) {
            const pct = (d.done / d.total) * 100;
            fill.style.width = pct.toFixed(1) + '%';
            const mb = (d.totalBytes || 0) / 1048576;
            ptext.textContent = `${d.stage}  ${d.done}/${d.total}` +
              (mb > 0 ? `  ·  ${((d.bytes || 0) / 1048576).toFixed(1)}/${mb.toFixed(1)} MB` : '');
          } else {
            ptext.textContent = d.stage;
          }
        });

        try {
          const res = await window.pulses.mc.install({
            root: this.state.root,
            mc: mcVal,
            loader,
            loaderVer,
            instance,
            mirror: true,
            java: javaPath,
            content: this.state.content || '',
          });

          this.state.busy = false;
          if (footBtn) footBtn.disabled = false;

          if (res && res.ok) {
            fill.style.width = '100%';
            ptext.textContent = '安装完成 ✓';
            start.textContent = '已完成';
            App.setStatus('整合包已装好：' + instance);
            Log.ok(`安装完成：${instance}（${res.files} 个文件）`);
            Log.debug('位置：' + res.dir);
            // 装完直接定位过去，玩家端可以接着打更新包
            if (res.dir && window.Player) {
              try { await Player.setModpack(res.dir); } catch (_) {}
            }
            setTimeout(() => {
              this._cleanup();
              done(res);
            }, 1400);
          } else {
            const msg = (res && res.error) || '安装失败';
            ptext.textContent = '安装失败';
            err.textContent = msg;
            start.disabled = false;
            start.textContent = '重试';
            Log.error('安装失败：' + msg);
          }
        } catch (e) {
          this.state.busy = false;
          if (footBtn) footBtn.disabled = false;
          err.textContent = String(e);
          ptext.textContent = '安装失败';
          start.disabled = false;
          start.textContent = '重试';
          Log.error('安装异常：' + e);
        }
      });
    });

    this._cleanup();
  },

  // 解压目录用完就删
  async _cleanup() {
    const st = this.state;
    if (st && st.extracted) {
      try { await window.pulses.pack.cleanExtract(st.extracted); } catch (_) {}
    }
    if (this.state) {
      this.state.extracted = null;
      this.state.content = null;
    }
  },
};

// ── 小工具 ──

function el(tag, cls, text) {
  const e = document.createElement(tag);
  if (cls) e.className = cls;
  if (text != null) e.textContent = text;
  return e;
}

function infoRow(label, value) {
  const row = el('div', 'pref-row');
  const main = el('div', 'pref-main');
  main.appendChild(el('div', 'pref-label', label));
  row.appendChild(main);
  const v = el('div', 'pref-value', value);
  v.title = value;
  row.appendChild(v);
  return row;
}

function fieldRow(label, control, hint) {
  const row = el('div', 'pref-row pref-row-field');
  const main = el('div', 'pref-main');
  main.appendChild(el('div', 'pref-label', label));
  if (hint) main.appendChild(el('div', 'pref-hint', hint));
  row.appendChild(main);
  control.classList.add('ins-control');
  row.appendChild(control);
  return row;
}

function loaderLabel(loader, ver) {
  const names = {
    vanilla: '原版', fabric: 'Fabric', quilt: 'Quilt',
    forge: 'Forge', neoforge: 'NeoForge',
  };
  const n = names[loader] || loader;
  return ver ? `${n} ${ver}` : n;
}

// 实例名：去掉 Windows 不允许的字符，兜底用 modpack
function sanitizeInstance(s) {
  let t = String(s || '').replace(/[\\/:*?"<>|]/g, '_').replace(/[\x00-\x1f]/g, '');
  t = t.trim().replace(/\.+$/, '');
  return t || 'modpack';
}

window.Installer = Installer;
window.sanitizeInstance = sanitizeInstance;
window.loaderLabel = loaderLabel;
