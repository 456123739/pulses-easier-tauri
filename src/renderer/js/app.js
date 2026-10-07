// app.js — 主控制器（启动流程 + 模式切换 + 侧边栏选项动画）
// 对应 Python: main.py + app/ui/main_window.py + sidebar.py + polish.py

(function () {
  'use strict';

  // 各模式下的侧边栏选项（切换模式时整组换掉 + 错开滑入）
  const SIDEBAR_OPTIONS = {
    player: [
      { id: 'locate',     icon: '📁', label: '定位整合包' },
      { id: 'updatepack', icon: '📦', label: '更新包' },
      { id: 'whitelist',  icon: '✅', label: '校验白名单' },
      { id: 'cache',      icon: '🗑', label: '清理缓存' },
    ],
    developer: [
      { id: 'compare',    icon: '🔍', label: '比对整合包' },
      { id: 'strategies', icon: '⚙', label: '策略表' },
      { id: 'changelog',  icon: '📝', label: '更新日志' },
      { id: 'export',     icon: '📤', label: '导出更新包' },
    ],
  };

  let currentMode = 'player';

  // ── 渲染侧边栏选项 ──
  function renderSidebar(mode) {
    const host = document.getElementById('sidebar-options');
    host.innerHTML = '';
    const opts = SIDEBAR_OPTIONS[mode] || [];
    opts.forEach((o) => {
      const btn = document.createElement('button');
      btn.className = 'sidebar-option';
      btn.dataset.id = o.id;
      btn.innerHTML = `<span>${o.icon}</span><span>${o.label}</span>`;
      btn.addEventListener('click', () => {
        document.querySelectorAll('.sidebar-option')
          .forEach(b => b.classList.toggle('active', b === btn));
        onSidebarOption(o.id);
      });
      host.appendChild(btn);
    });
  }

  // ── 侧边栏选项错开滑入 ──
  function staggerSidebar() {
    const options = Array.from(document.querySelectorAll('.sidebar-option'));
    if (!Motion.enabled) return;
    Motion.stagger(options, 45, 240);
  }

  function onSidebarOption(id) {
    document.getElementById('status-text').textContent = '选中：' + id;
    if (id === 'locate')     Player._locateModpack();
    if (id === 'updatepack') Player._pickPack();
    if (id === 'compare')    Developer._compare();
    if (id === 'export')     Developer._export();
    if (id === 'cache')      openMore();
  }

  // ── 启动流程 ──
  async function boot() {
    Overlay.init();

    const ver = await window.pulses.version();
    document.getElementById('status-version').textContent = 'v' + ver;

    Player.init();
    Developer.init();

    document.querySelectorAll('.mode-tab').forEach(tab => {
      tab.addEventListener('click', () => switchMode(tab.dataset.mode));
    });

    document.getElementById('btn-prefs').addEventListener('click', openPrefs);
    document.getElementById('btn-more').addEventListener('click', openMore);

    // 初始侧边栏
    renderSidebar('player');

    // 启动动画 → 主界面
    setTimeout(() => {
      const splash = document.getElementById('splash');
      splash.classList.add('fade-out');
      const appEl = document.getElementById('app');
      appEl.classList.remove('hidden');
      appEl.classList.add('fade-in');
      // 主界面出现后，侧边栏选项错开滑入
      setTimeout(staggerSidebar, 80);
      setTimeout(() => { splash.style.display = 'none'; }, 320);
    }, 1200);
  }

  // ── 模式切换（旧视图滑出 → 新视图滑入 + 侧边栏整组错开滑入）──
  function switchMode(mode) {
    if (mode === currentMode) return;

    const oldView = document.getElementById(currentMode + '-view');
    const newView = document.getElementById(mode + '-view');

    document.querySelectorAll('.mode-tab').forEach(t =>
      t.classList.toggle('active', t.dataset.mode === mode));

    // 旧视图往左滑出
    oldView.classList.remove('active');
    oldView.classList.add('exit-left');

    // 新视图滑入（延迟一帧，让 transition 生效）
    setTimeout(() => {
      oldView.classList.remove('exit-left');
      oldView.classList.add('hidden');
      newView.classList.remove('hidden');
      void newView.offsetWidth;      // 强制重排，保证起始 transform 生效
      newView.classList.add('active');
    }, 60);

    currentMode = mode;

    // 侧边栏：先清空（避免新旧重叠），再整组错开滑入
    const host = document.getElementById('sidebar-options');
    host.innerHTML = '';
    renderSidebar(mode);
    staggerSidebar();
  }

  // ── 首选项（窗内弹层）──
  async function openPrefs() {
    const cfg = await window.pulses.db.loadConfig();
    if (!cfg) {
      await Overlay.alert(null, '提示', '请先创建或定位数据库');
      return;
    }
    await Overlay.alert(null, '首选项',
      `数据库路径：\n${cfg.db_path}\n\n更多设置项将在后续版本中完善。`);
  }

  // ── 更多（窗内弹层）──
  async function openMore() {
    const choice = await Overlay.confirm(null, '更多操作',
      '打开数据库目录？',
      { confirmText: '打开', cancelText: '关闭' });
    if (choice) {
      const p = await window.pulses.db.getPath();
      if (p) window.pulses.dialog.showItem(p);
    }
  }

  if (document.readyState === 'loading') {
    document.addEventListener('DOMContentLoaded', boot);
  } else {
    boot();
  }
})();
