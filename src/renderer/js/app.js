// app.js — 主控制器
// 对应 Python: main.py + main_window.py + sidebar.py + polish.py
// 负责：启动流程、身份切换（玩家/开发者）、侧边栏、弹层入口

const App = {
  identity: 'player',
  _switching: false,

  setStatus(text) {
    const el = document.getElementById('status-text');
    if (el) el.textContent = text;
  },

  // ── 身份切换 ──
  async switchIdentity(next) {
    if (this._switching || next === this.identity) return;
    this._switching = true;

    const oldView = document.getElementById(this.identity + '-view');
    const newView = document.getElementById(next + '-view');

    // 侧栏 tab 状态
    document.querySelectorAll('.seg-btn').forEach(b =>
      b.classList.toggle('is-active', b.dataset.id === next));

    // 白名单只在开发者显示
    document.getElementById('sb-whitelist')
      .classList.toggle('is-hidden', next !== 'developer');

    // 旧视图左滑出
    oldView.classList.remove('is-active');
    oldView.classList.add('exit-left');

    await new Promise(r => setTimeout(r, 90));

    oldView.classList.add('is-hidden');
    oldView.classList.remove('exit-left');
    newView.classList.remove('is-hidden');
    void newView.offsetWidth;              // 强制重排，让 transition 生效
    newView.classList.add('is-active');

    // 侧边栏卡片重新错开入场
    const cards = Array.from(document.querySelectorAll('.sb-card'));
    if (Motion.enabled) {
      cards.forEach((c, i) => {
        c.style.animation = 'none';
        void c.offsetWidth;
        c.style.animation = `cardIn 260ms var(--ease-out) ${i * 45}ms both`;
      });
    }

    this.identity = next;
    // 日志目标切到当前视图
    Log.attach(document.getElementById(next === 'developer' ? 'dev-log-list' : 'log-list'));
    this.setStatus(next === 'developer' ? '开发者模式' : '玩家模式');
    Log.info(next === 'developer' ? '已切换到开发者模式' : '已切换到玩家模式');
    this._switching = false;
  },

  // ── 侧边栏动作 ──
  async openPrefs() {
    const cfg = await window.pulses.db.loadConfig();
    if (!cfg) {
      await Overlay.alert('提示', '还没有数据库。\n请先选择或新建一个数据库位置。');
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
    Log.warn('缓存清理功能将在后续版本接入（当前仅确认流程）');
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
    try {
      const cfg = await window.pulses.db.loadConfig();
      const wl = (cfg && cfg.whitelist) || [];
      const list = document.getElementById('sb-whitelist-list');
      list.innerHTML = '';
      if (!wl.length) {
        const e = document.createElement('div');
        e.className = 'sb-list-empty';
        e.textContent = '暂无条目';
        list.appendChild(e);
        return;
      }
      wl.forEach(w => {
        const b = document.createElement('button');
        b.className = 'sb-list-item';
        b.textContent = w;
        b.title = w;
        list.appendChild(b);
      });
    } catch (_) { /* 静默 */ }
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
          const s = String(v || '').trim().replace(/\\/g, '/').replace(/^\/+|\/+$/g, '');
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

// ── 启动 ──
async function boot() {
  Overlay.init();
  Log.attach(document.getElementById('log-list'));

  let ver = '';
  try { ver = await window.pulses.version(); } catch (_) {}
  document.getElementById('status-version').textContent = ver ? 'v' + ver : '';

  // 开发者端日志也接上（切过去时用同一个 Log 目标）
  Player.init();
  Developer.init();

  // 身份切换
  document.querySelectorAll('.seg-btn').forEach(b =>
    b.addEventListener('click', () => App.switchIdentity(b.dataset.id)));

  // 侧边栏按钮
  document.getElementById('btn-prefs').addEventListener('click', () => App.openPrefs());
  document.getElementById('btn-clean-cache').addEventListener('click', () => App.cleanCache());
  document.getElementById('btn-db-location').addEventListener('click', () => App.openDbLocation());
  document.getElementById('btn-recent-clear').addEventListener('click', async () => {
    await Recent.clear();
    Log.info('已清空最近打开');
  });
  document.getElementById('btn-whitelist-add').addEventListener('click', () => App.addWhitelist());

  // 最近打开 + 白名单
  await Recent.load();
  Recent.renderAll();
  await App.loadWhitelist();

  // 启动动画 → 主界面
  setTimeout(() => {
    const splash = document.getElementById('splash');
    const appEl = document.getElementById('app');
    splash.classList.add('fade-out');
    appEl.classList.remove('is-hidden');
    appEl.classList.add('fade-in');

    // 侧边栏卡片错开入场
    const cards = Array.from(document.querySelectorAll('.sb-card'));
    cards.forEach((c, i) => {
      c.style.animation = `cardIn 260ms var(--ease-out) ${i * 50}ms both`;
    });

    setTimeout(() => { splash.style.display = 'none'; }, 300);
    App.setStatus('就绪');
    Log.info('Pulses Easier 已启动');
  }, 1100);
}

window.App = App;

if (document.readyState === 'loading') {
  document.addEventListener('DOMContentLoaded', boot);
} else {
  boot();
}
