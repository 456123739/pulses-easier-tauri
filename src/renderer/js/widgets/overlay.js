// overlay.js — 窗内弹窗（遮罩 + 居中卡片，替代 Python overlay.py）
// 所有弹窗都在 DOM 内画，不新开 BrowserWindow。

const Overlay = {
  container: null,

  init() {
    this.container = document.getElementById('overlay-container');
  },

  // 底层遮罩
  _scrim() {
    const s = document.createElement('div');
    s.className = 'overlay-scrim';
    s.style.cssText = `
      position: fixed; inset: 0; z-index: 8000;
      background: var(--scrim);
      opacity: 0; transition: opacity var(--t-dialog) var(--ease);
    `;
    // 吞掉所有鼠标事件
    s.addEventListener('mousedown', e => e.stopPropagation());
    s.addEventListener('click', e => e.stopPropagation());
    return s;
  },

  // 居中卡片
  _card(title, message, opts = {}) {
    const card = document.createElement('div');
    card.className = 'overlay-card';
    card.style.cssText = `
      position: fixed; top: 50%; left: 50%;
      transform: translate(-50%, -50%) translateY(14px);
      z-index: 8001; opacity: 0;
      background: var(--glass-surface);
      backdrop-filter: blur(20px) saturate(150%);
      border: 1px solid var(--glass-border);
      border-radius: var(--r-card);
      padding: 0; min-width: 420px; max-width: 560px;
      box-shadow: 0 24px 64px rgba(0,0,0,0.4);
      transition: opacity var(--t-dialog) var(--ease-out),
                  transform var(--t-dialog) var(--ease-out);
    `;

    const accent = opts.level === 'error' ? 'var(--error)' :
                   opts.level === 'warn'  ? 'var(--warning)' : 'var(--accent)';

    card.innerHTML = `
      <div style="height:3px;background:${accent};border-radius:14px 14px 0 0;"></div>
      <div style="padding: 18px 24px 8px;">
        <h3 style="font-size:var(--fs-subtitle);margin-bottom:8px;color:var(--text-primary);">${title}</h3>
        <p style="font-size:var(--fs-body);color:var(--text-secondary);line-height:1.6;white-space:pre-wrap;">${message}</p>
      </div>
      <div class="overlay-body" style="padding: 0 24px;"></div>
      <div class="overlay-btns" style="display:flex;gap:8px;padding:16px 24px 20px;"></div>
    `;
    return card;
  },

  // 弹窗入场
  _enter(scrim, card) {
    this.container.appendChild(scrim);
    this.container.appendChild(card);
    requestAnimationFrame(() => {
      scrim.style.opacity = '1';
      card.style.opacity = '1';
      card.style.transform = 'translate(-50%, -50%) translateY(0)';
    });
  },

  // 弹窗退场
  _leave(scrim, card, after) {
    scrim.style.opacity = '0';
    card.style.opacity = '0';
    card.style.transform = 'translate(-50%, -50%) translateY(14px)';
    setTimeout(() => {
      scrim.remove();
      card.remove();
      if (after) after();
    }, 260);
  },

  // 提示框
  alert(parent, title, message, opts = {}) {
    return new Promise(resolve => {
      const scrim = this._scrim();
      const card = this._card(title, message, opts);
      const btns = card.querySelector('.overlay-btns');
      const btn = document.createElement('button');
      btn.className = 'btn btn-primary';
      btn.textContent = opts.confirmText || '知道了';
      btn.onclick = () => { this._leave(scrim, card, resolve); };
      btns.appendChild(btn);
      this._enter(scrim, card);
      btn.focus();
    });
  },

  // 确认框
  confirm(parent, title, message, opts = {}) {
    return new Promise(resolve => {
      const scrim = this._scrim();
      const card = this._card(title, message, opts);
      const btns = card.querySelector('.overlay-btns');

      const cancelBtn = document.createElement('button');
      cancelBtn.className = 'btn';
      cancelBtn.style.cssText = 'background:var(--log-bg);color:var(--text-primary);';
      cancelBtn.textContent = opts.cancelText || '取消';
      cancelBtn.onclick = () => { this._leave(scrim, card, () => resolve(false)); };

      const okBtn = document.createElement('button');
      okBtn.className = 'btn btn-primary';
      okBtn.textContent = opts.confirmText || '确定';
      okBtn.onclick = () => { this._leave(scrim, card, () => resolve(true)); };

      btns.appendChild(cancelBtn);
      btns.appendChild(okBtn);
      this._enter(scrim, card);
      okBtn.focus();
    });
  },

  // 输入框弹窗
  prompt(parent, title, message, opts = {}) {
    return new Promise(resolve => {
      const scrim = this._scrim();
      const card = this._card(title, message, opts);
      const body = card.querySelector('.overlay-body');

      const input = document.createElement('input');
      input.type = 'text';
      input.placeholder = opts.placeholder || '';
      input.value = opts.initial || '';
      input.style.cssText = `
        width: 100%; padding: 8px 12px; margin-bottom: 8px;
        background: var(--input-bg); border: 1px solid var(--border);
        border-radius: var(--r-input); color: var(--text-primary);
        font-size: var(--fs-body); font-family: var(--font); outline: none;
        transition: border var(--t-hover) var(--ease);
      `;
      input.addEventListener('focus', () => { input.style.borderColor = 'var(--accent)'; });
      input.addEventListener('blur', () => { input.style.borderColor = 'var(--border)'; });

      const errLabel = document.createElement('div');
      errLabel.style.cssText = 'font-size:var(--fs-tiny);color:var(--error);min-height:1em;margin-bottom:8px;';

      body.appendChild(input);
      body.appendChild(errLabel);

      const btns = card.querySelector('.overlay-btns');
      const cancelBtn = document.createElement('button');
      cancelBtn.className = 'btn';
      cancelBtn.style.cssText = 'background:var(--log-bg);color:var(--text-primary);';
      cancelBtn.textContent = opts.cancelText || '取消';
      cancelBtn.onclick = () => { this._leave(scrim, card, () => resolve(null)); };

      const okBtn = document.createElement('button');
      okBtn.className = 'btn btn-primary';
      okBtn.textContent = opts.confirmText || '确定';
      okBtn.onclick = () => {
        const val = input.value.trim();
        if (opts.validate) {
          const [ok, norm, err] = opts.validate(val);
          if (!ok) { errLabel.textContent = err; return; }
          this._leave(scrim, card, () => resolve(norm));
        } else {
          this._leave(scrim, card, () => resolve(val));
        }
      };

      input.addEventListener('keydown', e => {
        if (e.key === 'Enter') okBtn.click();
        if (e.key === 'Escape') cancelBtn.click();
      });

      btns.appendChild(cancelBtn);
      btns.appendChild(okBtn);
      this._enter(scrim, card);
      setTimeout(() => input.focus(), 300);
    });
  },
};

window.Overlay = Overlay;
