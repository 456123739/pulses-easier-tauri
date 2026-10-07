// recent.js — 最近打开列表（对应 Python core/recent.py 的界面部分）
// 存 ~/.pulses_easier/recent.json，上限 8 条，去重，最新在前。

const Recent = {
  _items: [],
  _targets: [],       // [{el, onPick}]

  async load() {
    try { this._items = await window.pulses.recent.load() || []; }
    catch (_) { this._items = []; }
    return this._items;
  },

  async add(path) {
    try { this._items = await window.pulses.recent.add(path) || []; }
    catch (_) { /* 静默 */ }
    this.renderAll();
  },

  async clear() {
    try { this._items = await window.pulses.recent.clear() || []; }
    catch (_) { this._items = []; }
    this.renderAll();
  },

  // 注册一个渲染目标
  bind(el, onPick) {
    if (!el) return;
    this._targets.push({ el, onPick });
    this._render(el, onPick);
  },

  renderAll() {
    for (const t of this._targets) this._render(t.el, t.onPick);
  },

  _render(el, onPick) {
    el.innerHTML = '';
    if (!this._items.length) {
      const empty = document.createElement('div');
      empty.className = 'recent-empty';
      empty.textContent = '暂无记录';
      el.appendChild(empty);
      return;
    }
    this._items.forEach((p, i) => {
      const name = baseName(p);
      const btn = document.createElement('button');
      btn.className = 'recent-item';
      btn.style.animationDelay = (i * 30) + 'ms';

      const n = document.createElement('span');
      n.className = 'recent-item-name';
      n.textContent = name;

      const d = document.createElement('span');
      d.className = 'recent-item-path';
      d.textContent = parentDir(p);

      btn.appendChild(n);
      btn.appendChild(d);
      btn.title = p;
      btn.addEventListener('click', () => onPick && onPick(p));
      el.appendChild(btn);
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
