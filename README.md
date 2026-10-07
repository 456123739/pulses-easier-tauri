# Pulses Easier（Tauri 版）

Minecraft 整合包更新器 —— 用 **Tauri 2 + Rust** 重写，前端是纯 HTML/CSS/JS。

## 为什么用 Tauri

| | Python/CTk 版 | Electron 版 | **Tauri 版** |
|---|---|---|---|
| 运行时 | Tkinter | Chromium（~60MB） | **系统 WebView2（0MB）** |
| 安装包 | 27MB | 76MB | **~5-8MB** |
| 动画 | Tk `after` 补间，打包后失效 | CSS，正常 | **CSS，正常** |
| 玻璃质感 | ctypes 调 Win32 | vibrancy | **windowEffects 原生** |

Tk 版那些"打包后动画全丢、侧边栏重叠崩坏"的问题，根因是 Tk 控件没有真正的合成层——
所有动画都得靠 `after` 手写补间，而补间依赖 `customtkinter.__file__` 判断真实 GUI，
打包后判断失效 → 动画跳终态 → 颜色压暗不恢复、padx 停在偏移位 → 重叠。

WebView 里这些全都不存在：CSS transition / animation 由合成器直接跑，GPU 加速，
和进程是否打包、库在哪，毫无关系。

## 结构

```
src/
  renderer/            # 前端（纯静态，直接被打进二进制）
    index.html
    css/theme.css      # 主题变量（对应 Python theme.py）
    css/main.css       # 布局 + 全部动画（对应 motion.py + glass.py + polish.py）
    js/motion.js       # 动画编排（CSS transition 驱动）
    js/glass.js        # 玻璃质感
    js/widgets/        # overlay.js（窗内弹窗）/ tooltip.js
    js/views/          # player.js / developer.js
    js/preload-tauri.js# Tauri invoke 桥接（对应 Electron preload.js）
    js/app.js          # 主控制器 + 模式切换 + 侧边栏错开动画
    assets/            # logo（必须在 frontendDist 内才能被加载）
src-tauri/
  src/main.rs          # 入口 + command 注册
  src/commands.rs      # 所有 #[tauri::command]
  src/db.rs            # 数据库（database.py + dbmigrate.py）
  src/differ.rs        # 差异比对（differ.py）
  src/updater.rs       # 更新计划构建与执行（updater.py）
  src/eapack.rs        # 更新包读写/解压/导出（eapack.py + pack_info.py）
  src/downloader.rs    # 下载（downloader.py）
  tauri.conf.json      # 窗口/亚克力/NSIS 安装程序配置
  capabilities/        # 权限（dialog / shell）
```

## 开发

```bash
npm install          # 装 Tauri CLI
npm run dev          # 本地开发（需要系统 WebView）
npm run build        # 出安装程序（Windows 上是 NSIS .exe）
```

## 打包

Windows 安装程序由 GitHub Actions 在 `windows-latest` 上生成：

```bash
git tag v0.7.0-glass-beta
git push origin v0.7.0-glass-beta
```

产物：`src-tauri/target/release/bundle/nsis/*.exe`

## 功能

- 玩家端：拖入更新包 → 比对 → 应用更新 → 完成后提示拖入下一个
- 开发者端：选源/目标整合包 → 比对 → 编辑更新日志 → 导出更新包
- 数据库：迁移 / 切换 / 新建（`~/.pulses_easier/config.json` 记录路径）
- 全窗内弹窗（不新开窗口）、窗内悬停提示、液态玻璃、全链路动画
