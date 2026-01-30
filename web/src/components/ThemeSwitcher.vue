<script setup>
import { ref, computed, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { themes, getSavedTheme, applyTheme, getThemeList } from '../themes.js'

const props = defineProps({
  // 显示模式：dropdown | panel | mini
  mode: { type: String, default: 'dropdown' }
})

const emit = defineEmits(['change'])

const { t } = useI18n()
const currentTheme = ref(getSavedTheme())
const showPanel = ref(false)
const themeList = getThemeList()

function selectTheme(themeId) {
  currentTheme.value = themeId
  applyTheme(themeId)
  showPanel.value = false
  emit('change', themeId)
}

function getThemeName(theme) {
  try {
    return t(theme.nameKey)
  } catch {
    return theme.name
  }
}

const currentThemeData = computed(() => themes[currentTheme.value] || themes.industrial)

onMounted(() => {
  currentTheme.value = getSavedTheme()
})
</script>

<template>
  <!-- Dropdown 模式 -->
  <el-dropdown v-if="mode === 'dropdown'" trigger="click" @command="selectTheme">
    <button class="theme-trigger">
      <span class="theme-icon">{{ currentThemeData.icon }}</span>
      <span class="theme-name">{{ getThemeName(currentThemeData) }}</span>
      <svg viewBox="0 0 20 20" fill="currentColor" width="16" height="16" class="dropdown-arrow">
        <path fill-rule="evenodd" d="M5.293 7.293a1 1 0 011.414 0L10 10.586l3.293-3.293a1 1 0 111.414 1.414l-4 4a1 1 0 01-1.414 0l-4-4a1 1 0 010-1.414z" clip-rule="evenodd"/>
      </svg>
    </button>
    <template #dropdown>
      <el-dropdown-menu class="theme-dropdown-menu">
        <el-dropdown-item 
          v-for="theme in themeList" 
          :key="theme.id"
          :command="theme.id"
          :class="{ active: currentTheme === theme.id }"
        >
          <div class="theme-option">
            <span class="option-icon">{{ theme.icon }}</span>
            <div class="option-info">
              <span class="option-name">{{ getThemeName(theme) }}</span>
              <span class="option-desc">{{ theme.description }}</span>
            </div>
            <span v-if="currentTheme === theme.id" class="option-check">✓</span>
          </div>
        </el-dropdown-item>
      </el-dropdown-menu>
    </template>
  </el-dropdown>

  <!-- Panel 模式（设置页面使用） -->
  <div v-else-if="mode === 'panel'" class="theme-panel">
    <h3 class="panel-title">{{ t('theme.selectTheme') }}</h3>
    <div class="theme-grid">
      <button
        v-for="theme in themeList"
        :key="theme.id"
        class="theme-card"
        :class="{ active: currentTheme === theme.id, [theme.id]: true }"
        @click="selectTheme(theme.id)"
      >
        <div class="card-preview" :class="theme.id">
          <div class="preview-header"></div>
          <div class="preview-sidebar"></div>
          <div class="preview-content">
            <div class="preview-card"></div>
            <div class="preview-card"></div>
          </div>
        </div>
        <div class="card-info">
          <span class="card-icon">{{ theme.icon }}</span>
          <span class="card-name">{{ getThemeName(theme) }}</span>
        </div>
        <span v-if="currentTheme === theme.id" class="card-check">
          <svg viewBox="0 0 20 20" fill="currentColor" width="16" height="16">
            <path fill-rule="evenodd" d="M16.707 5.293a1 1 0 010 1.414l-8 8a1 1 0 01-1.414 0l-4-4a1 1 0 011.414-1.414L8 12.586l7.293-7.293a1 1 0 011.414 0z" clip-rule="evenodd"/>
          </svg>
        </span>
      </button>
    </div>
  </div>

  <!-- Mini 模式（仅图标） -->
  <button v-else-if="mode === 'mini'" class="theme-mini-btn" @click="showPanel = !showPanel" :title="t('theme.switchTheme')">
    <span class="mini-icon">{{ currentThemeData.icon }}</span>
  </button>
</template>

<style scoped>
/* Dropdown 模式样式 */
.theme-trigger {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.4rem 0.75rem;
  background: transparent;
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-md);
  cursor: pointer;
  color: var(--text-secondary);
  font-size: 0.85rem;
  transition: all var(--transition-fast);
}

.theme-trigger:hover {
  background: var(--bg-hover);
  border-color: var(--border-default);
  color: var(--text-primary);
}

.theme-icon {
  font-size: 1rem;
}

.theme-name {
  font-weight: 500;
}

.dropdown-arrow {
  color: var(--text-muted);
  transition: transform var(--transition-fast);
}

/* Dropdown 菜单 */
.theme-dropdown-menu {
  min-width: 220px;
}

.theme-option {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  padding: 0.25rem 0;
}

.option-icon {
  font-size: 1.25rem;
  width: 32px;
  text-align: center;
}

.option-info {
  flex: 1;
  display: flex;
  flex-direction: column;
  gap: 0.1rem;
}

.option-name {
  font-weight: 500;
  color: var(--text-primary);
}

.option-desc {
  font-size: 0.75rem;
  color: var(--text-muted);
}

.option-check {
  color: var(--accent);
  font-weight: 600;
}

:deep(.el-dropdown-menu__item.active) {
  background: var(--accent-glow);
}

/* Panel 模式样式 */
.theme-panel {
  width: 100%;
}

.panel-title {
  font-size: 1rem;
  font-weight: 600;
  color: var(--text-primary);
  margin: 0 0 1rem;
}

.theme-grid {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(180px, 1fr));
  gap: 1rem;
}

.theme-card {
  position: relative;
  background: var(--bg-surface);
  border: 2px solid var(--border-subtle);
  border-radius: var(--radius-lg);
  padding: 0;
  cursor: pointer;
  overflow: hidden;
  transition: all var(--transition-fast);
}

.theme-card:hover {
  border-color: var(--border-default);
  box-shadow: var(--shadow-md);
}

.theme-card.active {
  border-color: var(--accent);
  box-shadow: 0 0 0 3px var(--accent-glow);
}

/* 主题预览 */
.card-preview {
  height: 100px;
  position: relative;
  overflow: hidden;
}

/* 不同主题预览的配色 */
.card-preview.industrial {
  background: #f7f8fa;
}
.card-preview.industrial .preview-header {
  background: #2d3748;
}
.card-preview.industrial .preview-sidebar {
  background: #ffffff;
  border-right: 1px solid #e8ecf1;
}
.card-preview.industrial .preview-card {
  background: #ffffff;
  border: 1px solid #e8ecf1;
}

.card-preview.smartFactory {
  background: #0f172a;
}
.card-preview.smartFactory .preview-header {
  background: #1e293b;
  border-bottom: 1px solid rgba(6, 182, 212, 0.3);
}
.card-preview.smartFactory .preview-sidebar {
  background: #1e293b;
}
.card-preview.smartFactory .preview-card {
  background: #1e293b;
  border: 1px solid rgba(6, 182, 212, 0.2);
  box-shadow: 0 0 10px rgba(6, 182, 212, 0.1);
}

.card-preview.classic {
  background: #f3f4f6;
}
.card-preview.classic .preview-header {
  background: #1e40af;
}
.card-preview.classic .preview-sidebar {
  background: #ffffff;
}
.card-preview.classic .preview-card {
  background: #ffffff;
  border: 1px solid #e5e7eb;
}

.card-preview.dark {
  background: #09090b;
}
.card-preview.dark .preview-header {
  background: #18181b;
}
.card-preview.dark .preview-sidebar {
  background: #18181b;
}
.card-preview.dark .preview-card {
  background: #18181b;
  border: 1px solid rgba(168, 85, 247, 0.2);
}

.preview-header {
  height: 20px;
  position: absolute;
  top: 0;
  left: 0;
  right: 0;
}

.preview-sidebar {
  width: 30%;
  position: absolute;
  top: 20px;
  left: 0;
  bottom: 0;
}

.preview-content {
  position: absolute;
  top: 28px;
  left: 35%;
  right: 8px;
  bottom: 8px;
  display: flex;
  flex-direction: column;
  gap: 6px;
}

.preview-card {
  flex: 1;
  border-radius: 4px;
}

/* 卡片信息 */
.card-info {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.75rem;
  border-top: 1px solid var(--border-subtle);
}

.card-icon {
  font-size: 1.1rem;
}

.card-name {
  font-weight: 500;
  font-size: 0.9rem;
  color: var(--text-primary);
}

.card-check {
  position: absolute;
  top: 8px;
  right: 8px;
  width: 24px;
  height: 24px;
  background: var(--accent);
  color: white;
  border-radius: 50%;
  display: flex;
  align-items: center;
  justify-content: center;
}

/* Mini 模式样式 */
.theme-mini-btn {
  width: 36px;
  height: 36px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-md);
  cursor: pointer;
  font-size: 1.1rem;
  transition: all var(--transition-fast);
}

.theme-mini-btn:hover {
  background: var(--bg-hover);
  border-color: var(--border-default);
}
</style>
