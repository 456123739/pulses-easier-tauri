// recent.js — 最近打开列表（对应 Python core/recent.py 的界面部分）
// 存 ~/.pulses_easier/recent.json，上限 8 条，去重，最新在前。

// 路径自动滚动的总开关（设置里可关）
const Marquee = {
  _on: true,
  enabled() { return this._on; },
  set(flag) {
    this._on = !!flag;
    if (!this._on) {
      document.querySelectorAll('.li-path').forEach(el => {
        el.style.animation = 'none';
      });
    } else {
      Recent.renderAll();
    }
  },
};

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

      // 名字固定不动，路径放在裁剪容器里自动来回滚动
      const wrap = document.createElement('span');
      wrap.className = 'li-path-wrap';
      const dir = document.createElement('span');
      dir.className = 'li-path';
      dir.textContent = p;
      wrap.appendChild(dir);

      btn.appendChild(name);
      btn.appendChild(wrap);
      btn.addEventListener('click', () => onPick && onPick(p));
      el.appendChild(btn);
      pairs.push([wrap, dir]);
    });
  },
};

function baseName(p) {
  const parts = String(p).split(/[\\/]/).filter(Boolean);
  return parts.length ? parts[parts.length - 1] : String(p);
}

window.Recent = Recent;
window.Marquee = Marquee;
