# Warm Craft — IoT Gateway Frontend Redesign

**Date:** 2026-05-07
**Status:** Design approved

## Overview

Redesign the IoT Gateway frontend with a "Warm Craft" aesthetic — human-centered, calm, and refined. Replace the 4-theme system with a single polished light theme + dark mode. Switch from top navigation to a collapsible sidebar for better discoverability.

### Goals

- **Usability**: All pages reachable from sidebar, no buried settings
- **Aesthetics**: Distinctive warm, paper-like visual identity — refined and memorable
- **Comfort**: Easy on the eyes during long monitoring sessions (both light and dark modes)
- **Practicality**: Collapsible sidebar for space-constrained situations

## Design Direction

### Tone

Warm Craft — like a finely printed technical manual on archival paper. Understated, human, precise. Terracotta accents on cream whites. Serif headings with monospace data. Subtle textures and soft shadows instead of harsh borders.

### Differentiation

Most IoT dashboards are either cold industrial blue or sci-fi neon. This one feels like a well-designed physical instrument — a vintage analog gauge brought into the digital age. Warm, trustworthy, calm.

## Color Palette

### Light Mode

| Role | Value | Description |
|------|-------|-------------|
| Base background | `#faf7f2` | Warm cream paper |
| Surface | `#fffbf5` | Card/panel white |
| Elevated | `#f5efe5` | Hover/active states |
| Overlay/Inset | `#ede6db` | Input backgrounds, dividers |
| Primary text | `#5c4a3a` | Deep warm brown |
| Secondary text | `#7a6b5c` | Muted brown |
| Muted text | `#bfae9a` | Faded clay |
| Accent | `#c17f4f` | Terracotta — buttons, links, active states |
| Accent hover | `#a8653a` | Darker terracotta |
| Success | `#6b8e6b` | Sage green |
| Warning | `#c47b4a` | Warm amber |
| Danger | `#c45a4a` | Muted red |
| Border subtle | `#e8ddd0` | Card/panel borders |
| Border default | `#d4c5b2` | Input borders |

### Dark Mode

Warm dark — deep espresso browns, not cold blacks. Warm amber glow accents.

| Role | Value | Description |
|------|-------|-------------|
| Base background | `#1c1917` | Deep espresso |
| Surface | `#252220` | Slightly lifted |
| Elevated | `#2d2a27` | Hover/active |
| Primary text | `#d4c5b2` | Warm light tan |
| Secondary text | `#a0917b` | Muted tan |
| Accent | `#d4a574` | Warm amber |
| Success | `#7aaa7a` | Soft sage |
| Danger | `#d48a7a` | Warm rose |
| Border subtle | `#3d3833` | Subtle warm border |

## Typography

| Role | Font | Justification |
|------|------|---------------|
| Headings (h1-h4) | **Source Serif 4** | Warm literary serif, distinctive character, excellent readability at display sizes |
| Body | System sans-serif stack: `-apple-system, BlinkMacSystemFont, 'Segoe UI', 'PingFang SC', 'Microsoft YaHei', sans-serif` | Crisp, native rendering, zero latency |
| Code/Data/Mono | **JetBrains Mono** | Already in project, excellent for data values and IDs |

Font loading: Both Source Serif 4 and JetBrains Mono served as local WOFF2 from `public/fonts/` (subset to latin). System sans-serif requires no loading.

## Layout

### Application Shell

```
┌──────────────────────────────────────────────────────┐
│ ┌──────────┐  ┌────────────────────────────────────┐ │
│ │          │  │                                    │ │
│ │ Sidebar  │  │  Content Area                      │ │
│ │          │  │  (scrollable)                      │ │
│ │ 200px    │  │                                    │ │
│ │ (60px    │  │                                    │ │
│ │  icon)   │  │                                    │ │
│ │          │  │                                    │ │
│ │          │  │                                    │ │
│ └──────────┘  └────────────────────────────────────┘ │
│ ┌────────────────────────────────────────────────────┐│
│ │ Status Bar (health, node count, last refresh)     ││
│ └────────────────────────────────────────────────────┘│
└──────────────────────────────────────────────────────┘
```

### Sidebar

- **Expanded mode** (default, 200px): Icon + label for each item. Brand at top with version. User avatar + dark mode toggle at bottom.
- **Collapsed mode** (60px): Icons only with tooltips. Toggle via hamburger/chevron button at top.
- **Sections**: "Main" group (Dashboard, South Devices, North Apps, Data Monitor, Plugins) separated from "System" group (Info, Data Flow, License, Users, Logs, Config) by a divider.
- **Active state**: Left border accent (terracotta, 2px) + subtle warm background.

### Content Area

- Full-height scrollable area, no nested scroll container
- Pages use consistent padding (24px)
- Cards use soft rounded corners (8px), subtle border, very light shadow
- Page header uses Source Serif 4 for title

### Status Bar

- Fixed bottom bar, 32px height
- Shows: running nodes / total, data points/sec, uptime, last refresh time
- Subtle warm background, muted text
- Auto-refresh indicator (pulsing dot when live)

### Breadcrumb

- Removed as standalone bar; replaced by sidebar active state + optional page header subtitle showing context path

## Theme System

### Before
4 themes (industrial, smartFactory, classic, dark) managed via `themes.js` with `data-theme` attribute + `.dark` class for Element Plus.

### After
Single design system with light/dark toggle:

- **themes.js** simplified to manage 2 modes: `light` and `dark`
- A single `ModeSwitcher` component (sun/moon icon toggle) at sidebar bottom
- Mode stored in `localStorage` key `gateway_mode` (`light` | `dark`)
- CSS variables applied to `:root` — `dark` class toggled on `<html>` for Element Plus
- Old theme references cleaned up: remove ThemeSwitcher dropdown from nav, remove unused theme definitions

### Transition Behavior

Theme CSS variables apply instantly; a 200ms `transition` on `background-color` and `color` properties ensures smooth switching.

## Component Redesign

### Cards (Device Cards, Stat Cards, Plugin Cards)

- Soft paper texture: subtle CSS `background-image` noise/grain (semi-transparent SVG data URI)
- Border: 1px solid var(--border-subtle) replaces heavy borders
- Shadow: `0 1px 3px rgba(0,0,0,0.03)` — very subtle lift
- State indicators: colored left-border (3px) instead of badges
- Running: sage green border, Stopped: muted clay border, Error: warm rose border

### Buttons

- Primary: terracotta background (`--accent`), white text, subtle shadow
- Secondary: warm elevated background, muted brown border
- Ghost: transparent, text only, hover shows subtle background
- Size variants: sm (compact table actions), default, lg (primary page actions)

### Tables

- Clean, minimal borders — horizontal dividers only between rows
- Striped rows with very subtle warm tone
- Hover: slightly elevated background
- Header: uppercase muted labels, 11px
- Data cells: JetBrains Mono for IDs/values, system sans-serif for names

### Forms

- Inputs: warm white background, subtle clay border, rounded 6px
- Focus: terracotta ring (2px) with soft glow
- Labels: above field, secondary text color, 12px
- Validation: sage green success, warm rose error — with subtle left border on input

### Status Indicators

- Running: sage green dot with soft pulse animation, subtle glow
- Stopped: muted clay dot, static
- Error: warm rose dot, subtle pulse

### Modals

- Soft overlay: warm dark with 30% opacity + subtle blur
- Modal panel: surface white, subtle shadow, rounded 10px
- Header/footer with subtle divider

## Usability Improvements

1. **All settings visible**: License, Logs, Config, Users, Info, DataFlow all in sidebar — no dropdown menu
2. **Collapsible sidebar**: Icon-only mode (60px) for data monitoring or wide table views
3. **Persistent status bar**: Always-visible health summary at bottom
4. **Improved Dashboard**: Quick-action buttons more prominent, device list shows recent activity
5. **Better Data Monitor**: Auto-refresh with visual pulse indicator, clearer value display
6. **Consistent page headers**: Every page uses the same header pattern with Source Serif 4 title

## Implementation Scope

### Files to Modify

| File | Changes |
|------|---------|
| `style.css` | Complete rewrite: new CSS variables (light + dark), new component styles, remove old theme code |
| `themes.js` | Simplify to light/dark mode manager, remove 4-theme system |
| `App.vue` | New sidebar layout, remove top nav, add status bar, add collapsible sidebar logic |
| `ThemeSwitcher.vue` | Replace with simple light/dark `ModeToggle.vue` |
| `main.js` | Update imports (remove old theme init) |
| `index.html` | Update meta theme-color, title |
| All view files | Update to new page header pattern, use new CSS classes, clean up Element Plus overrides |

### Files to Create

| File | Purpose |
|------|---------|
| `components/ModeToggle.vue` | Light/dark toggle button |
| `components/SidebarNav.vue` | Sidebar navigation component |
| `components/StatusBar.vue` | Bottom status bar |
| `components/PageHeader.vue` | Reusable page header |

### Fonts to Add

| Font | File | Purpose |
|------|------|---------|
| Source Serif 4 (latin subset) | `public/fonts/SourceSerif4-latin.woff2` | Headings |

### What Stays

- Vue 3 + Element Plus + Vite + Vue Router + vue-i18n stack
- All existing API calls and data flow
- JetBrains Mono (already local)
- All existing route definitions (sidebar links updated to match)
- Element Plus as component library (restyled via CSS variables)

### What Goes

- 4-theme system (`industrial`, `smartFactory`, `classic`, `dark`)
- ThemeSwitcher dropdown in navigation
- Top navigation bar layout
- Breadcrumb bar
- Nested scroll container (content scrolls at body level with sidebar fixed)
- Emoji icons in theme names

## Edge Cases & States

- **Login page**: Full-screen centered card, no sidebar, no status bar. Clean minimal form.
- **Loading**: Skeleton cards for dashboard, skeleton rows for tables. Warm pulse animation.
- **Empty state**: Gentle illustration area with muted text, clear CTA button. Per-page empty messages.
- **Error state**: Alert banner at page top (warm rose for errors, warm amber for warnings). Inline error messages below form fields.
- **Long node names**: Truncated with ellipsis in cards and tables, full name in tooltip.
- **Very wide tables**: Horizontal scroll within content area when sidebar is collapsed.
- **Mobile (< 768px)**: Sidebar auto-collapses to icons. Content padding reduces. Status bar becomes single-line condensed.

## Non-Goals (Out of Scope)

- Mobile app / PWA
- Real-time WebSocket data (stay with polling)
- Changing any backend API
- Adding new pages or features
- Changing i18n translation keys
- Drag-and-drop dashboard customization
