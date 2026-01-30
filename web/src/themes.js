/**
 * IoT Gateway 多主题管理系统
 * 
 * 提供多种视觉风格：
 * - industrial: 现代工业（石墨暖灰 + 琥珀橙）
 * - smartFactory: 智能工厂（深蓝科技 + 青色荧光）
 * - classic: 经典专业（稳重蓝 + 灰调）
 * - dark: 暗夜模式（深色背景 + 亮色强调）
 */

export const themes = {
  // ═══════════════════════════════════════════
  // 现代工业风格 - 默认主题
  // ═══════════════════════════════════════════
  industrial: {
    id: 'industrial',
    name: 'Industrial',
    nameKey: 'theme.industrial',
    description: '石墨暖灰 + 琥珀橙',
    icon: '🏭',
    colors: {
      // 主色
      '--primary': '#2d3748',
      '--primary-dim': '#1a202c',
      '--primary-light': '#4a5568',
      '--primary-glow': 'rgba(45, 55, 72, 0.08)',
      '--on-primary': '#ffffff',
      // 强调色
      '--accent': '#ed8936',
      '--accent-dim': '#dd6b20',
      '--accent-light': '#fbd38d',
      '--accent-glow': 'rgba(237, 137, 54, 0.12)',
      // 语义色
      '--success': '#38a169',
      '--success-light': '#c6f6d5',
      '--warning': '#ecc94b',
      '--warning-light': '#fefcbf',
      '--danger': '#e53e3e',
      '--danger-light': '#fed7d7',
      '--info': '#718096',
      '--accent-purple': '#805ad5',
      // 背景
      '--bg-base': '#f7f8fa',
      '--bg-surface': '#ffffff',
      '--bg-elevated': '#fafbfc',
      '--bg-overlay': '#edf2f7',
      '--bg-inset': '#e2e8f0',
      '--bg-hover': 'rgba(45, 55, 72, 0.04)',
      // 边框
      '--border-subtle': '#e8ecf1',
      '--border-default': '#d4dae3',
      '--border-strong': '#a0aec0',
      // 文字
      '--text-primary': '#1a202c',
      '--text-secondary': '#4a5568',
      '--text-muted': '#a0aec0',
      '--text-inverse': '#ffffff',
      // 状态
      '--state-running': '#38a169',
      '--state-stopped': '#a0aec0',
      '--state-error': '#e53e3e',
      '--state-syncing': '#ed8936',
      // 阴影
      '--shadow-xs': '0 1px 2px rgba(0, 0, 0, 0.04)',
      '--shadow-sm': '0 2px 8px rgba(0, 0, 0, 0.06)',
      '--shadow-md': '0 4px 16px rgba(0, 0, 0, 0.08)',
      '--shadow-lg': '0 8px 32px rgba(0, 0, 0, 0.12)',
      '--shadow-glow': '0 0 0 3px rgba(237, 137, 54, 0.12)',
      '--shadow-accent': '0 4px 14px rgba(237, 137, 54, 0.25)',
      // Element Plus
      '--el-color-primary': '#ed8936',
      '--el-color-primary-light-3': '#f6ad55',
      '--el-color-primary-light-5': '#fbd38d',
      '--el-color-primary-light-7': '#feebc8',
      '--el-color-primary-light-8': '#fffaf0',
      '--el-color-primary-light-9': '#fffaf0',
      '--el-color-primary-dark-2': '#dd6b20',
      // 表格行（柔和条纹与悬停）
      '--table-stripe-bg': 'rgba(45, 55, 72, 0.03)',
      '--table-row-hover-bg': 'rgba(45, 55, 72, 0.06)',
    }
  },

  // ═══════════════════════════════════════════
  // 智能工厂风格 - 科技感、未来感
  // ═══════════════════════════════════════════
  smartFactory: {
    id: 'smartFactory',
    name: 'Smart Factory',
    nameKey: 'theme.smartFactory',
    description: '深蓝科技 + 青色荧光',
    icon: '🤖',
    colors: {
      // 主色 - 深蓝科技
      '--primary': '#0f172a',
      '--primary-dim': '#020617',
      '--primary-light': '#1e293b',
      '--primary-glow': 'rgba(15, 23, 42, 0.1)',
      '--on-primary': '#ffffff',
      // 强调色 - 青色荧光（科技感）
      '--accent': '#06b6d4',
      '--accent-dim': '#0891b2',
      '--accent-light': '#67e8f9',
      '--accent-glow': 'rgba(6, 182, 212, 0.15)',
      // 语义色
      '--success': '#10b981',
      '--success-light': '#d1fae5',
      '--warning': '#f59e0b',
      '--warning-light': '#fef3c7',
      '--danger': '#ef4444',
      '--danger-light': '#fee2e2',
      '--info': '#6366f1',
      '--accent-purple': '#8b5cf6',
      // 背景 - 深色科技感
      '--bg-base': '#0f172a',
      '--bg-surface': '#1e293b',
      '--bg-elevated': '#334155',
      '--bg-overlay': '#475569',
      '--bg-inset': '#0f172a',
      '--bg-hover': 'rgba(6, 182, 212, 0.08)',
      // 边框 - 发光效果
      '--border-subtle': 'rgba(71, 85, 105, 0.5)',
      '--border-default': 'rgba(100, 116, 139, 0.5)',
      '--border-strong': '#64748b',
      // 文字
      '--text-primary': '#f1f5f9',
      '--text-secondary': '#cbd5e1',
      '--text-muted': '#94a3b8',
      '--text-inverse': '#0f172a',
      // 状态 - 发光指示
      '--state-running': '#10b981',
      '--state-stopped': '#64748b',
      '--state-error': '#ef4444',
      '--state-syncing': '#06b6d4',
      // 阴影 - 发光效果
      '--shadow-xs': '0 1px 2px rgba(0, 0, 0, 0.3)',
      '--shadow-sm': '0 2px 8px rgba(0, 0, 0, 0.4)',
      '--shadow-md': '0 4px 16px rgba(0, 0, 0, 0.5)',
      '--shadow-lg': '0 8px 32px rgba(0, 0, 0, 0.6)',
      '--shadow-glow': '0 0 0 3px rgba(6, 182, 212, 0.3), 0 0 20px rgba(6, 182, 212, 0.2)',
      '--shadow-accent': '0 4px 20px rgba(6, 182, 212, 0.4)',
      // Element Plus
      '--el-color-primary': '#06b6d4',
      '--el-color-primary-light-3': '#22d3ee',
      '--el-color-primary-light-5': '#67e8f9',
      '--el-color-primary-light-7': '#a5f3fc',
      '--el-color-primary-light-8': '#cffafe',
      '--el-color-primary-light-9': '#ecfeff',
      '--el-color-primary-dark-2': '#0891b2',
      // 额外的科技感变量
      '--glow-primary': '0 0 10px rgba(6, 182, 212, 0.5)',
      '--glow-success': '0 0 10px rgba(16, 185, 129, 0.5)',
      '--glow-danger': '0 0 10px rgba(239, 68, 68, 0.5)',
      '--gradient-tech': 'linear-gradient(135deg, #0f172a 0%, #1e293b 100%)',
      '--gradient-accent': 'linear-gradient(135deg, #06b6d4 0%, #8b5cf6 100%)',
      // 表格行（柔和条纹与悬停）
      '--table-stripe-bg': 'rgba(6, 182, 212, 0.06)',
      '--table-row-hover-bg': 'rgba(6, 182, 212, 0.1)',
    }
  },

  // ═══════════════════════════════════════════
  // 经典专业风格 - 稳重、商务
  // ═══════════════════════════════════════════
  classic: {
    id: 'classic',
    name: 'Classic',
    nameKey: 'theme.classic',
    description: '稳重蓝 + 专业灰',
    icon: '💼',
    colors: {
      // 主色 - 稳重蓝
      '--primary': '#1e40af',
      '--primary-dim': '#1e3a8a',
      '--primary-light': '#3b82f6',
      '--primary-glow': 'rgba(30, 64, 175, 0.1)',
      '--on-primary': '#ffffff',
      // 强调色
      '--accent': '#3b82f6',
      '--accent-dim': '#2563eb',
      '--accent-light': '#93c5fd',
      '--accent-glow': 'rgba(59, 130, 246, 0.12)',
      // 语义色
      '--success': '#16a34a',
      '--success-light': '#dcfce7',
      '--warning': '#ca8a04',
      '--warning-light': '#fef9c3',
      '--danger': '#dc2626',
      '--danger-light': '#fee2e2',
      '--info': '#6b7280',
      '--accent-purple': '#7c3aed',
      // 背景
      '--bg-base': '#f3f4f6',
      '--bg-surface': '#ffffff',
      '--bg-elevated': '#f9fafb',
      '--bg-overlay': '#e5e7eb',
      '--bg-inset': '#e5e7eb',
      '--bg-hover': 'rgba(59, 130, 246, 0.04)',
      // 边框
      '--border-subtle': '#e5e7eb',
      '--border-default': '#d1d5db',
      '--border-strong': '#9ca3af',
      // 文字
      '--text-primary': '#111827',
      '--text-secondary': '#374151',
      '--text-muted': '#9ca3af',
      '--text-inverse': '#ffffff',
      // 状态
      '--state-running': '#16a34a',
      '--state-stopped': '#9ca3af',
      '--state-error': '#dc2626',
      '--state-syncing': '#3b82f6',
      // 阴影
      '--shadow-xs': '0 1px 2px rgba(0, 0, 0, 0.05)',
      '--shadow-sm': '0 2px 4px rgba(0, 0, 0, 0.06)',
      '--shadow-md': '0 4px 12px rgba(0, 0, 0, 0.08)',
      '--shadow-lg': '0 8px 24px rgba(0, 0, 0, 0.1)',
      '--shadow-glow': '0 0 0 3px rgba(59, 130, 246, 0.15)',
      '--shadow-accent': '0 4px 14px rgba(59, 130, 246, 0.25)',
      // Element Plus
      '--el-color-primary': '#3b82f6',
      '--el-color-primary-light-3': '#60a5fa',
      '--el-color-primary-light-5': '#93c5fd',
      '--el-color-primary-light-7': '#bfdbfe',
      '--el-color-primary-light-8': '#dbeafe',
      '--el-color-primary-light-9': '#eff6ff',
      '--el-color-primary-dark-2': '#2563eb',
      // 表格行（柔和条纹与悬停）
      '--table-stripe-bg': 'rgba(59, 130, 246, 0.04)',
      '--table-row-hover-bg': 'rgba(59, 130, 246, 0.08)',
    }
  },

  // ═══════════════════════════════════════════
  // 暗夜模式 - 深色背景
  // ═══════════════════════════════════════════
  dark: {
    id: 'dark',
    name: 'Dark',
    nameKey: 'theme.dark',
    description: '深色背景 + 荧光强调',
    icon: '🌙',
    colors: {
      // 主色
      '--primary': '#18181b',
      '--primary-dim': '#09090b',
      '--primary-light': '#27272a',
      '--primary-glow': 'rgba(24, 24, 27, 0.1)',
      '--on-primary': '#ffffff',
      // 强调色 - 紫色渐变
      '--accent': '#a855f7',
      '--accent-dim': '#9333ea',
      '--accent-light': '#d8b4fe',
      '--accent-glow': 'rgba(168, 85, 247, 0.15)',
      // 语义色
      '--success': '#22c55e',
      '--success-light': '#86efac',
      '--warning': '#eab308',
      '--warning-light': '#fde047',
      '--danger': '#f43f5e',
      '--danger-light': '#fda4af',
      '--info': '#a1a1aa',
      '--accent-purple': '#a855f7',
      // 背景 - 纯黑系
      '--bg-base': '#09090b',
      '--bg-surface': '#18181b',
      '--bg-elevated': '#27272a',
      '--bg-overlay': '#3f3f46',
      '--bg-inset': '#09090b',
      '--bg-hover': 'rgba(168, 85, 247, 0.08)',
      // 边框
      '--border-subtle': 'rgba(63, 63, 70, 0.5)',
      '--border-default': '#3f3f46',
      '--border-strong': '#52525b',
      // 文字
      '--text-primary': '#fafafa',
      '--text-secondary': '#d4d4d8',
      '--text-muted': '#a1a1aa',
      '--text-inverse': '#18181b',
      // 状态
      '--state-running': '#22c55e',
      '--state-stopped': '#71717a',
      '--state-error': '#f43f5e',
      '--state-syncing': '#a855f7',
      // 阴影
      '--shadow-xs': '0 1px 2px rgba(0, 0, 0, 0.4)',
      '--shadow-sm': '0 2px 8px rgba(0, 0, 0, 0.5)',
      '--shadow-md': '0 4px 16px rgba(0, 0, 0, 0.6)',
      '--shadow-lg': '0 8px 32px rgba(0, 0, 0, 0.7)',
      '--shadow-glow': '0 0 0 3px rgba(168, 85, 247, 0.3)',
      '--shadow-accent': '0 4px 20px rgba(168, 85, 247, 0.35)',
      // Element Plus
      '--el-color-primary': '#a855f7',
      '--el-color-primary-light-3': '#c084fc',
      '--el-color-primary-light-5': '#d8b4fe',
      '--el-color-primary-light-7': '#e9d5ff',
      '--el-color-primary-light-8': '#f3e8ff',
      '--el-color-primary-light-9': '#faf5ff',
      '--el-color-primary-dark-2': '#9333ea',
      // 表格行（柔和条纹与悬停）
      '--table-stripe-bg': 'rgba(168, 85, 247, 0.06)',
      '--table-row-hover-bg': 'rgba(168, 85, 247, 0.1)',
    }
  }
}

// 默认主题
export const DEFAULT_THEME = 'industrial'

// 主题存储键
const THEME_STORAGE_KEY = 'gateway_theme'

/**
 * 获取当前保存的主题ID
 */
export function getSavedTheme() {
  try {
    return localStorage.getItem(THEME_STORAGE_KEY) || DEFAULT_THEME
  } catch {
    return DEFAULT_THEME
  }
}

/**
 * 保存主题到 localStorage
 */
export function saveTheme(themeId) {
  try {
    localStorage.setItem(THEME_STORAGE_KEY, themeId)
  } catch (e) {
    console.warn('Failed to save theme:', e)
  }
}

/**
 * 应用主题到 DOM
 */
export function applyTheme(themeId) {
  const theme = themes[themeId]
  if (!theme) {
    console.warn(`Theme "${themeId}" not found, using default`)
    applyTheme(DEFAULT_THEME)
    return
  }

  const root = document.documentElement

  // 应用所有 CSS 变量
  Object.entries(theme.colors).forEach(([property, value]) => {
    root.style.setProperty(property, value)
  })

  // 设置 data-theme 属性用于额外的 CSS 选择器
  root.setAttribute('data-theme', themeId)

  // 设置暗色模式类（用于 Element Plus）
  if (themeId === 'smartFactory' || themeId === 'dark') {
    root.classList.add('dark')
    document.body.classList.add('dark')
  } else {
    root.classList.remove('dark')
    document.body.classList.remove('dark')
  }

  // 更新 Element Plus 组件的背景色
  root.style.setProperty('--el-bg-color', theme.colors['--bg-surface'])
  root.style.setProperty('--el-fill-color-blank', theme.colors['--bg-surface'])
  root.style.setProperty('--el-fill-color-light', theme.colors['--bg-elevated'])
  root.style.setProperty('--el-border-color', theme.colors['--border-default'])
  root.style.setProperty('--el-border-color-light', theme.colors['--border-subtle'])
  root.style.setProperty('--el-text-color-primary', theme.colors['--text-primary'])
  root.style.setProperty('--el-text-color-regular', theme.colors['--text-secondary'])
  root.style.setProperty('--el-text-color-secondary', theme.colors['--text-muted'])

  saveTheme(themeId)
}

/**
 * 获取主题列表
 */
export function getThemeList() {
  return Object.values(themes)
}

/**
 * 初始化主题（在应用启动时调用）
 */
export function initTheme() {
  const savedTheme = getSavedTheme()
  applyTheme(savedTheme)
  return savedTheme
}

export default {
  themes,
  DEFAULT_THEME,
  getSavedTheme,
  saveTheme,
  applyTheme,
  getThemeList,
  initTheme
}
