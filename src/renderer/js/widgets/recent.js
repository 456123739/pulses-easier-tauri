// recent.js — 最近打开列表
// 存 ~/.pulses_easier/recent.json，上限 8 条，去重，最新在前。
//
// 行结构：[名字（固定不动）] [路径（超出宽度则来回滚动）]

// ── 路径自动滚动的总开关（设置里可关）──
const Marquee = {
  _on: true,
  enabled() { return this._on; },
  set(flag) {
    this._on = !!flag;
    if (this._on) {
      Recent.renderAll();                 // 重渲染以挂上滚动
    } else {
      document.querySelectorAll('.li-path').forEach(el => {
        el.style.animation = 'none';
        el.style.removeProperty('--marquee-x');
      });
    }
  },
};

const Recent = {
  _items: [],
  _targets: [],       // [{el, onPick, maxItems}]
  _pairs: [],         // 上次渲染出的 [裁剪容器, 内层路径]，供 refreshMarquee 用

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
    for (const t of this._targets) {
      try { this._render(t.el, t.onPick, t.maxItems); }
      catch (_) { /* 单个列表渲染失败不影响其它 */ }
    }
  },

  _render(el, onPick, maxItems) {
    if (!el) return;
    el.innerHTML = '';

    const items = maxItems ? this._items.slice(0, maxItems) : this._items;

    if (!items.length) {
      const empty = document.createElement('div');
      empty.className = 'list-empty';
      empty.textContent = '暂无记录';
      el.appendChild(empty);
      return;
    }

    const pairs = [];      // [裁剪容器, 内层路径] —— 渲染完统一挂滚动

    items.forEach((p, i) => {
      const btn = document.createElement('button');
      btn.className = 'list-item';
      btn.style.animation = `softUp 260ms var(--ease-out) ${i * 28}ms both`;

      // 名字：固定不动
      const name = document.createElement('span');
      name.className = 'li-name';
      name.textContent = baseName(p);

      // 路径：放进裁剪容器，超出宽度就来回滚动
      const wrap = document.createElement('span');
      wrap.className = 'li-path-wrap';
      const dir = document.createElement('span');
      dir.className = 'li-path';
      dir.textContent = p;
      wrap.appendChild(dir);

      btn.appendChild(name);
      btn.appendChild(wrap);
      btn.addEventListener('click', () => { if (onPick) onPick(p); });
      el.appendChild(btn);

      pairs.push([wrap, dir]);
    });

    this._startMarquee(pairs);
  },

  // 路径比容器宽就来回滚动；不需要滚动就保持静止
  _startMarquee(pairs) {
    this._pairs = pairs;                    // 记下来，界面显示后要重新量
    requestAnimationFrame(() => {
      for (const pair of pairs) {
        const wrap = pair[0];
        const inner = pair[1];
        if (!wrap.isConnected) continue;

        const overflow = inner.scrollWidth - wrap.clientWidth;
        if (overflow > 6 && Marquee.enabled()) {
          inner.style.setProperty('--marquee-x', '-' + overflow + 'px');
          const dur = Math.max(2600, overflow * 30);   // 越长越慢
          inner.style.animation =
            'marqueeX ' + dur + 'ms ease-in-out infinite alternate';
        } else {
          inner.style.animation = 'none';
          inner.style.removeProperty('--marquee-x');
        }
      }
    });
  },

  // 启动时 #app 还是 display:none，元素宽高都是 0，量不出溢出。
  // 主界面显示出来之后必须重新量一次，否则要等用户点一下才滚。
  refreshMarquee() {
    if (this._pairs && this._pairs.length) this._startMarquee(this._pairs);
  },
};

function baseName(p) {
  const parts = String(p).split(/[\\/]/).filter(Boolean);
  return parts.length ? parts[parts.length - 1] : String(p);
}

// 拼路径：沿用原路径的分隔符（Windows 是 \，其它是 /）
function joinPath(dir, name) {
  const sep = String(dir).includes('\\') ? '\\' : '/';
  return String(dir).replace(/[\\/]+$/, '') + sep + name;
}

window.Recent = Recent;
window.Marquee = Marquee;
