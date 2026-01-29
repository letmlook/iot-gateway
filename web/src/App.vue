<script setup>
import { ref, onMounted, provide, computed } from 'vue'
import { useRouter, useRoute } from 'vue-router'
import { api } from './api.js'

const router = useRouter()
const route = useRoute()

const sidebarCollapsed = ref(false)
const health = ref(null)
const version = ref(null)
const southPlugins = ref([])
const northPlugins = ref([])

// 提供全局数据
provide('southPlugins', southPlugins)
provide('northPlugins', northPlugins)

const menuItems = [
  { path: '/dashboard', icon: 'dashboard', label: '概览', desc: '运行状态与快捷入口' },
  { path: '/south', icon: 'south', label: '南向设备', desc: '设备驱动管理' },
  { path: '/north', icon: 'north', label: '北向应用', desc: '数据上报应用' },
  { path: '/plugins', icon: 'plugins', label: '插件管理', desc: '插件列表与 Schema' },
  { path: '/monitor', icon: 'monitor', label: '数据监控', desc: '实时数据与写值' },
  { path: '/system', icon: 'system', label: '系统管理', desc: '配置与导出' },
]

const currentTitle = computed(() => {
  const item = menuItems.find(m => route.path.startsWith(m.path))
  return item?.label || 'IoT 网关'
})

const locale = ref('zh')
const userInitial = ref('U')

async function loadInitData() {
  try {
    const [h, v, sp, np] = await Promise.all([
      api.health().catch(() => null),
      api.version().catch(() => null),
      api.pluginsSouth().catch(() => []),
      api.pluginsNorth().catch(() => []),
    ])
    health.value = h
    version.value = v
    southPlugins.value = sp
    northPlugins.value = np
  } catch (e) {
    console.error('加载初始数据失败', e)
  }
}

onMounted(loadInitData)
</script>

<template>
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
        <button class="toggle-btn" @click="sidebarCollapsed = !sidebarCollapsed">
          <svg viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="2">
            <path v-if="sidebarCollapsed" d="M9 18l6-6-6-6"/>
            <path v-else d="M15 18l-6-6 6-6"/>
          </svg>
        </button>
      </div>

      <nav class="sidebar-nav">
        <router-link
          v-for="item in menuItems"
          :key="item.path"
          :to="item.path"
          class="nav-item"
          :class="{ active: route.path.startsWith(item.path) }"
        >
          <span class="nav-icon" :data-icon="item.icon">
            <!-- 概览图标 -->
            <svg v-if="item.icon === 'dashboard'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
              <rect x="3" y="3" width="7" height="9" rx="1"/>
              <rect x="14" y="3" width="7" height="5" rx="1"/>
              <rect x="14" y="12" width="7" height="9" rx="1"/>
              <rect x="3" y="16" width="7" height="5" rx="1"/>
            </svg>
            <!-- 南向设备图标 -->
            <svg v-else-if="item.icon === 'south'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
              <rect x="3" y="3" width="7" height="7" rx="1"/>
              <rect x="14" y="3" width="7" height="7" rx="1"/>
              <rect x="3" y="14" width="7" height="7" rx="1"/>
              <rect x="14" y="14" width="7" height="7" rx="1"/>
              <circle cx="6.5" cy="6.5" r="1.5" fill="currentColor"/>
              <circle cx="17.5" cy="6.5" r="1.5" fill="currentColor"/>
            </svg>
            <!-- 北向应用图标 -->
            <svg v-else-if="item.icon === 'north'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
              <path d="M12 2L2 7l10 5 10-5-10-5z"/>
              <path d="M2 17l10 5 10-5"/>
              <path d="M2 12l10 5 10-5"/>
            </svg>
            <!-- 插件管理图标 -->
            <svg v-else-if="item.icon === 'plugins'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
              <path d="M21 16V8a2 2 0 0 0-1-1.73l-7-4a2 2 0 0 0-2 0l-7 4A2 2 0 0 0 3 8v8a2 2 0 0 0 1 1.73l7 4a2 2 0 0 0 2 0l7-4A2 2 0 0 0 21 16z"/>
              <polyline points="3.27,6.96 12,12.01 20.73,6.96"/>
              <line x1="12" y1="22.08" x2="12" y2="12"/>
            </svg>
            <!-- 数据监控图标 -->
            <svg v-else-if="item.icon === 'monitor'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
              <rect x="2" y="3" width="20" height="14" rx="2"/>
              <line x1="8" y1="21" x2="16" y2="21"/>
              <line x1="12" y1="17" x2="12" y2="21"/>
              <polyline points="6,10 9,7 12,11 15,8 18,10"/>
            </svg>
            <!-- 系统管理图标 -->
            <svg v-else-if="item.icon === 'system'" viewBox="0 0 24 24" fill="none" stroke="currentColor" stroke-width="1.5">
              <circle cx="12" cy="12" r="3"/>
              <path d="M19.4 15a1.65 1.65 0 0 0 .33 1.82l.06.06a2 2 0 0 1 0 2.83 2 2 0 0 1-2.83 0l-.06-.06a1.65 1.65 0 0 0-1.82-.33 1.65 1.65 0 0 0-1 1.51V21a2 2 0 0 1-2 2 2 2 0 0 1-2-2v-.09A1.65 1.65 0 0 0 9 19.4a1.65 1.65 0 0 0-1.82.33l-.06.06a2 2 0 0 1-2.83 0 2 2 0 0 1 0-2.83l.06-.06a1.65 1.65 0 0 0 .33-1.82 1.65 1.65 0 0 0-1.51-1H3a2 2 0 0 1-2-2 2 2 0 0 1 2-2h.09A1.65 1.65 0 0 0 4.6 9a1.65 1.65 0 0 0-.33-1.82l-.06-.06a2 2 0 0 1 0-2.83 2 2 0 0 1 2.83 0l.06.06a1.65 1.65 0 0 0 1.82.33H9a1.65 1.65 0 0 0 1-1.51V3a2 2 0 0 1 2-2 2 2 0 0 1 2 2v.09a1.65 1.65 0 0 0 1 1.51 1.65 1.65 0 0 0 1.82-.33l.06-.06a2 2 0 0 1 2.83 0 2 2 0 0 1 0 2.83l-.06.06a1.65 1.65 0 0 0-.33 1.82V9a1.65 1.65 0 0 0 1.51 1H21a2 2 0 0 1 2 2 2 2 0 0 1-2 2h-.09a1.65 1.65 0 0 0-1.51 1z"/>
            </svg>
          </span>
          <span class="nav-label" v-if="!sidebarCollapsed">{{ item.label }}</span>
        </router-link>
      </nav>

      <div class="sidebar-footer" v-if="!sidebarCollapsed">
        <div class="status-indicator" :class="health?.status === 'ok' ? 'online' : 'offline'">
          <span class="status-dot"></span>
          <span>{{ health?.status === 'ok' ? '系统正常' : '连接中...' }}</span>
        </div>
        <div class="version-info" v-if="version">
          v{{ version.version }}
        </div>
      </div>
    </aside>

    <!-- 主内容区 -->
    <main class="main-content">
      <header class="top-header">
        <div class="header-left">
          <h1 class="page-title">{{ currentTitle }}</h1>
          <div class="breadcrumb" v-if="route.params.id || route.path.endsWith('/new') || route.path.startsWith('/plugins/schema')">
            <template v-if="route.path.startsWith('/south')">
              <router-link to="/south">南向设备</router-link>
              <span class="separator">/</span>
              <span>{{ route.path.endsWith('/new') ? '添加设备' : '详情' }}</span>
            </template>
            <template v-else-if="route.path.startsWith('/north')">
              <router-link to="/north">北向应用</router-link>
              <span class="separator">/</span>
              <span>{{ route.path.endsWith('/new') ? '添加应用' : '详情' }}</span>
            </template>
            <template v-else-if="route.path.startsWith('/plugins/schema')">
              <router-link to="/plugins">插件管理</router-link>
              <span class="separator">/</span>
              <span>Schema</span>
            </template>
          </div>
        </div>
        <div class="header-right header-actions">
          <el-select v-model="locale" size="small" style="width: 90px" class="mr-1">
            <el-option label="中文" value="zh" />
            <el-option label="English" value="en" />
          </el-select>
          <el-dropdown trigger="click" class="user-dropdown">
            <span class="user-trigger">
              <el-avatar :size="28" class="user-avatar">{{ userInitial }}</el-avatar>
            </span>
            <template #dropdown>
              <el-dropdown-menu>
                <el-dropdown-item>当前用户</el-dropdown-item>
              </el-dropdown-menu>
            </template>
          </el-dropdown>
          <div class="stats-bar" v-if="health && !route.params.id && !route.path.endsWith('/new') && !route.path.startsWith('/plugins/schema')">
            <div class="stat-item">
              <span class="stat-value">{{ health.nodes_count || 0 }}</span>
              <span class="stat-label">节点总数</span>
            </div>
            <div class="stat-item">
              <span class="stat-value running">{{ health.nodes_running || 0 }}</span>
              <span class="stat-label">运行中</span>
            </div>
            <div class="stat-item">
              <span class="stat-value south">{{ health.plugins_south || 0 }}</span>
              <span class="stat-label">南向插件</span>
            </div>
            <div class="stat-item">
              <span class="stat-value north">{{ health.plugins_north || 0 }}</span>
              <span class="stat-label">北向插件</span>
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
</template>
