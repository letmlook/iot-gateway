# 免费可商用字体调研与选型

## 来源

- **Google Fonts**：https://fonts.google.com（国内镜像：googlefonts.cn）
- 全部为开源字体，可免费用于个人与商业项目（SIL OFL、Apache 等协议）

## 无衬线（界面 / 正文）候选

| 字体 | 特点 | 适用场景 |
|------|------|----------|
| **Inter** | 中性、高可读，专为屏幕优化 | 通用 UI、仪表盘 |
| **Plus Jakarta Sans** | 几何感、偏商务与科技，多字重 | 仪表盘、后台、品牌感更强 |
| **DM Sans** | 简洁现代 | 产品 UI |
| **Outfit** | 几何、偏科技与冷静 | 仪表盘、数据界面 |
| **Roboto** | 安卓默认，辨识度高 | 通用 |
| **Open Sans** | 清晰易读 | 正文、长文 |
| **Noto Sans / Noto Sans SC** | 多语言、含简体中文 | 中英混排、国际化 |

## 等宽（代码 / ID / 数据）候选

| 字体 | 特点 |
|------|------|
| **JetBrains Mono** | 编程友好、连字、多字重 |
| **Fira Code** | 连字、GitHub 常用 |
| **Source Code Pro** | Adobe 开源、稳重 |

## 中文显示

- 界面主字体（如 Plus Jakarta Sans、Inter）**不包含中文**，需依赖系统字体回退。
- 当前回退顺序：`PingFang SC`（苹果）、`Microsoft YaHei`（Windows）、`Hiragino Sans GB`，保证中文显示正常。
- 若需统一中英风格，可额外加载 **Noto Sans SC**（体积较大，按需引入）。

## 本项目选型与实施

- **无衬线（全站正文/标题/按钮）**：**Plus Jakarta Sans**  
  - 字重：400、500、600、700  
  - 回退：Inter → PingFang SC → Microsoft YaHei → sans-serif  

- **等宽（代码、设备 ID、数值）**：**JetBrains Mono**  
  - 字重：400、500、600  
  - 回退：ui-monospace → SF Mono → Consolas → monospace  

**本地字体文件**（不依赖 Google Fonts CDN）：

- `web/public/fonts/PlusJakartaSans-latin.woff2` — 拉丁字符集
- `web/public/fonts/JetBrainsMono-latin.woff2` — 拉丁字符集  

在 `web/src/style.css` 顶部通过 `@font-face` 引用上述路径（`/fonts/...`），全站通过 CSS 变量 `--font-sans`、`--font-mono` 统一使用。
