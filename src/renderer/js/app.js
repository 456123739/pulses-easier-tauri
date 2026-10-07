// app.js — 主控制器
// 对应 Python: main.py + main_window.py + sidebar.py + polish.py
//
// 模式切换的设计要点（上一版的问题就出在这里）：
//   ✗ 旧做法：隐藏旧视图 → 等 170ms → 重建侧边栏面板
//            → 看起来像「闪一下再重载」
//   ✓ 新做法：两个视图常驻 DOM，只切 class，两边同时过渡；
//            侧边栏的模式相关项用高度动画「伸出来 / 收回去」，
//            不销毁、不重建。

const App = {
  identity: 'player',
  _switching: false,

  setStatus(text) {
    const el = document.getElementById('status-text');
    if (el) el.textContent = text;
  },

  // ── 身份切换 ──
  switchIdentity(next) {
    if (this._switching || next === this.identity) return;
    this._switching = true;

    const from = document.getElementById(this.identity + '-view');
    const to = document.getElementById(next + '-view');

    // 1) 滑块跟随
    document.querySelectorAll('.seg-btn').forEach(b =>
      b.classList.toggle('is-active', b.dataset.id === next));
    const pill = document.getElementById('seg-pill');
    if (pill) pill.classList.toggle('is-right', next === 'developer');

    // 2) 旧视图往左滑出（同时淡出）
    from.classList.remove('is-active');
    from.style.transform = 'translateX(-30px)';

    // 3) 新视图从右滑入（同时淡入）—— 与上一步并发，没有等待
    to.style.transform = '';           // 清掉上次留下的残留
    void to.offsetWidth;
    to.style.transform = 'translateX(30px)';
    void to.offsetWidth;
    to.classList.add('is-active');
    to.style.transform = '';           // 交还给 class → 过渡到 none

    // 4) 侧边栏：模式相关项「伸出来 / 收回去」
    this._revealModeOptions(next);

    this.identity = next;
    Log.attach(document.getElementById(
      next === 'developer' ? 'dev-log-list' : 'log-list'));
    this.setStatus(next === 'developer' ? '开发者模式' : '玩家模式');
    Log.info(next === 'developer' ? '已切换到开发者模式' : '已切换到玩家模式');

    this._switching = false;
  },

  // 把侧边栏里 data-mode 与当前身份不符的项收起，相符的展开
  _revealModeOptions(mode, animate = true) {
    let delay = 0;
    document.querySelectorAll('#mode-options .opt, #mode-options .opt-sep')
      .forEach(el => {
        const show = (el.dataset.mode === mode);
        if (!animate) {
          Reveal.apply(el, show);
        } else {
          Reveal.set(el, show, show ? delay : 0);
          if (show) delay += 45;        // 一条条伸出来
        }
      });

    // 白名单面板整块长出来（仅开发者）
    const wl = document.getElementById('sb-whitelist');
    if (wl) {
      if (animate) {
        Reveal.set(wl, mode === 'developer', mode === 'developer' ? 120 : 0);
      } else {
        Reveal.apply(wl, mode === 'developer');
      }
    }
  },

  // ── 侧边栏动作 ──
  async openPrefs() {
    const cfg = await window.pulses.db.loadConfig();
    if (!cfg) {
      await Overlay.alert('提示', '还没有数据库。\n请先在「数据库位置」里选择或新建。');
      return;
    }
    await Overlay.alert('首选项', `数据库路径：\n${cfg.db_path}`);
  },

  async cleanCache() {
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

  async openDbLocation() {
    const p = await window.pulses.db.getPath();
    if (!p) {
      const picked = await window.pulses.dialog.openFolder('选择数据库位置');
      if (!picked) return;
      const created = await window.pulses.db.create(picked);
      if (created) {
        Log.ok('已创建并切换到数据库：' + picked);
        this.setStatus('数据库：' + picked);
      } else {
        await Overlay.alert('创建失败', '无法在该位置创建数据库。',
                            { level: 'error' });
      }
      return;
    }
    await Overlay.alert('数据库位置', p);
    window.pulses.dialog.showItem(p);
  },

  async loadWhitelist() {
    const list = document.getElementById('sb-whitelist-list');
    if (!list) return;
    let wl = [];
    try {
      const cfg = await window.pulses.db.loadConfig();
      wl = (cfg && cfg.whitelist) || [];
    } catch (_) { /* 静默 */ }

    list.innerHTML = '';
    if (!wl.length) {
      const e = document.createElement('div');
      e.className = 'list-empty';
      e.textContent = '暂无条目';
      list.appendChild(e);
      return;
    }
    wl.forEach((w, i) => {
      const b = document.createElement('button');
      b.className = 'list-item';
      b.style.animation = `softUp 260ms var(--ease-out) ${i * 30}ms both`;
      const n = document.createElement('span');
      n.className = 'li-name';
      n.textContent = w;
      b.appendChild(n);
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

// ── 快捷键 ──
function bindShortcuts() {
  document.addEventListener('keydown', e => {
    if (e.ctrlKey && e.key.toLowerCase() === 'o') {
      e.preventDefault();
      if (App.identity === 'developer') Developer.pickNew();
      else Player.pickModpack();
    }
    if (e.ctrlKey && e.key === '1') {
      e.preventDefault();
      App.switchIdentity('player');
    }
    if (e.ctrlKey && e.key === '2') {
      e.preventDefault();
      App.switchIdentity('developer');
    }
  });
}

// ── 动画自检 ──
// 造一个带过渡的元素，改属性后在中途采样 opacity：
// 拿到 0~1 之间的中间值 → 过渡确实在跑；拿到 0 或 1 → 过渡没生效。
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

// ── 启动 ──
async function boot() {
  Overlay.init();
  Log.attach(document.getElementById('log-list'));

  let ver = '';
  try { ver = await window.pulses.version(); } catch (_) {}
  document.getElementById('status-version').textContent = ver ? 'v' + ver : '';

  Player.init();
  Developer.init();
  bindShortcuts();

  // 侧边栏模式项：按当前身份一次性摆好，不播动画
  App._revealModeOptions('player', false);

  document.querySelectorAll('.seg-btn').forEach(b =>
    b.addEventListener('click', () => App.switchIdentity(b.dataset.id)));

  document.getElementById('btn-prefs')
    .addEventListener('click', () => App.openPrefs());
  document.getElementById('btn-clean-cache')
    .addEventListener('click', () => App.cleanCache());
  document.getElementById('btn-db-location')
    .addEventListener('click', () => App.openDbLocation());
  document.getElementById('btn-recent-clear')
    .addEventListener('click', async () => {
      await Recent.clear();
      Log.info('已清空最近打开');
    });
  document.getElementById('btn-whitelist-add')
    .addEventListener('click', () => App.addWhitelist());

  // 侧边栏快捷入口
  const nav = (id, fn) => {
    const el = document.getElementById(id);
    if (el) el.addEventListener('click', fn);
  };
  nav('nav-locate',    () => Player.pickModpack());
  nav('nav-pickpack',  () => Player.pickPack());
  nav('nav-newpack',   () => Developer.pickNew());
  nav('nav-oldpack',   () => Developer.pickOld());
  nav('nav-whitelist', () => {
    const wl = document.getElementById('sb-whitelist');
    if (wl) Reveal.set(wl, true);
    App.addWhitelist();
  });

  await Recent.load();
  Recent.bind(document.getElementById('sb-recent-list'),
              p => Player.setModpack(p));
  Recent.renderAll();
  await App.loadWhitelist();

  // ── 动画自检：不再靠猜，用真实过渡测一次 ──
  // 桌面软件不跟随系统「减少动态效果」开关（那会把动画全砍掉），
  // 但要把检测结果报出来，方便定位。
  try {
    const rm = window.matchMedia('(prefers-reduced-motion: reduce)').matches;
    if (rm) Log.info('检测到系统「减少动态效果」已开启（本程序不受其影响）');
  } catch (_) { /* 忽略 */ }

  animationSelfTest().then(ok => {
    if (ok) {
      Log.info('动画自检：通过');
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
      p.style.animation = `panelIn 380ms var(--ease-out) ${120 + i * 55}ms both`;
    });

    setTimeout(() => { splash.style.display = 'none'; }, 340);
    App.setStatus('就绪');
    Log.info('Pulses Easier 已启动');
  }, 1250);
}

window.App = App;

if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', boot);
} else {
  boot();
}
