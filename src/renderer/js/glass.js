// glass.js — 玻璃质感（CSS + Electron vibrancy，替代 Python glass.py）
// Win10/Win11 亚克力由主进程 setVibrancy 处理；这里做 CSS 模拟层。

const Glass = {
  // 给元素加玻璃表面效果
  apply(el, opts = {}) {
    el.style.background = opts.bg || 'var(--glass-surface)';
    el.style.backdropFilter = 'blur(20px) saturate(150%)';
    el.style.webkitBackdropFilter = 'blur(20px) saturate(150%)';
    el.style.border = `1px solid ${opts.border || 'var(--glass-border)'}`;
    el.style.borderRadius = opts.radius || 'var(--r-card)';
  },

  // 背景渐变
  gradient(el, from, to) {
    el.style.background = `linear-gradient(135deg, ${from}, ${to})`;
  },
};

window.Glass = Glass;
