// log.js — 简洁日志列表
//
// 分级（首选项里的「完整日志」开关控制）：
//   Log.debug()  —— 过程流水（解压中 / 比对中 / 切换模式…）默认隐藏
//   Log.info()   —— 有意义的状态变化（已定位 / 已加载）默认显示
//   Log.ok()     —— 成功结果，默认显示
//   Log.warn()   —— 警告，默认显示
//   Log.error()  —— 错误，默认显示
//
// 用法：Log.attach(document.getElementById('log-list'))

const Log = {
  _el: null,
  _max: 300,
  _verbose: false,

  attach(el) { this._el = el; },

  /** 首选项「完整日志」开关 */
  setVerbose(flag) {
    this._verbose = !!flag;
  },

  isVerbose() { return this._verbose; },

  clear() { if (this._el) this._el.innerHTML = ''; },

  _add(level, msg) {
    if (!this._el) return;
    if (level === 'debug' && !this._verbose) return;

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

  debug(msg) { this._add('debug', msg); },
  info(msg)  { this._add('info', msg); },
  ok(msg)    { this._add('ok', msg); },
  warn(msg)  { this._add('warn', msg); },
  error(msg) { this._add('error', msg); },
};

window.Log = Log;
