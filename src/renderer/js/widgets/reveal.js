// reveal.js — 高度展开/收起动画（用于侧边栏选项「伸出来」）
//
// 为什么不用 CSS 的 grid-template-rows: 0fr→1fr：
//   那个特性依赖较新的 Chromium，WebView2 版本参差。
// 这里用「量高度 + 动 height」，任何内核都能跑，且动画可控。

const Reveal = {
  DUR: 340,

  /** 元素当前是否处于展开态 */
  isExpanded(el) {
    return !el.classList.contains('is-collapsed');
  },

  /**
   * 展开 / 收起一个块。
   * @param el      目标元素（需要 overflow:hidden）
   * @param expand  true=展开
   * @param delay   延迟毫秒（用于错开）
   */
  set(el, expand, delay = 0) {
    if (!el) return;
    clearTimeout(el._revealTimer);
    clearTimeout(el._revealSettle);

    if (expand) {
      el.classList.remove('is-collapsed');
      el.style.overflow = 'hidden';

      // 先归零量出目标高度
      el.style.height = '0px';
      el.style.opacity = '0';
      el.style.transform = 'translateX(-12px)';
      const target = el.scrollHeight;
      void el.offsetWidth;

      el._revealTimer = setTimeout(() => {
        el.style.height = target + 'px';
        el.style.opacity = '1';
        el.style.transform = 'none';
        // 动画结束后放开高度，内容变化也不会被截断
        el._revealSettle = setTimeout(() => {
          el.style.height = '';
          el.style.overflow = '';
        }, this.DUR + 30);
      }, delay);
    } else {
      // 从当前高度收到 0
      el.style.overflow = 'hidden';
      el.style.height = el.scrollHeight + 'px';
      void el.offsetWidth;
      el._revealTimer = setTimeout(() => {
        el.classList.add('is-collapsed');
        el.style.height = '0px';
        el.style.opacity = '0';
        el.style.transform = 'translateX(-12px)';
      }, delay);
    }
  },

  /** 一次性同步（初始化用，不播动画） */
  apply(el, expand) {
    if (!el) return;
    clearTimeout(el._revealTimer);
    clearTimeout(el._revealSettle);
    el.style.transition = 'none';
    el.style.overflow = 'hidden';
    el.classList.toggle('is-collapsed', !expand);
    if (expand) {
      el.style.height = '';
      el.style.opacity = '';
      el.style.transform = '';
    } else {
      // 关键：折叠态必须显式写 0，光靠 class 是撑不开的
      el.style.height = '0px';
      el.style.opacity = '0';
      el.style.transform = 'translateX(-12px)';
    }
    void el.offsetWidth;
    el.style.transition = '';
    el.style.overflow = '';
  },
};

window.Reveal = Reveal;
