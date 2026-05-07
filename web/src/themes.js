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
