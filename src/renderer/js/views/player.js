// player.js — 玩家端（STEP 1 / STEP 2 / STEP 3）
// 对应 Python: app/ui/player_view.py + sidebar 的定位区

// ── 剥掉更新包多余的顶层文件夹 ──
// 7-Zip、资源管理器「压缩到文件夹」都会多套一层（PackName/mods/...）。
// 不剥的话两边路径完全没有交集，比对结果是
//   「新增 N · 修改 0 · 删除 M」   ← 修改 0 就是特征
// 然后 M 个玩家文件会被判成删除、真的删掉。
//
// 判定规则（宁可不动，不能乱剥）：
//   · 目录里只有 1 个子目录、且没有其它文件
//   · 这个子目录里确实有「整合包根该有的东西」
// 这样单一个 mods/ 不会被误当成根。
const PACK_ROOT_MARKERS = [
  'mods', 'config', 'resourcepacks', 'shaderpacks',
  'options.txt', 'manifest.json', 'instance.cfg', 'mmc-pack.json',
];

async function unwrapPackRoot(dir, depth = 0) {
  if (depth >= 3) return dir;

  let names = [];
  try { names = await window.pulses.files.readDir(dir) || []; }
  catch (_) { return dir; }
  if (!Array.isArray(names) || names.length !== 1) return dir;

  const child = joinPath(dir, names[0]);
  let st = null;
  try { st = await window.pulses.files.stat(child); }
  catch (_) { return dir; }
  if (!st || !st.isDir) return dir;

  let inner = [];
  try { inner = await window.pulses.files.readDir(child) || []; }
  catch (_) { return dir; }
  if (!Array.isArray(inner)) return dir;
  if (!inner.some(n => PACK_ROOT_MARKERS.includes(n))) return dir;

  return unwrapPackRoot(child, depth + 1);
}

const Player = {
  state: {
    modpackPath: null,
    updateZip: null,
    extractedDir: null,
    diff: null,
    busy: false,
  },

  init() {
    // STEP 1：选择整合包文件夹
    document.getElementById('btn-pick-modpack')
      .addEventListener('click', () => this.pickModpack());

    // STEP 2：拖入 / 点击选择（更新包和整合包都往这儿扔，自动认）
    //
    // HTML5 的 drop 事件在 Tauri 里收不到：窗口默认 dragDropEnabled=true，
    // 拖放被 Tauri 接管了。所以走原生事件 —— 好处是能拿到真实文件路径。
    const dz = document.getElementById('drop-zone');
    if (dz) dz.addEventListener('click', () => this.pickPack());
    this._initDrop();

    document.getElementById('btn-clear-pack')
      .addEventListener('click', () => this.clearPack());

    // STEP 3：开始更新
    document.getElementById('btn-start')
      .addEventListener('click', () => this.startUpdate());

    // 最近打开（STEP 1 内）
    Recent.bind(document.getElementById('step1-recent'),
                p => this.setModpack(p), 5);

    // 模组列表先摆好空概要（三色计数），加载更新包后再填行
    this._renderChanges({ added: [], modified: [], deleted: [], unchanged: [] }, []);
  },

  // ── STEP 1 ──
  async pickModpack() {
    const p = await window.pulses.dialog.openFolder('选择整合包文件夹');
    if (!p) return;
    await this.setModpack(p);
  },

  async setModpack(path) {
    // 重复点同一个文件夹（最常见的是在「最近」里连点）：直接不做任何反应。
    // 否则会重复读整合包信息、重复写日志、重复 Recent.add → 列表重渲染 + 动画重载。
    if (path && path === this.state.modpackPath) return;

    this.state.modpackPath = path;
    let info = { name: baseName(path), version: '', mods: 0 };
    try { info = await window.pulses.files.readModpackInfo(path) || info; }
    catch (_) { /* 用默认值 */ }

    const title = info.name + (info.version ? '  v' + info.version : '');

    // STEP 1 已选信息
    document.getElementById('modpack-picked').classList.remove('is-hidden');
    document.getElementById('modpack-name').textContent = title;
    document.getElementById('modpack-meta').textContent =
      path + (info.mods ? `\n${info.mods} 个 mod` : '');

    // 侧边栏「当前整合包」
    document.getElementById('sb-modpack-name').textContent = title;
    document.getElementById('sb-modpack-meta').textContent =
      path + (info.mods ? ` · ${info.mods} mod` : '');

    App.setStatus('已定位整合包：' + title);
    Log.ok('已定位整合包：' + title);

    await Recent.add(path);
    await this._checkRecovery(path);
    this._refreshSteps();
  },

  // ── 拖放 ──
  _initDrop() {
    if (typeof window.pulses.fileDrop !== 'function') return;
    const dz = document.getElementById('drop-zone');
    window.pulses.fileDrop(({ type, paths, x, y }) => {
      if (type === 'leave') {
        if (dz) dz.classList.remove('is-over');
        return;
      }
      if (type === 'enter' || type === 'over') {
        if (dz) dz.classList.toggle('is-over', this._hitDrop(x, y));
        return;
      }
      if (type === 'drop') {
        if (dz) dz.classList.remove('is-over');
        const p = paths && paths[0];
        if (p) this.acceptFile(p);
      }
    });
  },

  // 鼠标是不是落在投放区上（只用来点亮高亮，落哪儿都认）
  _hitDrop(x, y) {
    const dz = document.getElementById('drop-zone');
    if (!dz || x < 0 || y < 0) return false;
    let r = null;
    try { r = dz.getBoundingClientRect(); } catch (_) { return false; }
    if (!r) return false;
    return x >= r.left && x <= r.right && y >= r.top && y <= r.bottom;
  },

  // 定位整合包时顺手做一次自检：上次更新有没有留残渣、有没有没下完的文件
  async _checkRecovery(path) {
    try {
      const left = await window.pulses.pack.leftovers(path);
      const n = (left.dirs || []).length + (left.files || []).length;
      if (n > 0) {
        const r = await window.pulses.pack.recover(path);
        if (r && (r.restored || r.removed)) {
          Log.warn(`上次更新没收尾：回滚 ${r.restored} 个目录替换，清掉 ${r.removed} 个临时文件`);
        }
      }
    } catch (_) { /* 自检失败不影响定位 */ }

    try {
      const m = await window.pulses.pack.markerCheck(path);
      if (m && m.ok && !m.all_done) {
        Log.warn(`这个整合包上次有 ${m.remaining.length} 个文件没下载完`);
        Log.warn('补齐后会自动清理凭证，详情见根目录的「!!!更新未完成-请阅读!!!.txt」');
      } else if (m && m.ok && m.all_done && m.total > 0) {
        await window.pulses.pack.markerClear(path);
        Log.ok('上次没下完的文件已补齐，凭证已清理');
      }
    } catch (_) {}
  },

  // ── STEP 2 ──
  async pickPack() {
    const p = await window.pulses.dialog.openFile('选择更新包或整合包', [
      { name: '更新包 / 整合包', extensions: ['zip', 'eapack', 'mrpack'] },
    ]);
    if (!p) return;
    await this.acceptFile(p);
  },

  // 自动识别：整合包 → 安装流程；更新包 → 原来的比对流程
  //
  // 之前不认就一律当更新包，把 CF 整合包拖进来会去跟玩家整合包做全量比对
  // （整棵树遍历 + 每个文件算 sha1），主线程被占满，窗口直接「未响应」。
  async acceptFile(p) {
    if (this.state.busy) {
      Log.warn('正在处理上一个文件，请稍候');
      return;
    }

    let k = null;
    try { k = await window.pulses.pack.kind(p); } catch (_) {}

    if (k && k.ok && k.kind === 'modpack') {
      const fmt = window.formatLabel ? formatLabel(k.format) : '整合包';
      Log.ok(`识别为${fmt}，转到安装`);
      App.setStatus('识别为' + fmt);
      if (window.Installer) await Installer.startWith(p, k);
      return;
    }

    if (k && k.ok) {
      Log.debug(`识别为${formatLabel(k.format)}`);
    }
    await this.onPackDropped(p);
  },

  async onPackDropped(packPath) {
    if (this.state.busy) {
      Log.warn('正在处理上一个更新包，请稍候');
      return;
    }
    if (!this.state.modpackPath) {
      Log.warn('请先在 STEP 1 选择整合包文件夹');
      App.setStatus('请先选择整合包文件夹（STEP 1）');
      return;
    }

    this.state.updateZip = packPath;
    document.getElementById('pack-name').textContent = baseName(packPath);
    document.getElementById('btn-clear-pack').classList.remove('is-hidden');
    Log.ok('已加载更新包：' + baseName(packPath));

    // 解压 → 比对
    const out = packPath.replace(/\.[^.]+$/, '') + '_pulses_extract';
    // 解压前先清掉上一次的残留 —— 否则旧文件会被当成「新增」拷进玩家整合包
    try { await window.pulses.pack.cleanExtract(out); } catch (_) { /* 忽略 */ }
    Log.debug('解压更新包…');
    const unzip = await window.pulses.files.unzip(packPath, out);
    if (!unzip || !unzip.ok) {
      Log.error('解压失败：' + (unzip && unzip.msg ? unzip.msg : '未知错误'));
      await Overlay.alert('解压失败', (unzip && unzip.msg) || '未知错误',
                          { level: 'error' });
      return;
    }
    Log.debug(`解压完成（${unzip.files} 个条目）`);

    // 剥掉多余的顶层文件夹（7-Zip 会多套一层，不剥会把玩家文件全判成删除）
    const root = await unwrapPackRoot(out);
    if (root !== out) {
      Log.warn('更新包多套了一层目录，已自动剥到：' + baseName(root));
    }
    this.state.extractedDir = root;

    Log.debug('比对差异…');
    // 更新包是增量的：旧包里没被带到的文件不能判删除
    const diff = await window.pulses.diff.packs(this.state.modpackPath, root, true);
    this.state.diff = diff;

    const nAdd = (diff.added || []).length;
    const nMod = (diff.modified || []).length;
    const nDel = (diff.deleted || []).length;
    Log.ok(`比对完成：新增 ${nAdd} · 修改 ${nMod} · 删除 ${nDel}`);

    // 预览「智能剔除」会放过哪些配置：本地已经有、且属于优化/渲染类模组
    this.state.smart = new Set();
    try {
      const rels = (diff.modified || []).map(f => f.rel).filter(Boolean);
      const r = await window.pulses.update.smartCheck(rels, this.state.modpackPath);
      if (r && r.skipped) {
        this.state.smart = new Set(r.skipped);
        if (r.summary) {
          Log.ok(`config 使用「智能剔除」：${r.summary} 保持你本地的版本`);
        }
      }
    } catch (_) { /* 预览失败不影响更新 */ }

    this._renderChanges(diff);

    // 更新日志预览
    try {
      const md = await window.pulses.pack.readChangelog(packPath);
      if (md) Log.debug('更新日志：' + firstLine(md));
    } catch (_) { /* 没有日志也正常 */ }

    this._refreshSteps();
  },

  clearPack() {
    this.state.updateZip = null;
    this.state.extractedDir = null;
    this.state.diff = null;
    this.state.smart = new Set();
    document.getElementById('pack-name').textContent = '';
    document.getElementById('btn-clear-pack').classList.add('is-hidden');
    this._renderChanges({ added: [], modified: [], deleted: [], unchanged: [] }, []);
    Log.debug('已清空更新包');
    this._refreshSteps();
  },

  // ── 变更列表 ──
  // 配色按需求：绿色=没变、紫色=变更、红色=出错
  _renderChanges(diff, errors) {
    const box = document.getElementById('change-list');
    if (!box) return;
    const smart = this.state.smart || new Set();

    const add = diff.added || [];
    const mod = diff.modified || [];
    const del = diff.deleted || [];
    const same = diff.unchanged || [];
    const errs = errors || this._errors || [];

    box.innerHTML = '';

    // 概要三色
    const sum = document.createElement('div');
    sum.className = 'change-sum';
    const chip = (cls, label, n) => {
      const c = document.createElement('span');
      c.className = 'change-chip ' + cls;
      c.textContent = `${label} ${n}`;
      return c;
    };
    sum.appendChild(chip('unchanged', '没变', same.length));
    sum.appendChild(chip('changed', '变更', add.length + mod.length + del.length));
    sum.appendChild(chip('error', '出错', errs.length));
    box.appendChild(sum);

    const MAX = 300;
    let n = 0;

    // 出错项优先（红色）
    for (const e of errs) {
      if (n >= MAX) break;
      const row = document.createElement('div');
      row.className = 'change-row';
      const b = document.createElement('span');
      b.className = 'change-badge error';
      b.textContent = '出错';
      const t = document.createElement('span');
      t.className = 'change-name';
      t.textContent = e.rel || e;
      t.title = t.textContent;
      row.appendChild(b);
      row.appendChild(t);
      box.appendChild(row);
      n++;
    }

    // 变更项（紫色）；被智能剔除的再加一个标记，不展开也看得见
    const push = (list, label) => {
      for (const f of list) {
        if (n >= MAX) return;
        const row = document.createElement('div');
        row.className = 'change-row';
        row.style.animationDelay = Math.min(n * 6, 260) + 'ms';
        const b = document.createElement('span');
        b.className = 'change-badge changed';
        b.textContent = label;
        const t = document.createElement('span');
        t.className = 'change-name';
        t.textContent = f.rel;
        t.title = f.rel;
        row.appendChild(b);
        row.appendChild(t);
        if (smart.has(f.rel)) {
          const s = document.createElement('span');
          s.className = 'change-badge smart';
          s.textContent = '智能剔除';
          row.appendChild(s);
        }
        box.appendChild(row);
        n++;
      }
    };
    push(add, '新增');
    push(mod, '修改');
    push(del, '删除');

    const total = add.length + mod.length + del.length + errs.length;
    if (total > MAX) {
      const more = document.createElement('div');
      more.className = 'change-sum';
      more.textContent = `… 还有 ${total - MAX} 项`;
      box.appendChild(more);
    }

  },

  // ── STEP 3 ──
  _refreshSteps() {
    const btn = document.getElementById('btn-start');
    const ready = !!this.state.modpackPath && !!this.state.diff && !this.state.busy;
    btn.disabled = !ready;
    if (ready) setBtnLabel(btn, '开始更新');
  },

  async startUpdate() {
    if (this.state.busy) return;

    // 上一次更新成功后 clearPack() 会把 extractedDir 置空。
    // 这时再点「重试」就会把 null 传给 Rust，报
    //   invalid args `newRoot` ... invalid type: null, expected a string
    const oldRoot = this.state.modpackPath;
    const newRoot = this.state.extractedDir;
    if (!oldRoot || !newRoot) {
      Log.warn('更新包已清空，请重新拖入更新包');
      App.setStatus('请先拖入更新包（STEP 2）');
      return;
    }

    this.state.busy = true;
    this._errors = [];

    const btn = document.getElementById('btn-start');
    btn.disabled = true;
    setBtnLabel(btn, '更新中…');

    const panel = document.getElementById('progress-panel');
    panel.classList.remove('is-hidden');
    const fill = document.getElementById('progress-fill');
    const ptext = document.getElementById('progress-text');

    window.pulses.update.onProgress(d => {
      if (d.error) {
        this._errors.push({ rel: d.error });
        if (this.state.diff) this._renderChanges(this.state.diff, this._errors);
        return;
      }
      const pct = ((d.percent || 0) * 100).toFixed(1);
      fill.style.width = pct + '%';
      ptext.textContent = `已处理 ${d.done}/${d.total}  ·  ${pct}%`;
    });

    try {
      Log.debug('构建更新计划…');
      const plan = await window.pulses.update.buildPlan(
        this.state.diff, { all: true }, {}, oldRoot, newRoot);
      const total = (plan.tasks || []).length;
      Log.debug(`计划：${total} 个文件`);

      Log.debug('开始应用更新…');
      try { await window.pulses.update.checkpoint(oldRoot, 'apply', 0, total); } catch (_) {}
      const res = await window.pulses.update.execute(plan, oldRoot, newRoot);

      if (res && res.ok) {
        fill.style.width = '100%';
        ptext.textContent = '更新完成 ✓';
        Log.ok(`更新完成：${res.done} 个文件已处理`);
        App.setStatus('更新完成');

        try { await window.pulses.update.checkpoint(oldRoot, 'done', total, total); } catch (_) {}

        // 解压目录用完就删，不占磁盘
        try { await window.pulses.pack.cleanExtract(newRoot); } catch (_) { /* 忽略 */ }

        // 回到「请拖入下一个更新包」
        // 取元素必须判空：之前 .drop-sub 在 HTML 里根本不存在，
        // querySelector 返回 null，赋值直接抛
        //   TypeError: Cannot set properties of null (setting 'textContent')
        this.clearPack();
        const dz = document.getElementById('drop-zone');
        const dzMain = dz && dz.querySelector('.drop-main');
        const dzSub = dz && dz.querySelector('.drop-sub');
        if (dzMain) dzMain.textContent = '本次更新已全部应用 ✓';
        if (dzSub) dzSub.textContent = '请拖入下一个更新包';
        setBtnLabel(btn, '已完成');
        btn.classList.add('is-hidden');

        setTimeout(() => panel.classList.add('is-hidden'), 1800);
      } else {
        Log.error('更新失败');
        // 失败的文件写进凭证，玩家下次定位就能看到还差什么
        try {
          const missing = (res && res.failed ? res.failed : []).map(e => ({
            rel: e.rel, size: 0, sha1: '', urls: [],
          }));
          if (missing.length) {
            await window.pulses.pack.markerWrite(oldRoot, {
              packName: baseName(oldRoot), version: '', zip: this.state.updateZip || '',
              fingerprint: '', cacheRoot: '', missing,
            });
          }
        } catch (_) {}
        ptext.textContent = '更新失败';
        setBtnLabel(btn, '重试');
        btn.disabled = false;
      }
    } catch (e) {
      Log.error('更新异常：' + e);
      setBtnLabel(btn, '重试');
      btn.disabled = false;
    } finally {
      this.state.busy = false;
    }
  },
};

function firstLine(s) {
  const line = String(s).split('\n').find(l => l.trim());
  return line ? line.trim().slice(0, 80) : '';
}

window.Player = Player;
window.unwrapPackRoot = unwrapPackRoot;
