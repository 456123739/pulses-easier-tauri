// player.js — 玩家端逻辑
// 对应 Python: app/ui/player_view.py

const Player = {
  state: {
    modpackPath: null,
    isLocked: false,
    updateZip: null,
    diff: null,
    plan: null,
  },

  init() {
    // 拖入区
    const dz = document.getElementById('drop-zone');
    dz.addEventListener('click', () => this._pickPack());
    dz.addEventListener('dragover', e => { e.preventDefault(); dz.classList.add('dragover'); });
    dz.addEventListener('dragleave', () => dz.classList.remove('dragover'));
    dz.addEventListener('drop', e => {
      e.preventDefault();
      dz.classList.remove('dragover');
      const files = Array.from(e.dataTransfer.files);
      if (files.length > 0) this._onPackDropped(files[0].path);
    });

    // 定位按钮
    document.getElementById('btn-locate').addEventListener('click',
      () => this._locateModpack());

    // 开始更新按钮
    document.getElementById('btn-start').addEventListener('click',
      () => this._startUpdate());
  },

  async _locateModpack() {
    const p = await window.pulses.dialog.openFolder('选择整合包根目录');
    if (!p) return;
    this.state.modpackPath = p;
    const info = await window.pulses.files.readModpackInfo(p);
    document.getElementById('modpack-name').textContent =
      info.name + (info.version ? ' v' + info.version : '');
    document.getElementById('status-text').textContent = '已定位：' + info.name;
  },

  async _pickPack() {
    const p = await window.pulses.dialog.openFile('选择更新包', [
      { name: '更新包', extensions: ['zip', 'eapack'] },
    ]);
    if (!p) return;
    this._onPackDropped(p);
  },

  async _onPackDropped(packPath) {
    this.state.updateZip = packPath;
    document.getElementById('status-text').textContent =
      '已加载更新包：' + packPath.split(/[\\/]/).pop();

    // 读取更新包信息
    const settings = await window.pulses.pack.readSettings(packPath);
    const manifest = await window.pulses.pack.readManifest(packPath);
    const changelog = await window.pulses.pack.readChangelog(packPath);

    // 显示更新日志
    if (changelog) {
      const preview = document.getElementById('changelog-preview');
      const content = document.getElementById('changelog-content');
      content.textContent = changelog;
      preview.classList.remove('hidden');
      Motion.slideUp(preview);
    }

    // 读取整合包路径
    if (!this.state.modpackPath) {
      await this._locateModpack();
      if (!this.state.modpackPath) return;
    }

    // 解压更新包到临时目录
    const tmpDir = packPath.replace(/\.[^.]+$/, '') + '_extracted';
    const unzipResult = await window.pulses.files.unzip(packPath, tmpDir);
    if (!unzipResult.ok) {
      await Overlay.alert(null, '解压失败', unzipResult.msg, { level: 'error' });
      return;
    }

    // 比对
    const diff = await window.pulses.diff.packs(
      this.state.modpackPath, tmpDir, {});
    this.state.diff = diff;
    this._renderChangeList(diff);

    // 显示开始更新按钮
    const btn = document.getElementById('btn-start');
    btn.classList.remove('hidden');
    Motion.slideUp(btn);
  },

  _renderChangeList(diff) {
    const list = document.getElementById('change-list');
    list.innerHTML = '';

    const summary = document.createElement('div');
    summary.style.cssText = 'padding:8px 10px;font-size:var(--fs-small);color:var(--text-muted);border-bottom:1px solid var(--border);margin-bottom:4px;';
    summary.textContent = `变更：+${diff.added.length} ~${diff.modified.length} -${diff.deleted.length}`;
    list.appendChild(summary);

    let idx = 0;
    for (const f of diff.added) {
      this._addRow(list, f.rel, 'add', idx++);
    }
    for (const f of diff.modified) {
      this._addRow(list, f.rel, 'mod', idx++);
    }
    for (const f of diff.deleted) {
      this._addRow(list, f.rel, 'del', idx++);
    }
  },

  _addRow(parent, rel, type, idx) {
    const row = document.createElement('div');
    row.className = 'change-row';
    row.style.animationDelay = `${idx * 20}ms`;

    const badge = document.createElement('span');
    badge.className = 'change-badge ' + type;
    const labels = { add: '新增', mod: '修改', del: '删除' };
    badge.textContent = labels[type];

    const text = document.createElement('span');
    text.textContent = rel;
    text.style.cssText = 'flex:1;overflow:hidden;text-overflow:ellipsis;white-space:nowrap;';

    row.appendChild(badge);
    row.appendChild(text);
    parent.appendChild(row);

    // 长文本挂 tooltip
    if (rel.length > 30) {
      Tooltip.attach(row, rel);
    }
  },

  async _startUpdate() {
    if (!this.state.diff) return;
    const btn = document.getElementById('btn-start');
    btn.disabled = true;
    btn.textContent = '更新中…';

    const progressPanel = document.getElementById('progress-panel');
    progressPanel.classList.remove('hidden');
    const bar = document.getElementById('progress-bar');
    const ptext = document.getElementById('progress-text');

    // 监听进度
    window.pulses.update.onProgress(d => {
      bar.style.width = (d.percent * 100).toFixed(1) + '%';
      ptext.textContent = `${d.done} / ${d.total}` + (d.moved ? `  ${d.moved} bytes` : '');
    });

    const oldRoot = this.state.modpackPath;
    const newRoot = this.state.updateZip.replace(/\.[^.]+$/, '') + '_extracted';

    // 构建计划
    const plan = await window.pulses.update.buildPlan(
      this.state.diff, { all: true }, {}, oldRoot, newRoot, {});
    this.state.plan = plan;

    // 执行
    const result = await window.pulses.update.execute(plan, oldRoot, newRoot, {});
    if (result.ok) {
      bar.style.width = '100%';
      ptext.textContent = '更新完成 ✓';
      btn.textContent = '完成';
      btn.disabled = false;
      btn.classList.add('hidden');

      // 提示拖入下一个更新包
      document.getElementById('drop-zone').querySelector('.drop-text')
        .textContent = '本次更新已全部应用 ✓\n请拖入下一个更新包';
      this.state.updateZip = null;
      this.state.diff = null;

      setTimeout(() => progressPanel.classList.add('hidden'), 2000);
    } else {
      ptext.textContent = '更新失败';
      btn.disabled = false;
      btn.textContent = '重试';
      await Overlay.alert(null, '更新失败', result.msg || '未知错误', { level: 'error' });
    }
  },
};

window.Player = Player;
