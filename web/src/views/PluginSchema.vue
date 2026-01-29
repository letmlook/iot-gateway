<script setup>
import { ref, computed, onMounted, watch } from 'vue'
import { useRouter, useRoute } from 'vue-router'
import { ElMessage } from 'element-plus'
import { ArrowLeft } from '@element-plus/icons-vue'
import { api } from '../api.js'

const router = useRouter()
const route = useRoute()

const kind = computed(() => route.params.kind || 'south')
const pluginName = computed(() => decodeURIComponent(route.params.name || ''))
const schemaType = computed(() => route.query.type || 'config') // config | tag

const schemaData = ref(null)
const loading = ref(true)
const error = ref('')

async function loadSchema() {
  loading.value = true
  error.value = ''
  schemaData.value = null
  try {
    if (schemaType.value === 'tag') {
      schemaData.value = await api.pluginSouthTagSchema(pluginName.value)
    } else if (kind.value === 'south') {
      schemaData.value = await api.pluginSouthSchema(pluginName.value)
    } else {
      schemaData.value = await api.pluginNorthSchema(pluginName.value)
    }
  } catch (e) {
    schemaData.value = { _error: e.message }
  } finally {
    loading.value = false
  }
}

async function copyJson() {
  if (!schemaData.value || schemaData.value._error) return
  try {
    await navigator.clipboard.writeText(JSON.stringify(schemaData.value, null, 2))
    ElMessage.success('已复制到剪贴板')
  } catch (e) {
    ElMessage.error('复制失败: ' + e.message)
  }
}

function goBack() {
  router.push('/plugins')
}

onMounted(loadSchema)
watch([() => route.params.kind, () => route.params.name, () => route.query.type], loadSchema)
</script>

<template>
  <div class="page-container schema-page">
    <div class="page-header">
      <el-button :icon="ArrowLeft" @click="goBack">返回插件</el-button>
      <h2 class="page-title">{{ pluginName }} - {{ schemaType === 'tag' ? '标签 Schema' : '配置 Schema' }}</h2>
      <el-button type="primary" :disabled="!schemaData || schemaData._error" @click="copyJson">复制 JSON</el-button>
    </div>
    <el-card class="schema-card">
      <el-skeleton v-if="loading" :rows="6" animated />
      <el-alert v-else-if="schemaData?._error" type="warning" :title="schemaData._error" show-icon />
      <template v-else-if="schemaData && schemaType === 'config'">
        <div v-if="schemaData.params?.length" class="schema-section">
          <h4>配置参数</h4>
          <el-table :data="schemaData.params" size="small" stripe>
            <el-table-column prop="name" label="参数名" width="120">
              <template #default="{ row }"><code>{{ row.name }}</code></template>
            </el-table-column>
            <el-table-column prop="ty" label="类型" width="80">
              <template #default="{ row }">{{ row.ty || row.type || '-' }}</template>
            </el-table-column>
            <el-table-column label="必填" width="70">
              <template #default="{ row }">{{ row.attribute === 'required' ? '是' : '否' }}</template>
            </el-table-column>
            <el-table-column label="默认值" width="100">
              <template #default="{ row }"><span class="font-mono">{{ row.default != null ? JSON.stringify(row.default) : '-' }}</span></template>
            </el-table-column>
            <el-table-column prop="description" label="说明" min-width="160" show-overflow-tooltip />
          </el-table>
        </div>
        <div v-else class="schema-empty">无配置参数</div>
        <div v-if="schemaData.tag_regex?.length" class="schema-section">
          <h4>标签地址正则</h4>
          <el-table :data="schemaData.tag_regex" size="small" stripe>
            <el-table-column prop="data_type" label="数据类型" width="120">
              <template #default="{ row }"><code>{{ row.data_type }}</code></template>
            </el-table-column>
            <el-table-column prop="regex" label="正则" show-overflow-tooltip>
              <template #default="{ row }"><span class="font-mono">{{ row.regex }}</span></template>
            </el-table-column>
          </el-table>
        </div>
      </template>
      <template v-else-if="schemaData && schemaType === 'tag'">
        <div v-if="schemaData.data_types?.length" class="schema-section">
          <h4>支持的数据类型</h4>
          <div class="schema-tags">
            <el-tag v-for="dt in schemaData.data_types" :key="dt" size="small" class="mr-1">{{ dt }}</el-tag>
          </div>
        </div>
        <div v-if="schemaData.address_format" class="schema-section">
          <h4>地址格式</h4>
          <p class="schema-desc font-mono">{{ schemaData.address_format }}</p>
        </div>
        <div v-if="(!schemaData.data_types?.length) && !schemaData.address_format" class="schema-empty">无标签 Schema 详情</div>
      </template>
    </el-card>
  </div>
</template>

<style scoped>
.schema-page { max-width: 800px; }
.page-header { display: flex; align-items: center; gap: 1rem; margin-bottom: 1rem; flex-wrap: wrap; }
.page-title { margin: 0; font-size: 1.25rem; }
.schema-card { padding: 1.5rem; }
.schema-section { margin-bottom: 1.5rem; }
.schema-section h4 { font-size: 0.95rem; margin-bottom: 0.5rem; color: var(--text-secondary); }
.schema-empty { color: var(--text-muted); font-size: 0.9rem; }
.schema-tags { display: flex; flex-wrap: wrap; gap: 0.35rem; }
.schema-desc { font-size: 0.9rem; color: var(--text-secondary); margin: 0; padding: 0.5rem 0.75rem; background: var(--el-fill-color-light); border-radius: 8px; }
.font-mono { font-family: var(--font-mono); }
.mr-1 { margin-right: 0.25rem; }
</style>
