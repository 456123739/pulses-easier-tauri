// theme.js — 主题色（默认青灰，可自定义）
//
// 支持的输入格式：
//   #rgb / #rrggbb / rrggbb（不带 #）
//   rgb(r, g, b) / rgba(r, g, b, a)
//   hsl(h, s%, l%) / hsla(h, s%, l%, a)
//
// 应用时同时算出：
//   --accent       基色
//   --accent-hi    提亮版（hover）
//   --accent-soft  半透明版（选中底色 / 弱强调）

const Theme = {
  DEFAULT: '#6E8FA3',        // 青灰
  _base: '#6E8FA3',

  // ── 解析各种颜色写法 ──
  parse(input) {
    if (!input) return null;
    let s = String(input).trim().toLowerCase();

    // #rgb / #rrggbb / rrggbb
    const hexMatch = s.match(/^#?([0-9a-f]{3}|[0-9a-f]{6})$/);
    if (hexMatch) {
      let h = hexMatch[1];
      if (h.length === 3) h = h[0] + h[0] + h[1] + h[1] + h[2] + h[2];
      return this._fromRgb(
        parseInt(h.slice(0, 2), 16),
        parseInt(h.slice(2, 4), 16),
        parseInt(h.slice(4, 6), 16));
    }

    // rgb() / rgba()
    let m = s.match(/^rgba?\(\s*([\d.]+)[\s,]+([\d.]+)[\s,]+([\d.]+)/);
    if (m) {
      return this._fromRgb(+m[1], +m[2], +m[3]);
    }

    // hsl() / hsla()
    m = s.match(/^hsla?\(\s*([\d.]+)[\s,]+([\d.]+)%?[\s,]+([\d.]+)%?/);
    if (m) {
      return this._fromHsl(+m[1], +m[2], +m[3]);
    }

    return null;
  },

  _fromRgb(r, g, b) {
    const clamp = (v) => Math.max(0, Math.min(255, Math.round(v)));
    return { r: clamp(r), g: clamp(g), b: clamp(b) };
  },

  _fromHsl(h, s, l) {
    h = ((h % 360) + 360) % 360;
    s = Math.max(0, Math.min(100, s)) / 100;
    l = Math.max(0, Math.min(100, l)) / 100;
    const c = (1 - Math.abs(2 * l - 1)) * s;
    const x = c * (1 - Math.abs(((h / 60) % 2) - 1));
    const m = l - c / 2;
    let rgb;
    if (h < 60)       rgb = [c, x, 0];
    else if (h < 120) rgb = [x, c, 0];
    else if (h < 180) rgb = [0, c, x];
    else if (h < 240) rgb = [0, x, c];
    else if (h < 300) rgb = [x, 0, c];
    else              rgb = [c, 0, x];
    return this._fromRgb((rgb[0] + m) * 255, (rgb[1] + m) * 255, (rgb[2] + m) * 255);
  },

  toHex(rgb) {
    const h = (v) => v.toString(16).padStart(2, '0');
    return '#' + h(rgb.r) + h(rgb.g) + h(rgb.b);
  },

  toRgbString(rgb) {
    return `rgb(${rgb.r}, ${rgb.g}, ${rgb.b})`;
  },

  toHslString(rgb) {
    const r = rgb.r / 255, g = rgb.g / 255, b = rgb.b / 255;
    const max = Math.max(r, g, b), min = Math.min(r, g, b);
    const l = (max + min) / 2;
    let h = 0, s = 0;
    if (max !== min) {
      const d = max - min;
      s = l > 0.5 ? d / (2 - max - min) : d / (max + min);
      if (max === r)      h = ((g - b) / d + (g < b ? 6 : 0)) * 60;
      else if (max === g) h = ((b - r) / d + 2) * 60;
      else                h = ((r - g) / d + 4) * 60;
    }
    // 保留 1 位小数：整数取整会让 hsl 解析回来差 1~2/255
    const r1 = (v) => Math.round(v * 10) / 10;
    return `hsl(${r1(h)}, ${r1(s * 100)}%, ${r1(l * 100)}%)`;
  },

  lighten(rgb, amount) {
    const f = (v) => Math.max(0, Math.min(255, Math.round(v + (255 - v) * amount)));
    return { r: f(rgb.r), g: f(rgb.g), b: f(rgb.b) };
  },

  // ── 应用到界面 ──
  apply(input) {
    const rgb = this.parse(input);
    if (!rgb) return false;

    const hex = this.toHex(rgb);
    this._base = hex;

    const hi = this.toHex(this.lighten(rgb, 0.16));
    const soft = `rgba(${rgb.r}, ${rgb.g}, ${rgb.b}, 0.16)`;

    const root = document.documentElement;
    root.style.setProperty('--accent', hex);
    root.style.setProperty('--accent-hi', hi);
    root.style.setProperty('--accent-soft', soft);
    return true;
  },

  current() { return this._base; },
  currentRgb() { return this.parse(this._base) || this.parse(this.DEFAULT); },

  // 预设
  PRESETS: [
    ['青灰', '#6E8FA3'],
    ['天青', '#4FA8C0'],
    ['靛蓝', '#5B7BE8'],
    ['紫',   '#BF5AF2'],
    ['粉',   '#FF375F'],
    ['橙',   '#FF9F0A'],
    ['绿',   '#30D158'],
    ['红',   '#FF453A'],
    ['石板', '#8E8E93'],
  ],
};

window.Theme = Theme;
