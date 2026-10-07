// anim.js — Web Animations API 封装
//
// 为什么不用 CSS transition：
//   CSS 过渡依赖「类切换时机 + 样式重算 + 过渡属性是否被覆盖」三者配合，
//   任何一环出问题都会静默失效，而且从外部完全看不出来。
//   WAAPI 是命令式的：调用即返回一个 Animation 对象，
//   能查到 playState、能 cancel、能读 currentTime —— 可验证。
//
// 不支持 WAAPI 时全部退化为「瞬间切换」，不会卡住任何流程。

const Anim = {
  _capable: null,

  /** 是否支持 WAAPI */
  capable() {
    if (this._capable !== null) return this._capable;
    try {
      const el = document.createElement('div');
      if (typeof el.animate !== 'function') {
        this._capable = false;
        return false;
      }
      const a = el.animate([{ opacity: 1 }, { opacity: 0 }], { duration: 1 });
      this._capable = !!(a && typeof a.cancel === 'function');
      try { a.cancel(); } catch (_) { /* 忽略 */ }
    } catch (_) {
      this._capable = false;
    }
    return this._capable;
  },

  /** 清掉元素上正在跑的动画 */
  cancelAll(el) {
    try {
      el.getAnimations().forEach(a => {
        try { a.cancel(); } catch (_) { /* 忽略 */ }
      });
    } catch (_) { /* 忽略 */ }
  },

  /**
   * 播放一段动画。返回 Animation 或 null。
   * @param el        目标元素
   * @param keyframes 关键帧数组
   * @param options   { duration, easing, delay, fill }
   */
  play(el, keyframes, options) {
    if (!el || !this.capable()) return null;
    try {
      this.cancelAll(el);
      return el.animate(keyframes, options);
    } catch (_) {
      return null;
    }
  },

  /** 缓动曲线 */
  EASE: 'cubic-bezier(0.16, 1, 0.30, 1)',
  SOFT: 'cubic-bezier(0.32, 0.72, 0, 1)',
};

window.Anim = Anim;
