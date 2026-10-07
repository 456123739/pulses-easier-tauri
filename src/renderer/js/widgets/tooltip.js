// tooltip.js — 窗内悬停提示（DOM 定位，替代 Python tooltip.py）

const Tooltip = {
  _box: null,
  _timer: null,

  attach(el, text, opts = {}) {
    el.addEventListener('mouseenter', () => {
      if (opts.delay) {
        this._timer = setTimeout(() => this._show(el, text, opts),
                                 opts.delay);
      } else {
        this._show(el, text, opts);
      }
    });
    el.addEventListener('mouseleave', () => this._hide());
    el.addEventListener('mousedown', () => this._hide());
  },

  _show(el, text, opts) {
    this._hide();
    const box = document.createElement('div');
    box.style.cssText = `
      position: fixed; z-index: 9000;
      background: var(--log-bg);
      border: 1px solid var(--border);
      border-radius: 6px;
      padding: 6px 10px;
      font-size: var(--fs-small); color: var(--text-primary);
      max-width: ${opts.wraplength || 520}px;
      word-break: break-all; white-space: pre-wrap;
      opacity: 0; transform: translateY(6px);
      transition: opacity var(--t-tip) var(--ease-out),
                  transform var(--t-tip) var(--ease-out);
      pointer-events: none;
      box-shadow: 0 4px 16px rgba(0,0,0,0.3);
    `;
    box.textContent = typeof text === 'function' ? text() : text;

    document.body.appendChild(box);
    this._box = box;

    // 定位
    const rect = el.getBoundingClientRect();
    const bw = box.offsetWidth, bh = box.offsetHeight;
    let x = rect.left + 8;
    let y = rect.bottom + 6;

    // 下方装不下 → 翻到上方
    if (y + bh > window.innerHeight - 6) {
      y = rect.top - bh - 6;
    }
    // 右侧超界 → 往左让
    x = Math.max(6, Math.min(x, window.innerWidth - bw - 6));
    box.style.left = x + 'px';
    box.style.top = y + 'px';

    requestAnimationFrame(() => {
      box.style.opacity = '1';
      box.style.transform = 'translateY(0)';
    });
  },

  _hide() {
    if (this._timer) { clearTimeout(this._timer); this._timer = null; }
    if (this._box) {
      this._box.style.opacity = '0';
      const b = this._box;
      this._box = null;
      setTimeout(() => b.remove(), 160);
    }
  },
};

window.Tooltip = Tooltip;
