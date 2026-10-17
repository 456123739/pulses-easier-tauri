// installer.js — 安装整合包（把 CF / MR 整合包变成 PCL 能直接扫到的实例）
//
// 能不问就不问：
//   · 游戏目录 —— 记住上次用的；没有就从已定位的整合包路径推（.../versions/X → ...）
//   · 版本 / 加载器 —— 直接从 zip 里的 manifest.json / mmc-pack.json 读，不解压
//   · 实例名 —— 取整合包自带的名字，没有就用 zip 文件名
//   · Java —— 只在 Forge / NeoForge 时需要，本机有就直接用
// 真正需要玩家点头的只有两件事：装不装、装不装 Java。
//
// 产出（PCL 版本隔离的标准布局，PCL 一扫描就能看到）：
//   <游戏目录>/versions/<实例名>/<实例名>.json + mods/ config/ …
//   <游戏目录>/libraries/  assets/

const MCRootKey = 'pulses.mcRoot';

const Installer = {
  state: {
    root: null,
    zip: null,
    meta: null,
    busy: false,
  },

  // 侧栏入口：只问整合包，其余能推就推
  async open() {
    if (this.state.busy) {
      Log.warn('正在安装中，请稍候');
      return;
    }
    const zip = await window.pulses.dialog.openFile('选择整合包', [
      { name: '整合包', extensions: ['zip'] },
    ]);
    if (!zip) return;

    let kind = null;
    try { kind = await window.pulses.pack.kind(zip); } catch (_) {}
    await this.startWith(zip, kind);
  },

  // 已经知道 zip 了（拖进来的 / 别处识别出来的）
  async startWith(zip, kind) {
    if (this.state.busy) {
      Log.warn('正在安装中，请稍候');
      return;
    }

    // 游戏目录：记住的 → 从已定位整合包推 → 实在没有才问
    let root = guessGameDir();
    if (!root) {
      root = await window.pulses.dialog.openFolder('选择游戏目录（.minecraft 那一层）');
      if (!root) return;
    }

    this.state = { root, zip, meta: kind || {}, busy: false };
    await this._panel();
  },

  // ── 配置面板 ──
  async _panel() {
    const st = this.state;
    const meta = st.meta || {};
    const known = !!(meta && meta.ok && meta.mc);

    // 认不出来才需要版本列表
    let releases = [];
    if (!known) {
      App.setStatus('正在获取版本列表…');
      try {
        const r = await window.pulses.mc.versions(true);
        releases = (r && r.versions) || [];
      } catch (_) {}
      App.setStatus('就绪');
    }

    const instName = sanitizeInstance(meta.name || zipName(st.zip));

    await Overlay.panel('安装整合包', (body, done) => {
      const card = body.closest('.ov-card');
      const footBtn = card && card.querySelector('.ov-foot .ov-btn');

      // 概要
      const sum = el('div', 'ins-sum');
      sum.appendChild(el('div', 'ins-title', instName));
      sum.appendChild(el('div', 'ins-sub',
        `${formatLabel(meta.format)} · ${baseName(st.zip)}`));
      body.appendChild(sum);

      // 版本
      let mcVal = known ? meta.mc : '';
      if (known) {
        body.appendChild(infoRow('版本', mcVal));
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
        body.appendChild(fieldRow('版本', sel, '请手动选'));
        sel.addEventListener('change', () => { mcVal = sel.value; refreshJava(); });
      }

      // 加载器
      let loader = meta.loader || (known ? 'vanilla' : 'forge');
      let loaderVer = meta.loaderVer || '';

      if (known) {
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
        inp.placeholder = '版本';
        inp.value = loaderVer;
        inp.title = '加载器版本，留空自动取最新';

        const wrap = el('div', 'ins-inline');
        wrap.appendChild(sel);
        wrap.appendChild(inp);
        body.appendChild(fieldRow('加载器', wrap, '版本可留空'));

        sel.addEventListener('change', () => { loader = sel.value; refreshJava(); });
        inp.addEventListener('input', () => { loaderVer = inp.value.trim(); });
      }

      // 游戏目录（只展示，不占一次询问）
      body.appendChild(infoRow('游戏目录', st.root));

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

        // 解压整合包 → 找内容目录
        let out = '';
        try {
          ptext.textContent = '正在解压整合包…';
          out = st.zip.replace(/\.[^.]+$/, '') + '_pulses_extract';
          try { await window.pulses.pack.cleanExtract(out); } catch (_) {}
          const unzip = await window.pulses.files.unzip(st.zip, out);
          if (!unzip || !unzip.ok) {
            throw new Error((unzip && unzip.msg) || '解压失败');
          }
        } catch (e) {
          this.state.busy = false;
          if (footBtn) footBtn.disabled = false;
          ptext.textContent = '安装失败';
          err.textContent = String(e && e.message ? e.message : e);
          start.disabled = false;
          start.textContent = '重试';
          Log.error('解压失败：' + e);
          return;
        }

        try {
          // 交给 Rust 摊平：overrides/ 和 .minecraft/ 的内容会直接落到实例根，
          // 根目录的 mods/ 也一并并进去。前端只负责剥掉多套的那一层。
          let content = out;
          try { content = await unwrapPackRoot(out); } catch (_) {}

          const res = await window.pulses.mc.install({
            root: st.root,
            mc: mcVal,
            loader,
            loaderVer,
            instance,
            mirror: true,
            java: javaPath,
            content: content || '',
          });

          this.state.busy = false;
          if (footBtn) footBtn.disabled = false;

          if (res && res.ok) {
            fill.style.width = '100%';
            ptext.textContent = '安装完成 ✓';
            start.textContent = '已完成';
            App.setStatus('整合包已装好：' + instance);
            Log.ok(`安装完成：${instance}（下载 ${res.files} 个文件，写入整合包 ${res.moved} 个，模组 ${res.mods || 0} 个）`);
            Log.debug('位置：' + res.dir);
            // 官方 CF 导出包只有 overrides，模组不在包里 —— 装完是空的，得说清楚
            if (!res.mods && (meta.format === 'curseforge' || meta.format === 'modrinth')) {
              Log.warn('这个整合包里没有模组文件，模组列表会是空的');
              Log.warn('官方 CurseForge 导出包通常不含模组，需要用 PCL 导入补全');
            }
            try { localStorage.setItem(MCRootKey, st.root); } catch (_) {}
            // 装完直接定位过去，玩家端可以接着打更新包
            if (res.dir && window.Player) {
              try { await Player.setModpack(res.dir); } catch (_) {}
            }
            setTimeout(() => {
              this._cleanup(out);
              done(res);
            }, 1400);
          } else {
            const msg = (res && res.error) || '安装失败';
            ptext.textContent = '安装失败';
            err.textContent = msg;
            start.disabled = false;
            start.textContent = '重试';
            Log.error('安装失败：' + msg);
            this._cleanup(out);
          }
        } catch (e) {
          this.state.busy = false;
          if (footBtn) footBtn.disabled = false;
          err.textContent = String(e);
          ptext.textContent = '安装失败';
          start.disabled = false;
          start.textContent = '重试';
          Log.error('安装异常：' + e);
          this._cleanup(out);
        }
      });
    });
  },

  // 解压目录用完就删
  async _cleanup(out) {
    if (!out) return;
    try { await window.pulses.pack.cleanExtract(out); } catch (_) {}
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

function formatLabel(fmt) {
  const names = {
    curseforge: 'CurseForge 整合包',
    modrinth: 'Modrinth 整合包',
    prism: 'Prism 整合包',
    pulses: 'Pulses 更新包',
    unknown: '整合包',
  };
  return names[fmt] || '整合包';
}

// 从 zip 文件名里取个能当实例名的名字
function zipName(p) {
  let n = baseName(String(p || ''));
  n = n.replace(/\.(zip|eapack)$/i, '');
  n = n.replace(/_pulses_extract$/i, '');
  return n;
}

// 实例名：去掉 Windows 不允许的字符，兜底用 modpack
function sanitizeInstance(s) {
  let t = String(s || '').replace(/[\\/:*?"<>|]/g, '_').replace(/[\x00-\x1f]/g, '');
  t = t.trim().replace(/\.+$/, '');
  return t || 'modpack';
}

// 猜游戏目录：记住的 → 从已定位整合包推（.../versions/<名字> → ...）
function guessGameDir() {
  try {
    const saved = localStorage.getItem(MCRootKey);
    if (saved) return saved;
  } catch (_) {}
  const mp = (window.Player && Player.state && Player.state.modpackPath) || '';
  const norm = String(mp).replace(/\\/g, '/').replace(/\/+$/, '');
  const m = norm.match(/^(.*)\/versions\/[^/]+$/i);
  if (m && m[1]) return m[1];
  return '';
}

window.Installer = Installer;
window.sanitizeInstance = sanitizeInstance;
window.loaderLabel = loaderLabel;
window.zipName = zipName;
window.guessGameDir = guessGameDir;
window.formatLabel = formatLabel;
