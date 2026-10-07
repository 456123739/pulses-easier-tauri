// overlay.js — 窗内弹窗（遮罩 + 居中卡片，替代 Python overlay.py）
// 全部在 DOM 内画，不新开窗口。
// 动画要点：元素插入后必须强制重排（void offsetWidth）再改样式，
//           否则浏览器会把「插入 + 改样式」合并成一帧，transition 不触发。

const Overlay = {
  container: null,

  init() {
    this.container = document.getElementById('overlay-container');
  },

  _scrim() {
    const s = document.createElement('div');
    s.style.cssText = `
      position: fixed; inset: 0; z-index: 8000;
      background: rgba(6, 6, 9, 0.66);
      opacity: 0;
      transition: opacity 200ms var(--ease);
    `;
    s.addEventListener('mousedown', e => e.stopPropagation());
    s.addEventListener('click', e => e.stopPropagation());
    return s;
  },

  _card(title, message, opts = {}) {
    const accent = opts.level === 'error' ? 'var(--error)'
                 : opts.level === 'warn'  ? 'var(--warning)'
                 : 'var(--accent)';

    const card = document.createElement('div');
    card.style.cssText = `
      position: fixed; top: 50%; left: 50%; z-index: 8001;
      transform: translate(-50%, -46%) scale(0.97);
      opacity: 0;
      width: min(520px, calc(100vw - 64px));
      background: #1C1C21;
      border: 1px solid rgba(255,255,255,0.09);
      border-radius: 14px;
      box-shadow: 0 24px 64px rgba(0,0,0,0.55);
      overflow: hidden;
      transition: opacity 220ms var(--ease-out), transform 220ms var(--ease-out);
    `;

    card.innerHTML = `
      <div style="height:3px;background:${accent};"></div>
      <div style="padding:18px 22px 4px;">
        <div style="font-size:15px;line-height:1.45;font-weight:600;color:var(--text-primary);">
          ${escapeHTML(title)}
        </div>
        <div style="font-size:13px;line-height:1.65;color:var(--text-secondary);
                    margin-top:8px;white-space:pre-wrap;overflow-wrap:anywhere;">
          ${escapeHTML(message)}
        </div>
      </div>
      <div class="overlay-body" style="padding:0 22px;"></div>
      <div class="overlay-btns"
           style="display:flex;gap:8px;justify-content:flex-end;padding:16px 22px 18px;"></div>
    `;
    return card;
  },

  _enter(scrim, card) {
    this.container.appendChild(scrim);
    this.container.appendChild(card);
    void scrim.offsetWidth;          // 强制重排 → 让 transition 生效
    scrim.style.opacity = '1';
    card.style.opacity = '1';
    card.style.transform = 'translate(-50%, -50%) scale(1)';
  },

  _leave(scrim, card, after) {
    scrim.style.opacity = '0';
    card.style.opacity = '0';
    card.style.transform = 'translate(-50%, -46%) scale(0.97)';
    setTimeout(() => {
      scrim.remove();
      card.remove();
      if (after) after();
    }, 230);
  },

  _btn(text, kind) {
    const b = document.createElement('button');
    b.textContent = text;
    const base = 'padding:9px 18px;border-radius:8px;font-size:13px;line-height:1.4;'
      + 'transition:background 150ms var(--ease),transform 150ms var(--ease);';
    b.style.cssText = base + (kind === 'primary'
      ? 'background:var(--accent);color:#fff;'
      : 'background:#26262C;color:var(--text-primary);');
    b.addEventListener('mouseenter', () => {
      b.style.background = kind === 'primary' ? 'var(--accent-hover)' : '#30303A';
    });
    b.addEventListener('mouseleave', () => {
      b.style.background = kind === 'primary' ? 'var(--accent)' : '#26262C';
    });
    return b;
  },

  // 提示框
  alert(title, message, opts = {}) {
    return new Promise(resolve => {
      const scrim = this._scrim();
      const card = this._card(title, message, opts);
      const btn = this._btn(opts.confirmText || '知道了', 'primary');
      btn.onclick = () => this._leave(scrim, card, resolve);
      card.querySelector('.overlay-btns').appendChild(btn);
      this._enter(scrim, card);
      setTimeout(() => btn.focus(), 240);
    });
  },

  // 确认框
  confirm(title, message, opts = {}) {
    return new Promise(resolve => {
      const scrim = this._scrim();
      const card = this._card(title, message, opts);
      const btns = card.querySelector('.overlay-btns');

      const cancel = this._btn(opts.cancelText || '取消');
      cancel.onclick = () => this._leave(scrim, card, () => resolve(false));

      const ok = this._btn(opts.confirmText || '确定', 'primary');
      ok.onclick = () => this._leave(scrim, card, () => resolve(true));

      btns.appendChild(cancel);
      btns.appendChild(ok);
      this._enter(scrim, card);
      setTimeout(() => ok.focus(), 240);
    });
  },

  // 输入框弹窗
  prompt(title, message, opts = {}) {
    return new Promise(resolve => {
      const scrim = this._scrim();
      const card = this._card(title, message, opts);
      const body = card.querySelector('.overlay-body');

      const input = document.createElement('input');
      input.type = 'text';
      input.placeholder = opts.placeholder || '';
      input.value = opts.initial || '';
      input.style.cssText = `
        width: 100%; padding: 9px 12px; margin-top: 14px;
        background: var(--input-bg); border: 1px solid var(--border);
        border-radius: 6px; color: var(--text-primary);
        font-size: 13px; line-height: 1.4; font-family: var(--font); outline: none;
        transition: border-color 150ms var(--ease);
      `;
      input.addEventListener('focus', () => { input.style.borderColor = 'var(--accent)'; });
      input.addEventListener('blur',  () => { input.style.borderColor = 'var(--border)'; });

      const err = document.createElement('div');
      err.style.cssText = 'font-size:11px;line-height:1.5;color:var(--error);'
                        + 'min-height:1.1em;margin-top:5px;';

      body.appendChild(input);
      body.appendChild(err);

      const btns = card.querySelector('.overlay-btns');
      const cancel = this._btn(opts.cancelText || '取消');
      cancel.onclick = () => this._leave(scrim, card, () => resolve(null));

      const ok = this._btn(opts.confirmText || '确定', 'primary');
      ok.onclick = () => {
        const val = input.value.trim();
        if (opts.validate) {
          const [pass, norm, msg] = opts.validate(val);
          if (!pass) { err.textContent = msg; return; }
          this._leave(scrim, card, () => resolve(norm));
        } else {
          this._leave(scrim, card, () => resolve(val));
        }
      };

      input.addEventListener('keydown', e => {
        if (e.key === 'Enter') ok.click();
        if (e.key === 'Escape') cancel.click();
      });

      btns.appendChild(cancel);
      btns.appendChild(ok);
      this._enter(scrim, card);
      setTimeout(() => input.focus(), 250);
    });
  },
};

function escapeHTML(s) {
  return String(s == null ? '' : s)
    .replace(/&/g, '&amp;').replace(/</g, '&lt;').replace(/>/g, '&gt;');
}

window.Overlay = Overlay;
