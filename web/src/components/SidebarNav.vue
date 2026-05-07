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
