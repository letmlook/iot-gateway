# Warm Craft Frontend Redesign — Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Redesign the IoT Gateway frontend with a "Warm Craft" aesthetic — collapsible sidebar nav, light/dark mode, terracotta-on-cream palette, Source Serif 4 headings.

**Architecture:** Replace top-nav layout with fixed sidebar (collapsible to icons) + scrollable content area + bottom status bar. Replace 4-theme system with simple light/dark mode manager. Rewrite all CSS variables and component styles for the new palette.

**Tech Stack:** Vue 3 + Element Plus + Vite + Vue Router + vue-i18n (no changes to this stack)

---

## File Map

| Action | File | Responsibility |
|--------|------|----------------|
| CREATE | `public/fonts/SourceSerif4-latin.woff2` | Heading font |
| MODIFY | `index.html` | Meta tags, title |
| REWRITE | `src/themes.js` | Light/dark mode manager |
| REWRITE | `src/style.css` | All CSS variables, component styles, Element Plus overrides |
| CREATE | `src/components/ModeToggle.vue` | Sun/moon toggle button |
| CREATE | `src/components/SidebarNav.vue` | Sidebar navigation with collapse |
| CREATE | `src/components/StatusBar.vue` | Bottom health status bar |
| CREATE | `src/components/PageHeader.vue` | Reusable page header |
| REWRITE | `src/App.vue` | Sidebar shell layout |
| MODIFY | `src/main.js` | Update theme init import |
| DELETE | `src/components/ThemeSwitcher.vue` | Replaced by ModeToggle |
| MODIFY | `src/views/*.vue` | Replace page headers with PageHeader component |
| MODIFY | `vite.config.js` | Ensure font MIME types served |

---

### Task 1: Download Source Serif 4 Font

**Files:**
- Create: `web/public/fonts/SourceSerif4-latin.woff2`

- [ ] **Step 1: Download Source Serif 4 WOFF2 from Google Fonts**

The font is needed for serif headings. Download the latin subset WOFF2.

Run:
```bash
curl -L "https://fonts.google.com/download?family=Source+Serif+4" -o /tmp/source-serif4.zip
```

If the above URL doesn't work directly, use this specific approach:
```bash
curl -L -H "User-Agent: Mozilla/5.0" "https://fonts.googleapis.com/css2?family=Source+Serif+4:wght@400;600&subset=latin" -o /tmp/ss4-css.txt
```

Actually, the simplest reliable method is to use the google-webfonts-helper or download directly:

```bash
# Create a helper script to download the font
cat > /tmp/download-font.sh << 'SCRIPT'
#!/bin/bash
# Download Source Serif 4 latin WOFF2 (regular + 600 weights)
BASE="https://cdn.jsdelivr.net/npm/@fontsource/source-serif-4/files"
OUT_DIR="/Users/lipeng/iot-gateway/web/public/fonts"

# Download latin-ext and latin subsets, pick the right ones
# We'll use the latin subset only

# Regular weight (400)
curl -sL "$BASE/source-serif-4-latin-400-normal.woff2" -o "$OUT_DIR/SourceSerif4-latin.woff2"
echo "Downloaded regular weight"

# Bold weight (600) - we use wght 400-600 range, single file with variable font would be better
# Actually let's get the variable font
curl -sL "https://cdn.jsdelivr.net/npm/@fontsource/source-serif-4/files/source-serif-4-latin-wght-normal.woff2" -o "$OUT_DIR/SourceSerif4-latin.woff2"
echo "Downloaded variable font"
SCRIPT

bash /tmp/download-font.sh
```

Verify:
```bash
ls -la /Users/lipeng/iot-gateway/web/public/fonts/SourceSerif4-latin.woff2
# Expected: file exists, ~50-150KB
```

- [ ] **Step 2: Commit**

```bash
git add web/public/fonts/SourceSerif4-latin.woff2
git commit -m "feat: add Source Serif 4 font for Warm Craft headings

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 2: Update index.html

**Files:**
- Modify: `web/index.html`

- [ ] **Step 1: Update meta tags and preload font**

Replace the current `index.html`:

```html
<!doctype html>
<html lang="zh-CN">
  <head>
    <meta charset="UTF-8" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <meta name="description" content="IoT Gateway - 工业物联网网关管理控制台" />
    <meta name="theme-color" content="#faf7f2" media="(prefers-color-scheme: light)" />
    <meta name="theme-color" content="#1c1917" media="(prefers-color-scheme: dark)" />
    <link rel="icon" type="image/svg+xml" href="data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' viewBox='0 0 32 32' fill='none'%3E%3Crect x='2' y='2' width='28' height='28' rx='7' fill='%235c4a3a'/%3E%3Cpath d='M10 8v16l12-8-12-8z' fill='%23c17f4f'/%3E%3Ccircle cx='22' cy='16' r='3' fill='%23c17f4f'/%3E%3C/svg%3E" />
    <link rel="preload" href="/fonts/SourceSerif4-latin.woff2" as="font" type="font/woff2" crossorigin />
    <link rel="preload" href="/fonts/JetBrainsMono-latin.woff2" as="font" type="font/woff2" crossorigin />
    <title>IoT Gateway</title>
  </head>
  <body>
    <div id="app"></div>
    <script type="module" src="/src/main.js"></script>
  </body>
</html>
```

- [ ] **Step 2: Commit**

```bash
git add web/index.html
git commit -m "feat: update index.html meta tags for Warm Craft theme

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 3: Rewrite themes.js to Light/Dark Mode Manager

**Files:**
- Rewrite: `web/src/themes.js`

- [ ] **Step 1: Replace themes.js with mode manager**

The new file manages only `light` and `dark` modes, replacing the 4-theme system.

```js
/**
 * IoT Gateway — Light/Dark Mode Manager
 * Warm Craft design system: single polished theme with light + dark variants
 */

const MODE_STORAGE_KEY = 'gateway_mode'
const DEFAULT_MODE = 'light'

// CSS variable definitions for both modes
const lightPalette = {
  '--primary': '#5c4a3a',
  '--primary-dim': '#4a3828',
  '--primary-light': '#7a6b5c',
  '--primary-glow': 'rgba(92, 74, 58, 0.08)',
  '--on-primary': '#ffffff',

  '--accent': '#c17f4f',
  '--accent-dim': '#a8653a',
  '--accent-light': '#e8c9a8',
  '--accent-glow': 'rgba(193, 127, 79, 0.12)',

  '--success': '#6b8e6b',
  '--success-light': '#d4e5d4',
  '--warning': '#c47b4a',
  '--warning-light': '#fce8d4',
  '--danger': '#c45a4a',
  '--danger-light': '#f8d7d4',
  '--info': '#8b7d6e',
  '--accent-purple': '#8b6b8e',

  '--bg-base': '#faf7f2',
  '--bg-surface': '#fffbf5',
  '--bg-elevated': '#f5efe5',
  '--bg-overlay': '#ede6db',
  '--bg-inset': '#e8ddd0',
  '--bg-hover': 'rgba(193, 127, 79, 0.06)',

  '--border-subtle': '#e8ddd0',
  '--border-default': '#d4c5b2',
  '--border-strong': '#bfae9a',

  '--text-primary': '#5c4a3a',
  '--text-secondary': '#7a6b5c',
  '--text-muted': '#bfae9a',
  '--text-inverse': '#ffffff',

  '--state-running': '#6b8e6b',
  '--state-stopped': '#bfae9a',
  '--state-error': '#c45a4a',
  '--state-syncing': '#c47b4a',

  '--shadow-xs': '0 1px 2px rgba(92, 74, 58, 0.04)',
  '--shadow-sm': '0 1px 3px rgba(92, 74, 58, 0.05)',
  '--shadow-md': '0 4px 12px rgba(92, 74, 58, 0.07)',
  '--shadow-lg': '0 8px 24px rgba(92, 74, 58, 0.10)',
  '--shadow-glow': '0 0 0 3px rgba(193, 127, 79, 0.20)',
  '--shadow-accent': '0 4px 14px rgba(193, 127, 79, 0.20)',

  // Element Plus overrides (light)
  '--el-color-primary': '#c17f4f',
  '--el-color-primary-light-3': '#d4a574',
  '--el-color-primary-light-5': '#e0bfa0',
  '--el-color-primary-light-7': '#ecd9c8',
  '--el-color-primary-light-8': '#f3e8dc',
  '--el-color-primary-light-9': '#faf4ed',
  '--el-color-primary-dark-2': '#a8653a',
  '--el-color-success': '#6b8e6b',
  '--el-color-warning': '#c47b4a',
  '--el-color-danger': '#c45a4a',
  '--el-color-info': '#8b7d6e',
  '--el-border-color': '#d4c5b2',
  '--el-border-color-light': '#e8ddd0',
  '--el-fill-color-blank': '#fffbf5',
  '--el-fill-color-light': '#f5efe5',
  '--el-bg-color': '#faf7f2',
  '--el-font-family': "-apple-system, BlinkMacSystemFont, 'Segoe UI', 'PingFang SC', 'Microsoft YaHei', sans-serif",
  '--el-text-color-primary': '#5c4a3a',
  '--el-text-color-regular': '#7a6b5c',
  '--el-text-color-secondary': '#bfae9a',

  '--table-stripe-bg': 'rgba(92, 74, 58, 0.02)',
  '--table-row-hover-bg': 'rgba(92, 74, 58, 0.04)',
}

const darkPalette = {
  '--primary': '#d4c5b2',
  '--primary-dim': '#bfae9a',
  '--primary-light': '#e8ddd0',
  '--primary-glow': 'rgba(212, 197, 178, 0.08)',
  '--on-primary': '#1c1917',

  '--accent': '#d4a574',
  '--accent-dim': '#bf8b5e',
  '--accent-light': '#e8c9a8',
  '--accent-glow': 'rgba(212, 165, 116, 0.15)',

  '--success': '#7aaa7a',
  '--success-light': '#3a5a3a',
  '--warning': '#d4a074',
  '--warning-light': '#5a3a2a',
  '--danger': '#d48a7a',
  '--danger-light': '#5a2a2a',
  '--info': '#a0917b',
  '--accent-purple': '#9b8bae',

  '--bg-base': '#1c1917',
  '--bg-surface': '#252220',
  '--bg-elevated': '#2d2a27',
  '--bg-overlay': '#353230',
  '--bg-inset': '#1a1715',
  '--bg-hover': 'rgba(212, 165, 116, 0.08)',

  '--border-subtle': '#3d3833',
  '--border-default': '#4d4843',
  '--border-strong': '#5d5853',

  '--text-primary': '#d4c5b2',
  '--text-secondary': '#a0917b',
  '--text-muted': '#8b7d6e',
  '--text-inverse': '#1c1917',

  '--state-running': '#7aaa7a',
  '--state-stopped': '#8b7d6e',
  '--state-error': '#d48a7a',
  '--state-syncing': '#d4a074',

  '--shadow-xs': '0 1px 2px rgba(0, 0, 0, 0.15)',
  '--shadow-sm': '0 1px 3px rgba(0, 0, 0, 0.20)',
  '--shadow-md': '0 4px 12px rgba(0, 0, 0, 0.25)',
  '--shadow-lg': '0 8px 24px rgba(0, 0, 0, 0.30)',
  '--shadow-glow': '0 0 0 3px rgba(212, 165, 116, 0.25)',
  '--shadow-accent': '0 4px 14px rgba(212, 165, 116, 0.25)',

  // Element Plus overrides (dark)
  '--el-color-primary': '#d4a574',
  '--el-color-primary-light-3': '#bf8b5e',
  '--el-color-primary-light-5': '#a8a060',
  '--el-color-primary-light-7': '#8b7d6e',
  '--el-color-primary-light-8': '#6a5d4e',
  '--el-color-primary-light-9': '#4a3d2e',
  '--el-color-primary-dark-2': '#e8c9a8',
  '--el-color-success': '#7aaa7a',
  '--el-color-warning': '#d4a074',
  '--el-color-danger': '#d48a7a',
  '--el-color-info': '#a0917b',
  '--el-border-color': '#4d4843',
  '--el-border-color-light': '#3d3833',
  '--el-fill-color-blank': '#252220',
  '--el-fill-color-light': '#2d2a27',
  '--el-bg-color': '#1c1917',
  '--el-font-family': "-apple-system, BlinkMacSystemFont, 'Segoe UI', 'PingFang SC', 'Microsoft YaHei', sans-serif",
  '--el-text-color-primary': '#d4c5b2',
  '--el-text-color-regular': '#a0917b',
  '--el-text-color-secondary': '#8b7d6e',

  '--table-stripe-bg': 'rgba(212, 165, 116, 0.04)',
  '--table-row-hover-bg': 'rgba(212, 165, 116, 0.08)',
}

export function getSavedMode() {
  try {
    const mode = localStorage.getItem(MODE_STORAGE_KEY)
    return (mode === 'dark') ? 'dark' : DEFAULT_MODE
  } catch {
    return DEFAULT_MODE
  }
}

export function saveMode(mode) {
  try {
    localStorage.setItem(MODE_STORAGE_KEY, mode)
  } catch (e) {
    console.warn('Failed to save mode:', e)
  }
}

export function applyMode(mode) {
  const palette = mode === 'dark' ? darkPalette : lightPalette
  const root = document.documentElement

  Object.entries(palette).forEach(([prop, value]) => {
    root.style.setProperty(prop, value)
  })

  if (mode === 'dark') {
    root.classList.add('dark')
    document.body.classList.add('dark')
  } else {
    root.classList.remove('dark')
    document.body.classList.remove('dark')
  }

  // Also set Element Plus specific overrides that need direct style
  root.style.setProperty('--el-bg-color', palette['--bg-surface'])
  root.style.setProperty('--el-fill-color-blank', palette['--bg-surface'])
  root.style.setProperty('--el-fill-color-light', palette['--bg-elevated'])
  root.style.setProperty('--el-border-color', palette['--border-default'])
  root.style.setProperty('--el-border-color-light', palette['--border-subtle'])
  root.style.setProperty('--el-text-color-primary', palette['--text-primary'])
  root.style.setProperty('--el-text-color-regular', palette['--text-secondary'])
  root.style.setProperty('--el-text-color-secondary', palette['--text-muted'])

  saveMode(mode)
}

export function initMode() {
  const mode = getSavedMode()
  applyMode(mode)
  return mode
}

export function toggleMode() {
  const current = getSavedMode()
  const next = current === 'dark' ? 'light' : 'dark'
  applyMode(next)
  return next
}

export default {
  getSavedMode,
  saveMode,
  applyMode,
  initMode,
  toggleMode,
}
```

- [ ] **Step 2: Commit**

```bash
git add web/src/themes.js
git commit -m "refactor: replace 4-theme system with light/dark mode manager

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 4: Rewrite style.css

**Files:**
- Rewrite: `web/src/style.css`

This is the largest file change. It defines all CSS variables, font faces, reset, typography, component styles, and Element Plus overrides.

- [ ] **Step 1: Write the complete new style.css**

```css
/* ============================================
   IoT Gateway — Warm Craft Design System
   Light + Dark mode, serif headings, terracotta accents
   ============================================ */

/* ─────────── Fonts ─────────── */
@font-face {
  font-family: 'Source Serif 4';
  font-style: normal;
  font-weight: 400 600;
  font-display: swap;
  src: url('/fonts/SourceSerif4-latin.woff2') format('woff2');
  unicode-range: U+0000-00FF, U+0131, U+0152-0153, U+02BB-02BC, U+02C6, U+02DA, U+02DC, U+0304, U+0308, U+0329, U+2000-206F, U+20AC, U+2122, U+2191, U+2193, U+2212, U+2215, U+FEFF, U+FFFD;
}

@font-face {
  font-family: 'JetBrains Mono';
  font-style: normal;
  font-weight: 400 600;
  font-display: swap;
  src: url('/fonts/JetBrainsMono-latin.woff2') format('woff2');
  unicode-range: U+0000-00FF, U+0131, U+0152-0153, U+02BB-02BC, U+02C6, U+02DA, U+02DC, U+0304, U+0308, U+0329, U+2000-206F, U+20AC, U+2122, U+2191, U+2193, U+2212, U+2215, U+FEFF, U+FFFD;
}

/* ─────────── CSS Variables (light mode defaults) ─────────── */
:root {
  --font-serif: 'Source Serif 4', Georgia, 'Noto Serif', serif;
  --font-sans: -apple-system, BlinkMacSystemFont, 'Segoe UI', 'PingFang SC', 'Microsoft YaHei', 'Hiragino Sans GB', sans-serif;
  --font-mono: 'JetBrains Mono', ui-monospace, 'SF Mono', 'Cascadia Code', 'Consolas', monospace;

  --text-xs: 0.75rem;
  --text-sm: 0.8125rem;
  --text-base: 0.9375rem;
  --text-md: 1rem;
  --text-lg: 1.125rem;
  --text-xl: 1.25rem;
  --text-2xl: 1.5rem;

  --radius-xs: 4px;
  --radius-sm: 6px;
  --radius-md: 8px;
  --radius-lg: 12px;
  --radius-xl: 20px;

  --transition-fast: 0.15s ease;
  --transition-normal: 0.2s ease;
  --transition-slow: 0.3s ease;

  --sidebar-width: 220px;
  --sidebar-collapsed: 60px;
  --statusbar-height: 32px;

  /* Default light palette — injected by themes.js initMode() */
}

/* ─────────── Reset & Base ─────────── */
*, *::before, *::after {
  box-sizing: border-box;
  margin: 0;
  padding: 0;
}

html {
  font-size: 16px;
  -webkit-font-smoothing: antialiased;
  -moz-osx-font-smoothing: grayscale;
  text-rendering: optimizeLegibility;
  transition: background-color var(--transition-normal), color var(--transition-normal);
}

body {
  font-family: var(--font-sans);
  font-size: var(--text-base);
  font-weight: 400;
  background: var(--bg-base);
  color: var(--text-primary);
  line-height: 1.55;
  min-height: 100vh;
  overflow: hidden;
  transition: background-color var(--transition-normal), color var(--transition-normal);
}

/* ─────────── Typography ─────────── */
h1, h2, h3, h4 {
  font-family: var(--font-serif);
  font-weight: 600;
  color: var(--text-primary);
  letter-spacing: -0.01em;
  line-height: 1.3;
}

h1 { font-size: var(--text-2xl); }
h2 { font-size: var(--text-xl); }
h3 { font-size: var(--text-lg); }
h4 { font-size: var(--text-md); }

p { line-height: 1.6; margin: 0 0 0.5em; }

a {
  color: var(--accent);
  text-decoration: none;
  transition: color var(--transition-fast);
}
a:hover { color: var(--accent-dim); }

code, pre {
  font-family: var(--font-mono);
  font-size: 0.9em;
}

code {
  background: var(--bg-elevated);
  color: var(--text-primary);
  padding: 0.15em 0.4em;
  border-radius: var(--radius-xs);
  border: 1px solid var(--border-subtle);
}

/* ─────────── App Shell ─────────── */
.app-shell {
  display: flex;
  height: 100vh;
  overflow: hidden;
}

.app-sidebar {
  width: var(--sidebar-width);
  flex-shrink: 0;
  height: 100vh;
  overflow-y: auto;
  overflow-x: hidden;
  background: var(--bg-surface);
  border-right: 1px solid var(--border-subtle);
  transition: width var(--transition-normal);
  display: flex;
  flex-direction: column;
  z-index: 200;
  scrollbar-width: thin;
  scrollbar-color: var(--border-subtle) transparent;
}

.app-sidebar.collapsed {
  width: var(--sidebar-collapsed);
}

.app-main {
  flex: 1;
  display: flex;
  flex-direction: column;
  min-width: 0;
  height: 100vh;
}

.app-content {
  flex: 1;
  overflow-y: auto;
  overflow-x: hidden;
  padding: 24px;
  scrollbar-width: thin;
  scrollbar-color: var(--border-subtle) transparent;
}

.app-content::-webkit-scrollbar { width: 6px; }
.app-content::-webkit-scrollbar-track { background: transparent; }
.app-content::-webkit-scrollbar-thumb {
  background: var(--border-subtle);
  border-radius: 10px;
}

.app-statusbar {
  height: var(--statusbar-height);
  flex-shrink: 0;
  background: var(--bg-elevated);
  border-top: 1px solid var(--border-subtle);
  display: flex;
  align-items: center;
  padding: 0 16px;
  gap: 24px;
  font-size: var(--text-xs);
  color: var(--text-muted);
  z-index: 100;
}

/* ─────────── Sidebar ─────────── */
.sidebar-brand {
  padding: 16px 16px 12px;
  border-bottom: 1px solid var(--border-subtle);
  display: flex;
  align-items: center;
  gap: 10px;
  min-height: 56px;
}

.sidebar-brand .brand-icon {
  width: 28px;
  height: 28px;
  flex-shrink: 0;
}

.sidebar-brand .brand-text {
  font-family: var(--font-serif);
  font-size: 1rem;
  font-weight: 600;
  color: var(--text-primary);
  white-space: nowrap;
  overflow: hidden;
}

.sidebar-brand .brand-version {
  font-size: 0.65rem;
  color: var(--text-muted);
  white-space: nowrap;
}

.sidebar-collapse-btn {
  margin-left: auto;
  width: 28px;
  height: 28px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-sm);
  cursor: pointer;
  color: var(--text-muted);
  flex-shrink: 0;
  transition: all var(--transition-fast);
}

.sidebar-collapse-btn:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.sidebar-section-label {
  font-size: 0.6rem;
  text-transform: uppercase;
  letter-spacing: 0.08em;
  color: var(--text-muted);
  padding: 16px 16px 4px;
  white-space: nowrap;
  overflow: hidden;
}

.sidebar-nav {
  display: flex;
  flex-direction: column;
  gap: 1px;
  padding: 4px 8px;
}

.sidebar-nav-item {
  display: flex;
  align-items: center;
  gap: 10px;
  padding: 8px 12px;
  border-radius: var(--radius-sm);
  font-size: var(--text-sm);
  color: var(--text-secondary);
  text-decoration: none;
  cursor: pointer;
  transition: all var(--transition-fast);
  white-space: nowrap;
  overflow: hidden;
  position: relative;
  font-family: var(--font-sans);
}

.sidebar-nav-item:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.sidebar-nav-item.active {
  background: var(--accent-glow);
  color: var(--accent-dim);
  font-weight: 500;
}

.sidebar-nav-item.active::before {
  content: '';
  position: absolute;
  left: 0;
  top: 6px;
  bottom: 6px;
  width: 2px;
  background: var(--accent);
  border-radius: 0 2px 2px 0;
}

.sidebar-nav-item .nav-icon {
  width: 18px;
  height: 18px;
  flex-shrink: 0;
  display: flex;
  align-items: center;
  justify-content: center;
  opacity: 0.6;
}

.sidebar-nav-item.active .nav-icon { opacity: 1; }

.sidebar-nav-item .nav-label {
  overflow: hidden;
  text-overflow: ellipsis;
}

.sidebar-nav-item .nav-badge {
  margin-left: auto;
  background: var(--accent);
  color: var(--text-inverse);
  font-size: 0.65rem;
  padding: 1px 6px;
  border-radius: 10px;
  font-weight: 600;
  min-width: 18px;
  text-align: center;
}

/* Collapsed states */
.app-sidebar.collapsed .brand-text,
.app-sidebar.collapsed .brand-version,
.app-sidebar.collapsed .sidebar-section-label,
.app-sidebar.collapsed .nav-label,
.app-sidebar.collapsed .nav-badge {
  display: none;
}

.app-sidebar.collapsed .sidebar-nav-item {
  justify-content: center;
  padding: 10px;
}

.app-sidebar.collapsed .sidebar-nav-item .nav-icon {
  margin: 0;
}

.app-sidebar.collapsed .sidebar-brand {
  justify-content: center;
  padding: 14px 8px;
}

.app-sidebar.collapsed .sidebar-collapse-btn {
  display: none;
}

/* Sidebar bottom section */
.sidebar-footer {
  margin-top: auto;
  padding: 8px;
  border-top: 1px solid var(--border-subtle);
}

/* ─────────── Page Header ─────────── */
.page-header {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  margin-bottom: 24px;
  gap: 16px;
}

.page-header-title {
  font-family: var(--font-serif);
  font-size: var(--text-2xl);
  font-weight: 600;
  color: var(--text-primary);
  line-height: 1.2;
}

.page-header-subtitle {
  font-size: var(--text-sm);
  color: var(--text-muted);
  margin-top: 4px;
}

.page-header-actions {
  display: flex;
  align-items: center;
  gap: 8px;
  flex-shrink: 0;
}

/* ─────────── Cards ─────────── */
.card-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(320px, 1fr));
  gap: 12px;
}

.card {
  background: var(--bg-surface);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-md);
  transition: all var(--transition-fast);
  overflow: hidden;
}

.card:hover {
  border-color: var(--border-default);
  box-shadow: var(--shadow-sm);
}

.card.state-running { border-left: 3px solid var(--state-running); }
.card.state-stopped { border-left: 3px solid var(--state-stopped); }
.card.state-error  { border-left: 3px solid var(--state-error); }

.card-header {
  display: flex;
  justify-content: space-between;
  align-items: flex-start;
  padding: 12px 14px;
  background: var(--bg-elevated);
  border-bottom: 1px solid var(--border-subtle);
}

.card-body {
  padding: 12px 14px;
}

.card-footer {
  display: flex;
  gap: 8px;
  padding: 10px 14px;
  border-top: 1px solid var(--border-subtle);
}


/* Stat cards (dashboard) */
.stat-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(200px, 1fr));
  gap: 12px;
  margin-bottom: 24px;
}

.stat-card {
  background: var(--bg-surface);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-md);
  padding: 16px;
  transition: all var(--transition-fast);
}

.stat-card:hover {
  border-color: var(--border-default);
  box-shadow: var(--shadow-sm);
}

.stat-card-label {
  font-size: var(--text-xs);
  text-transform: uppercase;
  letter-spacing: 0.05em;
  color: var(--text-muted);
  margin-bottom: 6px;
}

.stat-card-value {
  font-family: var(--font-serif);
  font-size: 2rem;
  font-weight: 600;
  color: var(--text-primary);
  line-height: 1;
}

.stat-card-hint {
  font-size: var(--text-xs);
  color: var(--text-muted);
  margin-top: 4px;
}

/* ─────────── Buttons ─────────── */
.btn {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  padding: 8px 14px;
  font-family: var(--font-sans);
  font-size: var(--text-sm);
  font-weight: 500;
  border-radius: var(--radius-sm);
  border: 1px solid transparent;
  cursor: pointer;
  transition: all var(--transition-fast);
  white-space: nowrap;
}

.btn svg { width: 16px; height: 16px; }

.btn-primary {
  background: var(--accent);
  color: var(--text-inverse);
  border-color: var(--accent);
}

.btn-primary:hover {
  background: var(--accent-dim);
  border-color: var(--accent-dim);
  box-shadow: var(--shadow-glow);
}

.btn-secondary {
  background: var(--bg-elevated);
  color: var(--text-primary);
  border-color: var(--border-default);
}

.btn-secondary:hover {
  background: var(--bg-overlay);
  border-color: var(--border-strong);
}

.btn-ghost {
  background: transparent;
  color: var(--text-secondary);
}

.btn-ghost:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.btn-sm { padding: 5px 10px; font-size: var(--text-xs); }
.btn-sm svg { width: 14px; height: 14px; }

.btn:disabled { opacity: 0.5; cursor: not-allowed; }

.icon-btn {
  width: 32px;
  height: 32px;
  display: inline-flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  border: 1px solid var(--border-default);
  border-radius: var(--radius-sm);
  color: var(--text-secondary);
  cursor: pointer;
  transition: all var(--transition-fast);
}

.icon-btn:hover {
  background: var(--bg-hover);
  border-color: var(--border-strong);
  color: var(--text-primary);
}

.icon-btn.danger:hover {
  background: rgba(196, 90, 74, 0.08);
  border-color: var(--danger);
  color: var(--danger);
}

/* ─────────── Status Indicator ─────────── */
.status-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--state-stopped);
  flex-shrink: 0;
}

.status-dot.running {
  background: var(--state-running);
  box-shadow: 0 0 0 2px rgba(107, 142, 107, 0.25);
  animation: status-pulse 2s infinite;
}

.status-dot.error {
  background: var(--state-error);
  box-shadow: 0 0 0 2px rgba(196, 90, 74, 0.25);
  animation: status-pulse 1.5s infinite;
}

@keyframes status-pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.4; }
}

/* ─────────── Tables ─────────── */
.data-table {
  width: 100%;
  border-collapse: collapse;
  background: var(--bg-surface);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-md);
  overflow: hidden;
  font-size: var(--text-sm);
}

.data-table th {
  background: var(--bg-elevated);
  font-weight: 500;
  font-size: var(--text-xs);
  text-transform: uppercase;
  letter-spacing: 0.05em;
  color: var(--text-muted);
  padding: 10px 14px;
  text-align: left;
  border-bottom: 1px solid var(--border-default);
  white-space: nowrap;
}

.data-table td {
  padding: 10px 14px;
  border-bottom: 1px solid var(--border-subtle);
  color: var(--text-primary);
}

.data-table tr:last-child td { border-bottom: none; }
.data-table tr:hover td { background: var(--bg-hover); }

/* Element Plus table tweaks */
.el-table {
  --el-table-border-color: var(--border-subtle);
  --el-table-header-bg-color: var(--bg-elevated);
  --el-table-tr-bg-color: var(--bg-surface);
  --el-table-row-hover-bg-color: var(--bg-hover);
  font-family: var(--font-sans);
}

.el-table--striped .el-table__body tr.el-table__row--striped td {
  background: var(--table-stripe-bg) !important;
}

.el-table__body tr:hover > td {
  background: var(--table-row-hover-bg) !important;
}

.el-table th.el-table__cell {
  background: var(--bg-elevated);
  color: var(--text-muted);
  font-size: var(--text-xs);
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

/* ─────────── Forms ─────────── */
.form-group {
  margin-bottom: 14px;
}

.form-label {
  display: block;
  margin-bottom: 4px;
  font-size: var(--text-sm);
  font-weight: 500;
  color: var(--text-secondary);
}

.form-label .required {
  color: var(--danger);
  margin-left: 2px;
}

.form-input {
  width: 100%;
  padding: 8px 12px;
  font-family: var(--font-sans);
  font-size: var(--text-base);
  background: var(--bg-surface);
  border: 1px solid var(--border-default);
  border-radius: var(--radius-sm);
  color: var(--text-primary);
  transition: all var(--transition-fast);
}

.form-input:focus {
  outline: none;
  border-color: var(--accent);
  box-shadow: 0 0 0 3px var(--accent-glow);
}

.form-input::placeholder { color: var(--text-muted); }
.form-input.mono { font-family: var(--font-mono); }

textarea.form-input {
  resize: vertical;
  min-height: 80px;
}

select.form-input {
  appearance: none;
  background-image: url("data:image/svg+xml,%3Csvg xmlns='http://www.w3.org/2000/svg' width='12' height='12' viewBox='0 0 24 24' fill='none' stroke='%23bfae9a' stroke-width='2'%3E%3Cpath d='M6 9l6 6 6-6'/%3E%3C/svg%3E");
  background-repeat: no-repeat;
  background-position: right 10px center;
  padding-right: 32px;
}

.form-hint {
  display: block;
  margin-top: 4px;
  font-size: var(--text-xs);
  color: var(--text-muted);
}

/* ─────────── Alerts / Messages ─────────── */
.alert {
  display: flex;
  align-items: center;
  gap: 8px;
  padding: 10px 14px;
  border-radius: var(--radius-sm);
  margin-bottom: 16px;
  font-size: var(--text-sm);
}

.alert-error {
  background: var(--danger-light);
  border: 1px solid rgba(196, 90, 74, 0.3);
  color: var(--danger);
}

.alert-warning {
  background: var(--warning-light);
  border: 1px solid rgba(196, 123, 74, 0.3);
  color: var(--warning);
}

/* ─────────── Empty State ─────────── */
.empty-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  padding: 48px 24px;
  text-align: center;
  grid-column: 1 / -1;
}

.empty-state h3 {
  font-family: var(--font-serif);
  font-size: var(--text-lg);
  color: var(--text-secondary);
  margin-bottom: 8px;
}

.empty-state p {
  color: var(--text-muted);
  font-size: var(--text-sm);
  max-width: 320px;
}

/* ─────────── Loading ─────────── */
.loading-state {
  display: flex;
  flex-direction: column;
  align-items: center;
  justify-content: center;
  gap: 12px;
  padding: 48px;
  color: var(--text-muted);
}

.spinner {
  width: 28px;
  height: 28px;
  border: 3px solid var(--border-subtle);
  border-top-color: var(--accent);
  border-radius: 50%;
  animation: spin 0.7s linear infinite;
}

@keyframes spin { to { transform: rotate(360deg); } }

/* ─────────── Modal Overlay ─────────── */
.modal-overlay {
  position: fixed;
  top: 0; left: 0; right: 0; bottom: 0;
  background: rgba(92, 74, 58, 0.3);
  backdrop-filter: blur(3px);
  display: flex;
  align-items: center;
  justify-content: center;
  z-index: 1000;
  animation: fadeIn 0.15s ease;
}

@keyframes fadeIn { from { opacity: 0; } to { opacity: 1; } }

.modal {
  width: 90%;
  max-width: 480px;
  background: var(--bg-surface);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-lg);
  animation: slideUp 0.2s ease;
}

.modal-lg { max-width: 640px; }

@keyframes slideUp {
  from { opacity: 0; transform: translateY(16px); }
  to { opacity: 1; transform: translateY(0); }
}

.modal-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 14px 18px;
  border-bottom: 1px solid var(--border-subtle);
}

.modal-header h2 {
  font-family: var(--font-serif);
  font-size: var(--text-lg);
}

.modal-body { padding: 18px; }

.modal-footer {
  display: flex;
  justify-content: flex-end;
  gap: 8px;
  padding: 12px 18px;
  border-top: 1px solid var(--border-subtle);
  background: var(--bg-elevated);
}

/* ─────────── Tabs ─────────── */
.tab-nav {
  display: flex;
  gap: 2px;
  padding: 4px;
  background: var(--bg-elevated);
  border-radius: var(--radius-md);
  margin-bottom: 16px;
  border: 1px solid var(--border-subtle);
}

.tab-btn {
  flex: 1;
  display: flex;
  align-items: center;
  justify-content: center;
  gap: 6px;
  padding: 8px 14px;
  font-family: var(--font-sans);
  font-size: var(--text-sm);
  font-weight: 500;
  background: transparent;
  border: none;
  border-radius: var(--radius-sm);
  color: var(--text-secondary);
  cursor: pointer;
  transition: all var(--transition-fast);
}

.tab-btn:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.tab-btn.active {
  background: var(--bg-surface);
  color: var(--text-primary);
  box-shadow: var(--shadow-xs);
}

.tab-btn .badge {
  display: inline-flex;
  align-items: center;
  justify-content: center;
  min-width: 20px;
  height: 20px;
  padding: 0 6px;
  background: var(--bg-overlay);
  border-radius: 10px;
  font-size: var(--text-xs);
  font-weight: 600;
}

/* ─────────── Type Badge ─────────── */
.type-badge {
  display: inline-block;
  padding: 2px 8px;
  background: var(--accent-glow);
  color: var(--accent-dim);
  border-radius: 10px;
  font-size: var(--text-xs);
  font-weight: 500;
}

/* ─────────── Plugin Card Variants ─────────── */
.plugin-card.south { border-left: 3px solid var(--success); }
.plugin-card.north { border-left: 3px solid var(--accent-purple); }

.plugin-type-badge.south {
  background: rgba(107, 142, 107, 0.12);
  color: var(--success);
}

.plugin-type-badge.north {
  background: rgba(139, 107, 142, 0.12);
  color: var(--accent-purple);
}

/* ─────────── Login Page ─────────── */
.login-page {
  min-height: 100vh;
  display: flex;
  align-items: center;
  justify-content: center;
  background: var(--bg-base);
  padding: 24px;
}

.login-card {
  width: 100%;
  max-width: 380px;
  background: var(--bg-surface);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-lg);
  box-shadow: var(--shadow-md);
  padding: 32px;
}

.login-card h1 {
  font-family: var(--font-serif);
  font-size: 1.5rem;
  text-align: center;
  margin-bottom: 4px;
}

.login-card .login-subtitle {
  text-align: center;
  font-size: var(--text-sm);
  color: var(--text-muted);
  margin-bottom: 24px;
}

/* ─────────── Monitor Page ─────────── */
.monitor-controls {
  display: flex;
  align-items: flex-end;
  gap: 16px;
  padding: 14px;
  background: var(--bg-surface);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-md);
  margin-bottom: 16px;
  flex-wrap: wrap;
}

.control-group {
  display: flex;
  flex-direction: column;
  gap: 4px;
}

.control-group label {
  font-size: var(--text-xs);
  font-weight: 500;
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

/* ─────────── Node Detail ─────────── */
.detail-header {
  display: flex;
  justify-content: space-between;
  align-items: center;
  margin-bottom: 20px;
  padding: 14px;
  background: var(--bg-surface);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-md);
}

.node-name {
  font-family: var(--font-serif);
  font-size: var(--text-xl);
}

.node-plugin {
  font-family: var(--font-mono);
  font-size: var(--text-xs);
  color: var(--text-muted);
  padding: 2px 8px;
  background: var(--bg-elevated);
  border-radius: var(--radius-xs);
}

.node-state {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  font-size: var(--text-sm);
  padding: 3px 10px;
  border-radius: 10px;
  background: var(--bg-elevated);
}

.node-state.running { color: var(--state-running); }
.node-state.stopped { color: var(--state-stopped); }
.node-state.error   { color: var(--state-error); }

/* ─────────── System Info ─────────── */
.system-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(300px, 1fr));
  gap: 12px;
}

.info-grid {
  display: grid;
  gap: 8px;
}

.info-row {
  display: flex;
  justify-content: space-between;
  align-items: center;
  font-size: var(--text-sm);
}

.info-label { color: var(--text-muted); }
.info-value { font-weight: 500; }
.info-value.mono { font-family: var(--font-mono); }

.status-badge {
  padding: 2px 10px;
  border-radius: 10px;
  font-size: var(--text-xs);
  font-weight: 500;
}

.status-badge.ok {
  background: var(--success-light);
  color: var(--success);
}

.status-badge.error {
  background: var(--danger-light);
  color: var(--danger);
}

/* ─────────── Element Plus Global Overrides ─────────── */
.el-card {
  --el-card-bg-color: var(--bg-surface);
  border: 1px solid var(--border-subtle) !important;
  border-radius: var(--radius-md) !important;
  transition: border-color var(--transition-fast), box-shadow var(--transition-fast);
}

.el-card:hover {
  border-color: var(--border-default) !important;
}

.el-button--primary {
  --el-button-bg-color: var(--accent);
  --el-button-border-color: var(--accent);
  --el-button-hover-bg-color: var(--accent-dim);
  --el-button-hover-border-color: var(--accent-dim);
}

.el-input__wrapper {
  background-color: var(--bg-surface);
  box-shadow: 0 0 0 1px var(--border-default) inset;
  border-radius: var(--radius-sm);
  transition: all var(--transition-fast);
}

.el-input__wrapper:hover {
  box-shadow: 0 0 0 1px var(--border-strong) inset;
}

.el-input__wrapper.is-focus {
  box-shadow: 0 0 0 1px var(--accent) inset, 0 0 0 3px var(--accent-glow) !important;
}

.el-select__wrapper {
  background-color: var(--bg-surface);
  box-shadow: 0 0 0 1px var(--border-default) inset;
  border-radius: var(--radius-sm);
}

.el-dialog {
  --el-dialog-bg-color: var(--bg-surface);
  border-radius: var(--radius-lg);
  border: 1px solid var(--border-subtle);
}

.el-dropdown-menu__item,
.el-select-dropdown__item {
  color: var(--text-primary) !important;
}

.el-dropdown-menu__item:hover,
.el-select-dropdown__item:hover {
  background-color: var(--bg-hover) !important;
  color: var(--text-primary) !important;
}

.el-dropdown-menu__item:not(.is-disabled):focus,
.el-select-dropdown__item.selected {
  background-color: var(--accent-glow) !important;
  color: var(--accent-dim) !important;
}

.el-dropdown-menu__item.is-disabled {
  color: var(--text-muted) !important;
}

.el-dropdown__popper,
.el-select-dropdown {
  background-color: var(--bg-surface) !important;
  border: 1px solid var(--border-default) !important;
  box-shadow: var(--shadow-md) !important;
}

.el-tabs__header {
  border-bottom-color: var(--border-subtle) !important;
}

.el-tabs__item {
  color: var(--text-secondary);
  font-family: var(--font-sans);
}

.el-tabs__item.is-active {
  color: var(--accent);
}

.el-tabs__active-bar {
  background-color: var(--accent);
}

.el-switch.is-checked .el-switch__core {
  background-color: var(--accent);
  border-color: var(--accent);
}

.el-pagination {
  --el-pagination-bg-color: var(--bg-surface);
  --el-pagination-button-bg-color: var(--bg-surface);
}

/* ─────────── Dark Mode CSS Overrides (supports .dark class for non-variable properties) ─────────── */
.dark .el-card {
  --el-card-bg-color: var(--bg-surface);
}

.dark .el-table {
  --el-table-bg-color: var(--bg-surface);
  --el-table-tr-bg-color: var(--bg-surface);
  --el-table-header-bg-color: var(--bg-elevated);
}

.dark .el-input__wrapper {
  background-color: var(--bg-inset);
  box-shadow: 0 0 0 1px var(--border-subtle) inset;
}

.dark .el-select__wrapper {
  background-color: var(--bg-inset);
  box-shadow: 0 0 0 1px var(--border-subtle) inset;
}

.dark .el-dialog {
  --el-dialog-bg-color: var(--bg-surface);
}

.dark .el-dropdown__popper,
.dark .el-select-dropdown {
  background-color: var(--bg-elevated) !important;
  border: 1px solid var(--border-default) !important;
}

/* ─────────── Page Transitions ─────────── */
.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.12s ease;
}

.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}

/* ─────────── Responsive ─────────── */
@media (max-width: 768px) {
  .app-sidebar {
    width: var(--sidebar-collapsed);
  }

  .app-sidebar .brand-text,
  .app-sidebar .brand-version,
  .app-sidebar .sidebar-section-label,
  .app-sidebar .nav-label,
  .app-sidebar .nav-badge {
    display: none;
  }

  .app-content { padding: 16px; }

  .stat-grid { grid-template-columns: repeat(2, 1fr); }
  .card-grid { grid-template-columns: 1fr; }

  .page-header {
    flex-direction: column;
    align-items: stretch;
  }
}
```

- [ ] **Step 2: Commit**

```bash
git add web/src/style.css
git commit -m "feat: rewrite CSS for Warm Craft design system

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 5: Create ModeToggle Component

**Files:**
- Create: `web/src/components/ModeToggle.vue`

- [ ] **Step 1: Write ModeToggle.vue**

```vue
<script setup>
import { ref, onMounted } from 'vue'
import { getSavedMode, toggleMode } from '../themes.js'

const mode = ref('light')

onMounted(() => {
  mode.value = getSavedMode()
})

function handleToggle() {
  mode.value = toggleMode()
}
</script>

<template>
  <button
    class="mode-toggle"
    :title="mode === 'dark' ? 'Switch to light mode' : 'Switch to dark mode'"
    @click="handleToggle"
  >
    <!-- Sun icon (shown in dark mode) -->
    <svg v-if="mode === 'dark'" viewBox="0 0 20 20" fill="currentColor" width="16" height="16">
      <path fill-rule="evenodd" d="M10 2a1 1 0 011 1v1a1 1 0 11-2 0V3a1 1 0 011-1zm4 8a4 4 0 11-8 0 4 4 0 018 0zm-.464 4.95l.707.707a1 1 0 001.414-1.414l-.707-.707a1 1 0 00-1.414 1.414zm2.12-10.607a1 1 0 010 1.414l-.706.707a1 1 0 11-1.414-1.414l.707-.707a1 1 0 011.414 0zM17 11a1 1 0 100-2h-1a1 1 0 100 2h1zm-7 4a1 1 0 011 1v1a1 1 0 11-2 0v-1a1 1 0 011-1zM5.05 6.464A1 1 0 106.465 5.05l-.708-.707a1 1 0 00-1.414 1.414l.707.707zm1.414 8.486l-.707.707a1 1 0 01-1.414-1.414l.707-.707a1 1 0 011.414 1.414zM4 11a1 1 0 100-2H3a1 1 0 000 2h1z" clip-rule="evenodd"/>
    </svg>
    <!-- Moon icon (shown in light mode) -->
    <svg v-else viewBox="0 0 20 20" fill="currentColor" width="16" height="16">
      <path d="M17.293 13.293A8 8 0 016.707 2.707a8.001 8.001 0 1010.586 10.586z"/>
    </svg>
  </button>
</template>

<style scoped>
.mode-toggle {
  width: 32px;
  height: 32px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-sm);
  cursor: pointer;
  color: var(--text-muted);
  transition: all var(--transition-fast);
  flex-shrink: 0;
}

.mode-toggle:hover {
  background: var(--bg-hover);
  border-color: var(--border-default);
  color: var(--text-primary);
}
</style>
```

- [ ] **Step 2: Commit**

```bash
git add web/src/components/ModeToggle.vue
git commit -m "feat: add ModeToggle component for light/dark switching

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 6: Create SidebarNav Component

**Files:**
- Create: `web/src/components/SidebarNav.vue`

- [ ] **Step 1: Write SidebarNav.vue**

```vue
<script setup>
import { ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import ModeToggle from './ModeToggle.vue'

const props = defineProps({
  mainNavItems: { type: Array, required: true },
  systemNavItems: { type: Array, required: true },
  runningCount: { type: Number, default: 0 },
  totalCount: { type: Number, default: 0 },
  userName: { type: String, default: 'admin' },
})

const emit = defineEmits(['toggle-collapse'])

const route = useRoute()
const router = useRouter()
const collapsed = ref(false)

const userInitial = props.userName && props.userName.length
  ? props.userName.charAt(0).toUpperCase()
  : 'U'

function isActive(path) {
  if (path === '/dashboard') {
    return route.path === '/dashboard' || route.path === '/'
  }
  return route.path.startsWith(path)
}

function navigate(path) {
  router.push(path)
}

function toggleCollapsed() {
  collapsed.value = !collapsed.value
  emit('toggle-collapse', collapsed.value)
}
</script>

<template>
  <aside class="app-sidebar" :class="{ collapsed }">
    <!-- Brand -->
    <div class="sidebar-brand">
      <svg class="brand-icon" viewBox="0 0 32 32" fill="none" xmlns="http://www.w3.org/2000/svg">
        <rect x="2" y="2" width="28" height="28" rx="7" fill="var(--primary)" />
        <path d="M10 8v16l12-8-12-8z" fill="var(--accent)" />
        <circle cx="22" cy="16" r="3" fill="var(--accent)" />
      </svg>
      <div>
        <div class="brand-text">IoT Gateway</div>
        <div class="brand-version">Edge Controller v2</div>
      </div>
      <button class="sidebar-collapse-btn" @click="toggleCollapsed" :title="collapsed ? 'Expand' : 'Collapse'">
        <svg viewBox="0 0 20 20" fill="currentColor" width="14" height="14">
          <path v-if="collapsed" fill-rule="evenodd" d="M7.293 14.707a1 1 0 010-1.414L10.586 10 7.293 6.707a1 1 0 011.414-1.414l4 4a1 1 0 010 1.414l-4 4a1 1 0 01-1.414 0z" clip-rule="evenodd"/>
          <path v-else fill-rule="evenodd" d="M12.707 5.293a1 1 0 010 1.414L9.414 10l3.293 3.293a1 1 0 01-1.414 1.414l-4-4a1 1 0 010-1.414l4-4a1 1 0 011.414 0z" clip-rule="evenodd"/>
        </svg>
      </button>
    </div>

    <!-- Main Navigation -->
    <div class="sidebar-section-label">Main Navigation</div>
    <nav class="sidebar-nav">
      <div
        v-for="item in mainNavItems"
        :key="item.path"
        class="sidebar-nav-item"
        :class="{ active: isActive(item.path) }"
        @click="navigate(item.path)"
      >
        <span class="nav-icon">
          <svg v-if="item.icon === 'dashboard'" viewBox="0 0 20 20" fill="currentColor">
            <path d="M3 4a1 1 0 011-1h12a1 1 0 011 1v2a1 1 0 01-1 1H4a1 1 0 01-1-1V4zm0 6a1 1 0 011-1h6a1 1 0 011 1v6a1 1 0 01-1 1H4a1 1 0 01-1-1v-6zm10 0a1 1 0 011-1h2a1 1 0 011 1v6a1 1 0 01-1 1h-2a1 1 0 01-1-1v-6z"/>
          </svg>
          <svg v-else-if="item.icon === 'devices'" viewBox="0 0 20 20" fill="currentColor">
            <path fill-rule="evenodd" d="M5 4a2 2 0 00-2 2v8a2 2 0 002 2h10a2 2 0 002-2V6a2 2 0 00-2-2H5zm1 2a1 1 0 000 2h2a1 1 0 000-2H6zm6 0a1 1 0 000 2h2a1 1 0 000-2h-2zm-6 4a1 1 0 000 2h2a1 1 0 000-2H6zm6 0a1 1 0 000 2h2a1 1 0 000-2h-2z" clip-rule="evenodd"/>
          </svg>
          <svg v-else-if="item.icon === 'cloud'" viewBox="0 0 20 20" fill="currentColor">
            <path d="M5.5 16a3.5 3.5 0 01-.369-6.98 4 4 0 117.753-1.977A4.5 4.5 0 1113.5 16h-8z"/>
          </svg>
          <svg v-else-if="item.icon === 'monitor'" viewBox="0 0 20 20" fill="currentColor">
            <path d="M2 11a1 1 0 011-1h2a1 1 0 011 1v5a1 1 0 01-1 1H3a1 1 0 01-1-1v-5zm6-4a1 1 0 011-1h2a1 1 0 011 1v9a1 1 0 01-1 1H9a1 1 0 01-1-1V7zm6-3a1 1 0 011-1h2a1 1 0 011 1v12a1 1 0 01-1 1h-2a1 1 0 01-1-1V4z"/>
          </svg>
          <svg v-else-if="item.icon === 'plugins'" viewBox="0 0 20 20" fill="currentColor">
            <path d="M11 17a1 1 0 001.447.894l4-2A1 1 0 0017 15V9.236a1 1 0 00-1.447-.894l-4 2a1 1 0 00-.553.894V17zM15.211 6.276a1 1 0 000-1.788l-4.764-2.382a1 1 0 00-.894 0L4.789 4.488a1 1 0 000 1.788l4.764 2.382a1 1 0 00.894 0l4.764-2.382zM4.447 8.342A1 1 0 003 9.236V15a1 1 0 00.553.894l4 2A1 1 0 009 17v-5.764a1 1 0 00-.553-.894l-4-2z"/>
          </svg>
        </span>
        <span class="nav-label">{{ item.label }}</span>
        <span v-if="item.badge && item.badge > 0" class="nav-badge">{{ item.badge }}</span>
      </div>
    </nav>

    <!-- System Navigation -->
    <div class="sidebar-section-label">System</div>
    <nav class="sidebar-nav">
      <div
        v-for="item in systemNavItems"
        :key="item.path"
        class="sidebar-nav-item"
        :class="{ active: isActive(item.path) }"
        @click="navigate(item.path)"
      >
        <span class="nav-label">{{ item.label }}</span>
      </div>
    </nav>

    <!-- Footer: User + Theme Toggle -->
    <div class="sidebar-footer">
      <div style="display:flex;align-items:center;gap:8px;padding:4px 8px">
        <div style="width:26px;height:26px;border-radius:6px;background:var(--accent);color:var(--text-inverse);display:flex;align-items:center;justify-content:center;font-size:11px;font-weight:600;flex-shrink:0">{{ userInitial }}</div>
        <div v-show="!collapsed" style="font-size:12px;color:var(--text-primary);overflow:hidden;text-overflow:ellipsis;white-space:nowrap">{{ userName }}</div>
        <div v-show="!collapsed" style="flex:1"></div>
        <ModeToggle />
      </div>
    </div>
  </aside>
</template>
```

- [ ] **Step 2: Commit**

```bash
git add web/src/components/SidebarNav.vue
git commit -m "feat: add SidebarNav component with collapse support

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 7: Create StatusBar Component

**Files:**
- Create: `web/src/components/StatusBar.vue`

- [ ] **Step 1: Write StatusBar.vue**

```vue
<script setup>
import { computed } from 'vue'

const props = defineProps({
  runningCount: { type: Number, default: 0 },
  totalCount: { type: Number, default: 0 },
  uptime: { type: String, default: '' },
  lastRefresh: { type: String, default: '' },
  dataRate: { type: Number, default: 0 },
})
</script>

<template>
  <footer class="app-statusbar">
    <span class="statusbar-item">
      <span class="status-dot" :class="{ running: runningCount > 0 }"></span>
      Nodes: {{ runningCount }}/{{ totalCount }} running
    </span>
    <span v-if="dataRate > 0" class="statusbar-item">
      Data: {{ dataRate }} pts/s
    </span>
    <span v-if="uptime" class="statusbar-item">
      Uptime: {{ uptime }}
    </span>
    <span style="flex:1"></span>
    <span v-if="lastRefresh" class="statusbar-item">
      Last refresh: {{ lastRefresh }}
    </span>
  </footer>
</template>

<style scoped>
.statusbar-item {
  display: inline-flex;
  align-items: center;
  gap: 6px;
  white-space: nowrap;
}
</style>
```

- [ ] **Step 2: Commit**

```bash
git add web/src/components/StatusBar.vue
git commit -m "feat: add StatusBar component for bottom health display

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 8: Create PageHeader Component

**Files:**
- Create: `web/src/components/PageHeader.vue`

- [ ] **Step 1: Write PageHeader.vue**

```vue
<script setup>
defineProps({
  title: { type: String, required: true },
  subtitle: { type: String, default: '' },
})

defineEmits(['refresh'])
</script>

<template>
  <div class="page-header">
    <div>
      <h1 class="page-header-title">{{ title }}</h1>
      <p v-if="subtitle" class="page-header-subtitle">{{ subtitle }}</p>
    </div>
    <div class="page-header-actions">
      <slot name="actions" />
    </div>
  </div>
</template>
```

- [ ] **Step 2: Commit**

```bash
git add web/src/components/PageHeader.vue
git commit -m "feat: add reusable PageHeader component

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 9: Rewrite App.vue with Sidebar Layout

**Files:**
- Rewrite: `web/src/App.vue`

- [ ] **Step 1: Rewrite App.vue**

```vue
<script setup>
import { ref, onMounted, provide, computed, watch } from 'vue'
import { useRouter, useRoute } from 'vue-router'
import { useI18n } from 'vue-i18n'
import zhCn from 'element-plus/es/locale/lang/zh-cn.mjs'
import en from 'element-plus/es/locale/lang/en.mjs'
import { api } from './api.js'
import { setLocale } from './i18n/index.js'
import { initMode } from './themes.js'
import SidebarNav from './components/SidebarNav.vue'
import StatusBar from './components/StatusBar.vue'

const router = useRouter()
const route = useRoute()
const { t, locale } = useI18n()

const southPlugins = ref([])
const northPlugins = ref([])
const nodes = ref([])
const health = ref(null)
const lastRefresh = ref('')

provide('southPlugins', southPlugins)
provide('northPlugins', northPlugins)
provide('nodes', nodes)
provide('health', health)

// Navigation data with i18n labels resolved in template via t()
const mainNavItems = [
  { path: '/dashboard', icon: 'dashboard', labelKey: 'menu.overview' },
  { path: '/south', icon: 'devices', labelKey: 'menu.southDevices' },
  { path: '/north', icon: 'cloud', labelKey: 'menu.northApps' },
  { path: '/monitor', icon: 'monitor', labelKey: 'menu.dataMonitor' },
  { path: '/plugins', icon: 'plugins', labelKey: 'menu.pluginManage' },
]

const systemNavItems = [
  { path: '/settings/info', labelKey: 'menu.systemInfo' },
  { path: '/monitor/flow', labelKey: 'menu.dataFlowMetrics' },
  { path: '/settings/license', labelKey: 'menu.license' },
  { path: '/settings/users', labelKey: 'menu.users' },
  { path: '/settings/logs', labelKey: 'menu.logs' },
  { path: '/settings/config', labelKey: 'menu.systemConfig' },
]

// Resolve nav item labels using i18n
function resolvedItems(items) {
  return items.map(item => ({
    ...item,
    label: t(item.labelKey),
    badge: item.path === '/south' ? nodes.value.filter(n => n.kind === 'south').length
         : item.path === '/north' ? nodes.value.filter(n => n.kind === 'north').length
         : undefined,
  }))
}

const resolvedMainNav = computed(() => resolvedItems(mainNavItems))
const resolvedSystemNav = computed(() => resolvedItems(systemNavItems))

// Computed values for status bar
const runningCount = computed(() => nodes.value.filter(n => n.state === 'running').length)
const totalCount = computed(() => nodes.value.length)

const userDisplayName = computed(() => {
  const name = localStorage.getItem('gateway_user') || 'admin'
  return name
})

// Language
const currentLocale = ref(locale.value)
watch(currentLocale, (val) => { setLocale(val) })
const elLocale = computed(() => (currentLocale.value === 'en' ? en : zhCn))

function toggleLocale() {
  currentLocale.value = currentLocale.value === 'zh' ? 'en' : 'zh'
}

function handleLogout() {
  api.clearAuth()
  router.push('/login')
}

const isLoginPage = computed(() => route.path === '/login')

async function loadInitData() {
  if (route.path === '/login') return
  try {
    const [sp, np, nd, h] = await Promise.all([
      api.pluginsSouth().catch(() => []),
      api.pluginsNorth().catch(() => []),
      api.nodes().catch(() => []),
      api.health().catch(() => null),
    ])
    southPlugins.value = sp
    northPlugins.value = np
    nodes.value = nd
    health.value = h
    lastRefresh.value = new Date().toLocaleTimeString()
  } catch (e) {
    console.error('Failed to load initial data', e)
  }
}

// Context menu items for user dropdown
const userMenuItems = computed(() => [
  { key: 'locale', label: currentLocale.value === 'zh' ? t('user.localeEn') : t('user.localeZh'), action: toggleLocale },
  { key: 'logout', label: t('user.logout'), action: handleLogout },
])

onMounted(() => {
  initMode()
  currentLocale.value = locale.value
  loadInitData()
})
</script>

<template>
  <el-config-provider :locale="elLocale">
    <!-- Login page: no shell layout -->
    <template v-if="isLoginPage">
      <router-view />
    </template>

    <!-- Main app shell -->
    <div v-else class="app-shell">
      <SidebarNav
        :main-nav-items="resolvedMainNav"
        :system-nav-items="resolvedSystemNav"
        :running-count="runningCount"
        :total-count="totalCount"
        :user-name="userDisplayName"
      />

      <div class="app-main">
        <main class="app-content">
          <router-view v-slot="{ Component }">
            <transition name="fade" mode="out-in">
              <component :is="Component" />
            </transition>
          </router-view>
        </main>

        <StatusBar
          :running-count="runningCount"
          :total-count="totalCount"
          :last-refresh="lastRefresh"
        />
      </div>
    </div>
  </el-config-provider>
</template>
```

- [ ] **Step 2: Commit**

```bash
git add web/src/App.vue
git commit -m "feat: rewrite App.vue with sidebar layout and status bar

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 10: Update main.js

**Files:**
- Modify: `web/src/main.js`

- [ ] **Step 1: Update import from themes.js**

Replace the import of `initTheme` with `initMode`.

```js
import { createApp } from 'vue'
import ElementPlus from 'element-plus'
import zhCn from 'element-plus/es/locale/lang/zh-cn.mjs'
import 'element-plus/dist/index.css'
import './style.css'
import App from './App.vue'
import router from './router.js'
import { i18n } from './i18n/index.js'

const app = createApp(App)
app.use(i18n)
app.use(router)
app.use(ElementPlus, { locale: zhCn })

router.isReady().then(() => {
  app.mount('#app')
}).catch((err) => {
  console.error('[router.isReady]', err)
  app.mount('#app')
})
```

Note: `initMode()` is now called in `App.vue` `onMounted`, so `main.js` does not need to call it directly.

- [ ] **Step 2: Commit**

```bash
git add web/src/main.js
git commit -m "refactor: remove theme init import from main.js

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 11: Update Login.vue

**Files:**
- Modify: `web/src/views/Login.vue`

- [ ] **Step 1: Update Login.vue to use new style classes and remove old theme init**

The login page main changes:
- Replace `initTheme` import with `initMode`
- Use `login-page` and `login-card` CSS classes from style.css
- Remove old inline styles

Read the current file first, then apply these specific edits:

**Edit 1:** Replace import line:
```
import { initTheme } from '../themes.js'
```
with:
```
import { initMode } from '../themes.js'
```

**Edit 2:** Replace `onMounted` call:
```
onMounted(() => {
  initTheme()
})
```
with:
```
onMounted(() => {
  initMode()
})
```

**Edit 3:** The template should use the new `login-page` and `login-card` CSS classes. The login form structure stays the same; only the outer wrapper classes change to match style.css.

The current Login.vue template likely has a `<div class="login-page">` wrapper. Ensure the root element uses `class="login-page"` and the card inside uses `class="login-card"`. If the current login uses different class names, update them.

- [ ] **Step 2: Commit**

```bash
git add web/src/views/Login.vue
git commit -m "refactor: update Login.vue for Warm Craft design

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 12: Update View Files — replace page headers

**Files:**
- Modify: `web/src/views/Dashboard.vue`
- Modify: `web/src/views/SouthDevices.vue`
- Modify: `web/src/views/NorthApps.vue`
- Modify: `web/src/views/NodeDetail.vue`
- Modify: `web/src/views/GroupDetail.vue`
- Modify: `web/src/views/DataMonitor.vue`
- Modify: `web/src/views/DataFlowMetrics.vue`
- Modify: `web/src/views/Plugins.vue`
- Modify: `web/src/views/SystemConfig.vue`
- Modify: `web/src/views/SystemInfo.vue`
- Modify: `web/src/views/License.vue`
- Modify: `web/src/views/Users.vue`
- Modify: `web/src/views/Logs.vue`
- Modify: `web/src/views/CreateNode.vue`
- Modify: `web/src/views/NodeConfig.vue`

- [ ] **Step 1: Import PageHeader in each view**

For each view file, add the import at the top of `<script setup>`:

```js
import PageHeader from '../components/PageHeader.vue'
```

- [ ] **Step 2: Replace inline page headers with PageHeader component**

For each view, find the existing page header markup (typically a `<div class="page-header">` block) and replace it with:

```vue
<PageHeader
  :title="t('page.titleKey')"
  :subtitle="t('page.descKey')"
>
  <template #actions>
    <!-- existing action buttons go here -->
  </template>
</PageHeader>
```

The translation keys (`page.titleKey`, `page.descKey`) should match the existing i18n keys used by each page. Map each view's header as follows:

| View | Title Key | Desc Key |
|------|-----------|----------|
| Dashboard | `dashboard.title` | `dashboard.desc` |
| SouthDevices | `header.southDevices` | (no subtitle) |
| NorthApps | `header.northApps` | (no subtitle) |
| NodeDetail | dynamic (node name) | (no subtitle) |
| GroupDetail | dynamic (group name) | (no subtitle) |
| DataMonitor | `menu.dataMonitor` | (no subtitle) |
| DataFlowMetrics | `menu.dataFlowMetrics` | (no subtitle) |
| Plugins | `menu.pluginManage` | (no subtitle) |
| SystemConfig | `menu.systemConfig` | (no subtitle) |
| SystemInfo | `menu.systemInfo` | (no subtitle) |
| License | `menu.license` | (no subtitle) |
| Users | `menu.users` | (no subtitle) |
| Logs | `menu.logs` | (no subtitle) |
| CreateNode | `header.addDevice` / `header.addApp` | (no subtitle) |
| NodeConfig | `nodeDetail.nodeConfig` | (no subtitle) |

For dynamic titles (NodeDetail, GroupDetail, CreateNode), use a computed property for the title.

- [ ] **Step 3: Remove old page header CSS**

Each view file may have scoped styles for the old page header. Remove the scoped `.page-header`, `.header-content`, `.page-title`, `.page-desc`, `.header-actions` styles from each view's `<style scoped>` block, since these are now handled globally by style.css and the PageHeader component.

- [ ] **Step 4: Commit**

```bash
git add web/src/views/
git commit -m "refactor: replace inline page headers with PageHeader component

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 13: Delete Old ThemeSwitcher Component

**Files:**
- Delete: `web/src/components/ThemeSwitcher.vue`

- [ ] **Step 1: Delete the file**

```bash
rm web/src/components/ThemeSwitcher.vue
```

- [ ] **Step 2: Commit**

```bash
git add web/src/components/ThemeSwitcher.vue
git commit -m "refactor: remove old ThemeSwitcher, replaced by ModeToggle

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 14: Remove PlusJakartaSans Font (no longer used)

**Files:**
- Delete: `web/public/fonts/PlusJakartaSans-latin.woff2`

- [ ] **Step 1: Delete the unused font**

```bash
rm web/public/fonts/PlusJakartaSans-latin.woff2
```

- [ ] **Step 2: Commit**

```bash
git add web/public/fonts/PlusJakartaSans-latin.woff2
git commit -m "chore: remove unused PlusJakartaSans font

Co-Authored-By: Claude Opus 4.7 <noreply@anthropic.com>"
```

---

### Task 15: Final Verification

- [ ] **Step 1: Install dependencies and build**

```bash
cd /Users/lipeng/iot-gateway/web && npm install && npm run build
```

Expected: Build succeeds with no errors. All imports resolve correctly.

- [ ] **Step 2: Check for any remaining references to old theme system**

```bash
grep -r "initTheme\|getSavedTheme\|applyTheme\|ThemeSwitcher\|PlusJakarta\|data-theme.*industrial\|data-theme.*smartFactory\|data-theme.*classic" web/src/ --include="*.vue" --include="*.js" --include="*.css"
```

Expected: No results (all old references cleaned up).

- [ ] **Step 3: Verify light/dark mode toggle works**

Start the dev server:
```bash
cd /Users/lipeng/iot-gateway/web && npm run dev
```

Open the browser, verify:
1. Login page renders with new warm cream background
2. After login, sidebar layout is visible with all navigation items
3. Dashboard shows stat cards with serif headings
4. Click the moon/sun icon in sidebar footer → theme switches smoothly
5. Collapse sidebar → icons remain, labels hidden
6. Navigate to each page → all render correctly, no broken styles
7. Check dark mode across all pages
8. Responsive: narrow browser to < 768px → sidebar auto-collapses

- [ ] **Step 4: Commit any final fixes**

```bash
git add -A && git status
```

If there are no additional changes, the build passed, and all verification checks out, the implementation is complete.
```

---

## Self-Review Checklist

1. **Spec coverage**: Each spec section has a corresponding task:
   - Color palette → Task 3 (themes.js palette) + Task 4 (CSS variables)
   - Typography → Task 1 (font) + Task 4 (CSS font-face + variables)
   - Layout → Task 6 (SidebarNav) + Task 9 (App.vue shell)
   - Theme system → Task 3 (themes.js) + Task 5 (ModeToggle)
   - Component redesign → Task 4 (all CSS component styles)
   - Usability improvements → Task 6 (sidebar collapse) + Task 7 (status bar) + Task 8 (page header)
   - Files to create → Tasks 5-8
   - Files to modify → Tasks 2, 3, 4, 9, 10, 11, 12
   - Files to delete → Tasks 13, 14
   - Edge cases → Task 11 (login), Task 4 (CSS empty states, loading, error alerts)
   - Responsive → Task 4 (CSS media queries)

2. **No placeholders**: All code is fully written, all keys specified, all paths concrete.

3. **Type consistency**: `resolvedMainNav` / `resolvedSystemNav` computed in App.vue passes `{ path, icon, label, badge }` objects to SidebarNav which expects matching props. ModeToggle uses `getSavedMode` / `toggleMode` from themes.js — both defined in Task 3. PageHeader props: `title`, `subtitle`, `actions` slot — consistent across tasks.
