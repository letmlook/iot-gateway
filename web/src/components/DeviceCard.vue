<script setup>
import { computed } from 'vue'
import { useI18n } from 'vue-i18n'
import StatusIndicator from './StatusIndicator.vue'
import { Edit, Delete, DataLine, MoreFilled } from '@element-plus/icons-vue'

const props = defineProps({
  device: { type: Object, required: true },
  type: { type: String, default: 'south' } // south | north
})

const emit = defineEmits(['click', 'toggle', 'edit', 'monitor', 'delete', 'copy'])

const { t } = useI18n()

const stateClass = computed(() => props.device.state || 'stopped')
const isRunning = computed(() => props.device.state === 'running')

function formatTime(timestamp) {
  if (!timestamp) return '-'
  const date = new Date(timestamp)
  return date.toLocaleString()
}
</script>

<template>
  <div class="device-card" :class="[stateClass, type]" @click="$emit('click')">
    <!-- 顶部状态栏 -->
    <div class="card-status-bar">
      <StatusIndicator :status="device.state" size="small" />
      <div class="card-actions" @click.stop>
        <el-switch 
          :model-value="isRunning" 
          size="small"
          @change="$emit('toggle')"
        />
        <el-dropdown trigger="click" @command="(cmd) => $emit(cmd)">
          <button class="more-btn">
            <el-icon><MoreFilled /></el-icon>
          </button>
          <template #dropdown>
            <el-dropdown-menu>
              <el-dropdown-item command="edit">
                <el-icon class="menu-icon"><Edit /></el-icon>
                {{ t('south.editDevice') }}
              </el-dropdown-item>
              <el-dropdown-item command="monitor">
                <el-icon class="menu-icon"><DataLine /></el-icon>
                {{ t('south.dataMonitor') }}
              </el-dropdown-item>
              <el-dropdown-item command="copy">
                {{ t('common.copy') }}
              </el-dropdown-item>
              <el-dropdown-item command="delete" divided>
                <span class="text-danger">
                  <el-icon class="menu-icon"><Delete /></el-icon>
                  {{ t('common.delete') }}
                </span>
              </el-dropdown-item>
            </el-dropdown-menu>
          </template>
        </el-dropdown>
      </div>
    </div>

    <!-- 主内容 -->
    <div class="card-body">
      <h3 class="device-name">{{ device.name }}</h3>
      <div class="device-meta">
        <span class="device-plugin">{{ device.plugin_name }}</span>
        <span v-if="type === 'north'" class="device-type-badge north">{{ t('common.northApp') }}</span>
        <span v-else class="device-type-badge south">{{ t('common.southDevice') }}</span>
      </div>
    </div>

    <!-- 指标区域 -->
    <div class="card-metrics">
      <div class="metric">
        <span class="metric-value">{{ device.tags_count || 0 }}</span>
        <span class="metric-label">{{ t('common.tagsCount') }}</span>
      </div>
      <div class="metric">
        <span class="metric-value" :class="{ 'value-error': device.error_count > 0 }">
          {{ device.error_count || 0 }}
        </span>
        <span class="metric-label">{{ t('common.errors') }}</span>
      </div>
      <div class="metric">
        <span class="metric-value">{{ device.groups_count || 0 }}</span>
        <span class="metric-label">{{ t('common.groups') }}</span>
      </div>
    </div>

  </div>
</template>

<style scoped>
.device-card {
  background: var(--bg-surface);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-lg);
  cursor: pointer;
  transition: all var(--transition-normal);
  overflow: hidden;
  display: flex;
  flex-direction: column;
}

.device-card:hover {
  border-color: var(--border-default);
  box-shadow: var(--shadow-md);
  transform: translateY(-2px);
}

/* 状态边框高亮 - 顶部 */
.device-card.running {
  border-top: 3px solid var(--success);
}

.device-card.stopped {
  border-top: 3px solid var(--info);
}

.device-card.error {
  border-top: 3px solid var(--danger);
}

/* 北向应用特殊样式 */
.device-card.north.running {
  border-top-color: var(--accent-purple);
}

/* 状态栏 */
.card-status-bar {
  display: flex;
  justify-content: space-between;
  align-items: center;
  padding: 0.65rem 0.85rem;
  background: var(--bg-elevated);
}

.card-actions {
  display: flex;
  align-items: center;
  gap: 0.5rem;
}

.more-btn {
  width: 28px;
  height: 28px;
  display: flex;
  align-items: center;
  justify-content: center;
  background: transparent;
  border: none;
  border-radius: var(--radius-sm);
  cursor: pointer;
  color: var(--text-muted);
  transition: all var(--transition-fast);
}

.more-btn:hover {
  background: var(--bg-inset);
  color: var(--text-primary);
}

.menu-icon {
  margin-right: 0.35rem;
}

/* 主体内容 */
.card-body {
  padding: 0.85rem 1rem;
  flex: 1;
}

.device-name {
  font-size: 1rem;
  font-weight: 600;
  color: var(--text-primary);
  margin: 0 0 0.4rem;
  white-space: nowrap;
  overflow: hidden;
  text-overflow: ellipsis;
}

.device-meta {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  flex-wrap: wrap;
}

.device-plugin {
  font-size: 0.8rem;
  color: var(--text-muted);
  font-family: var(--font-mono);
  background: var(--bg-inset);
  padding: 0.15rem 0.5rem;
  border-radius: var(--radius-xs);
}

.device-type-badge {
  font-size: 0.7rem;
  padding: 0.1rem 0.4rem;
  border-radius: 100px;
  font-weight: 500;
}

.device-type-badge.south {
  background: rgba(56, 161, 105, 0.1);
  color: var(--success);
}

.device-type-badge.north {
  background: rgba(128, 90, 213, 0.1);
  color: var(--accent-purple);
}

/* 指标区域 */
.card-metrics {
  display: grid;
  grid-template-columns: repeat(3, 1fr);
  gap: 0.5rem;
  padding: 0.65rem 0.85rem;
  background: var(--bg-elevated);
  border-top: 1px solid var(--border-subtle);
}

.metric {
  text-align: center;
}

.metric-value {
  display: block;
  font-size: 1rem;
  font-weight: 600;
  color: var(--text-primary);
  font-family: var(--font-mono);
}

.metric-value.value-error {
  color: var(--danger);
}

.metric-label {
  font-size: 0.65rem;
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.04em;
}

.text-danger {
  color: var(--danger);
  display: flex;
  align-items: center;
}
</style>
