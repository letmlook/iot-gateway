import { createRouter, createWebHistory } from 'vue-router'

const routes = [
  {
    path: '/',
    redirect: '/south'
  },
  {
    path: '/south',
    name: 'SouthDevices',
    component: () => import('./views/SouthDevices.vue'),
    meta: { title: '南向设备' }
  },
  {
    path: '/south/:id',
    name: 'SouthDetail',
    component: () => import('./views/NodeDetail.vue'),
    meta: { title: '设备详情', kind: 'south' }
  },
  {
    path: '/north',
    name: 'NorthApps',
    component: () => import('./views/NorthApps.vue'),
    meta: { title: '北向应用' }
  },
  {
    path: '/north/:id',
    name: 'NorthDetail',
    component: () => import('./views/NodeDetail.vue'),
    meta: { title: '应用详情', kind: 'north' }
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
  {
    path: '/system',
    name: 'System',
    component: () => import('./views/System.vue'),
    meta: { title: '系统管理' }
  }
]

const router = createRouter({
  history: createWebHistory(),
  routes
})

export default router
