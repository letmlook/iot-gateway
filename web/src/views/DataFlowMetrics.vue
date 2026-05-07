<script setup>
import { ref, onMounted, onUnmounted } from 'vue'
import { useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { Refresh, DataLine, InfoFilled } from '@element-plus/icons-vue'
import { api } from '../api.js'
import PageHeader from '../components/PageHeader.vue'

const router = useRouter()
const { t } = useI18n()
const loading = ref(false)
const error = ref('')
const data = ref(null)
const autoRefresh = ref(false)
const refreshInterval = ref(5000)
let timer = null

async function load() {
  loading.value = true
  error.value = ''
  try {
    data.value = await api.dataFlow()
  } catch (e) {
    error.value = e?.message || String(e)
  } finally {
    loading.value = false
  }
}

function toggleAutoRefresh() {
  autoRefresh.value = !autoRefresh.value
  if (autoRefresh.value) {
    timer = setInterval(load, refreshInterval.value)
  } else {
    clearInterval(timer)
    timer = null
  }
}

function goMonitor() {
  router.push('/monitor')
}

onMounted(load)
onUnmounted(() => { if (timer) clearInterval(timer) })
</script>

<template>
  <div class="data-flow-page">
    <PageHeader :title="t('dataFlow.title')" :subtitle="t('dataFlow.desc')">
      <template #actions>
        <el-button link type="primary" @click="goMonitor">
          {{ t('dataFlow.goToMonitor') }}
        </el-button>
        <el-select v-model="refreshInterval" class="interval-select" :disabled="autoRefresh" style="width: 100px">
          <el-option :value="3000" :label="t('dataFlow.intervalSeconds', { n: 3 })" />
          <el-option :value="5000" :label="t('dataFlow.intervalSeconds', { n: 5 })" />
          <el-option :value="10000" :label="t('dataFlow.intervalSeconds', { n: 10 })" />
        </el-select>
        <el-button
          :type="autoRefresh ? 'warning' : 'primary'"
          :icon="Refresh"
          :loading="loading"
          @click="toggleAutoRefresh"
        >
          {{ autoRefresh ? t('dataFlow.stopRefresh') : t('dataFlow.autoRefresh') }}
        </el-button>
        <el-button :icon="Refresh" :loading="loading" @click="load">
          {{ t('common.refresh') }}
        </el-button>
      </template>
    </PageHeader>

    <el-alert v-if="error" type="error" :title="error" closable show-icon @close="error = ''" class="mb-4" />

    <el-skeleton v-if="loading && !data" :rows="8" animated />

    <template v-else-if="data">
      <!-- 流程说明 -->
      <div class="flow-desc card">
        <div class="card-header">
          <el-icon><DataLine /></el-icon>
          <span>{{ t('dataFlow.flowPath') }}</span>
        </div>
        <p class="flow-text">{{ data.flow }}</p>
      </div>

      <!-- 总览指标 -->
      <div class="metrics-grid">
        <div class="metric-card">
          <span class="metric-label">{{ t('dataFlow.southPublished') }}</span>
          <span class="metric-value">{{ data.metrics?.south_published ?? 0 }}</span>
        </div>
        <div class="metric-card">
          <span class="metric-label">{{ t('dataFlow.busNoSubscribers') }}</span>
          <span class="metric-value" :class="{ warn: (data.metrics?.bus_no_subscribers ?? 0) > 0 }">
            {{ data.metrics?.bus_no_subscribers ?? 0 }}
          </span>
        </div>
        <div class="metric-card">
          <span class="metric-label">{{ t('dataFlow.northReceived') }}</span>
          <span class="metric-value">{{ data.metrics?.north_received ?? 0 }}</span>
        </div>
        <div class="metric-card">
          <span class="metric-label">{{ t('dataFlow.northFiltered') }}</span>
          <span class="metric-value">{{ data.metrics?.north_filtered ?? 0 }}</span>
        </div>
        <div class="metric-card">
          <span class="metric-label">{{ t('dataFlow.northForwarded') }}</span>
          <span class="metric-value success">{{ data.metrics?.north_forwarded ?? 0 }}</span>
        </div>
        <div class="metric-card">
          <span class="metric-label">{{ t('dataFlow.northOnGroupDataOk') }}</span>
          <span class="metric-value success">{{ data.metrics?.north_on_group_data_ok ?? 0 }}</span>
        </div>
        <div class="metric-card">
          <span class="metric-label">{{ t('dataFlow.northOnGroupDataErr') }}</span>
          <span class="metric-value" :class="{ warn: (data.metrics?.north_on_group_data_err ?? 0) > 0 }">
            {{ data.metrics?.north_on_group_data_err ?? 0 }}
          </span>
        </div>
        <div class="metric-card">
          <span class="metric-label">{{ t('dataFlow.northLagged') }}</span>
          <span class="metric-value" :class="{ warn: (data.metrics?.north_lagged ?? 0) > 0 }">
            {{ data.metrics?.north_lagged ?? 0 }}
          </span>
        </div>
      </div>

      <!-- 排查提示 -->
      <div v-if="data.troubleshoot && Object.keys(data.troubleshoot).length" class="troubleshoot card">
        <div class="card-header">
          <el-icon><InfoFilled /></el-icon>
          <span>{{ t('dataFlow.troubleshootTips') }}</span>
        </div>
        <ul class="troubleshoot-list">
          <li v-for="(tip, key) in data.troubleshoot" :key="key">
            <strong>{{ key }}</strong>: {{ tip }}
          </li>
        </ul>
      </div>

      <!-- 点位级：南向发布 -->
      <div class="section card">
        <div class="card-header">
          <span>{{ t('dataFlow.publishedPerTag') }}</span>
          <el-tag size="small">{{ (data.per_tag?.published || []).length }} {{ t('dataFlow.tags') }}</el-tag>
        </div>
        <el-table
          v-if="(data.per_tag?.published || []).length"
          :data="data.per_tag.published"
          size="small"
          stripe
          max-height="280"
        >
          <el-table-column prop="south_node_name" :label="t('dataFlow.southNode')" min-width="120" />
          <el-table-column prop="group_name" :label="t('dataFlow.group')" min-width="100" />
          <el-table-column prop="tag_name" :label="t('dataFlow.tag')" min-width="120" />
          <el-table-column prop="count" :label="t('dataFlow.count')" width="100" align="right">
            <template #default="{ row }">
              <span class="count-cell">{{ row.count }}</span>
            </template>
          </el-table-column>
        </el-table>
        <el-empty v-else :description="t('dataFlow.noPerTagData')" :image-size="60" />
      </div>

      <!-- 点位级：北向转发 -->
      <div class="section card">
        <div class="card-header">
          <span>{{ t('dataFlow.forwardedPerTag') }}</span>
          <el-tag size="small">{{ (data.per_tag?.forwarded || []).length }} {{ t('dataFlow.tags') }}</el-tag>
        </div>
        <el-table
          v-if="(data.per_tag?.forwarded || []).length"
          :data="data.per_tag.forwarded"
          size="small"
          stripe
          max-height="280"
        >
          <el-table-column prop="north_node_name" :label="t('dataFlow.northNode')" min-width="120" />
          <el-table-column prop="south_node_name" :label="t('dataFlow.southNode')" min-width="120" />
          <el-table-column prop="group_name" :label="t('dataFlow.group')" min-width="100" />
          <el-table-column prop="tag_name" :label="t('dataFlow.tag')" min-width="120" />
          <el-table-column prop="count" :label="t('dataFlow.count')" width="100" align="right">
            <template #default="{ row }">
              <span class="count-cell success">{{ row.count }}</span>
            </template>
          </el-table-column>
        </el-table>
        <el-empty v-else :description="t('dataFlow.noPerTagData')" :image-size="60" />
      </div>
    </template>

    <el-empty v-else-if="!loading" :description="t('dataFlow.loadFailed')" />
  </div>
</template>

<style scoped>
.data-flow-page {
  max-width: 1200px;
}

.mb-4 { margin-bottom: 1.5rem; }

.interval-select {
  width: 100px;
}

.card {
  background: var(--bg-surface);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-lg);
  padding: 1rem 1.25rem;
  margin-bottom: 1rem;
}

.card-header {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  font-weight: 600;
  font-size: 0.95rem;
  color: var(--text-primary);
  margin-bottom: 0.75rem;
}

.card-header .el-icon {
  color: var(--accent);
}

.flow-text {
  font-family: var(--font-mono);
  font-size: 0.85rem;
  color: var(--text-secondary);
  margin: 0;
}

.metrics-grid {
  display: grid;
  grid-template-columns: repeat(4, 1fr);
  gap: 0.75rem;
  margin-bottom: 1.5rem;
}

@media (max-width: 900px) {
  .metrics-grid { grid-template-columns: repeat(2, 1fr); }
}

@media (max-width: 500px) {
  .metrics-grid { grid-template-columns: 1fr; }
}

.metric-card {
  background: var(--bg-surface);
  border: 1px solid var(--border-subtle);
  border-radius: var(--radius-md);
  padding: 1rem;
  display: flex;
  flex-direction: column;
  gap: 0.35rem;
}

.metric-label {
  font-size: 0.75rem;
  color: var(--text-muted);
  text-transform: uppercase;
  letter-spacing: 0.03em;
}

.metric-value {
  font-size: 1.5rem;
  font-weight: 700;
  font-family: var(--font-mono);
  color: var(--text-primary);
}

.metric-value.success {
  color: var(--success);
}

.metric-value.warn {
  color: var(--danger);
}

.troubleshoot-list {
  margin: 0;
  padding-left: 1.25rem;
  font-size: 0.85rem;
  color: var(--text-secondary);
  line-height: 1.7;
}

.troubleshoot-list li {
  margin-bottom: 0.35rem;
}

.troubleshoot-list strong {
  color: var(--text-primary);
}

.section {
  margin-bottom: 1rem;
}

.count-cell {
  font-family: var(--font-mono);
  font-weight: 600;
  color: var(--text-primary);
}

.count-cell.success {
  color: var(--success);
}
</style>
