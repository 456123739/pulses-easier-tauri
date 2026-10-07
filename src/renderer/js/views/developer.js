// developer.js — 开发者端（STEP 1 / STEP 2 / STEP 3）
// 对应 Python: app/ui/developer_view.py
//   STEP 1 = 新版本整合包   STEP 2 = 旧版本整合包   STEP 3 = 比对并导出

const Developer = {
  state: {
    newPath: null,      // 新版本
    oldPath: null,      // 旧版本
    diff: null,
    busy: false,
  },

  init() {
    document.getElementById('btn-pick-newpack')
      .addEventListener('click', () => this.pickNew());
    document.getElementById('btn-pick-oldpack')
      .addEventListener('click', () => this.pickOld());
    document.getElementById('btn-build')
      .addEventListener('click', () => this.buildAndExport());

    // 开发者端的「已选过的」列表
    Recent.bind(document.getElementById('dev-recent'),
                p => this.setNew(p));
  },

  // ── STEP 1：新版本 ──
  async pickNew() {
    const p = await window.pulses.dialog.openFolder('选择新版本整合包文件夹');
    if (!p) return;
    await this.setNew(p);
  },

  async setNew(path) {
    this.state.newPath = path;
    const info = await this._info(path);
    document.getElementById('newpack-picked').classList.remove('is-hidden');
    document.getElementById('newpack-name').textContent = info.title;
    document.getElementById('newpack-meta').textContent = info.meta;
    Log.debug('新版本：' + info.title);
    await Recent.add(path);
    this._refresh();
  },

  // ── STEP 2：旧版本 ──
  async pickOld() {
    const p = await window.pulses.dialog.openFolder('选择旧版本整合包文件夹');
    if (!p) return;
    await this.setOld(p);
  },

  async setOld(path) {
    this.state.oldPath = path;
    const info = await this._info(path);
    document.getElementById('oldpack-picked').classList.remove('is-hidden');
    document.getElementById('oldpack-name').textContent = info.title;
    document.getElementById('oldpack-meta').textContent = info.meta;
    Log.debug('旧版本：' + info.title);
    this._refresh();
  },

  async _info(path) {
    let info = { name: baseName(path), version: '', mods: 0 };
    try { info = await window.pulses.files.readModpackInfo(path) || info; }
    catch (_) { /* 默认值 */ }
    return {
      title: info.name + (info.version ? '  v' + info.version : ''),
      meta: path + (info.mods ? `\n${info.mods} 个 mod` : ''),
    };
  },

  _refresh() {
    const btn = document.getElementById('btn-build');
    const ready = !!this.state.newPath && !!this.state.oldPath && !this.state.busy;
    btn.disabled = !ready;
    if (ready) setBtnLabel(btn, '比对并导出更新包');
  },

  // ── STEP 3：比对 + 导出 ──
  async buildAndExport() {
    if (this.state.busy) return;
    this.state.busy = true;
    const btn = document.getElementById('btn-build');
    btn.disabled = true;
    setBtnLabel(btn, '比对中…');

    try {
      Log.debug('比对两个版本…');
      const diff = await window.pulses.diff.packs(this.state.oldPath,
                                                  this.state.newPath);
      this.state.diff = diff;

      const nAdd = (diff.added || []).length;
      const nMod = (diff.modified || []).length;
      const nDel = (diff.deleted || []).length;

      const box = document.getElementById('diff-summary');
      box.innerHTML = '';
      box.classList.remove('is-hidden');
      for (const [cls, label, n] of [
        ['add', '新增', nAdd], ['mod', '修改', nMod], ['del', '删除', nDel],
      ]) {
        const chip = document.createElement('span');
        chip.className = 'diff-chip ' + cls;
        chip.textContent = `${label} ${n}`;
        box.appendChild(chip);
      }
      Log.ok(`比对完成：新增 ${nAdd} · 修改 ${nMod} · 删除 ${nDel}`);

      if (nAdd + nMod + nDel === 0) {
        Log.warn('两个版本没有差异，无需导出');
        setBtnLabel(btn, '无差异');
        btn.disabled = false;
        return;
      }

      // 选保存位置
      const out = await window.pulses.dialog.saveFile('导出更新包', 'update.eapack');
      if (!out) {
        Log.debug('已取消导出');
        setBtnLabel(btn, '比对并导出更新包');
        btn.disabled = false;
        return;
      }

      setBtnLabel(btn, '打包中…');
      Log.debug('打包更新包…');

      const changelog = document.getElementById('changelog-input').value || '';
      const files = [...(diff.added || []), ...(diff.modified || [])]
        .map(f => f.rel);

      const res = await window.pulses.pack.export({
        root: this.state.newPath,
        output: out,
        manifest: {
          version: '1.0',
          created: new Date().toISOString(),
          files: files.length,
        },
        settings: { source: this.state.newPath },
        changelog,
        files,
      });

      if (res && res.ok) {
        Log.ok(`导出成功：${res.path}`);
        Log.debug(`大小：${(res.size / 1048576).toFixed(2)} MB`);
        App.setStatus('已导出更新包');
        await Overlay.alert('导出成功',
          `已保存到：\n${res.path}\n\n大小：${(res.size / 1048576).toFixed(2)} MB`);
        setBtnLabel(btn, '导出完成');
      } else {
        Log.error('导出失败：' + ((res && res.msg) || '未知错误'));
        await Overlay.alert('导出失败', (res && res.msg) || '未知错误',
                            { level: 'error' });
        setBtnLabel(btn, '比对并导出更新包');
        btn.disabled = false;
      }
    } catch (e) {
      Log.error('异常：' + e);
      setBtnLabel(btn, '比对并导出更新包');
      btn.disabled = false;
    } finally {
      this.state.busy = false;
    }
  },
};

window.Developer = Developer;
