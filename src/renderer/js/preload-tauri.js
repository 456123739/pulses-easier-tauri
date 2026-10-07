// preload-tauri.js — Tauri 前端桥接
// 替代 Electron preload.js：用 Tauri 的 invoke / event 系统
// 需要 tauri.conf.json 里开启 app.withGlobalTauri = true

// Tauri 桥。任何一项缺失都不能让本文件加载失败 ——
// 否则 window.pulses 永远不会被定义，整个前端就废了。
const T = window.__TAURI__ || {};
const invoke = (T.core && typeof T.core.invoke === 'function')
  ? T.core.invoke
  : async () => null;
const listen = (T.event && typeof T.event.listen === 'function')
  ? T.event.listen
  : async () => () => {};
const dlg = (T.dialog && typeof T.dialog.open === 'function')
  ? T.dialog
  : { open: async () => null, save: async () => null };

async function onEvent(name, cb) {
  try { await listen(name, (e) => cb(e.payload)); } catch (_) {}
}

function parseJSON(text) {
  if (!text) return null;
  try { return JSON.parse(text); } catch (_) { return null; }
}

window.pulses = {
  // ── 数据库 ──
  db: {
    getPath:    () => invoke('db_get_path'),
    loadConfig: () => invoke('db_load_config'),
    saveConfig: (cfg) => invoke('db_save_config', { cfg }),
    create:     (p) => invoke('db_create', { path: p }),
    isValid:    (p) => invoke('db_is_valid', { path: p }),
    describe:   (p) => invoke('db_describe', { path: p }),
    migrate:    (src, dst, skip) => invoke('db_migrate', { src, dst, skip }),
    onProgress: (cb) => onEvent('db:progress', cb),
  },

  // ── 下载器 ──
  dl: {
    start:      (tasks, opts) => invoke('dl_start', { tasks, opts }),
    cancel:     () => invoke('dl_cancel'),
    onProgress: (cb) => onEvent('dl:progress', cb),
    onDone:     (cb) => onEvent('dl:done', cb),
    onLog:      (cb) => onEvent('dl:log', cb),
    diskType:   async () => ({ type: 'ssd', rotational: false }),
  },

  // ── 更新器 ──
  update: {
    buildPlan: (diff, checked, strategies, oldRoot, newRoot) =>
      invoke('update_build_plan', { diff, checked, strategies, oldRoot, newRoot }),
    execute:   (plan, oldRoot, newRoot) =>
      invoke('update_execute', { plan, oldRoot, newRoot }),
    verify:    (plan, oldRoot, newRoot) =>
      invoke('update_verify', { plan, oldRoot, newRoot }),
    estimate:  (plan, oldRoot) =>
      invoke('update_estimate', { plan, oldRoot }),
    onProgress: (cb) => onEvent('update:progress', cb),
    onDone:     (cb) => onEvent('update:done', cb),
  },

  // ── 更新包 ──
  pack: {
    readEntry:     (p, entry) => invoke('pack_read_entry', { path: p, entry }),
    export:        (opts) => invoke('pack_export', { opts }),
    onProgress:    (cb) => onEvent('pack:progress', cb),
    readSettings:  (p) => invoke('pack_read_entry', { path: p, entry: 'settings.json' }).then(parseJSON),
    readManifest:  (p) => invoke('pack_read_entry', { path: p, entry: 'manifest.json' }).then(parseJSON),
    readHashes:    (p) => invoke('pack_read_entry', { path: p, entry: 'hashes.json' }).then(parseJSON),
    readStrategies:(p) => invoke('pack_read_entry', { path: p, entry: 'strategies.json' }).then(parseJSON),
    readWhitelist: (p) => invoke('pack_read_entry', { path: p, entry: 'whitelist.json' }).then(parseJSON),
    readChangelog: (p) => invoke('pack_read_entry', { path: p, entry: 'CHANGELOG.md' }),
  },

  // ── 最近打开 ──
  recent: {
    load:   () => invoke('recent_load'),
    add:    (path) => invoke('recent_add', { path }),
    remove: (path) => invoke('recent_remove', { path }),
    clear:  () => invoke('recent_clear'),
  },

  // ── 差异比对 ──
  diff: {
    packs: (oldRoot, newRoot) => invoke('diff_packs', { oldRoot, newRoot }),
  },

  // ── 系统对话框（Tauri v2 dialog 插件，走 JS 侧）──
  dialog: {
    openFolder: async (title) => {
      const r = await dlg.open({ directory: true, multiple: false, title });
      return r || null;
    },
    openFile: async (title, filters) => {
      const r = await dlg.open({ directory: false, multiple: false, title, filters });
      return r || null;
    },
    saveFile: async (title, defaultPath) => {
      const r = await dlg.save({ title, defaultPath });
      return r || null;
    },
    openExternal: (url) => invoke('plugin:shell|open', { path: url, open: true }),
    showItem:     (p) => invoke('plugin:shell|open', { path: p, open: true }),
  },

  // ── 文件系统 ──
  files: {
    readDir:         (p) => invoke('fs_read_dir', { path: p }),
    stat:            (p) => invoke('fs_stat', { path: p }),
    exists:          (p) => invoke('fs_exists', { path: p }),
    readModpackInfo: (p) => invoke('fs_modpack_info', { path: p }),
    hashFile:        (p, algo) => invoke('fs_hash', { path: p, algo }),
    unzip:           (zipPath, destDir) => invoke('fs_unzip', { zipPath, destDir }),
    getSpace:        (p) => invoke('fs_space', { path: p }),
  },

  version: () => invoke('app_version'),
};
