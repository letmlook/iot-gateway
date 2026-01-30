import { createRouter, createWebHistory } from 'vue-router'
import { api } from './api.js'

const routes = [
  {
    path: '/',
    redirect: '/dashboard'
  },
  {
    path: '/login',
    name: 'Login',
    component: () => import('./views/Login.vue'),
    meta: { title: '登录', public: true }
  },
  {
    path: '/dashboard',
    name: 'Dashboard',
    component: () => import('./views/Dashboard.vue'),
    meta: { title: '概览' }
  },
  {
    path: '/south',
    name: 'SouthDevices',
    component: () => import('./views/SouthDevices.vue'),
    meta: { title: '南向设备' }
  },
  {
    path: '/south/new',
    name: 'SouthCreate',
    component: () => import('./views/CreateNode.vue'),
    meta: { title: '添加南向设备', kind: 'south' }
  },
  {
    path: '/south/:id',
    name: 'SouthDetail',
    component: () => import('./views/NodeDetail.vue'),
    meta: { title: '设备详情', kind: 'south' }
  },
  {
    path: '/south/:id/config',
    name: 'SouthConfig',
    component: () => import('./views/NodeConfig.vue'),
    meta: { title: '设备配置', kind: 'south' }
  },
  {
    path: '/north',
    name: 'NorthApps',
    component: () => import('./views/NorthApps.vue'),
    meta: { title: '北向应用' }
  },
  {
    path: '/north/new',
    name: 'NorthCreate',
    component: () => import('./views/CreateNode.vue'),
    meta: { title: '添加北向应用', kind: 'north' }
  },
  {
    path: '/north/:id',
    name: 'NorthDetail',
    component: () => import('./views/NodeDetail.vue'),
    meta: { title: '应用详情', kind: 'north' }
  },
  {
    path: '/north/:id/config',
    name: 'NorthConfig',
    component: () => import('./views/NodeConfig.vue'),
    meta: { title: '应用配置', kind: 'north' }
  },
  {
    path: '/monitor',
    name: 'DataMonitor',
    component: () => import('./views/DataMonitor.vue'),
    meta: { title: '数据监控' }
  },
  {
    path: '/plugins',
    name: 'Plugins',
    component: () => import('./views/Plugins.vue'),
    meta: { title: '插件管理' }
  },
  // 设置模块
  {
    path: '/settings/license',
    name: 'License',
    component: () => import('./views/License.vue'),
    meta: { title: '许可证', settingsGroup: true }
  },
  {
    path: '/settings/logs',
    name: 'Logs',
    component: () => import('./views/Logs.vue'),
    meta: { title: '日志', settingsGroup: true }
  },
  {
    path: '/settings/config',
    name: 'SystemConfig',
    component: () => import('./views/SystemConfig.vue'),
    meta: { title: '系统配置', settingsGroup: true }
  },
  {
    path: '/settings/info',
    name: 'SystemInfo',
    component: () => import('./views/SystemInfo.vue'),
    meta: { title: '系统信息', settingsGroup: true }
  },
  {
    path: '/settings/users',
    name: 'Users',
    component: () => import('./views/Users.vue'),
    meta: { title: '用户', settingsGroup: true }
  }
]

const router = createRouter({
  history: createWebHistory(),
  routes
})

router.beforeEach((to) => {
  let authenticated = false
  try {
    authenticated = api.isAuthenticated()
  } catch (_) {
    authenticated = false
  }
  if (to.meta.public) {
    if (authenticated && to.path === '/login') {
      return '/dashboard'
    }
    return true
  }
  if (!authenticated) {
    return { path: '/login', query: { redirect: to.fullPath } }
  }
  return true
})

router.onError((err) => {
  console.error('[router]', err)
})

export default router
