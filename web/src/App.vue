<script setup>
import { ref, onMounted, provide, computed, watch } from 'vue'
import { useRouter, useRoute } from 'vue-router'
import { useI18n } from 'vue-i18n'
import zhCn from 'element-plus/es/locale/lang/zh-cn.mjs'
import en from 'element-plus/es/locale/lang/en.mjs'
import { api } from './api.js'
import { setLocale } from './i18n/index.js'
import { initTheme } from './themes.js'
import ThemeSwitcher from './components/ThemeSwitcher.vue'

const router = useRouter()
const route = useRoute()
const { t, locale } = useI18n()

const southPlugins = ref([])
const northPlugins = ref([])
const nodes = ref([])
const health = ref(null)

// 提供全局数据
provide('southPlugins', southPlugins)
provide('northPlugins', northPlugins)
provide('nodes', nodes)
provide('health', health)

// 主导航项
const mainNavItems = [
  { path: '/dashboard', icon: 'dashboard', labelKey: 'menu.overview' },
  { path: '/south', icon: 'devices', labelKey: 'menu.southDevices', badge: () => nodes.value.filter(n => n.kind === 'south').length },
  { path: '/north', icon: 'cloud', labelKey: 'menu.northApps', badge: () => nodes.value.filter(n => n.kind === 'north').length },
  { path: '/monitor', icon: 'monitor', labelKey: 'menu.dataMonitor' },
  { path: '/plugins', icon: 'plugins', labelKey: 'menu.pluginManage' },
]

// 系统菜单项
const systemMenuItems = [
  { path: '/monitor/flow', labelKey: 'menu.dataFlowMetrics' },
  { path: '/settings/license', labelKey: 'menu.license' },
  { path: '/settings/logs', labelKey: 'menu.logs' },
  { path: '/settings/config', labelKey: 'menu.systemConfig' },
  { path: '/settings/info', labelKey: 'menu.systemInfo' },
  { path: '/settings/users', labelKey: 'menu.users' },
]

const allPaths = [...mainNavItems, ...systemMenuItems]

const currentTitle = computed(() => {
  const item = allPaths.find(m => route.path.startsWith(m.path))
  return item ? t(item.labelKey) : t('menu.appName')
})

// 语言切换
const currentLocale = ref(locale.value)
watch(currentLocale, (val) => {
  setLocale(val)
})
const elLocale = computed(() => (currentLocale.value === 'en' ? en : zhCn))

const userDisplayName = computed(() => {
  const name = localStorage.getItem('gateway_user') || 'admin'
  return name
})

const userInitial = computed(() => {
  const name = userDisplayName.value
  return name && name.length ? name.charAt(0).toUpperCase() : 'U'
})

// 统计数据
const runningCount = computed(() => nodes.value.filter(n => n.state === 'running').length)
const totalCount = computed(() => nodes.value.length)

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
  } catch (e) {
    console.error('加载初始数据失败', e)
  }
}

// 检查路由是否激活
function isRouteActive(path) {
  if (path === '/dashboard') {
    return route.path === '/dashboard' || route.path === '/'
  }
  return route.path.startsWith(path)
}

onMounted(() => {
  // 初始化主题
  initTheme()
  
  currentLocale.value = locale.value
  loadInitData()
})
</script>

<template>
  <el-config-provider :locale="elLocale">
    <!-- 登录页面：无布局 -->
    <template v-if="isLoginPage">
      <router-view />
    </template>

    <!-- 主应用布局：顶部导航 -->
    <div v-else class="app-layout">
      <!-- 顶部导航栏 -->
      <header class="top-nav">
        <!-- Logo 区域 -->
        <div class="nav-brand">
          <div class="brand-logo">
            <svg class="logo-icon" viewBox="0 0 32 32" fill="none" xmlns="http://www.w3.org/2000/svg" aria-hidden="true">
              <rect x="2" y="2" width="28" height="28" rx="7" fill="var(--primary)" />
              <path d="M10 8v16l12-8-12-8z" fill="var(--accent)" />
              <circle cx="22" cy="16" r="3" fill="var(--accent)" />
            </svg>
          </div>
          <span class="brand-name">IoT Gateway</span>
        </div>

        <!-- 主导航 -->
        <nav class="nav-main">
          <router-link
            v-for="item in mainNavItems"
            :key="item.path"
            :to="item.path"
            class="nav-link"
            :class="{ active: isRouteActive(item.path) }"
          >
            <!-- 图标 -->
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
            <span class="nav-text">{{ t(item.labelKey) }}</span>
            <span v-if="item.badge && item.badge() > 0" class="nav-badge">{{ item.badge() }}</span>
          </router-link>
        </nav>

        <!-- 右侧操作区 -->
        <div class="nav-actions">
          <!-- 状态指示 -->
          <div class="status-pill" :class="{ online: runningCount > 0 }">
            <span class="status-dot"></span>
            <span class="status-text">{{ runningCount }}/{{ totalCount }}</span>
          </div>

          <!-- 主题切换 -->
          <ThemeSwitcher mode="dropdown" />

          <!-- 系统菜单 -->
          <el-dropdown trigger="click" class="system-dropdown">
            <button class="action-btn" :title="t('menu.settings')">
              <svg viewBox="0 0 20 20" fill="currentColor" width="18" height="18">
                <path fill-rule="evenodd" d="M11.49 3.17c-.38-1.56-2.6-1.56-2.98 0a1.532 1.532 0 01-2.286.948c-1.372-.836-2.942.734-2.106 2.106.54.886.061 2.042-.947 2.287-1.561.379-1.561 2.6 0 2.978a1.532 1.532 0 01.947 2.287c-.836 1.372.734 2.942 2.106 2.106a1.532 1.532 0 012.287.947c.379 1.561 2.6 1.561 2.978 0a1.533 1.533 0 012.287-.947c1.372.836 2.942-.734 2.106-2.106a1.533 1.533 0 01.947-2.287c1.561-.379 1.561-2.6 0-2.978a1.532 1.532 0 01-.947-2.287c.836-1.372-.734-2.942-2.106-2.106a1.532 1.532 0 01-2.287-.947zM10 13a3 3 0 100-6 3 3 0 000 6z" clip-rule="evenodd"/>
              </svg>
            </button>
            <template #dropdown>
              <el-dropdown-menu>
                <el-dropdown-item v-for="item in systemMenuItems" :key="item.path" @click="router.push(item.path)">
                  {{ t(item.labelKey) }}
                </el-dropdown-item>
              </el-dropdown-menu>
            </template>
          </el-dropdown>

          <!-- 用户菜单 -->
          <el-dropdown trigger="click" class="user-dropdown">
            <div class="user-trigger">
              <span class="user-avatar">{{ userInitial }}</span>
              <span class="user-name">{{ userDisplayName }}</span>
              <svg viewBox="0 0 20 20" fill="currentColor" width="16" height="16" class="dropdown-arrow">
                <path fill-rule="evenodd" d="M5.293 7.293a1 1 0 011.414 0L10 10.586l3.293-3.293a1 1 0 111.414 1.414l-4 4a1 1 0 01-1.414 0l-4-4a1 1 0 010-1.414z" clip-rule="evenodd"/>
              </svg>
            </div>
            <template #dropdown>
              <el-dropdown-menu>
                <el-dropdown-item disabled class="dropdown-user-info">
                  <span class="user-role">{{ t('user.admin') }}</span>
                </el-dropdown-item>
                <el-dropdown-item divided @click="toggleLocale">
                  <svg viewBox="0 0 20 20" fill="currentColor" width="16" height="16" class="menu-icon">
                    <path fill-rule="evenodd" d="M7 2a1 1 0 011 1v1h3a1 1 0 110 2H9.578a18.87 18.87 0 01-1.724 4.78c.29.354.596.696.914 1.026a1 1 0 11-1.44 1.389c-.188-.196-.373-.396-.554-.6a19.098 19.098 0 01-3.107 3.567 1 1 0 01-1.334-1.49 17.087 17.087 0 003.13-3.733 18.992 18.992 0 01-1.487-2.494 1 1 0 111.79-.89c.234.47.489.928.764 1.372.417-.934.752-1.913.997-2.927H3a1 1 0 110-2h3V3a1 1 0 011-1zm6 6a1 1 0 01.894.553l2.991 5.982a.869.869 0 01.02.037l.99 1.98a1 1 0 11-1.79.895L15.383 16h-4.764l-.724 1.447a1 1 0 11-1.788-.894l.99-1.98.019-.038 2.99-5.982A1 1 0 0113 8zm-1.382 6h2.764L13 11.236 11.618 14z" clip-rule="evenodd"/>
                  </svg>
                  {{ currentLocale === 'zh' ? t('user.localeEn') : t('user.localeZh') }}
                </el-dropdown-item>
                <el-dropdown-item @click="handleLogout">
                  <svg viewBox="0 0 20 20" fill="currentColor" width="16" height="16" class="menu-icon">
                    <path fill-rule="evenodd" d="M3 3a1 1 0 00-1 1v12a1 1 0 102 0V4a1 1 0 00-1-1zm10.293 9.293a1 1 0 001.414 1.414l3-3a1 1 0 000-1.414l-3-3a1 1 0 10-1.414 1.414L14.586 9H7a1 1 0 100 2h7.586l-1.293 1.293z" clip-rule="evenodd"/>
                  </svg>
                  {{ t('user.logout') }}
                </el-dropdown-item>
              </el-dropdown-menu>
            </template>
          </el-dropdown>
        </div>
      </header>

      <!-- 面包屑导航（仅在详情页显示） -->
      <div v-if="route.params.id || route.path.endsWith('/new')" class="breadcrumb-bar">
        <div class="breadcrumb">
          <template v-if="route.path.startsWith('/south')">
            <router-link to="/south" class="breadcrumb-link">{{ t('header.southDevices') }}</router-link>
            <span class="breadcrumb-sep">/</span>
            <router-link v-if="route.path.endsWith('/config')" :to="`/south/${route.params.id}`" class="breadcrumb-link">{{ t('common.detail') }}</router-link>
            <template v-if="route.path.endsWith('/config')"><span class="breadcrumb-sep">/</span></template>
            <span class="breadcrumb-current">{{ route.path.endsWith('/new') ? t('header.addDevice') : route.path.endsWith('/config') ? t('nodeDetail.nodeConfig') : t('common.detail') }}</span>
          </template>
          <template v-else-if="route.path.startsWith('/north')">
            <router-link to="/north" class="breadcrumb-link">{{ t('header.northApps') }}</router-link>
            <span class="breadcrumb-sep">/</span>
            <router-link v-if="route.path.endsWith('/config')" :to="`/north/${route.params.id}`" class="breadcrumb-link">{{ t('common.detail') }}</router-link>
            <template v-if="route.path.endsWith('/config')"><span class="breadcrumb-sep">/</span></template>
            <span class="breadcrumb-current">{{ route.path.endsWith('/new') ? t('header.addApp') : route.path.endsWith('/config') ? t('nodeDetail.nodeConfig') : t('common.detail') }}</span>
          </template>
        </div>
      </div>

      <!-- 主内容区：圆角容器内滚动 -->
      <main class="main-content">
        <div class="content-container">
          <router-view v-slot="{ Component }">
            <transition name="fade" mode="out-in">
              <component :is="Component" />
            </transition>
          </router-view>
        </div>
      </main>
    </div>
  </el-config-provider>
</template>

<style scoped>
/* ═══════════════════════════════════════════
   App Layout - 顶部导航布局
   ═══════════════════════════════════════════ */
.app-layout {
  height: 100vh;
  display: flex;
  flex-direction: column;
  background: var(--bg-base);
  overflow: hidden;
}

/* ─────────── 顶部导航栏 ─────────── */
.top-nav {
  height: 60px;
  background: var(--bg-surface);
  border-bottom: 1px solid var(--border-subtle);
  display: flex;
  align-items: center;
  padding: 0 1.5rem;
  gap: 2rem;
  position: sticky;
  top: 0;
  z-index: 100;
  box-shadow: var(--shadow-xs);
}

/* Logo */
.nav-brand {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  flex-shrink: 0;
}

.brand-logo {
  width: 36px;
  height: 36px;
}

.logo-icon {
  width: 100%;
  height: 100%;
}

.brand-name {
  font-weight: 700;
  font-size: 1.1rem;
  color: var(--text-primary);
  letter-spacing: -0.02em;
}

/* 主导航 */
.nav-main {
  display: flex;
  align-items: center;
  gap: 0.25rem;
  flex: 1;
}

.nav-link {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.55rem 0.9rem;
  border-radius: var(--radius-md);
  color: var(--text-secondary);
  text-decoration: none;
  font-weight: 500;
  font-size: 0.875rem;
  transition: all var(--transition-fast);
  position: relative;
}

.nav-link:hover {
  background: var(--bg-hover);
  color: var(--text-primary);
}

.nav-link.active {
  background: var(--accent-glow);
  color: var(--accent-dim);
}

.nav-link.active::after {
  content: '';
  position: absolute;
  bottom: -11px;
  left: 50%;
  transform: translateX(-50%);
  width: 20px;
  height: 3px;
  background: var(--accent);
  border-radius: 3px 3px 0 0;
}

.nav-icon {
  width: 18px;
  height: 18px;
  display: flex;
  align-items: center;
  justify-content: center;
}

.nav-icon svg {
  width: 100%;
  height: 100%;
}

.nav-text {
  white-space: nowrap;
}

.nav-badge {
  background: var(--accent);
  color: white;
  font-size: 0.7rem;
  padding: 0.1rem 0.45rem;
  border-radius: 10px;
  font-weight: 600;
  min-width: 18px;
  text-align: center;
}

/* 右侧操作区 */
.nav-actions {
  display: flex;
  align-items: center;
  gap: 0.75rem;
  flex-shrink: 0;
}

/* 状态指示胶囊 */
.status-pill {
  display: flex;
  align-items: center;
  gap: 0.4rem;
  padding: 0.35rem 0.75rem;
  background: var(--bg-inset);
  border-radius: 100px;
  font-size: 0.8rem;
  color: var(--text-muted);
}

.status-pill.online {
  background: rgba(56, 161, 105, 0.1);
  color: var(--success);
}

.status-dot {
  width: 8px;
  height: 8px;
  border-radius: 50%;
  background: var(--text-muted);
}

.status-pill.online .status-dot {
  background: var(--success);
  box-shadow: 0 0 0 2px rgba(56, 161, 105, 0.2);
  animation: pulse 2s infinite;
}

@keyframes pulse {
  0%, 100% { opacity: 1; }
  50% { opacity: 0.5; }
}

/* 操作按钮 */
.action-btn {
  width: 36px;
  height: 36px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-md);
  cursor: pointer;
  color: var(--text-secondary);
  transition: all var(--transition-fast);
}

.action-btn:hover {
  background: var(--bg-hover);
  border-color: var(--border-default);
  color: var(--text-primary);
}

/* 用户菜单 */
.user-trigger {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  padding: 0.35rem 0.5rem 0.35rem 0.35rem;
  border-radius: var(--radius-md);
  cursor: pointer;
  transition: all var(--transition-fast);
}

.user-trigger:hover {
  background: var(--bg-hover);
}

.user-avatar {
  width: 30px;
  height: 30px;
  background: var(--primary);
  color: white;
  border-radius: var(--radius-sm);
  display: flex;
  align-items: center;
  justify-content: center;
  font-weight: 600;
  font-size: 0.85rem;
}

.user-name {
  font-size: 0.875rem;
  font-weight: 500;
  color: var(--text-primary);
}

.dropdown-arrow {
  color: var(--text-muted);
  transition: transform var(--transition-fast);
}

.user-dropdown:focus-within .dropdown-arrow {
  transform: rotate(180deg);
}

.dropdown-user-info {
  font-size: 0.75rem;
  color: var(--text-muted);
}

.user-role {
  text-transform: uppercase;
  letter-spacing: 0.05em;
}

.menu-icon {
  margin-right: 0.5rem;
  color: var(--text-muted);
}

/* ─────────── 面包屑导航栏 ─────────── */
.breadcrumb-bar {
  background: var(--bg-surface);
  border-bottom: 1px solid var(--border-subtle);
  padding: 0.6rem 1.5rem;
}

.breadcrumb {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  font-size: 0.85rem;
}

.breadcrumb-link {
  color: var(--text-secondary);
  text-decoration: none;
  transition: color var(--transition-fast);
}

.breadcrumb-link:hover {
  color: var(--accent);
}

.breadcrumb-sep {
  color: var(--text-muted);
}

.breadcrumb-current {
  color: var(--text-primary);
  font-weight: 500;
}

/* ─────────── 主内容区 ─────────── */
.main-content {
  flex: 1;
  display: flex;
  flex-direction: column;
  padding: 1rem 1.5rem;
  max-width: 1440px;
  margin: 0 auto;
  width: 100%;
  min-height: 0;
}

/* 圆角内容容器：与底板对比，仅此区域滚动 */
.content-container {
  --scrollbar-track: transparent;
  --scrollbar-thumb: var(--border-subtle);
  flex: 1;
  min-height: 0;
  overflow-y: auto;
  overflow-x: hidden;
  background: var(--bg-surface);
  border-radius: 12px;
  box-shadow: var(--shadow-sm, 0 1px 3px rgba(0, 0, 0, 0.08));
  border: 1px solid var(--border-subtle);
  padding: 1.5rem;
  scrollbar-width: thin;
  scrollbar-color: var(--scrollbar-thumb) var(--scrollbar-track);
}

/* 美化 content-container 滚动条（WebKit） */
.content-container::-webkit-scrollbar {
  width: 6px;
}

.content-container::-webkit-scrollbar-track {
  background: transparent;
  margin: 8px 0;
}

.content-container::-webkit-scrollbar-thumb {
  background: var(--border-subtle);
  border-radius: 10px;
  transition: background var(--transition-fast);
}

.content-container::-webkit-scrollbar-thumb:hover {
  background: var(--border-default);
}

.content-container::-webkit-scrollbar-thumb:active {
  background: var(--accent);
}

/* ─────────── 页面过渡动画 ─────────── */
.fade-enter-active,
.fade-leave-active {
  transition: opacity 0.15s ease;
}

.fade-enter-from,
.fade-leave-to {
  opacity: 0;
}

/* ─────────── 响应式 ─────────── */
@media (max-width: 1024px) {
  .nav-text {
    display: none;
  }
  
  .nav-link {
    padding: 0.6rem;
  }
  
  .brand-name {
    display: none;
  }
  
  .user-name {
    display: none;
  }
}

@media (max-width: 768px) {
  .top-nav {
    padding: 0 1rem;
    gap: 1rem;
  }
  
  .main-content {
    padding: 0.75rem 1rem;
  }

  .content-container {
    padding: 1rem;
  }
  
  .status-pill {
    display: none;
  }
}
</style>
