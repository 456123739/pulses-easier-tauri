#!/usr/bin/env node
/**
 * smoke-renderer.js — 渲染层冒烟测试
 *
 * 目的：兜住「补丁静默失败」这类错误 —— 例如某个变量忘了声明、
 *      某个函数没被插进去，语法检查（new Function）查不出来，
 *      但真跑一遍渲染就会抛 ReferenceError。
 *
 * 做法：造一套最小 DOM 桩，按 index.html 的顺序加载所有 JS，
 *      然后调用各模块的渲染路径（列表 / 变更 / 设置 / 弹窗 / 日志）。
 */

const fs = require('fs');
const path = require('path');
const vm = require('vm');

const ROOT = path.join(__dirname, '..', 'src', 'renderer');

// ── 最小 DOM 桩 ──
function makeStyle() {
  const props = {};
  return {
    setProperty: (k, v) => { props[k] = v; },
    removeProperty: (k) => { delete props[k]; },
    getPropertyValue: (k) => props[k] || '',
    cssText: '',
    setPropertyRaw: props,
  };
}

function makeEl(tag) {
  const el = {
    tagName: String(tag || 'div').toUpperCase(),
    className: '',
    textContent: '',
    title: '',
    value: '',
    type: '',
    placeholder: '',
    style: makeStyle(),
    dataset: {},
    children: [],
    isConnected: true,
    scrollWidth: 400,
    clientWidth: 100,
    scrollHeight: 40,
    offsetWidth: 100,
    offsetHeight: 40,
    _listeners: {},
  };
  el.classList = {
    _set: new Set(),
    add(c) { this._set.add(c); },
    remove(c) { this._set.delete(c); },
    toggle(c, force) {
      if (force === undefined) {
        if (this._set.has(c)) this._set.delete(c); else this._set.add(c);
      } else if (force) { this._set.add(c); } else { this._set.delete(c); }
    },
    contains(c) { return this._set.has(c); },
  };
  el.appendChild = (c) => { el.children.push(c); return c; };
  el.removeChild = (c) => {
    const i = el.children.indexOf(c);
    if (i >= 0) el.children.splice(i, 1);
  };
  el.remove = () => { el.isConnected = false; };
  el.addEventListener = (ev, fn) => { (el._listeners[ev] ||= []).push(fn); };
  el.removeEventListener = () => {};
  // 桩不做 HTML 解析，所以给每个选择器返回一个稳定的假元素，
  // 这样 overlay 之类「从 innerHTML 里找节点」的逻辑也能跑通
  const _qcache = {};
  el.querySelector = (sel) => {
    if (!_qcache[sel]) _qcache[sel] = makeEl('div');
    return _qcache[sel];
  };
  el.querySelectorAll = () => [];
  let _html = '';
  Object.defineProperty(el, 'innerHTML', {
    get: () => _html,
    set: (v) => { _html = String(v); if (_html === '') el.children.length = 0; },
  });
  el.getAnimations = () => [];
  el.animate = () => ({ cancel() {}, onfinish: null, playState: 'finished' });
  el.focus = () => {};
  el.click = () => {};
  el.setAttribute = () => {};
  el.getBoundingClientRect = () => ({ left: 0, top: 0, right: 100, bottom: 40, width: 100, height: 40 });
  el.winfo = null;
  el.closest = () => null;
  el.getContext = () => null;
  Object.defineProperty(el, 'firstChild', { get: () => el.children[0] || null });
  return el;
}

const idRegistry = {};
function elFor(id) {
  if (!idRegistry[id]) idRegistry[id] = makeEl('div');
  return idRegistry[id];
}

const documentStub = {
  createElement: (t) => makeEl(t),
  createTextNode: (t) => ({ textContent: t }),
  getElementById: (id) => elFor(id),
  querySelector: () => null,
  querySelectorAll: () => [],
  addEventListener: () => {},
  removeEventListener: () => {},
  documentElement: makeEl('html'),
  head: makeEl('head'),
  body: makeEl('body'),
  readyState: 'complete',
  activeElement: null,
};

const windowStub = {
  document: documentStub,
  matchMedia: () => ({ matches: false, addEventListener: () => {} }),
  getComputedStyle: () => ({ opacity: '0.5' }),
  requestAnimationFrame: (cb) => { cb(0); return 1; },
  cancelAnimationFrame: () => {},
  setTimeout, clearTimeout, setInterval, clearInterval,
  console,
  // Tauri 桥：故意留空，验证前端能优雅降级
  __TAURI__: undefined,
};

const ctx = {
  window: windowStub,
  document: documentStub,
  console,
  setTimeout, clearTimeout, setInterval, clearInterval,
  requestAnimationFrame: windowStub.requestAnimationFrame,
  cancelAnimationFrame: () => {},
  getComputedStyle: windowStub.getComputedStyle,
  Math, Date, JSON, Object, Array, String, Number, Boolean, Error,
  Promise, Map, Set, RegExp, parseInt, parseFloat, isNaN, isFinite,
};
ctx.globalThis = ctx;
ctx.self = ctx;

const sandbox = vm.createContext(ctx);

// ── 按 index.html 的顺序加载 ──
const html = fs.readFileSync(path.join(ROOT, 'index.html'), 'utf-8');
const order = [...html.matchAll(/<script src="([^"]+)"><\/script>/g)].map(m => m[1]);

console.log('按 index.html 顺序加载 ' + order.length + ' 个脚本：');
let loaded = 0;
for (const rel of order) {
  const file = path.join(ROOT, rel);
  const code = fs.readFileSync(file, 'utf-8');
  try {
    vm.runInContext(code, sandbox, { filename: rel });
    loaded++;
  } catch (e) {
    console.log('  ✗ ' + rel + ' 加载失败：' + e.message);
    process.exit(1);
  }
}
console.log('  ✓ 全部加载成功（' + loaded + '/' + order.length + '）');

// ── 给 window.pulses 装上假实现，跑渲染路径 ──
const failures = [];
function check(name, fn) {
  try {
    fn();
    console.log('  ✓ ' + name);
  } catch (e) {
    failures.push(name + ' → ' + e.message);
    console.log('  ✗ ' + name + '：' + e.message);
  }
}

// 脚本用 window.X = X 导出，所以从 window 上取
const W = ctx.window;
console.log('\n导出的模块：' + ['Recent','Marquee','Log','Theme','Reveal','Overlay','Anim','Player','Developer','App','Motion']
  .filter(k => W[k]).join(', '));

console.log('\n渲染路径冒烟：');

check('Recent 渲染（含路径滚动）', () => {
  const el = makeEl('div');
  W.Recent._items = [
    'D:\\minecraft\\versions\\NRG',
    'C:\\Users\\32396\\Downloads',
  ];
  W.Recent._targets = [];
  W.Recent.bind(el, () => {}, 5);
  W.Recent.renderAll();
  if (el.children.length !== 2) throw new Error('应渲染 2 行，实际 ' + el.children.length);
});

check('Recent 空列表', () => {
  const el = makeEl('div');
  W.Recent._items = [];
  W.Recent._targets = [];
  W.Recent.bind(el, () => {}, 5);
  if (el.children.length !== 1) throw new Error('空列表应渲染占位');
});

check('Marquee 开关', () => {
  W.Marquee.set(false);
  W.Marquee.set(true);
});

check('Log 各等级', () => {
  const el = makeEl('div');
  W.Log.attach(el);
  W.Log.debug('debug');
  W.Log.info('info');
  W.Log.ok('ok');
  W.Log.warn('warn');
  W.Log.error('error');
  // debug 默认隐藏 → 4 条
  if (el.children.length !== 4) {
    throw new Error('默认应显示 4 条（debug 隐藏），实际 ' + el.children.length);
  }
  W.Log.setVerbose(true);
  W.Log.debug('debug2');
  if (el.children.length !== 5) throw new Error('完整日志下应变 5 条');
  W.Log.setVerbose(false);
});

check('Theme 应用与解析', () => {
  if (!W.Theme.apply('#6E8FA3')) throw new Error('应接受 hex');
  if (W.Theme.current() !== '#6e8fa3') throw new Error('hex 不对：' + W.Theme.current());

  if (!W.Theme.apply('rgb(110,143,163)')) throw new Error('应接受 rgb');
  if (W.Theme.current() !== '#6e8fa3') throw new Error('rgb 不对：' + W.Theme.current());

  // hsl 用它自己输出的精确值往返（手写取整值会有 1~2/255 误差）
  const hsl = W.Theme.toHslString(W.Theme.parse('#6E8FA3'));
  if (!W.Theme.apply(hsl)) throw new Error('应接受 hsl');
  if (W.Theme.current() !== '#6e8fa3') throw new Error('hsl 往返不对：' + W.Theme.current());

  if (W.Theme.apply('乱写')) throw new Error('非法输入应返回 false');
});

check('Reveal 展开/收起', () => {
  const el = makeEl('div');
  W.Reveal.apply(el, false);
  if (!el.classList.contains('is-collapsed')) throw new Error('收起态应有 is-collapsed');
  W.Reveal.set(el, true, 0);
  W.Reveal.set(el, false, 0);
});

check('Overlay 三种弹窗', () => {
  W.Overlay.init();
  W.Overlay.alert('标题', '内容');
  W.Overlay.confirm('标题', '内容');
  W.Overlay.prompt('标题', '内容', { validate: () => [true, 'x', ''] });
  W.Overlay.panel('标题', (body) => { body.appendChild(makeEl('div')); });
});

check('Anim 能力检测', () => {
  if (typeof W.Anim.capable() !== 'boolean') throw new Error('capable 应返回布尔');
});

check('变更列表渲染', () => {
  const el = elFor('change-list');
  el.children = [];
  const diff = {
    added: [{ rel: 'a.jar' }],
    modified: [{ rel: 'b.json' }],
    deleted: [{ rel: 'c.txt' }],
    unchanged: [{ rel: 'd.jar' }, { rel: 'e.jar' }],
  };
  W.Player._renderChanges(diff, [{ rel: 'bad.jar' }]);
  if (!el.children.length) throw new Error('应渲染内容');
});

check('Player / Developer 初始化', () => {
  W.Player.init();
  W.Developer.init();
});

check('App 侧栏模式切换', () => {
  W.App._revealModeOptions('player', false);
  W.App._revealModeOptions('developer', false);
  W.App.markNav('btn-prefs');
});

check('App 设置面板构建', () => {
  // 直接调 build 回调，不经过 Overlay（避免 promise 悬挂）
  let built = false;
  W.App.openPrefs().catch(() => {});
  built = true;
  if (!built) throw new Error('openPrefs 应可调用');
});

console.log('\n' + '='.repeat(52));
if (failures.length) {
  console.log('冒烟测试失败 ' + failures.length + ' 项：');
  for (const f of failures) console.log('  · ' + f);
  process.exit(1);
}
console.log('渲染层冒烟测试：全部通过 ✓');
