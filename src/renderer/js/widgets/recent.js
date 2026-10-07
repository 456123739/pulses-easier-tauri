// recent.js — 最近打开列表（对应 Python core/recent.py 的界面部分）
// 存 ~/.pulses_easier/recent.json，上限 8 条，去重，最新在前。

const Recent = {
  _items: [],
  _targets: [],       // [{el, onPick}]

  async load() {
    try { this._items = (await window.pulses.recent.load()) || []; }
    catch (_) { this._items = []; }
    return this._items;
  },

  async add(path) {
    try { this._items = (await window.pulses.recent.add(path)) || []; }
    catch (_) { /* 静默 */ }
    this.renderAll();
  },

  async clear() {
    try { this._items = (await window.pulses.recent.clear()) || []; }
    catch (_) { this._items = []; }
    this.renderAll();
  },

  bind(el, onPick, maxItems) {
    if (!el) return;
    this._targets.push({ el, onPick, maxItems });
    this._render(el, onPick, maxItems);
  },

  renderAll() {
    for (const t of this._targets) this._render(t.el, t.onPick, t.maxItems);
  },

  _render(el, onPick, maxItems) {
    el.innerHTML = '';
    const items = maxItems ? this._items.slice(0, maxItems) : this._items;
    if (!items.length) {
      const empty = document.createElement('div');
      empty.className = 'list-empty';
      empty.textContent = '暂无记录';
      el.appendChild(empty);
      return;
    }
    items.forEach((p, i) => {
      const btn = document.createElement('button');
      btn.className = 'list-item';
      btn.style.animation = `softUp 260ms var(--ease-out) ${i * 28}ms both`;

      const name = document.createElement('span');
      name.className = 'li-name';
      name.textContent = baseName(p);

      const dir = document.createElement('span');
      dir.className = 'li-path';
      dir.textContent = parentDir(p);

      btn.appendChild(name);
      btn.appendChild(dir);
      btn.title = p;
      btn.addEventListener('click', () => onPick && onPick(p));
      el.appendChild(btn);

      if (String(p).length > 26) Tooltip.attach(btn, p);
    });
  },
};

function baseName(p) {
  const parts = String(p).split(/[\\/]/).filter(Boolean);
  return parts.length ? parts[parts.length - 1] : String(p);
}

function parentDir(p) {
  const parts = String(p).split(/[\\/]/).filter(Boolean);
  if (parts.length <= 1) return '';
  return parts.slice(0, -1).join('\\');
}

window.Recent = Recent;
