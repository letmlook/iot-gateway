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
const health = ref(null)
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
const systemItem = { path: '/system', icon: 'system', labelKey: 'menu.system', descKey: 'menu.systemDesc' }

const allPaths = [
  ...topMenuItems,
  ...dataAcquisitionItems,
  systemItem,
]

const currentTitle = computed(() => {
  const item = allPaths.find(m => route.path.startsWith(m.path))
  return item ? t(item.labelKey) : t('menu.appName')
})

const isDataAcquisitionActive = computed(() =>
  dataAcquisitionItems.some(m => route.path.startsWith(m.path))
)

// 语言切换：同步到 i18n 并持久化
const currentLocale = ref(locale.value)
watch(currentLocale, (val) => {
  setLocale(val)
})
const elLocale = computed(() => (currentLocale.value === 'en' ? en : zhCn))
const userInitial = ref('U')

async function loadInitData() {
  try {
    const [h, sp, np] = await Promise.all([
      api.health().catch(() => null),
      api.pluginsSouth().catch(() => []),
      api.pluginsNorth().catch(() => []),
    ])
    health.value = h
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
  <div class="app-container" :class="{ collapsed: sidebarCollapsed }">
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

        <!-- 系统管理 -->
        <router-link
          :to="systemItem.path"
          class="nav-item"
          :class="{ active: route.path.startsWith(systemItem.path) }"
        >
          <span class="nav-icon" :data-icon="systemItem.icon">
            <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
              <circle cx="12" cy="12" r="3"/>
              <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"/>
            </svg>
          </span>
          <span class="nav-label" v-if="!sidebarCollapsed">{{ t(systemItem.labelKey) }}</span>
        </router-link>
      </nav>

      <div class="sidebar-footer" v-if="!sidebarCollapsed">
        <div class="status-indicator" :class="health?.status === 'ok' ? 'online' : 'offline'">
          <span class="status-dot"></span>
          <span>{{ health?.status === 'ok' ? t('common.statusOk') : t('common.connecting') + '...' }}</span>
        </div>
      </div>
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
              <span>{{ route.path.endsWith('/new') ? t('header.addDevice') : t('common.detail') }}</span>
            </template>
            <template v-else-if="route.path.startsWith('/north')">
              <router-link to="/north">{{ t('header.northApps') }}</router-link>
              <span class="separator">/</span>
              <span>{{ route.path.endsWith('/new') ? t('header.addApp') : t('common.detail') }}</span>
            </template>
          </div>
        </div>
        <div class="header-right header-actions">
          <el-select v-model="currentLocale" size="small" style="width: 90px" class="mr-1">
            <el-option label="中文" value="zh" />
            <el-option label="English" value="en" />
          </el-select>
          <el-dropdown trigger="click" class="user-dropdown">
            <span class="user-trigger">
              <el-avatar :size="28" class="user-avatar">{{ userInitial }}</el-avatar>
            </span>
            <template #dropdown>
              <el-dropdown-menu>
                <el-dropdown-item>{{ t('common.currentUser') }}</el-dropdown-item>
              </el-dropdown-menu>
            </template>
          </el-dropdown>
          <div class="stats-bar" v-if="health && !route.params.id && !route.path.endsWith('/new')">
            <div class="stat-item">
              <span class="stat-value">{{ health.nodes_count || 0 }}</span>
              <span class="stat-label">{{ t('header.nodesTotal') }}</span>
            </div>
            <div class="stat-item">
              <span class="stat-value running">{{ health.nodes_running || 0 }}</span>
              <span class="stat-label">{{ t('header.runningCount') }}</span>
            </div>
            <div class="stat-item">
              <span class="stat-value south">{{ health.plugins_south || 0 }}</span>
              <span class="stat-label">{{ t('header.southPlugins') }}</span>
            </div>
            <div class="stat-item">
              <span class="stat-value north">{{ health.plugins_north || 0 }}</span>
              <span class="stat-label">{{ t('header.northPlugins') }}</span>
            </div>
          </div>
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
