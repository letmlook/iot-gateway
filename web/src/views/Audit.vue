<script setup>
//! 审计日志页（Admin-only）：GET /api/audit 只读分页查询。
//! 后端只记请求元数据（方法/路径/状态码/用户/租户/request_id），不含请求体；
//! 本页纯展示，无任何写操作。非 Admin 访问返回 403，走错误提示。
import { ref, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { getErrorMessage } from '../i18n'
import { api } from '../api.js'
import PageHeader from '../components/PageHeader.vue'

const { t } = useI18n()

const loading = ref(true)
const error = ref('')
const rows = ref([])
const total = ref(0)
const page = ref(1)
const pageSize = ref(20)
const pageSizes = [20, 50, 100]

// AuditDto.ts 为 UNIX 秒（dto.rs），前端 ×1000 转 Date
function formatTs(ts) {
  if (!ts) return '-'
  try {
    const date = new Date(ts * 1000)
    if (isNaN(date.getTime())) return String(ts)
    const p = (n) => String(n).padStart(2, '0')
    return `${date.getFullYear()}-${p(date.getMonth() + 1)}-${p(date.getDate())} ${p(date.getHours())}:${p(date.getMinutes())}:${p(date.getSeconds())}`
  } catch {
    return String(ts)
  }
}

// 方法着色：写操作（POST/PUT/DELETE）与读（GET）区分开，扫一眼能看出动了什么
function methodTagType(method) {
  return { GET: 'info', POST: 'success', PUT: 'warning', DELETE: 'danger' }[method] || 'info'
}

// 状态码分档：4xx 警示、5xx 危险、其余正常
function statusTagType(status) {
  if (status >= 500) return 'danger'
  if (status >= 400) return 'warning'
  return 'success'
}

async function load() {
  loading.value = true
  error.value = ''
  try {
    const data = await api.audit({ page: page.value, page_size: pageSize.value })
    rows.value = data?.items ?? []
    total.value = data?.total ?? 0
  } catch (e) {
    // 403（非 Admin）时后端返回 forbidden 错误码，走 errors.forbidden 文案
    error.value = getErrorMessage(t, e)
    rows.value = []
    total.value = 0
  } finally {
    loading.value = false
  }
}

function changePage(p) {
  page.value = p
  load()
}

function changeSize(s) {
  pageSize.value = s
  page.value = 1
  load()
}

onMounted(() => {
  load()
})
</script>

<template>
  <div class="page-container audit-page">
    <PageHeader :title="t('audit.title')" :subtitle="t('audit.desc')" />

    <el-alert v-if="error" type="error" :title="error" show-icon class="mb-2" />

    <div v-loading="loading" class="audit-content">
      <el-table :data="rows" stripe>
        <el-table-column :label="t('audit.time')" min-width="160">
          <template #default="{ row }">{{ formatTs(row.ts) }}</template>
        </el-table-column>
        <el-table-column :label="t('audit.user')" min-width="100">
          <template #default="{ row }">{{ row.username || '-' }}</template>
        </el-table-column>
        <el-table-column :label="t('audit.tenant')" min-width="100">
          <template #default="{ row }">
            <el-tag size="small" type="info">{{ row.tenant || '-' }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="t('audit.method')" width="90">
          <template #default="{ row }">
            <el-tag size="small" :type="methodTagType(row.method)">{{ row.method }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column prop="path" :label="t('audit.path')" min-width="220" show-overflow-tooltip />
        <el-table-column :label="t('audit.status')" width="90">
          <template #default="{ row }">
            <el-tag size="small" :type="statusTagType(row.status)">{{ row.status }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column :label="t('audit.requestId')" min-width="180" show-overflow-tooltip>
          <template #default="{ row }">{{ row.requestId || '-' }}</template>
        </el-table-column>
      </el-table>

      <el-empty v-if="!loading && rows.length === 0" :description="t('audit.noData')" />

      <div v-if="total > 0" class="audit-pagination">
        <el-pagination
          :current-page="page"
          :page-size="pageSize"
          :page-sizes="pageSizes"
          :total="total"
          layout="total, sizes, prev, pager, next"
          @current-change="changePage"
          @size-change="changeSize"
        />
      </div>
    </div>
  </div>
</template>

<style scoped>
.audit-pagination {
  display: flex;
  justify-content: flex-end;
  margin-top: 1rem;
}
</style>
