<script setup>
import { ref, onMounted, provide, computed, watch } from 'vue'
import { useRouter, useRoute } from 'vue-router'
import { useI18n } from 'vue-i18n'
import zhCn from 'element-plus/es/locale/lang/zh-cn.mjs'
import en from 'element-plus/es/locale/lang/en.mjs'
import { api } from './api.js'
import { setLocale } from './i18n/index.js'

const router = useRouter()
const route = useRoute()
const { t, locale } = useI18n()

const sidebarCollapsed = ref(false)
const southPlugins = ref([])
const northPlugins = ref([])

// 提供全局数据
provide('southPlugins', southPlugins)
provide('northPlugins', northPlugins)

// 菜单项（label/desc 用 i18n key，模板中用 t()）
const topMenuItems = [
  { path: '/dashboard', icon: 'dashboard', labelKey: 'menu.overview', descKey: 'menu.overviewDesc' },
  { path: '/monitor', icon: 'monitor', labelKey: 'menu.dataMonitor', descKey: 'menu.dataMonitorDesc' },
]
const dataAcquisitionOpen = ref(true)
const dataAcquisitionItems = [
  { path: '/south', icon: 'south', labelKey: 'menu.southDevices', descKey: 'menu.southDevicesDesc' },
  { path: '/north', icon: 'north', labelKey: 'menu.northApps', descKey: 'menu.northAppsDesc' },
  { path: '/plugins', icon: 'plugins', labelKey: 'menu.pluginManage', descKey: 'menu.pluginManageDesc' },
]

// 设置菜单组
const settingsOpen = ref(true)
const settingsItems = [
  { path: '/settings/license', icon: 'license', labelKey: 'menu.license', descKey: 'menu.licenseDesc' },
  { path: '/settings/logs', icon: 'logs', labelKey: 'menu.logs', descKey: 'menu.logsDesc' },
  { path: '/settings/config', icon: 'config', labelKey: 'menu.systemConfig', descKey: 'menu.systemConfigDesc' },
  { path: '/settings/info', icon: 'info', labelKey: 'menu.systemInfo', descKey: 'menu.systemInfoDesc' },
  { path: '/settings/users', icon: 'users', labelKey: 'menu.users', descKey: 'menu.usersDesc' },
]

const allPaths = [
  ...topMenuItems,
  ...dataAcquisitionItems,
  ...settingsItems,
]

const currentTitle = computed(() => {
  const item = allPaths.find(m => route.path.startsWith(m.path))
  return item ? t(item.labelKey) : t('menu.appName')
})

const isDataAcquisitionActive = computed(() =>
  dataAcquisitionItems.some(m => route.path.startsWith(m.path))
)

const isSettingsActive = computed(() =>
  route.path.startsWith('/settings')
)

// 语言切换：同步到 i18n 并持久化
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
    const [sp, np] = await Promise.all([
      api.pluginsSouth().catch(() => []),
      api.pluginsNorth().catch(() => []),
    ])
    southPlugins.value = sp
    northPlugins.value = np
  } catch (e) {
    console.error('加载初始数据失败', e)
  }
}

onMounted(() => {
  currentLocale.value = locale.value
  loadInitData()
})
</script>

<template>
  <el-config-provider :locale="elLocale">
  <template v-if="isLoginPage">
    <router-view />
  </template>
  <div v-else class="app-container" :class="{ collapsed: sidebarCollapsed }">
    <!-- 侧边栏 -->
    <aside class="sidebar">
      <div class="sidebar-header">
        <div class="logo">
          <svg class="logo-icon" viewBox="0 0 32 32" fill="none">
            <rect x="2" y="2" width="28" height="28" rx="6" stroke="currentColor" stroke-width="2"/>
            <circle cx="10" cy="10" r="3" fill="currentColor"/>
            <circle cx="22" cy="10" r="3" fill="currentColor"/>
            <circle cx="10" cy="22" r="3" fill="currentColor"/>
            <circle cx="22" cy="22" r="3" fill="currentColor"/>
            <line x1="10" y1="13" x2="10" y2="19" stroke="currentColor" stroke-width="1.5"/>
            <line x1="22" y1="13" x2="22" y2="19" stroke="currentColor" stroke-width="1.5"/>
            <line x1="13" y1="10" x2="19" y2="10" stroke="currentColor" stroke-width="1.5"/>
            <line x1="13" y1="22" x2="19" y2="22" stroke="currentColor" stroke-width="1.5"/>
          </svg>
          <span class="logo-text" v-if="!sidebarCollapsed">IoT Gateway</span>
        </div>
      </div>

      <nav class="sidebar-nav">
        <!-- 概览、数据监控（在数据采集上面） -->
        <router-link
          v-for="item in topMenuItems"
          :key="item.path"
          :to="item.path"
          class="nav-item"
          :class="{ active: route.path.startsWith(item.path) }"
        >
          <span class="nav-icon" :data-icon="item.icon">
            <svg v-if="item.icon === 'dashboard'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
              <rect x="3" y="3" width="7" height="9" rx="1"/>
              <rect x="14" y="3" width="7" height="5" rx="1"/>
              <rect x="14" y="12" width="7" height="9" rx="1"/>
              <rect x="3" y="16" width="7" height="5" rx="1"/>
            </svg>
            <svg v-else-if="item.icon === 'monitor'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
              <rect x="2" y="3" width="20" height="14" rx="2"/>
              <line x1="8" y1="21" x2="16" y2="21"/>
              <line x1="12" y1="17" x2="12" y2="21"/>
              <polyline points="6,10 9,7 12,11 15,8 18,10"/>
            </svg>
          </span>
          <span class="nav-label" v-if="!sidebarCollapsed">{{ t(item.labelKey) }}</span>
        </router-link>

        <!-- 数据采集（可折叠；收起时直接显示子项） -->
        <template v-if="sidebarCollapsed">
          <router-link
            v-for="item in dataAcquisitionItems"
            :key="item.path"
            :to="item.path"
            class="nav-item"
            :class="{ active: route.path.startsWith(item.path) }"
          >
            <span class="nav-icon" :data-icon="item.icon">
              <svg v-if="item.icon === 'south'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
                <rect x="3" y="3" width="7" height="7" rx="1"/>
                <rect x="14" y="3" width="7" height="7" rx="1"/>
                <rect x="3" y="14" width="7" height="7" rx="1"/>
                <rect x="14" y="14" width="7" height="7" rx="1"/>
                <circle cx="6.5" cy="6.5" r="1.5" fill="currentColor"/>
                <circle cx="17.5" cy="6.5" r="1.5" fill="currentColor"/>
              </svg>
              <svg v-else-if="item.icon === 'north'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
                <path d="M12 2L2 7l10 5 10-5-10-5z"/>
                <path d="M2 17l10 5 10-5"/>
                <path d="M2 12l10 5 10-5"/>
              </svg>
              <svg v-else-if="item.icon === 'plugins'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
                <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z"/>
                <polyline points="3.27,6.96 12,12.01 20.73,6.96"/>
                <line x1="12" y1="22.08" x2="12" y2="12"/>
              </svg>
            </span>
          </router-link>
        </template>
        <div v-else class="nav-group" :class="{ open: dataAcquisitionOpen, active: isDataAcquisitionActive }">
          <button
            type="button"
            class="nav-group-title"
            @click="dataAcquisitionOpen = !dataAcquisitionOpen"
          >
            <span class="nav-icon nav-icon-group">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
                <path d="M12 2L2 7l10 5 10-5-10-5z"/>
                <path d="M2 17l10 5 10-5"/>
                <path d="M2 12l10 5 10-5"/>
              </svg>
            </span>
            <span class="nav-label">{{ t('menu.dataAcquisition') }}</span>
            <span class="nav-group-arrow">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <path d="M6 9l6 6 6-6"/>
              </svg>
            </span>
          </button>
          <div v-show="dataAcquisitionOpen" class="nav-sub">
            <router-link
              v-for="item in dataAcquisitionItems"
              :key="item.path"
              :to="item.path"
              class="nav-item nav-sub-item"
              :class="{ active: route.path.startsWith(item.path) }"
            >
              <span class="nav-icon" :data-icon="item.icon">
                <svg v-if="item.icon === 'south'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
                  <rect x="3" y="3" width="7" height="7" rx="1"/>
                  <rect x="14" y="3" width="7" height="7" rx="1"/>
                  <rect x="3" y="14" width="7" height="7" rx="1"/>
                  <rect x="14" y="14" width="7" height="7" rx="1"/>
                  <circle cx="6.5" cy="6.5" r="1.5" fill="currentColor"/>
                  <circle cx="17.5" cy="6.5" r="1.5" fill="currentColor"/>
                </svg>
                <svg v-else-if="item.icon === 'north'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
                  <path d="M12 2L2 7l10 5 10-5-10-5z"/>
                  <path d="M2 17l10 5 10-5"/>
                  <path d="M2 12l10 5 10-5"/>
                </svg>
                <svg v-else-if="item.icon === 'plugins'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
                  <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z"/>
                  <polyline points="3.27,6.96 12,12.01 20.73,6.96"/>
                  <line x1="12" y1="22.08" x2="12" y2="12"/>
                </svg>
              </span>
              <span class="nav-label">{{ t(item.labelKey) }}</span>
            </router-link>
          </div>
        </div>

        <!-- 设置（可折叠；收起时直接显示子项） -->
        <template v-if="sidebarCollapsed">
          <router-link
            v-for="item in settingsItems"
            :key="item.path"
            :to="item.path"
            class="nav-item"
            :class="{ active: route.path === item.path }"
          >
            <span class="nav-icon" :data-icon="item.icon">
              <svg v-if="item.icon === 'license'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
                <path d="M12 15v2m-6 4h12a2 2 0 0 0 2-2v-6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v6a2 2 0 0 0 2 2zm10-10V7a4 4 0 0 0-8 0v2h8z"/>
              </svg>
              <svg v-else-if="item.icon === 'logs'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
                <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/>
                <polyline points="14,2 14,8 20,8"/>
                <line x1="16" y1="13" x2="8" y2="13"/>
                <line x1="16" y1="17" x2="8" y2="17"/>
                <polyline points="10,9 9,9 8,9"/>
              </svg>
              <svg v-else-if="item.icon === 'config'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
                <circle cx="12" cy="12" r="3"/>
                <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"/>
              </svg>
              <svg v-else-if="item.icon === 'info'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
                <circle cx="12" cy="12" r="10"/>
                <line x1="12" y1="16" x2="12" y2="12"/>
                <line x1="12" y1="8" x2="12.01" y2="8"/>
              </svg>
              <svg v-else-if="item.icon === 'users'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
                <path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"/>
                <circle cx="9" cy="7" r="4"/>
                <path d="M23 21v-2a4 4 0 0 0-3-3.87"/>
                <path d="M16 3.13a4 4 0 0 1 0 7.75"/>
              </svg>
            </span>
          </router-link>
        </template>
        <div v-else class="nav-group" :class="{ open: settingsOpen, active: isSettingsActive }">
          <button
            type="button"
            class="nav-group-title"
            @click="settingsOpen = !settingsOpen"
          >
            <span class="nav-icon nav-icon-group">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
                <circle cx="12" cy="12" r="3"/>
                <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"/>
              </svg>
            </span>
            <span class="nav-label">{{ t('menu.settings') }}</span>
            <span class="nav-group-arrow">
              <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
                <path d="M6 9l6 6 6-6"/>
              </svg>
            </span>
          </button>
          <div v-show="settingsOpen" class="nav-sub">
            <router-link
              v-for="item in settingsItems"
              :key="item.path"
              :to="item.path"
              class="nav-item nav-sub-item"
              :class="{ active: route.path === item.path }"
            >
              <span class="nav-icon" :data-icon="item.icon">
                <svg v-if="item.icon === 'license'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
                  <path d="M12 15v2m-6 4h12a2 2 0 0 0 2-2v-6a2 2 0 0 0-2-2H6a2 2 0 0 0-2 2v6a2 2 0 0 0 2 2zm10-10V7a4 4 0 0 0-8 0v2h8z"/>
                </svg>
                <svg v-else-if="item.icon === 'logs'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
                  <path d="M14 2H6a2 2 0 0 0-2 2v16a2 2 0 0 0 2 2h12a2 2 0 0 0 2-2V8z"/>
                  <polyline points="14,2 14,8 20,8"/>
                  <line x1="16" y1="13" x2="8" y2="13"/>
                  <line x1="16" y1="17" x2="8" y2="17"/>
                  <polyline points="10,9 9,9 8,9"/>
                </svg>
                <svg v-else-if="item.icon === 'config'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
                  <circle cx="12" cy="12" r="3"/>
                  <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"/>
                </svg>
                <svg v-else-if="item.icon === 'info'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
                  <circle cx="12" cy="12" r="10"/>
                  <line x1="12" y1="16" x2="12" y2="12"/>
                  <line x1="12" y1="8" x2="12.01" y2="8"/>
                </svg>
                <svg v-else-if="item.icon === 'users'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
                  <path d="M17 21v-2a4 4 0 0 0-4-4H5a4 4 0 0 0-4 4v2"/>
                  <circle cx="9" cy="7" r="4"/>
                  <path d="M23 21v-2a4 4 0 0 0-3-3.87"/>
                  <path d="M16 3.13a4 4 0 0 1 0 7.75"/>
                </svg>
              </span>
              <span class="nav-label">{{ t(item.labelKey) }}</span>
            </router-link>
          </div>
        </div>
      </nav>
    </aside>

    <!-- 主内容区 -->
    <main class="main-content">
      <header class="top-header">
        <div class="header-left">
          <button type="button" class="sidebar-toggle" @click="sidebarCollapsed = !sidebarCollapsed" :title="sidebarCollapsed ? t('menu.expandMenu') : t('menu.collapseMenu')">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
              <path v-if="sidebarCollapsed" d="M9 18l6-6-6-6"/>
              <path v-else d="M15 18l-6-6 6-6"/>
            </svg>
          </button>
          <h1 class="page-title">{{ currentTitle }}</h1>
          <div class="breadcrumb" v-if="route.params.id || route.path.endsWith('/new')">
            <template v-if="route.path.startsWith('/south')">
              <router-link to="/south">{{ t('header.southDevices') }}</router-link>
              <span class="separator">/</span>
              <router-link v-if="route.path.endsWith('/config')" :to="`/south/${route.params.id}`">{{ t('common.detail') }}</router-link>
              <template v-if="route.path.endsWith('/config')"><span class="separator">/</span></template>
              <span>{{ route.path.endsWith('/new') ? t('header.addDevice') : route.path.endsWith('/config') ? t('nodeDetail.nodeConfig') : t('common.detail') }}</span>
            </template>
            <template v-else-if="route.path.startsWith('/north')">
              <router-link to="/north">{{ t('header.northApps') }}</router-link>
              <span class="separator">/</span>
              <router-link v-if="route.path.endsWith('/config')" :to="`/north/${route.params.id}`">{{ t('common.detail') }}</router-link>
              <template v-if="route.path.endsWith('/config')"><span class="separator">/</span></template>
              <span>{{ route.path.endsWith('/new') ? t('header.addApp') : route.path.endsWith('/config') ? t('nodeDetail.nodeConfig') : t('common.detail') }}</span>
            </template>
          </div>
        </div>
        <div class="header-right header-actions">
          <el-dropdown trigger="click" class="user-dropdown">
            <span class="user-trigger">
              <el-avatar :size="28" class="user-avatar">{{ userInitial }}</el-avatar>
            </span>
            <template #dropdown>
              <el-dropdown-menu>
                <el-dropdown-item disabled class="dropdown-user-name">{{ userDisplayName }}</el-dropdown-item>
                <el-dropdown-item divided @click="toggleLocale">
                  {{ currentLocale === 'zh' ? 'English' : '中文' }} ({{ t('user.switchLanguage') }})
                </el-dropdown-item>
                <el-dropdown-item @click="handleLogout">{{ t('user.logout') }}</el-dropdown-item>
              </el-dropdown-menu>
            </template>
          </el-dropdown>
        </div>
      </header>

      <div class="content-area">
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
