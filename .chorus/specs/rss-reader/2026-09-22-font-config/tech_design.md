# Tech Design: 自定义字体配置

- 模块: rss-reader / font-config
- 日期: 2026-09-22

## 架构

```
CSS 变量（:root 定义，body/.article/.article code 消费）
  --font-ui:    UI 字体族（body）
  --font-read:  正文字体族（.article）
  --font-mono:  等宽字体族（.article code）
  --font-read-size:  正文字号 px（.article）
  --font-read-line:  正文行高（.article）

Rust 侧（src-tauri）
  list_font_families() 命令 → Vec<String>（按平台调 font-kit 或 fontconfig）
  settings key: ui.font_ui / ui.font_read / ui.font_mono / ui.font_read_size / ui.font_read_line

JS 侧（ui/app.js）
  SETTING_DROPDOWNS 新增三个字体下拉（choices 来自 list_font_families）
  字号/行高滑块 → set_font_size/set_font_line 命令 → CSS 变量即时更新
```

## CSS 变量设计

```css
:root {
  --font-ui: -apple-system, "Noto Sans CJK SC", ...;  /* 现默认值 */
  --font-read: inherit;  /* 默认跟随 --font-ui */
  --font-mono: "JetBrains Mono", "DejaVu Sans Mono", ...;
  --font-read-size: 14px;
  --font-read-line: 1.55;
}
body { font-family: var(--font-ui); font-size: 14px; }
.article { font-family: var(--font-read); font-size: var(--font-read-size); line-height: var(--font-read-line); }
.article code { font-family: var(--font-mono); font-size: calc(var(--font-read-size) * 0.89); }
```

用户选字体 → `document.documentElement.style.setProperty('--font-ui', value)` → 即时生效。
「跟随系统」= 清除变量 → 回退到 :root 默认值。

## 字体枚举（src-tauri）

不引入 font-kit（重依赖传递树大），直接调平台命令：
- Linux: `fc-list --format='%{family[0]}\n' | sort -u`（fontconfig 必装，deb depends 已声明）
- Windows: `powershell -c "[System.Drawing.FontFamily]::Families | % {$_.Name}"`（或 DWrite via winapi）
- macOS: `system_profiler SPFontsDataType` 太慢 → `ATSUFontFindFromName` 简化为 `fc-list` fallback（macOS 装了 fontconfig 则可用）或嵌入 enum

简化：仅 Linux 走 fc-list，Windows/macOS 返回空列表（UI 只显示「跟随系统」），后续增强。避免跨平台复杂度。

## 测试

- core: settings 键往返
- src-tauri: list_font_families 命令测试（Linux 下非空）
- UI: 手动清单（真机切换三类字体 + 字号滑块）
