// log.js — 简洁日志列表（对应 Python LogPanel）
// 用法：Log.attach(document.getElementById('log-list'))
//       Log.info('已加载更新包')

const Log = {
  _el: null,
  _max: 300,

  attach(el) { this._el = el; },

  clear() { if (this._el) this._el.innerHTML = ''; },

  _add(level, msg) {
    if (!this._el) return;
    const row = document.createElement('div');
    row.className = 'log-row ' + level;
    const now = new Date();
    const t = String(now.getHours()).padStart(2, '0') + ':'
            + String(now.getMinutes()).padStart(2, '0') + ':'
            + String(now.getSeconds()).padStart(2, '0');
    const time = document.createElement('span');
    time.className = 'log-time';
    time.textContent = t;
    const text = document.createElement('span');
    text.className = 'log-msg';
    text.textContent = msg;
    row.appendChild(time);
    row.appendChild(text);
    this._el.appendChild(row);

    while (this._el.children.length > this._max) {
      this._el.removeChild(this._el.firstChild);
    }
    this._el.scrollTop = this._el.scrollHeight;
  },

  info(msg)  { this._add('info', msg); },
  ok(msg)    { this._add('ok', msg); },
  warn(msg)  { this._add('warn', msg); },
  error(msg) { this._add('error', msg); },
};

window.Log = Log;
