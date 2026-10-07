// overlay.js — 窗内弹窗（遮罩 + 居中卡片）
// 全部在 DOM 内画，不新开窗口。
//
// 动画要点：元素插入后必须强制重排（void offsetWidth）再改样式，
// 否则浏览器把「插入 + 改样式」合并到同一帧，transition 不触发。

const Overlay = {
  container: null,

  init() {
    this.container = document.getElementById('overlay-container');
  },

  _scrim() {
    const s = document.createElement('div');
    s.className = 'ov-scrim';
    s.addEventListener('mousedown', e => e.stopPropagation());
    s.addEventListener('click', e => e.stopPropagation());
    return s;
  },

  _card(title, message, opts = {}) {
    const accent = opts.level === 'error' ? 'var(--err)'
                 : opts.level === 'warn'  ? 'var(--warn)'
                 : 'linear-gradient(90deg, var(--accent-lo), var(--accent-hi))';

    const card = document.createElement('div');
    card.className = 'ov-card';
    card.innerHTML = `
      <div class="ov-accent" style="background:${accent};"></div>
      <div class="ov-head">
        <div class="ov-title">${esc(title)}</div>
        <div class="ov-msg">${esc(message)}</div>
      </div>
      <div class="ov-body"></div>
      <div class="ov-foot"></div>
    `;
    return card;
  },

  _enter(scrim, card) {
    this.container.appendChild(scrim);
    this.container.appendChild(card);
    void scrim.offsetWidth;          // 强制重排 → 让 transition 生效
    scrim.classList.add('is-in');
    card.classList.add('is-in');
  },

  _leave(scrim, card, after) {
    scrim.classList.remove('is-in');
    card.classList.remove('is-in');
    setTimeout(() => {
      scrim.remove();
      card.remove();
      if (after) after();
    }, 320);
  },

  _btn(text, kind) {
    const b = document.createElement('button');
    b.className = 'ov-btn ' + (kind === 'primary' ? 'primary' : 'ghost');
    b.textContent = text;
    return b;
  },

  // ── 提示框 ──
  alert(title, message, opts = {}) {
    return new Promise(resolve => {
      const scrim = this._scrim();
      const card = this._card(title, message, opts);
      const btn = this._btn(opts.confirmText || '知道了', 'primary');
      btn.onclick = () => this._leave(scrim, card, resolve);
      card.querySelector('.ov-foot').appendChild(btn);
      this._enter(scrim, card);
      setTimeout(() => btn.focus(), 340);
      document.addEventListener('keydown', function esc(e) {
        if (e.key === 'Escape' || e.key === 'Enter') {
          document.removeEventListener('keydown', esc);
          btn.click();
        }
      });
    });
  },

  // ── 确认框 ──
  confirm(title, message, opts = {}) {
    return new Promise(resolve => {
      const scrim = this._scrim();
      const card = this._card(title, message, opts);
      const foot = card.querySelector('.ov-foot');

      const cancel = this._btn(opts.cancelText || '取消');
      cancel.onclick = () => this._leave(scrim, card, () => resolve(false));

      const ok = this._btn(opts.confirmText || '确定', 'primary');
      if (opts.danger) ok.style.background = 'var(--err)';
      ok.onclick = () => this._leave(scrim, card, () => resolve(true));

      foot.appendChild(cancel);
      foot.appendChild(ok);
      this._enter(scrim, card);
      setTimeout(() => ok.focus(), 340);
    });
  },

  // ── 输入框弹窗 ──
  prompt(title, message, opts = {}) {
    return new Promise(resolve => {
      const scrim = this._scrim();
      const card = this._card(title, message, opts);
      const body = card.querySelector('.ov-body');

      const input = document.createElement('input');
      input.className = 'ov-input';
      input.type = 'text';
      input.placeholder = opts.placeholder || '';
      input.value = opts.initial || '';

      const err = document.createElement('div');
      err.className = 'ov-err';

      body.appendChild(input);
      body.appendChild(err);

      const foot = card.querySelector('.ov-foot');
      const cancel = this._btn(opts.cancelText || '取消');
      cancel.onclick = () => this._leave(scrim, card, () => resolve(null));

      const ok = this._btn(opts.confirmText || '确定', 'primary');
      ok.onclick = () => {
        const val = input.value.trim();
        if (opts.validate) {
          const [pass, norm, msg] = opts.validate(val);
          if (!pass) {
            err.textContent = msg;
            input.focus();
            return;
          }
          this._leave(scrim, card, () => resolve(norm));
        } else {
          this._leave(scrim, card, () => resolve(val));
        }
      };

      input.addEventListener('keydown', e => {
        if (e.key === 'Enter') ok.click();
        if (e.key === 'Escape') cancel.click();
      });

      foot.appendChild(cancel);
      foot.appendChild(ok);
      this._enter(scrim, card);
      setTimeout(() => input.focus(), 340);
    });
  },
};

function esc(s) {
  return String(s == null ? '' : s)
    .replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

window.Overlay = Overlay;
