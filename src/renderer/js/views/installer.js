// installer.js — 安装整合包
//
// 不是弹窗，是玩家端里的一个整屏状态：弹窗会盖住标题栏，窗口直接拖不动。
//
// 能不问就不问：
//   游戏目录 —— 记住上次的，没有就从已定位的整合包路径推
//   版本 / 加载器 —— 直接从 zip 里读，不解压
//   版本名 —— 取整合包自带的名字，没有就用 zip 文件名
//   Java —— 缺了就在点「开始安装」时自动下，装完接着往下走
//
// 产出是 PCL 版本隔离的标准布局，PCL 一扫描就能看到。

const MCRootKey = 'pulses.mcRoot';

const Installer = {
  state: {
    root: null,
    zip: null,
    meta: null,
    busy: false,
  },

  // 侧栏入口：只问整合包
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

  // 已经知道 zip 了，拖进来的就走这里
  async startWith(zip, kind) {
    if (this.state.busy) {
      Log.warn('正在安装中，请稍候');
      return;
    }

    let root = guessGameDir();
    if (!root) {
      root = await window.pulses.dialog.openFolder('选择游戏目录，就是 .minecraft 那一层');
      if (!root) return;
    }

    this.state = { root, zip, meta: kind || {}, busy: false };
    await this.show();
  },

  // ── 界面 ──
  async show() {
    const st = this.state;
    const meta = st.meta || {};
    const known = !!(meta && meta.ok && meta.mc);

    const view = document.getElementById('install-view');
    const pv = document.getElementById('player-view');
    if (!view || !pv) return;

    pv.classList.add('is-installing');
    view.classList.remove('is-hidden');

    const kicker = document.getElementById('ins-kicker');
    const name = document.getElementById('ins-name');
    const metaEl = document.getElementById('ins-meta');
    const input = document.getElementById('ins-instance');
    const hint = document.getElementById('ins-hint');
    const bar = document.getElementById('ins-bar');
    const fill = document.getElementById('ins-fill');
    const line = document.getElementById('ins-line');
    const go = document.getElementById('ins-go');
    const back = document.getElementById('ins-back');
    const detect = document.getElementById('ins-detect');

    const instName = sanitizeInstance(meta.name || zipName(st.zip));

    kicker.textContent = formatLabel(meta.format);
    name.textContent = meta.name || zipName(st.zip);
    metaEl.classList.remove('is-hidden');
    metaEl.textContent = known
      ? `${loaderLabel(meta.loader, meta.loaderVer)} · Minecraft ${meta.mc} · ${st.root}`
      : st.root;
    input.value = instName;

    bar.classList.add('is-hidden');
    fill.style.width = '0%';
    line.textContent = '';
    go.disabled = false;
    go.textContent = '开始安装';
    back.classList.remove('is-hidden');
    input.disabled = false;
    detect.classList.add('is-hidden');
    hint.classList.remove('is-bad');

    // 认不出整合包信息时才让人选
    let mcVal = known ? meta.mc : '';
    let loader = meta.loader || 'forge';
    let loaderVer = meta.loaderVer || '';

    if (!known) {
      const sel = document.getElementById('ins-mc');
      const lsel = document.getElementById('ins-loader');
      const lver = document.getElementById('ins-loaderver');
      detect.classList.remove('is-hidden');
      metaEl.classList.add('is-hidden');
      if (sel) sel.innerHTML = '';
      if (lver) lver.value = '';
      if (lsel) lsel.value = loader;

      let releases = [];
      try {
        const r = await window.pulses.mc.versions(true);
        releases = (r && r.versions) || [];
      } catch (_) {}
      for (const v of releases) {
        const o = document.createElement('option');
        o.value = v.id;
        o.textContent = v.id;
        if (sel) sel.appendChild(o);
      }
      if (releases.length) mcVal = releases[0].id;

      if (sel) sel.onchange = () => { mcVal = sel.value; refreshHint(); };
      if (lsel) lsel.onchange = () => { loader = lsel.value; refreshHint(); };
      if (lver) lver.oninput = () => { loaderVer = lver.value.trim(); };
    }

    // ── 状态提示 ──
    let javaList = null;
    const getJavaList = async () => {
      if (javaList) return javaList;
      try { javaList = (await window.pulses.mc.javaList()).list || []; }
      catch (_) { javaList = []; }
      return javaList;
    };

    let needMajor = 0;
    const refreshHint = async () => {
      hint.classList.remove('is-bad');
      if (!mcVal) {
        hint.textContent = '先选个版本';
        hint.classList.add('is-bad');
        return;
      }
      if (loader === 'vanilla' || loader === 'fabric' || loader === 'quilt') {
        hint.textContent = '这个加载器不需要 Java';
        return;
      }
      try { needMajor = (await window.pulses.mc.javaNeed(mcVal)).major || 17; }
      catch (_) { needMajor = 17; }

      const list = await getJavaList();
      const ok = list.filter(j => j.major >= needMajor).sort((a, b) => a.major - b.major)[0];
      hint.textContent = ok
        ? `Java ${ok.major} 已就绪`
        : `需要 Java ${needMajor}，点开始安装会自动下载`;
    };
    refreshHint();

    // ── 开始 ──
    const setBusy = (b) => {
      this.state.busy = b;
      go.disabled = b;
      input.disabled = b;
      back.classList.toggle('is-hidden', b);
    };

    go.onclick = async () => {
      if (this.state.busy) return;
      if (!mcVal) {
        hint.textContent = '先选个版本';
        hint.classList.add('is-bad');
        return;
      }

      const instance = sanitizeInstance(input.value.trim() || instName);
      setBusy(true);
      go.textContent = '安装中…';
      bar.classList.remove('is-hidden');
      hint.classList.remove('is-bad');
      hint.textContent = '';

      let out = '';
      try {
        // 1. Java：缺就装，装完接着走
        let javaPath = '';
        if (loader === 'forge' || loader === 'neoforge') {
          const list = await getJavaList();
          const ok = list.filter(j => j.major >= needMajor).sort((a, b) => a.major - b.major)[0];
          if (ok) {
            javaPath = ok.path;
          } else {
            line.textContent = `正在安装 Java ${needMajor}，约 200 MB`;
            const r = await window.pulses.mc.javaInstall(needMajor);
            if (!r || !r.ok) throw new Error((r && r.error) || 'Java 安装失败');
            javaPath = r.java.path;
            if (javaList) javaList.push(r.java);
            Log.ok(`已安装 Java ${r.java.major}`);
          }
        }

        // 2. 解压
        line.textContent = '正在解压整合包';
        out = st.zip.replace(/\.[^.]+$/, '') + '_pulses_extract';
        try { await window.pulses.pack.cleanExtract(out); } catch (_) {}
        const unzip = await window.pulses.files.unzip(st.zip, out);
        if (!unzip || !unzip.ok) throw new Error((unzip && unzip.msg) || '解压失败');

        let content = out;
        try { content = await unwrapPackRoot(out); } catch (_) {}

        // 3. 装
        window.pulses.mc.onProgress(d => {
          if (!d || !d.stage) return;
          const stage = String(d.stage).replace(/[…\.\s]+$/, '');
          if (d.total > 0) {
            fill.style.width = ((d.done / d.total) * 100).toFixed(1) + '%';
            const pct = Math.round((d.done / d.total) * 100);
            let t = `${stage} · ${pct}%`;
            if (d.totalBytes > 0) t += ` · ${fmtSize(d.bytes)} / ${fmtSize(d.totalBytes)}`;
            line.textContent = t;
          } else {
            line.textContent = stage;
          }
        });

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

        if (!res || !res.ok) throw new Error((res && res.error) || '安装失败');

        fill.style.width = '100%';
        line.textContent = `完成 · 写入 ${res.moved} 个文件 · 模组 ${res.mods || 0} 个`;
        go.textContent = '完成';
        App.setStatus('整合包已装好：' + instance);
        Log.ok(`安装完成：${instance}`);
        Log.debug('位置：' + res.dir);
        try { localStorage.setItem(MCRootKey, st.root); } catch (_) {}

        if (!res.mods && (meta.format === 'curseforge' || meta.format === 'modrinth')) {
          Log.warn('这个整合包里没有模组文件，模组列表会是空的');
        }

        if (res.dir && window.Player) {
          try { await Player.setModpack(res.dir); } catch (_) {}
        }

        await this._cleanup(out);
        setTimeout(() => this.hide(), 1600);
      } catch (e) {
        const msg = String(e && e.message ? e.message : e);
        hint.textContent = msg;
        hint.classList.add('is-bad');
        line.textContent = '';
        go.textContent = '重试';
        Log.error('安装失败：' + msg);
        await this._cleanup(out);
        setBusy(false);
      }
    };

    back.onclick = () => {
      if (this.state.busy) return;
      this.hide();
    };
  },

  hide() {
    const view = document.getElementById('install-view');
    const pv = document.getElementById('player-view');
    if (view) view.classList.add('is-hidden');
    if (pv) pv.classList.remove('is-installing');
    this.state.busy = false;
  },

  async _cleanup(out) {
    if (!out) return;
    try { await window.pulses.pack.cleanExtract(out); } catch (_) {}
  },
};

// ── 小工具 ──

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

function fmtSize(n) {
  const b = Number(n) || 0;
  if (b >= 1073741824) return (b / 1073741824).toFixed(1) + ' GB';
  if (b >= 1048576) return (b / 1048576).toFixed(0) + ' MB';
  return (b / 1024).toFixed(0) + ' KB';
}

// 从 zip 文件名里取个能当版本名的名字
function zipName(p) {
  let n = baseName(String(p || ''));
  n = n.replace(/\.(zip|eapack)$/i, '');
  n = n.replace(/_pulses_extract$/i, '');
  return n;
}

// 版本名：去掉 Windows 不允许的字符，兜底用 modpack
function sanitizeInstance(s) {
  let t = String(s || '').replace(/[\\/:*?"<>|]/g, '_').replace(/[\x00-\x1f]/g, '');
  t = t.trim().replace(/\.+$/, '');
  return t || 'modpack';
}

// 猜游戏目录：记住的 → 从已定位整合包推
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
window.fmtSize = fmtSize;
