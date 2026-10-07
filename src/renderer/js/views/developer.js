// developer.js — 开发者端逻辑
// 对应 Python: app/ui/developer_view.py

const Developer = {
  state: {
    sourcePath: null,
    targetPath: null,
    diff: null,
  },

  init() {
    document.getElementById('btn-select-source').addEventListener('click',
      () => this._pickSource());
    document.getElementById('btn-select-target').addEventListener('click',
      () => this._pickTarget());
    document.getElementById('btn-compare').addEventListener('click',
      () => this._compare());
    document.getElementById('btn-export').addEventListener('click',
      () => this._export());
  },

  async _pickSource() {
    const p = await window.pulses.dialog.openFolder('选择源整合包');
    if (!p) return;
    this.state.sourcePath = p;
    const info = await window.pulses.files.readModpackInfo(p);
    document.getElementById('source-name').textContent =
      info.name + (info.version ? ' v' + info.version : '');
  },

  async _pickTarget() {
    const p = await window.pulses.dialog.openFolder('选择目标整合包');
    if (!p) return;
    this.state.targetPath = p;
    const info = await window.pulses.files.readModpackInfo(p);
    document.getElementById('target-name').textContent =
      info.name + (info.version ? ' v' + info.version : '');
  },

  async _compare() {
    if (!this.state.sourcePath || !this.state.targetPath) {
      await Overlay.alert(null, '提示', '请先选择源和目标整合包');
      return;
    }
    const diff = await window.pulses.diff.packs(
      this.state.sourcePath, this.state.targetPath, {});
    this.state.diff = diff;
    this._renderDiff(diff);
    document.getElementById('btn-export').classList.remove('hidden');
    document.getElementById('changelog-editor').classList.remove('hidden');
    Motion.slideUp(document.getElementById('btn-export'));
  },

  _renderDiff(diff) {
    const list = document.getElementById('dev-change-list');
    list.innerHTML = '';

    const summary = document.createElement('div');
    summary.style.cssText = 'padding:8px 10px;font-size:var(--fs-small);color:var(--text-muted);';
    summary.textContent = `差异：+${diff.added.length} ~${diff.modified.length} -${diff.deleted.length}`;
    list.appendChild(summary);

    for (const f of [...diff.added, ...diff.modified, ...diff.deleted]) {
      const row = document.createElement('div');
      row.className = 'change-row';
      row.textContent = f.rel;
      list.appendChild(row);
    }
  },

  async _export() {
    const savePath = await window.pulses.dialog.saveFile('导出更新包', 'update.eapack');
    if (!savePath) return;

    const changelog = document.getElementById('changelog-input').value;
    const result = await window.pulses.pack.export({
      root: this.state.sourcePath,
      manifest: { version: '1.0', created: new Date().toISOString() },
      changelog,
      files: [...(this.state.diff?.added || []), ...(this.state.diff?.modified || [])]
        .map(f => f.rel),
    });

    if (result.ok) {
      await Overlay.alert(null, '导出成功',
        `已保存到：${result.path}\n大小：${(result.size / 1048576).toFixed(1)} MB`);
    } else {
      await Overlay.alert(null, '导出失败', result.msg, { level: 'error' });
    }
  },
};

window.Developer = Developer;
