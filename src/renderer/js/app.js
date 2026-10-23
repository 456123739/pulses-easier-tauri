// app.js — 主控制器
// 负责：启动流程、身份切换、侧边栏、快捷键、数据库位置、日志分级

// 构建戳：每次发版更新这里，装完能一眼确认是不是新包
const BUILD_STAMP = 'build 0.11.7';

const App = {
  identity: 'player',
  _switching: false,
  _navActive: null,

  setStatus(text) {
    const el = document.getElementById('status-text');
    if (el) el.textContent = text;
  },

  // ── 身份切换（交叉过渡：淡入快、滑动慢，位移才看得见）──
  switchIdentity(next) {
    if (this._switching || next === this.identity) return;
    this._switching = true;

    const from = document.getElementById(this.identity + '-view');
    const to = document.getElementById(next + '-view');

    // 滑块跟随
    document.querySelectorAll('.seg-btn').forEach(b =>
      b.classList.toggle('is-active', b.dataset.id === next));
    const pill = document.getElementById('seg-pill');
    if (pill) pill.classList.toggle('is-right', next === 'developer');

    // 整块滑动（像翻页），不是移一小段：
    //   切到开发者 → 整体往右滑走；切到玩家 → 整体往左滑走
    // 出场与入场用同样的时长与曲线 → 两块拼成一条连续带子一起移动。
    // 与 Python 版 Anim.SWITCH_MS = 250 对齐
    const dir = (next === 'developer') ? 1 : -1;
    const DUR = 250;

    from.classList.remove('is-active');
    to.classList.add('is-active');

    // 出场：整块划走（保持不透明，否则会「消失」而不是「划走」）
    Anim.play(from,
      [{ opacity: 1, transform: 'translateX(0)' },
       { opacity: 1, transform: `translateX(${dir * 100}%)` }],
      { duration: DUR, easing: Anim.SOFT, fill: 'both' });

    // 入场：从另一侧整块划进来
    Anim.play(to,
      [{ opacity: 1, transform: `translateX(${-dir * 100}%)` },
       { opacity: 1, transform: 'translateX(0)' }],
      { duration: DUR, easing: Anim.SOFT, fill: 'both' });

    // 侧边栏：模式相关项伸出来 / 收回去
    this._revealModeOptions(next);

    this.identity = next;
    Log.attach(document.getElementById(
      next === 'developer' ? 'dev-log-list' : 'log-list'));
    this.setStatus(next === 'developer' ? '开发者模式' : '玩家模式');
    Log.debug(next === 'developer' ? '已切换到开发者模式' : '已切换到玩家模式');

    this._switching = false;
  },

  _revealModeOptions(mode, animate = true) {
    let delay = 0;
    document.querySelectorAll('#mode-options .opt, #mode-options .opt-sep')
      .forEach(el => {
        const show = (el.dataset.mode === mode);
        if (!animate) {
          Reveal.apply(el, show);
        } else {
          Reveal.set(el, show, show ? delay : 0);
          if (show) delay += 45;
        }
      });
  },

  // ── 侧边栏项选中态（图标动画靠 CSS 的 .is-active 驱动）──
  markNav(id) {
    document.querySelectorAll('#mode-options .nav-item, #sb-more .nav-item')
      .forEach(el => el.classList.toggle('is-active', el.id === id));
    this._navActive = id;
  },

  // ── 设置（iOS 分组行）──
  // 侧栏「更改数据库位置」后面显示当前库路径；太长就来回滚（复用最近打开的滚动）
  async loadDbPath() {
    const el = document.getElementById('db-path');
    const wrap = document.getElementById('db-path-wrap');
    if (!el) return;
    let p = '';
    try { p = await window.pulses.db.getPath(); } catch (_) {}
    el.textContent = p || '未设置';
    el.title = p || '';
    try { if (wrap) Recent._startMarquee([[wrap, el]]); } catch (_) { /* 忽略 */ }
  },

  // ── 数据库（点侧栏弹出面板：清理缓存 / 更改位置）──
  async openDbPanel() {
    this.markNav('nav-db');

    let dbPath = '';
    try { dbPath = (await window.pulses.db.getPath()) || ''; } catch (_) {}

    await Overlay.panel('数据库', (body, done) => {
      const mkRow = (label, value, onClick) => {
        const row = document.createElement('button');
        row.className = 'pref-row pref-row-btn';

        const main = document.createElement('div');
        main.className = 'pref-main';
        const l = document.createElement('div');
        l.className = 'pref-label';
        l.textContent = label;
        main.appendChild(l);
        row.appendChild(main);

        if (value) {
          const v = document.createElement('div');
          v.className = 'pref-value';
          v.textContent = value;
          v.title = value;
          row.appendChild(v);
        }

        const chev = document.createElement('span');
        chev.className = 'chev';
        chev.textContent = '›';
        row.appendChild(chev);

        row.addEventListener('click', () => {
          if (done) done(null);        // 先关面板，再干活
          setTimeout(onClick, 0);
        });
        return row;
      };

      // 更改位置：后面跟当前路径
      body.appendChild(mkRow('更改位置', dbPath || '未设置',
        () => App.changeDbLocation()));
      // 清理缓存
      body.appendChild(mkRow('清理缓存', '', () => App.cleanCache()));
    });
  },

  // ── 数据库位置（原来在「设置」弹窗里，现在直接放侧边栏）──
  async changeDbLocation() {
    this.markNav('nav-db');

    let dbPath = null;
    try { dbPath = await window.pulses.db.getPath(); } catch (_) {}

    const picked = await window.pulses.dialog.openFolder('选择数据库位置');
    if (!picked || picked === dbPath) return;

    let valid = false;
    try { valid = await window.pulses.db.isValid(picked); } catch (_) {}

    const ok = await Overlay.confirm('更换数据库位置',
      `${picked}\n\n${valid ? '切换到该数据库。' : '在该位置新建数据库。'}\n当前数据库不会被删除。`,
      { confirmText: valid ? '切换' : '新建并切换' });
    if (!ok) return;

    const created = await window.pulses.db.create(picked);
    if (!created) {
      Log.error('更换数据库位置失败：' + picked);
      await Overlay.alert('更换失败', '无法使用该位置。', { level: 'error' });
      return;
    }
    Log.ok('数据库位置已更新：' + picked);
    App.setStatus('数据库：' + picked);
    await App.loadWhitelist();
    await App.loadDbPath();
  },

  async loadUiPrefs() {
    let ui = {};
    try {
      const cfg = await window.pulses.db.loadConfig();
      ui = (cfg && cfg.ui) || {};
    } catch (_) { /* 静默 */ }

    // 主题固定为默认青灰，不再允许自定义
    Theme.apply(Theme.DEFAULT);

    // 下面三项全部默认开启；设置面板已移除，不再提供开关
    Motion.setEnabled(ui.animations !== false);
    Marquee.set(ui.path_marquee !== false);
    Log.setVerbose(ui.verbose_log !== false);
  },

  // ── 其它侧边栏动作 ──
  async cleanCache() {
    this.markNav('btn-clean-cache');
    const cfg = await window.pulses.db.loadConfig();
    if (!cfg) {
      await Overlay.alert('提示', '还没有数据库。');
      return;
    }
    const ok = await Overlay.confirm('清理缓存',
      '将清理数据库 cache 目录（下载缓存 / mod 侧信息）。\n'
      + '已应用的更新不受影响。\n\n确定继续？',
      { confirmText: '清理', danger: true });
    if (!ok) return;
    let r = null;
    try { r = await window.pulses.db.clearCache(); } catch (_) { /* 忽略 */ }
    if (!r || !r.ok) {
      Log.error('清理缓存失败');
      this.setStatus('清理缓存失败');
      return;
    }
    const mb = (r.bytes / 1048576).toFixed(1);
    if (r.files) {
      Log.ok(`缓存已清理：${r.files} 个文件，释放 ${mb} MB`);
      this.setStatus(`缓存已清理（${mb} MB）`);
    } else {
      Log.debug('没有缓存需要清理');
      this.setStatus('没有缓存需要清理');
    }
  },

  // ── 白名单（在开发者端右面板左下角）──
  async loadWhitelist() {
    const list = document.getElementById('whitelist-list');
    if (!list) return;
    let wl = [];
    try {
      const cfg = await window.pulses.db.loadConfig();
      wl = (cfg && cfg.whitelist) || [];
    } catch (_) { /* 静默 */ }

    list.innerHTML = '';
    if (!wl.length) {
      const e = document.createElement('div');
      e.className = 'wl-empty';
      e.textContent = '暂无条目';
      list.appendChild(e);
      return;
    }
    wl.forEach(w => {
      const b = document.createElement('button');
      b.className = 'wl-chip';
      b.textContent = w;
      b.title = w;
      list.appendChild(b);
    });
  },

  async addWhitelist() {
    const cfg = await window.pulses.db.loadConfig();
    if (!cfg) {
      await Overlay.alert('提示', '还没有数据库。');
      return;
    }
    const cur = cfg.whitelist || [];
    const val = await Overlay.prompt('新增白名单',
      '相对整合包根目录；子文件夹用 / 分隔（例如 config/foo）',
      {
        placeholder: '例如 mods 或 config/foo',
        confirmText: '添加',
        validate: (v) => {
          const s = String(v || '').trim()
            .replace(/\\/g, '/').replace(/^\/+|\/+$/g, '');
          if (!s) return [false, '', '路径不能为空。'];
          if (s.split('/').includes('..')) return [false, '', '路径不能包含 “..”。'];
          if (/[<>:"|?*]/.test(s)) return [false, '', '路径不能包含 < > : " | ? * 等字符。'];
          if (cur.includes(s)) return [false, '', `条目“${s}”已存在。`];
          return [true, s, ''];
        },
      });
    if (!val) return;
    cfg.whitelist = [...cur, val];
    await window.pulses.db.saveConfig(cfg);
    Log.ok('白名单已添加：' + val);
    await this.loadWhitelist();
  },
};

// ── 自定义标题栏的窗口控制 ──
function bindWindowControls() {
  const W = window.__TAURI__ && window.__TAURI__.window;
  if (!W || typeof W.getCurrentWindow !== 'function') return;
  let win = null;
  try { win = W.getCurrentWindow(); } catch (_) { return; }
  if (!win) return;

  const on = (id, fn) => {
    const el = document.getElementById(id);
    if (el) el.addEventListener('click', () => { try { fn(); } catch (_) {} });
  };
  on('win-min',   () => win.minimize());
  on('win-max',   () => win.toggleMaximize());
  on('win-close', () => win.close());
}

// ── 动画自检 ──
// 造一个带过渡的元素，改属性后中途采样 opacity：
// 拿到 0~1 之间 → 过渡确实在跑；拿到 0 或 1 → 过渡没生效。
function animationSelfTest() {
  return new Promise(resolve => {
    try {
      const el = document.createElement('div');
      el.style.cssText =
        'position:fixed;left:-9999px;top:0;width:10px;height:10px;' +
        'opacity:1;transition:opacity 300ms linear;';
      document.body.appendChild(el);
      void el.offsetWidth;
      el.style.opacity = '0';
      setTimeout(() => {
        let mid = 0;
        try { mid = parseFloat(getComputedStyle(el).opacity); } catch (_) {}
        el.remove();
        resolve(mid > 0.02 && mid < 0.98);
      }, 90);
    } catch (_) {
      resolve(false);
    }
  });
}

// ── 快捷键 ──
function bindShortcuts() {
  document.addEventListener('keydown', e => {
    if (e.ctrlKey && e.key.toLowerCase() === 'o') {
      e.preventDefault();
      if (App.identity === 'developer') Developer.pickNew();
      else Player.pickModpack();
    }
    if (e.ctrlKey && e.key === '1') { e.preventDefault(); App.switchIdentity('player'); }
    if (e.ctrlKey && e.key === '2') { e.preventDefault(); App.switchIdentity('developer'); }
  });
}

// ── 启动页 ──
// 参数与 Python 版 app/ui/splash.py 顶部常量一一对应，改一个改另一个。
const Splash = {
  // ---- 伪进度参数（Python: _PROGRESS_CAP / _PROGRESS_TAU_MS / ...）----
  CAP: 0.90,            // 启动期间最多走到这里，剩下的留给「真的完成了」
  TAU_MS: 700,          // 时间常数：越小前段越快、后面越慢
  RIPPLE: 0.005,        // 轻微呼吸（相对整条长度）
  RIPPLE_MS: 420,       // 呼吸周期
  RIPPLE_DECAY: 1500,
  FINISH_MS: 260,       // 收尾补满的时长
  FADE_MS: 420,         // logo 淡入

  _raf: 0,
  _t0: 0,
  _p: 0,
  _fill: null,
  _started: false,

  _set(p) {
    this._p = p;
    if (this._fill) this._fill.style.transform = `scaleX(${p})`;
  },

  // 与 Python progress_at() 同一个公式：
  //   base   = cap * (1 - exp(-t / tau))
  //   ripple = RIPPLE * sin(t / RIPPLE_MS * 2π) * exp(-t / RIPPLE_DECAY)
  //   永远 < cap，到顶只可能来自 finish()
  progressAt(ms) {
    if (ms <= 0) return 0;
    const base = this.CAP * (1 - Math.exp(-ms / this.TAU_MS));
    const ripple = this.RIPPLE
      * Math.sin(ms / this.RIPPLE_MS * Math.PI * 2)
      * Math.exp(-ms / this.RIPPLE_DECAY);
    return Math.max(0, Math.min(this.CAP, base + ripple));
  },

  play() {
    if (this._started) return;
    this._started = true;

    const logo = document.querySelector('.splash-logo');
    this._fill = document.getElementById('splash-fill');

    // 双 rAF：等浏览器画完第一帧再开始，动画才不会被「窗口还没显示」吃掉
    requestAnimationFrame(() => requestAnimationFrame(() => {
      // Python 的 _fade_in() 是纯 alpha 0→1、_FADE_MS=420，不做位移缩放
      if (logo) {
        const a = Anim.play(logo,
          [{ opacity: 0 }, { opacity: 1 }],
          { duration: this.FADE_MS, easing: Anim.EASE, fill: 'both' });
        // WAAPI 不可用时退化成直接可见，不能留个看不见的 logo
        if (!a) logo.style.opacity = '1';
      }
      this._t0 = performance.now();
      this._tick();
    }));
  },

  _tick() {
    this._set(this.progressAt(performance.now() - this._t0));
    this._raf = requestAnimationFrame(() => this._tick());
  },

  // 真就绪：补满（_FINISH_MS）→ 停留一下 → 淡出
  finish() {
    cancelAnimationFrame(this._raf);
    this._started = true;

    const splash = document.getElementById('splash');
    const fill = this._fill || document.getElementById('splash-fill');

    if (fill) {
      const a = Anim.play(fill,
        [{ transform: `scaleX(${this._p})` }, { transform: 'scaleX(1)' }],
        { duration: this.FINISH_MS, easing: 'cubic-bezier(0.33, 1, 0.68, 1)', fill: 'both' });
      if (!a) fill.style.transform = 'scaleX(1)';
    }

    if (!splash) return;
    const a = Anim.play(splash, [{ opacity: 1 }, { opacity: 0 }],
      { duration: 300, delay: 220, easing: Anim.EASE, fill: 'both' });
    splash.classList.add('is-out');
    setTimeout(() => { splash.style.display = 'none'; }, a ? 560 : 0);
  },
};

// ── 启动 ──
// 设计原则：任何一步失败都不能把启动卡死。
//   1. 每一步独立 try/catch，失败的记下来继续走
//   2. 3 秒看门狗：无论如何强制进主界面
//   3. 失败原因写进日志与状态栏，方便定位
async function boot() {
  const failures = [];

  // 越早越好：让启动页的入场动画从「窗口可见」那一刻开始计时
  try { Splash.play(); } catch (_) { /* 忽略 */ }

  const step = async (name, fn) => {
    try {
      await fn();
    } catch (e) {
      const msg = (e && e.message) ? e.message : String(e);
      failures.push(name + ' → ' + msg);
      try { console.error('[boot]', name, e); } catch (_) {}
    }
  };

  let shown = false;
  const showMain = () => {
    if (shown) return;
    shown = true;

    // 启动时窗口只有卡片那么大：先让卡片淡出，再把窗口放大到主界面尺寸。
    try { Splash.finish(); } catch (_) { /* 忽略 */ }

    setTimeout(() => {
      // ① 窗口放大（此时卡片已经淡完，不会看到它被拉伸）
      try {
        const W = window.__TAURI__ && window.__TAURI__.window;
        const D = window.__TAURI__ && window.__TAURI__.dpi;
        if (W && typeof W.getCurrentWindow === 'function') {
          const win = W.getCurrentWindow();
          const mk = (w, h) => (D && D.LogicalSize)
            ? new D.LogicalSize(w, h) : { width: w, height: h };
          try { win.setMinSize(mk(960, 640)); } catch (_) { /* 忽略 */ }
          try { win.setSize(mk(1280, 820)); } catch (_) { /* 忽略 */ }
          try { win.center(); } catch (_) { /* 忽略 */ }
        }
      } catch (_) { /* 忽略 */ }

      // ② 显示主界面
      try {
        document.body.classList.remove('is-booting');
        const appEl = document.getElementById('app');
        if (appEl) appEl.classList.remove('is-hidden');
        document.querySelectorAll('.group').forEach((g, i) => {
          g.style.animation = `groupIn 420ms var(--ease-out) ${120 + i * 60}ms both`;
        });
        // 界面显示出来之后再量一次路径宽度 —— 隐藏时宽高都是 0，量不出溢出，
        // 不补这一下就要等用户点一下才开始滚。
        try { if (window.Recent) Recent.refreshMarquee(); } catch (_) { /* 忽略 */ }
        try { App.loadDbPath(); } catch (_) { /* 忽略 */ }
      } catch (_) { /* 忽略 */ }
    }, 300);

    try {
      if (failures.length) {
        Log.error('启动异常 ' + failures.length + ' 处：' + failures.join('；'));
        App.setStatus('启动异常：' + failures[0]);
      } else {
        App.setStatus('就绪');
      }
    } catch (_) { /* 忽略 */ }
  };

  // 看门狗：正常 1.25 秒进主界面，超 3 秒强制进
  const watchdog = setTimeout(showMain, 3000);

  await step('初始化', () => {
    Overlay.init();
    Log.attach(document.getElementById('log-list'));
  });

  await step('版本号', async () => {
    let ver = '';
    try { ver = await window.pulses.version(); } catch (_) {}
    const el = document.getElementById('status-version');
    if (el) el.textContent = (ver ? 'v' + ver : '') + ' · ' + BUILD_STAMP;
  });

  await step('视图', () => {
    Player.init();
    Developer.init();
  });

  await step('快捷键与窗口控制', () => {
    bindShortcuts();
    bindWindowControls();
  });

  await step('侧栏模式项', () => {
    App._revealModeOptions('player', false);
  });

  await step('事件绑定', () => {
    document.querySelectorAll('.seg-btn').forEach(b =>
      b.addEventListener('click', () => App.switchIdentity(b.dataset.id)));

    const on = (id, fn) => {
      const el = document.getElementById(id);
      if (el) el.addEventListener('click', fn);
    };
    on('btn-clean-cache',  () => App.cleanCache());
    on('btn-recent-clear', async () => {
      await Recent.clear();
      Log.debug('已清空最近打开');
    });
    on('btn-whitelist-add', () => App.addWhitelist());

    const nav = (id, fn) => {
      const el = document.getElementById(id);
      if (el) el.addEventListener('click', () => { App.markNav(id); fn(); });
    };
    nav('nav-install',  () => Installer.open());
    nav('nav-locate',   () => Player.pickModpack());
    nav('nav-pickpack', () => Player.pickPack());
    nav('nav-newpack',  () => Developer.pickNew());
    nav('nav-oldpack',  () => Developer.pickOld());
    nav('nav-db',       () => App.openDbPanel());
  });

  await step('最近打开', async () => {
    await Recent.load();
    Recent.bind(document.getElementById('sb-recent-list'),
                p => Player.setModpack(p));
    Recent.renderAll();
  });

  await step('界面设置', () => App.loadUiPrefs());
  await step('数据库路径', () => App.loadDbPath());
  await step('白名单', () => App.loadWhitelist());

  await step('动画自检', () => {
    animationSelfTest().then(ok => {
      if (ok) {
        Log.ok('自检通过');
      } else {
        Log.error('自检未通过：过渡动画没有生效');
        App.setStatus('动画未生效，请把这条日志反馈给开发者');
      }
    });
  });

  // 正常入场
  setTimeout(() => {
    clearTimeout(watchdog);
    showMain();
  }, 1250);
}

// ── 窗口显示 ──
// 窗口在 tauri.conf 里是 visible:false。透明窗口如果先显示再绘制，
// 会看到一下"透明"（只有窗口没有内容）。所以等首帧画完再 show。
(function showWindowWhenPainted() {
  let done = false;
  const show = () => {
    if (done) return;
    done = true;
    try {
      const W = window.__TAURI__ && window.__TAURI__.window;
      if (W && typeof W.getCurrentWindow === 'function') {
        W.getCurrentWindow().show();
      }
    } catch (_) { /* 忽略 */ }
  };
  requestAnimationFrame(() => requestAnimationFrame(show));
  setTimeout(show, 1200);          // 兜底：万一 rAF 没跑
})();

window.App = App;
window.Splash = Splash;

if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', boot);
} else {
  boot();
}
