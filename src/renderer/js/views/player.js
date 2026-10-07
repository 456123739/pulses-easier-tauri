// player.js — 玩家端（STEP 1 / STEP 2 / STEP 3）
// 对应 Python: app/ui/player_view.py + sidebar 的定位区

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

    // STEP 2：拖入 / 点击选择更新包
    const dz = document.getElementById('drop-zone');
    dz.addEventListener('click', () => this.pickPack());
    dz.addEventListener('dragover', e => {
      e.preventDefault();
      dz.classList.add('is-over');
    });
    dz.addEventListener('dragleave', () => dz.classList.remove('is-over'));
    dz.addEventListener('drop', e => {
      e.preventDefault();
      dz.classList.remove('is-over');
      const f = e.dataTransfer.files[0];
      if (f && f.path) this.onPackDropped(f.path);
    });

    document.getElementById('btn-clear-pack')
      .addEventListener('click', () => this.clearPack());

    // STEP 3：开始更新
    document.getElementById('btn-start')
      .addEventListener('click', () => this.startUpdate());

    // 最近打开（STEP 1 内）
    Recent.bind(document.getElementById('step1-recent'),
                p => this.setModpack(p), 5);
  },

  // ── STEP 1 ──
  async pickModpack() {
    const p = await window.pulses.dialog.openFolder('选择整合包文件夹');
    if (!p) return;
    await this.setModpack(p);
  },

  async setModpack(path) {
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
    this._refreshSteps();
  },

  // ── STEP 2 ──
  async pickPack() {
    const p = await window.pulses.dialog.openFile('选择更新包', [
      { name: '更新包', extensions: ['zip', 'eapack'] },
    ]);
    if (!p) return;
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
    Log.info('已加载更新包：' + baseName(packPath));

    // 解压 → 比对
    const out = packPath.replace(/\.[^.]+$/, '') + '_pulses_extract';
    Log.info('解压更新包…');
    const unzip = await window.pulses.files.unzip(packPath, out);
    if (!unzip || !unzip.ok) {
      Log.error('解压失败：' + (unzip && unzip.msg ? unzip.msg : '未知错误'));
      await Overlay.alert('解压失败', (unzip && unzip.msg) || '未知错误',
                          { level: 'error' });
      return;
    }
    this.state.extractedDir = out;
    Log.ok(`解压完成（${unzip.files} 个条目）`);

    Log.info('比对差异…');
    const diff = await window.pulses.diff.packs(this.state.modpackPath, out);
    this.state.diff = diff;

    const nAdd = (diff.added || []).length;
    const nMod = (diff.modified || []).length;
    const nDel = (diff.deleted || []).length;
    Log.ok(`比对完成：新增 ${nAdd} · 修改 ${nMod} · 删除 ${nDel}`);

    // 更新日志预览
    try {
      const md = await window.pulses.pack.readChangelog(packPath);
      if (md) Log.info('更新日志：' + firstLine(md));
    } catch (_) { /* 没有日志也正常 */ }

    this._refreshSteps();
  },

  clearPack() {
    this.state.updateZip = null;
    this.state.extractedDir = null;
    this.state.diff = null;
    document.getElementById('pack-name').textContent = '';
    document.getElementById('btn-clear-pack').classList.add('is-hidden');
    Log.info('已清空更新包');
    this._refreshSteps();
  },

  // ── STEP 3 ──
  _refreshSteps() {
    const btn = document.getElementById('btn-start');
    const ready = !!this.state.modpackPath && !!this.state.diff && !this.state.busy;
    btn.disabled = !ready;
    btn.classList.toggle('is-ready', ready);
    if (ready) setBtnLabel(btn, '开始更新');
  },

  async startUpdate() {
    if (this.state.busy) return;
    this.state.busy = true;

    const btn = document.getElementById('btn-start');
    btn.disabled = true;
    btn.classList.remove('is-ready');
    setBtnLabel(btn, '更新中…');

    const panel = document.getElementById('progress-panel');
    panel.classList.remove('is-hidden');
    const fill = document.getElementById('progress-fill');
    const ptext = document.getElementById('progress-text');

    window.pulses.update.onProgress(d => {
      const pct = ((d.percent || 0) * 100).toFixed(1);
      fill.style.width = pct + '%';
      ptext.textContent = `已处理 ${d.done}/${d.total}  ·  ${pct}%`;
    });

    const oldRoot = this.state.modpackPath;
    const newRoot = this.state.extractedDir;

    try {
      Log.info('构建更新计划…');
      const plan = await window.pulses.update.buildPlan(
        this.state.diff, { all: true }, {}, oldRoot, newRoot);
      const total = (plan.tasks || []).length;
      Log.info(`计划：${total} 个文件`);

      Log.info('开始应用更新…');
      const res = await window.pulses.update.execute(plan, oldRoot, newRoot);

      if (res && res.ok) {
        fill.style.width = '100%';
        ptext.textContent = '更新完成 ✓';
        Log.ok(`更新完成：${res.done} 个文件已处理`);
        App.setStatus('更新完成');

        // 回到「请拖入下一个更新包」
        this.clearPack();
        document.getElementById('drop-zone').querySelector('.drop-main')
          .textContent = '本次更新已全部应用 ✓';
        document.getElementById('drop-zone').querySelector('.drop-sub')
          .textContent = '请拖入下一个更新包';
        setBtnLabel(btn, '已完成');
        btn.classList.add('is-hidden');

        setTimeout(() => panel.classList.add('is-hidden'), 1800);
      } else {
        Log.error('更新失败');
        ptext.textContent = '更新失败';
        setBtnLabel(btn, '重试');
        btn.disabled = false;
        btn.classList.add('is-ready');
      }
    } catch (e) {
      Log.error('更新异常：' + e);
      setBtnLabel(btn, '重试');
      btn.disabled = false;
      btn.classList.add('is-ready');
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
