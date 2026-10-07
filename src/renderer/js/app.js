// app.js — 主控制器
// 负责：启动流程、身份切换、侧边栏、快捷键、首选项、日志分级

// 构建戳：每次发版更新这里，装完能一眼确认是不是新包
const BUILD_STAMP = 'build 0.8.3';

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

    // 旧视图往左滑出
    from.classList.remove('is-active');
    from.style.transform = 'translateX(-56px)';

    // 新视图从右滑入（并发，无等待）
    to.style.transform = '';
    void to.offsetWidth;
    to.style.transform = 'translateX(56px)';
    void to.offsetWidth;
    to.classList.add('is-active');
    to.style.transform = '';

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
  async openPrefs() {
    this.markNav('btn-prefs');

    let dbPath = null;
    try { dbPath = await window.pulses.db.getPath(); } catch (_) {}

    await Overlay.panel('设置', (body) => {
      const mkRow = (label, right) => {
        const row = document.createElement('div');
        row.className = 'pref-row';
        const main = document.createElement('div');
        main.className = 'pref-main';
        const l = document.createElement('div');
        l.className = 'pref-label';
        l.textContent = label;
        main.appendChild(l);
        row.appendChild(main);
        if (right) row.appendChild(right);
        return row;
      };

      const mkSwitch = (on, onChange) => {
        const sw = document.createElement('button');
        sw.className = 'pref-switch' + (on ? ' is-on' : '');
        sw.addEventListener('click', () => {
          const next = !sw.classList.contains('is-on');
          sw.classList.toggle('is-on', next);
          onChange(next);
        });
        return sw;
      };

      // 1. 数据库位置
      const val = document.createElement('div');
      val.className = 'pref-value';
      val.textContent = dbPath || '未设置';
      val.title = dbPath || '';
      const btn = document.createElement('button');
      btn.className = 'pref-btn';
      btn.textContent = '更改';
      btn.addEventListener('click', async () => {
        const picked = await window.pulses.dialog.openFolder('选择数据库位置');
        if (!picked) return;
        if (dbPath) {
          const valid = await window.pulses.db.isValid(picked);
          const ok = await Overlay.confirm('更换数据库位置',
            `${picked}\n\n${valid ? '切换到该数据库。' : '在该位置新建数据库。'}\n当前数据库不会被删除。`,
            { confirmText: valid ? '切换' : '新建并切换' });
          if (!ok) return;
        }
        const created = await window.pulses.db.create(picked);
        if (!created) {
          Log.error('更换数据库位置失败：' + picked);
          await Overlay.alert('更换失败', '无法使用该位置。', { level: 'error' });
          return;
        }
        dbPath = picked;
        val.textContent = dbPath;
        val.title = dbPath;
        Log.ok('数据库位置已更新：' + dbPath);
        App.setStatus('数据库：' + dbPath);
        await App.loadWhitelist();
      });

      const r1 = document.createElement('div');
      r1.className = 'pref-row';
      const m1 = document.createElement('div');
      m1.className = 'pref-main';
      const l1 = document.createElement('div');
      l1.className = 'pref-label';
      l1.textContent = '数据库位置';
      m1.appendChild(l1);
      r1.appendChild(m1);
      r1.appendChild(val);
      r1.appendChild(btn);
      body.appendChild(r1);

      // 2. 主题色（小方框，点开是调色盘）
      const sw = document.createElement('button');
      sw.className = 'pref-swatch';
      sw.style.background = Theme.current();
      sw.title = '主题色';
      sw.addEventListener('click', () => App.openThemePicker());
      body.appendChild(mkRow('主题色', sw));

      // 3. 界面动画
      body.appendChild(mkRow('界面动画', mkSwitch(Motion.enabled, (on) => {
        Motion.setEnabled(on);
        App.saveUiPref('animations', on);
      })));

      // 3. 路径自动滚动
      body.appendChild(mkRow('路径自动滚动', mkSwitch(Marquee.enabled(), (on) => {
        Marquee.set(on);
        App.saveUiPref('path_marquee', on);
      })));

      // 4. 完整日志
      body.appendChild(mkRow('完整日志', mkSwitch(Log.isVerbose(), (on) => {
        Log.setVerbose(on);
        App.saveUiPref('verbose_log', on);
      })));

      // 5. 版本
      const ver = document.createElement('div');
      ver.className = 'pref-value';
      ver.textContent = document.getElementById('status-version').textContent || '—';
      body.appendChild(mkRow('版本', ver));
    });
  },

  // ── 调色盘（覆盖层）──
  async openThemePicker() {
    const start = Theme.current();

    await Overlay.panel('主题色', (body) => {
      // 预设色点
      const grid = document.createElement('div');
      grid.className = 'picker-grid';
      const dots = [];
      for (const [name, hex] of Theme.PRESETS) {
        const dot = document.createElement('button');
        dot.className = 'picker-dot';
        dot.dataset.hex = hex.toLowerCase();
        dot.style.background = hex;
        dot.title = `${name} ${hex}`;
        dot.addEventListener('click', () => setColor(hex));
        grid.appendChild(dot);
        dots.push(dot);
      }
      body.appendChild(grid);

      // 原生调色盘
      const native = document.createElement('input');
      native.type = 'color';
      native.className = 'picker-native';
      native.value = start;
      native.addEventListener('input', () => setColor(native.value));
      body.appendChild(native);

      // 颜色代码输入（HEX / RGB / HSL 都认）
      const input = document.createElement('input');
      input.className = 'picker-input';
      input.placeholder = '#6E8FA3  ·  rgb(110,143,163)  ·  hsl(203,18%,54%)';
      input.value = start;
      const commit = () => {
        if (Theme.apply(input.value)) {
          input.style.color = '';
          refresh();
          App.saveUiPref('accent', Theme.current());
          swatchSync();
        } else {
          input.style.color = 'var(--red)';
        }
      };
      input.addEventListener('change', commit);
      input.addEventListener('keydown', e => { if (e.key === 'Enter') commit(); });
      body.appendChild(input);

      // 预览 + 三种格式回显
      const preview = document.createElement('div');
      preview.className = 'picker-preview';
      body.appendChild(preview);

      const formats = document.createElement('div');
      formats.className = 'picker-formats';
      body.appendChild(formats);

      const swatchSync = () => {
        const el = document.querySelector('.pref-swatch');
        if (el) el.style.background = Theme.current();
      };

      const refresh = () => {
        const rgb = Theme.currentRgb();
        const hex = Theme.toHex(rgb);
        preview.style.background = hex;
        formats.textContent =
          `${hex}\n${Theme.toRgbString(rgb)}\n${Theme.toHslString(rgb)}`;
        native.value = hex;
        if (document.activeElement !== input) input.value = hex;
        for (const d of dots) d.classList.toggle('is-on', d.dataset.hex === hex);
      };

      function setColor(v) {
        if (!Theme.apply(v)) return;
        refresh();
        App.saveUiPref('accent', Theme.current());
        swatchSync();
      }

      refresh();
    });
  },

  async saveUiPref(key, value) {
    try {
      const cfg = await window.pulses.db.loadConfig();
      if (!cfg) return;
      cfg.ui = cfg.ui || {};
      cfg.ui[key] = value;
      await window.pulses.db.saveConfig(cfg);
    } catch (_) { /* 静默 */ }
  },

  async loadUiPrefs() {
    let ui = {};
    try {
      const cfg = await window.pulses.db.loadConfig();
      ui = (cfg && cfg.ui) || {};
    } catch (_) { /* 静默 */ }

    // 主题色（默认青灰）
    Theme.apply(ui.accent || Theme.DEFAULT);

    const anim = ui.animations !== false;          // 默认开
    Motion.setEnabled(anim);

    const marquee = ui.path_marquee !== false;     // 默认开
    Marquee.set(marquee);

    Log.setVerbose(!!ui.verbose_log);              // 默认关
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
    Log.warn('缓存清理将在后续版本接入（当前仅确认流程）');
    this.setStatus('缓存清理：待接入');
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

// ── 启动 ──
// 设计原则：任何一步失败都不能把启动卡死。
//   1. 每一步独立 try/catch，失败的记下来继续走
//   2. 3 秒看门狗：无论如何强制进主界面
//   3. 失败原因写进日志与状态栏，方便定位
async function boot() {
  const failures = [];

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

    try {
      const splash = document.getElementById('splash');
      const appEl = document.getElementById('app');
      if (splash) {
        splash.classList.add('is-out');
        setTimeout(() => { splash.style.display = 'none'; }, 340);
      }
      if (appEl) appEl.classList.remove('is-hidden');
      document.querySelectorAll('.group').forEach((g, i) => {
        g.style.animation = `groupIn 420ms var(--ease-out) ${120 + i * 60}ms both`;
      });
    } catch (_) { /* 忽略 */ }

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
    on('btn-prefs',        () => App.openPrefs());
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
    nav('nav-locate',   () => Player.pickModpack());
    nav('nav-pickpack', () => Player.pickPack());
    nav('nav-newpack',  () => Developer.pickNew());
    nav('nav-oldpack',  () => Developer.pickOld());
  });

  await step('最近打开', async () => {
    await Recent.load();
    Recent.bind(document.getElementById('sb-recent-list'),
                p => Player.setModpack(p));
    Recent.renderAll();
  });

  await step('界面设置', () => App.loadUiPrefs());
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

window.App = App;

if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', boot);
} else {
  boot();
}
