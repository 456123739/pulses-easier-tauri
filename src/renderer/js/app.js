// app.js — 主控制器
// 负责：启动流程、身份切换、侧边栏、快捷键、首选项、日志分级

// 构建戳：每次发版更新这里，装完能一眼确认是不是新包
const BUILD_STAMP = 'build 0.7.7';

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

  // ── 设置（iOS 分组行：标签 + 当前值 + 箭头，无解释文字）──
  async openPrefs() {
    this.markNav('btn-prefs');

    let dbPath = null;
    try { dbPath = await window.pulses.db.getPath(); } catch (_) {}

    await Overlay.panel('设置', (body, done) => {
      // 数据库位置
      const r1 = document.createElement('div');
      r1.className = 'pref-row';

      const m1 = document.createElement('div');
      m1.className = 'pref-main';
      const l1 = document.createElement('div');
      l1.className = 'pref-label';
      l1.textContent = '数据库位置';
      m1.appendChild(l1);

      const v1 = document.createElement('div');
      v1.className = 'pref-value';
      v1.textContent = dbPath || '未设置';
      v1.title = dbPath || '';

      r1.appendChild(m1);
      r1.appendChild(v1);

      const b1 = document.createElement('button');
      b1.className = 'pref-btn';
      b1.textContent = '更改';
      b1.addEventListener('click', async () => {
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
        v1.textContent = dbPath;
        v1.title = dbPath;
        Log.ok('数据库位置已更新：' + dbPath);
        App.setStatus('数据库：' + dbPath);
        await App.loadWhitelist();
      });

      r1.appendChild(b1);
      body.appendChild(r1);

      // 完整日志（开关，无说明文字）
      const r2 = document.createElement('div');
      r2.className = 'pref-row';
      const m2 = document.createElement('div');
      m2.className = 'pref-main';
      const l2 = document.createElement('div');
      l2.className = 'pref-label';
      l2.textContent = '完整日志';
      m2.appendChild(l2);

      const sw = document.createElement('button');
      sw.className = 'pref-switch' + (Log.isVerbose() ? ' is-on' : '');
      sw.addEventListener('click', async () => {
        const on = !Log.isVerbose();
        Log.setVerbose(on);
        sw.classList.toggle('is-on', on);
        await App.saveVerbose(on);
      });

      r2.appendChild(m2);
      r2.appendChild(sw);
      body.appendChild(r2);

      // 版本
      const r3 = document.createElement('div');
      r3.className = 'pref-row';
      const m3 = document.createElement('div');
      m3.className = 'pref-main';
      const l3 = document.createElement('div');
      l3.className = 'pref-label';
      l3.textContent = '版本';
      m3.appendChild(l3);

      const v3 = document.createElement('div');
      v3.className = 'pref-value';
      v3.textContent = document.getElementById('status-version').textContent || '—';

      r3.appendChild(m3);
      r3.appendChild(v3);
      body.appendChild(r3);
    });
  },

  async saveVerbose(flag) {
    try {
      const cfg = await window.pulses.db.loadConfig();
      if (!cfg) return;
      cfg.ui = cfg.ui || {};
      cfg.ui.verbose_log = flag;
      await window.pulses.db.saveConfig(cfg);
    } catch (_) { /* 静默 */ }
  },

  async loadVerbose() {
    try {
      const cfg = await window.pulses.db.loadConfig();
      const on = !!(cfg && cfg.ui && cfg.ui.verbose_log);
      Log.setVerbose(on);
    } catch (_) {
      Log.setVerbose(false);
    }
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
async function boot() {
  Overlay.init();
  Log.attach(document.getElementById('log-list'));

  let ver = '';
  try { ver = await window.pulses.version(); } catch (_) {}
  const verEl = document.getElementById('status-version');
  if (verEl) verEl.textContent = (ver ? 'v' + ver : '') + ' · ' + BUILD_STAMP;

  Player.init();
  Developer.init();
  bindShortcuts();

  App._revealModeOptions('player', false);

  document.querySelectorAll('.seg-btn').forEach(b =>
    b.addEventListener('click', () => App.switchIdentity(b.dataset.id)));

  document.getElementById('btn-prefs')
    .addEventListener('click', () => App.openPrefs());
  document.getElementById('btn-clean-cache')
    .addEventListener('click', () => App.cleanCache());
  document.getElementById('btn-recent-clear')
    .addEventListener('click', async () => {
      await Recent.clear();
      Log.debug('已清空最近打开');
    });
  document.getElementById('btn-whitelist-add')
    .addEventListener('click', () => App.addWhitelist());

  // 侧边栏快捷入口（点完标记选中 → 图标播动画）
  const nav = (id, fn) => {
    const el = document.getElementById(id);
    if (el) el.addEventListener('click', () => { App.markNav(id); fn(); });
  };
  nav('nav-locate',   () => Player.pickModpack());
  nav('nav-pickpack', () => Player.pickPack());
  nav('nav-newpack',  () => Developer.pickNew());
  nav('nav-oldpack',  () => Developer.pickOld());

  await Recent.load();
  Recent.bind(document.getElementById('sb-recent-list'), p => Player.setModpack(p));
  Recent.renderAll();
  await App.loadVerbose();
  await App.loadWhitelist();

  animationSelfTest().then(ok => {
    if (ok) {
      Log.debug('动画自检：通过');
    } else {
      Log.error('动画自检：未通过 —— 过渡没有生效');
      App.setStatus('动画未生效，请把这条日志反馈给开发者');
    }
  });

  // 启动动画 → 主界面
  setTimeout(() => {
    const splash = document.getElementById('splash');
    const appEl = document.getElementById('app');

    splash.classList.add('is-out');
    appEl.classList.remove('is-hidden');

    document.querySelectorAll('.panel').forEach((p, i) => {
      p.style.animation = `panelIn 420ms var(--ease-out) ${120 + i * 60}ms both`;
    });

    setTimeout(() => { splash.style.display = 'none'; }, 340);
    App.setStatus('就绪');
    Log.debug('Pulses Easier 已启动');
  }, 1250);
}

window.App = App;

if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', boot);
} else {
  boot();
}
