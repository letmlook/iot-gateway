<script setup>
import { ref } from 'vue'
import { useRoute, useRouter } from 'vue-router'
import ModeToggle from './ModeToggle.vue'

const props = defineProps({
  mainNavItems: { type: Array, required: true },
  systemNavItems: { type: Array, required: true },
  runningCount: { type: Number, default: 0 },
  totalCount: { type: Number, default: 0 },
  userName: { type: String, default: 'admin' },
})

const emit = defineEmits(['toggle-collapse'])

const route = useRoute()
const router = useRouter()
const collapsed = ref(false)

const userInitial = props.userName && props.userName.length
  ? props.userName.charAt(0).toUpperCase()
  : 'U'

function isActive(path) {
  if (path === '/dashboard') {
    return route.path === '/dashboard' || route.path === '/'
  }
  // /monitor 的子页（/monitor/flow、/monitor/history、/monitor/alarms）各自高亮菜单项，不再连带命中父项
  if (path === '/monitor') {
    return route.path === '/monitor' || (route.path.startsWith('/monitor/') && !['/monitor/flow', '/monitor/history', '/monitor/alarms'].some(p => route.path.startsWith(p)))
  }
  return route.path.startsWith(path)
}

function navigate(path) {
  router.push(path)
}

function toggleCollapsed() {
  collapsed.value = !collapsed.value
  emit('toggle-collapse', collapsed.value)
}
</script>

<template>
  <aside class="app-sidebar" :class="{ collapsed }">
    <!-- Brand -->
    <div class="sidebar-brand">
      <svg class="brand-icon" viewBox="0 0 32 32" fill="none" xmlns="http://www.w3.org/2000/svg">
        <rect x="2" y="2" width="28" height="28" rx="7" fill="var(--primary)" />
        <path d="M10 8v16l12-8-12-8z" fill="var(--accent)" />
        <circle cx="22" cy="16" r="3" fill="var(--accent)" />
      </svg>
      <div>
        <div class="brand-text">IoT Gateway</div>
        <div class="brand-version">Edge Controller v2</div>
      </div>
      <button class="sidebar-collapse-btn" @click="toggleCollapsed" :title="collapsed ? 'Expand' : 'Collapse'">
        <svg viewBox="0 0 20 20" fill="currentColor" width="14" height="14">
          <path v-if="collapsed" fill-rule="evenodd" d="M7.293 14.707a1 1 0 010-1.414L10.586 10 7.293 6.707a1 1 0 011.414-1.414l4 4a1 1 0 010 1.414l-4 4a1 1 0 01-1.414 0z" clip-rule="evenodd"/>
          <path v-else fill-rule="evenodd" d="M12.707 5.293a1 1 0 010 1.414L9.414 10l3.293 3.293a1 1 0 01-1.414 1.414l-4-4a1 1 0 010-1.414l4-4a1 1 0 011.414 0z" clip-rule="evenodd"/>
        </svg>
      </button>
    </div>

    <!-- Main Navigation -->
    <div class="sidebar-section-label">Main Navigation</div>
    <nav class="sidebar-nav">
      <div
        v-for="item in mainNavItems"
        :key="item.path"
        class="sidebar-nav-item"
        :class="{ active: isActive(item.path) }"
        @click="navigate(item.path)"
      >
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
          <svg v-else-if="item.icon === 'history'" viewBox="0 0 20 20" fill="currentColor">
            <path fill-rule="evenodd" d="M10 18a8 8 0 100-16 8 8 0 000 16zm.75-12.25a.75.75 0 00-1.5 0v4c0 .27.14.51.37.65l3 1.75a.75.75 0 10.76-1.3l-2.63-1.53v-3.57z" clip-rule="evenodd"/>
          </svg>
          <svg v-else-if="item.icon === 'alarms'" viewBox="0 0 20 20" fill="currentColor">
            <path d="M10 2a6 6 0 00-6 6v3.586l-1.707 1.707A1 1 0 003 15h14a1 1 0 00.707-1.707L16 11.586V8a6 6 0 00-6-6zM7.5 16a2.5 2.5 0 005 0h-5z"/>
          </svg>
          <svg v-else-if="item.icon === 'flows'" viewBox="0 0 20 20" fill="currentColor">
            <path d="M3 3.5A1.5 1.5 0 014.5 2h3A1.5 1.5 0 019 3.5v1A1.5 1.5 0 017.5 6H7v2.5h6V8a1.5 1.5 0 011.5-1.5h3A1.5 1.5 0 0119 8v1a1.5 1.5 0 01-1.5 1.5h-3A1.5 1.5 0 0113 9v-.5H7v2.955a5.002 5.002 0 014 4.795V17a1 1 0 11-2 0 3 3 0 10-6 0 1 1 0 11-2 0v-1a5.002 5.002 0 014-4.795V6H4.5A1.5 1.5 0 013 4.5v-1z" transform="translate(-1 0)"/>
          </svg>
          <svg v-else-if="item.icon === 'plugins'" viewBox="0 0 20 20" fill="currentColor">
            <path d="M11 17a1 1 0 001.447.894l4-2A1 1 0 0017 15V9.236a1 1 0 00-1.447-.894l-4 2a1 1 0 00-.553.894V17zM15.211 6.276a1 1 0 000-1.788l-4.764-2.382a1 1 0 00-.894 0L4.789 4.488a1 1 0 000 1.788l4.764 2.382a1 1 0 00.894 0l4.764-2.382zM4.447 8.342A1 1 0 003 9.236V15a1 1 0 00.553.894l4 2A1 1 0 009 17v-5.764a1 1 0 00-.553-.894l-4-2z"/>
          </svg>
        </span>
        <span class="nav-label">{{ item.label }}</span>
        <span v-if="item.badge && item.badge > 0" class="nav-badge">{{ item.badge }}</span>
      </div>
    </nav>

    <!-- System Navigation -->
    <div class="sidebar-section-label">System</div>
    <nav class="sidebar-nav">
      <div
        v-for="item in systemNavItems"
        :key="item.path"
        class="sidebar-nav-item"
        :class="{ active: isActive(item.path) }"
        @click="navigate(item.path)"
      >
        <span class="nav-label">{{ item.label }}</span>
      </div>
    </nav>

    <!-- Footer: User + Theme Toggle -->
    <div class="sidebar-footer">
      <div style="display:flex;align-items:center;gap:8px;padding:4px 8px">
        <div style="width:26px;height:26px;border-radius:6px;background:var(--accent);color:var(--text-inverse);display:flex;align-items:center;justify-content:center;font-size:11px;font-weight:600;flex-shrink:0">{{ userInitial }}</div>
        <div v-show="!collapsed" style="font-size:12px;color:var(--text-primary);overflow:hidden;text-overflow:ellipsis;white-space:nowrap">{{ userName }}</div>
        <div v-show="!collapsed" style="flex:1"></div>
        <ModeToggle />
      </div>
    </div>
  </aside>
</template>
