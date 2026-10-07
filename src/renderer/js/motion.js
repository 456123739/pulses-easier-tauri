// motion.js — 动画辅助（CSS transition 驱动，替代 Python motion.py）
// 核心思想：CSS transition 天然 60fps、GPU 加速、不卡线程。
// 这里只做 JS 编排（串行 / 并行 / 延迟 / 错开）。

const Motion = {
  enabled: true,

  setEnabled(flag) {
    this.enabled = flag;
    document.documentElement.style.setProperty(
      '--_motion', flag ? 'initial' : 'none');
    if (!flag) {
      // 禁用所有 CSS animation / transition
      const style = document.getElementById('_motion-override');
      if (!style) {
        const s = document.createElement('style');
        s.id = '_motion-override';
        s.textContent = '*, *::before, *::after { animation: none !important; transition: none !important; }';
        document.head.appendChild(s);
      }
    } else {
      document.getElementById('_motion-override')?.remove();
    }
  },

  // 淡入
  fadeIn(el, duration = 200) {
    el.style.opacity = '0';
    el.style.transition = `opacity ${duration}ms var(--ease-out)`;
    requestAnimationFrame(() => { el.style.opacity = '1'; });
  },

  // 淡出
  fadeOut(el, duration = 200) {
    el.style.transition = `opacity ${duration}ms var(--ease)`;
    el.style.opacity = '0';
  },

  // 滑入（从下方上浮）
  slideUp(el, duration = 240) {
    el.style.opacity = '0';
    el.style.transform = 'translateY(14px)';
    el.style.transition = `opacity ${duration}ms var(--ease-out), transform ${duration}ms var(--ease-out)`;
    requestAnimationFrame(() => {
      el.style.opacity = '1';
      el.style.transform = 'translateY(0)';
    });
  },

  // 滑出（下沉 + 淡出）
  slideDown(el, duration = 200) {
    el.style.transition = `opacity ${duration}ms var(--ease), transform ${duration}ms var(--ease)`;
    el.style.opacity = '0';
    el.style.transform = 'translateY(14px)';
  },

  // 一组控件错开滑入
  stagger(elts, step = 45, duration = 240) {
    elts.forEach((el, i) => {
      el.style.opacity = '0';
      el.style.transform = 'translateX(-16px)';
      el.style.transition = `opacity ${duration}ms var(--ease-out), transform ${duration}ms var(--ease-out)`;
      setTimeout(() => {
        el.style.opacity = '1';
        el.style.transform = 'translateX(0)';
      }, i * step);
    });
  },

  // 颜色过渡（HoverGlow 替代）
  hoverGlow(el, normal, hover) {
    el.style.transition = `background ${150}ms var(--ease), color ${150}ms var(--ease)`;
    el.addEventListener('mouseenter', () => { el.style.background = hover; });
    el.addEventListener('mouseleave', () => { el.style.background = normal; });
  },

  // 等待动画结束
  wait(el, duration) {
    return new Promise(r => setTimeout(r, duration));
  },
};

// 只改按钮里的文字层（.cta-label），不破坏 .cta-shine 扫光层
function setBtnLabel(btn, text) {
  if (!btn) return;
  const label = btn.querySelector('.cta-label');
  if (label) label.textContent = text;
  else btn.textContent = text;
}

window.Motion = Motion;
window.setBtnLabel = setBtnLabel;
