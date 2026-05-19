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

// Navigation data with i18n labels resolved via computed
const mainNavItems = [
  { path: '/dashboard', icon: 'dashboard', labelKey: 'menu.overview' },
  { path: '/south', icon: 'devices', labelKey: 'menu.southDevices' },
  { path: '/north', icon: 'cloud', labelKey: 'menu.northApps' },
  { path: '/monitor', icon: 'monitor', labelKey: 'menu.dataMonitor' },
  { path: '/flows', icon: 'flow', labelKey: 'menu.flowOrchestration' },
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
function resolveItem(item) {
  return {
    ...item,
    label: t(item.labelKey),
    badge: item.path === '/south' ? nodes.value.filter(n => n.kind === 'south').length
         : item.path === '/north' ? nodes.value.filter(n => n.kind === 'north').length
         : undefined,
  }
}

const resolvedMainNav = computed(() => mainNavItems.map(resolveItem))
const resolvedSystemNav = computed(() => systemNavItems.map(i => ({ ...i, label: t(i.labelKey) })))

// Computed values for status bar
const runningCount = computed(() => nodes.value.filter(n => n.state === 'running').length)
const totalCount = computed(() => nodes.value.length)

const userDisplayName = computed(() => {
  return localStorage.getItem('gateway_user') || 'admin'
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
