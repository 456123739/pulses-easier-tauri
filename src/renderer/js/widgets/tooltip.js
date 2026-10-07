// tooltip.js — 窗内悬停提示（带小箭头，跟随定位）

const Tooltip = {
  _el: null,
  _timer: null,

  attach(el, text, opts = {}) {
    el.addEventListener('mouseenter', () => {
      if (opts.delay) {
        this._timer = setTimeout(() => this._show(el, text, opts), opts.delay);
      } else {
        this._show(el, text, opts);
      }
    });
    el.addEventListener('mouseleave', () => this._hide());
    el.addEventListener('mousedown', () => this._hide());
  },

  _show(el, text, opts) {
    this._hide();

    const tip = document.createElement('div');
    tip.className = 'tip';
    tip.textContent = typeof text === 'function' ? text() : text;
    document.body.appendChild(tip);
    this._el = tip;

    // 定位：默认在下方，装不下翻到上方
    const r = el.getBoundingClientRect();
    const tw = tip.offsetWidth;
    const th = tip.offsetHeight;

    let below = true;
    let top = r.bottom + 10;
    if (top + th > window.innerHeight - 8) {
      top = r.top - th - 10;
      below = false;
    }
    let left = r.left + 2;
    left = Math.max(8, Math.min(left, window.innerWidth - tw - 8));

    tip.style.left = left + 'px';
    tip.style.top = top + 'px';
    if (!below) tip.classList.add('below');

    // 箭头跟着控件左边缘
    const arrowX = Math.max(10, Math.min(r.left - left + 8, tw - 20));
    tip.style.setProperty('--arrow-x', arrowX + 'px');

    requestAnimationFrame(() => tip.classList.add('is-in'));
  },

  _hide() {
    if (this._timer) { clearTimeout(this._timer); this._timer = null; }
    const tip = this._el;
    if (!tip) return;
    this._el = null;
    tip.classList.remove('is-in');
    setTimeout(() => tip.remove(), 220);
  },
};

window.Tooltip = Tooltip;
