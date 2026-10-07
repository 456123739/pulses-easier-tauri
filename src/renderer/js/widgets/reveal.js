// reveal.js — 高度展开/收起（用 WAAPI 驱动，不依赖 CSS 过渡）
//
// 侧边栏「伸出来」的效果：
//   量出目标高度 → 用 WAAPI 把 height 从 0 动到目标值，
//   同时 opacity 0→1、translateX(-14px)→0。
// 动画结束后释放高度（改回 auto），内容变化不会被截断。

const Reveal = {
  DUR: 420,

  isExpanded(el) {
    return el && !el.classList.contains('is-collapsed');
  },

  /**
   * @param el      目标（需要 overflow:hidden）
   * @param expand  true=展开
   * @param delay   延迟毫秒（错开用）
   */
  set(el, expand, delay = 0) {
    if (!el) return;

    Anim.cancelAll(el);
    clearTimeout(el._revealSettle);

    const target = expand ? this._measure(el) : el.offsetHeight;

    el.classList.toggle('is-collapsed', !expand);
    el.style.overflow = 'hidden';

    const from = expand
      ? { height: '0px', opacity: 0, transform: 'translateX(-14px)' }
      : { height: target + 'px', opacity: 1, transform: 'translateX(0)' };
    const to = expand
      ? { height: target + 'px', opacity: 1, transform: 'translateX(0)' }
      : { height: '0px', opacity: 0, transform: 'translateX(-14px)' };

    const anim = Anim.play(el, [from, to], {
      duration: this.DUR,
      delay,
      easing: Anim.SOFT,
      fill: 'both',
    });

    if (!anim) {
      // 不支持 WAAPI：直接落到终态
      this._settle(el, expand);
      return;
    }

    anim.onfinish = () => this._settle(el, expand);
  },

  _measure(el) {
    const prevH = el.style.height;
    const prevO = el.style.overflow;
    el.style.height = '';
    el.style.overflow = '';
    const h = el.scrollHeight;
    el.style.height = prevH;
    el.style.overflow = prevO;
    return h;
  },

  // 动画结束后落定。
  // 关键：收起态必须保留内联的 height:0 —— 只加 class 是不够的，
  // 清空内联样式会让元素弹回原高度（表现为「动画播完又全部展开」）。
  _settle(el, expand) {
    Anim.cancelAll(el);
    el.style.overflow = '';
    if (expand) {
      el.style.height = '';
      el.style.opacity = '';
      el.style.transform = '';
    } else {
      el.style.height = '0px';
      el.style.opacity = '0';
      el.style.transform = 'translateX(-14px)';
    }
    el.classList.toggle('is-collapsed', !expand);
  },

  /** 初始化用：不播动画，直接摆好 */
  apply(el, expand) {
    if (!el) return;
    Anim.cancelAll(el);
    clearTimeout(el._revealSettle);
    el.style.overflow = '';
    el.style.height = '';
    el.style.opacity = '';
    el.style.transform = '';
    el.classList.toggle('is-collapsed', !expand);
  },
};

window.Reveal = Reveal;
