<script setup>
import { ref, onMounted, computed } from 'vue'
import { useRouter } from 'vue-router'
import { ElMessage } from 'element-plus'
import { Plus, Document, CollectionTag } from '@element-plus/icons-vue'
import { api } from '../api.js'

const router = useRouter()

const southPlugins = ref([])
const northPlugins = ref([])
const nodes = ref([])
const loading = ref(false)
const error = ref('')
const activeTab = ref('south')

const showSchemaModal = ref(false)
const schemaTitle = ref('')
const schemaKind = ref(null)
const schemaData = ref(null)
const schemaLoading = ref(false)

const plugins = computed(() => {
  return activeTab.value === 'south'
    ? southPlugins.value.map(([name, desc, ver]) => ({ name, description: desc, version: ver, kind: 'south' }))
    : northPlugins.value.map(([name, desc, ver]) => ({ name, description: desc, version: ver, kind: 'north' }))
})

function nodesUsingPlugin(pluginName, kind) {
  return nodes.value.filter(n => n.plugin_name === pluginName && n.kind === kind)
}

async function loadData() {
  loading.value = true
  error.value = ''
  try {
    const [sp, np, nd] = await Promise.all([
      api.pluginsSouth().catch(() => []),
      api.pluginsNorth().catch(() => []),
      api.nodes().catch(() => []),
    ])
    southPlugins.value = sp
    northPlugins.value = np
    nodes.value = nd
  } catch (e) {
    error.value = '加载插件列表失败: ' + e.message
  } finally {
    loading.value = false
  }
}

async function showConfigSchema(plugin) {
  schemaTitle.value = `${plugin.name} - 配置 Schema`
  schemaKind.value = 'config'
  schemaData.value = null
  showSchemaModal.value = true
  schemaLoading.value = true
  try {
    if (plugin.kind === 'south') {
      schemaData.value = await api.pluginSouthSchema(plugin.name)
    } else {
      schemaData.value = await api.pluginNorthSchema(plugin.name)
    }
  } catch (e) {
    schemaData.value = { _error: e.message }
  } finally {
    schemaLoading.value = false
  }
}

async function showTagSchema(plugin) {
  schemaTitle.value = `${plugin.name} - 标签 Schema`
  schemaKind.value = 'tag'
  schemaData.value = null
  showSchemaModal.value = true
  schemaLoading.value = true
  try {
    schemaData.value = await api.pluginSouthTagSchema(plugin.name)
  } catch (e) {
    schemaData.value = { _error: e.message }
  } finally {
    schemaLoading.value = false
  }
}

function closeSchemaModal() {
  showSchemaModal.value = false
  schemaData.value = null
}

async function copySchemaJson() {
  if (!schemaData.value || schemaData.value._error) return
  try {
    await navigator.clipboard.writeText(JSON.stringify(schemaData.value, null, 2))
    ElMessage.success('已复制到剪贴板')
  } catch (e) {
    ElMessage.error('复制失败: ' + e.message)
  }
}

function goCreateNode(plugin) {
  router.push(plugin.kind === 'south' ? '/south' : '/north')
}

function goToNode(node) {
  router.push(node.kind === 'south' ? `/south/${node.id}` : `/north/${node.id}`)
}

onMounted(loadData)
</script>

<template>
  <div class="page-container">
    <div class="page-header">
      <div class="header-info">
        <p class="header-desc">查看已加载的南向/北向插件、配置与标签 Schema，以及使用该插件的节点。</p>
      </div>
    </div>

    <el-alert v-if="error" type="error" :title="error" closable show-icon @close="error = ''" class="mb-2" />

    <el-skeleton v-if="loading" :rows="6" animated />

    <template v-else>
      <el-tabs v-model="activeTab" class="plugins-tabs">
        <el-tab-pane name="south">
          <template #label>
            <span class="tab-label"><span class="dot south" /> 南向插件 <el-tag size="small" type="info" class="ml-1">{{ southPlugins.length }}</el-tag></span>
          </template>
          <div class="plugins-grid">
            <el-card v-for="p in plugins" :key="p.name + p.kind" class="plugin-card" :class="p.kind" shadow="hover">
              <template #header>
                <div class="plugin-header">
                  <div class="plugin-meta">
                    <span class="plugin-name">{{ p.name }}</span>
                    <el-tag v-if="p.version" size="small" type="info">v{{ p.version }}</el-tag>
                    <el-tag :type="p.kind === 'south' ? 'success' : ''" size="small" effect="plain">
                      {{ p.kind === 'south' ? '南向' : '北向' }}
                    </el-tag>
                  </div>
                </div>
              </template>
              <p v-if="p.description" class="plugin-desc">{{ p.description }}</p>
              <div v-if="nodesUsingPlugin(p.name, p.kind).length" class="plugin-usage">
                <span class="usage-label">使用该插件的节点：</span>
                <div class="usage-nodes">
                  <el-button
                    v-for="n in nodesUsingPlugin(p.name, p.kind)"
                    :key="n.id"
                    type="primary"
                    link
                    size="small"
                    @click="goToNode(n)"
                  >
                    {{ n.name }}
                  </el-button>
                </div>
              </div>
              <template #footer>
                <div class="plugin-actions">
                  <el-button size="small" :icon="Document" @click="showConfigSchema(p)">配置 Schema</el-button>
                  <el-button v-if="p.kind === 'south'" size="small" :icon="CollectionTag" @click="showTagSchema(p)">标签 Schema</el-button>
                  <el-button type="primary" size="small" :icon="Plus" @click="goCreateNode(p)">
                    {{ p.kind === 'south' ? '创建设备' : '创建应用' }}
                  </el-button>
                </div>
              </template>
            </el-card>
            <el-empty v-if="!plugins.length" description="暂无南向插件" class="empty-block">
              <template #description>
                <p>插件从 plugins 目录加载 .so 动态库，请检查 GATEWAY_PLUGINS_DIR 配置。</p>
              </template>
            </el-empty>
          </div>
        </el-tab-pane>
        <el-tab-pane name="north">
          <template #label>
            <span class="tab-label"><span class="dot north" /> 北向插件 <el-tag size="small" type="info" class="ml-1">{{ northPlugins.length }}</el-tag></span>
          </template>
          <div class="plugins-grid">
            <el-card v-for="p in plugins" :key="p.name + p.kind" class="plugin-card" :class="p.kind" shadow="hover">
              <template #header>
                <div class="plugin-header">
                  <div class="plugin-meta">
                    <span class="plugin-name">{{ p.name }}</span>
                    <el-tag v-if="p.version" size="small" type="info">v{{ p.version }}</el-tag>
                    <el-tag :type="p.kind === 'south' ? 'success' : ''" size="small" effect="plain">
                      {{ p.kind === 'south' ? '南向' : '北向' }}
                    </el-tag>
                  </div>
                </div>
              </template>
              <p v-if="p.description" class="plugin-desc">{{ p.description }}</p>
              <div v-if="nodesUsingPlugin(p.name, p.kind).length" class="plugin-usage">
                <span class="usage-label">使用该插件的节点：</span>
                <div class="usage-nodes">
                  <el-button
                    v-for="n in nodesUsingPlugin(p.name, p.kind)"
                    :key="n.id"
                    type="primary"
                    link
                    size="small"
                    @click="goToNode(n)"
                  >
                    {{ n.name }}
                  </el-button>
                </div>
              </div>
              <template #footer>
                <div class="plugin-actions">
                  <el-button size="small" :icon="Document" @click="showConfigSchema(p)">配置 Schema</el-button>
                  <el-button v-if="p.kind === 'south'" size="small" :icon="CollectionTag" @click="showTagSchema(p)">标签 Schema</el-button>
                  <el-button type="primary" size="small" :icon="Plus" @click="goCreateNode(p)">
                    {{ p.kind === 'south' ? '创建设备' : '创建应用' }}
                  </el-button>
                </div>
              </template>
            </el-card>
            <el-empty v-if="!plugins.length" description="暂无北向插件" class="empty-block">
              <template #description>
                <p>插件从 plugins 目录加载 .so 动态库，请检查 GATEWAY_PLUGINS_DIR 配置。</p>
              </template>
            </el-empty>
          </div>
        </el-tab-pane>
      </el-tabs>
    </template>

    <el-dialog v-model="showSchemaModal" :title="schemaTitle" width="640px" destroy-on-close @closed="closeSchemaModal">
      <el-skeleton v-if="schemaLoading" :rows="4" animated />
      <el-alert v-else-if="schemaData?._error" type="warning" :title="schemaData._error" show-icon />
      <div v-else-if="schemaData && schemaKind === 'config'" class="schema-content">
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
            <el-table-column prop="description" label="说明" />
          </el-table>
        </div>
        <div v-else class="schema-empty">无配置参数</div>
        <div v-if="schemaData.tag_regex?.length" class="schema-section">
          <h4>标签地址正则</h4>
          <el-table :data="schemaData.tag_regex" size="small" stripe>
            <el-table-column prop="data_type" label="数据类型" width="120">
              <template #default="{ row }"><code>{{ row.data_type }}</code></template>
            </el-table-column>
            <el-table-column prop="regex" label="正则">
              <template #default="{ row }"><span class="font-mono">{{ row.regex }}</span></template>
            </el-table-column>
          </el-table>
        </div>
      </div>
      <div v-else-if="schemaData && schemaKind === 'tag'" class="schema-content">
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
      </div>
      <template #footer>
        <el-button @click="closeSchemaModal">关闭</el-button>
        <el-button v-if="schemaData && !schemaData._error" type="primary" @click="copySchemaJson">复制 JSON</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<style scoped>
.mb-2 { margin-bottom: 1rem; }
.ml-1 { margin-left: 0.25rem; }
.mr-1 { margin-right: 0.25rem; }
.tab-label { display: inline-flex; align-items: center; gap: 0.35rem; }
.dot { width: 8px; height: 8px; border-radius: 50%; display: inline-block; }
.dot.south { background: var(--el-color-success); }
.dot.north { background: var(--el-color-primary); }
.plugins-grid { display: grid; grid-template-columns: repeat(auto-fill, minmax(320px, 1fr)); gap: 1rem; margin-top: 1rem; }
.plugin-card.south { border-left: 3px solid var(--el-color-success); }
.plugin-card.north { border-left: 3px solid var(--el-color-primary); }
.plugin-header { display: flex; justify-content: space-between; }
.plugin-meta { display: flex; align-items: center; flex-wrap: wrap; gap: 0.35rem; }
.plugin-name { font-weight: 600; font-size: 1rem; }
.plugin-desc { font-size: 0.9rem; color: var(--text-secondary); margin: 0 0 0.75rem; line-height: 1.5; }
.plugin-usage { margin-bottom: 0.75rem; }
.usage-label { font-size: 0.8rem; color: var(--text-muted); display: block; margin-bottom: 0.35rem; }
.usage-nodes { display: flex; flex-wrap: wrap; gap: 0.25rem; }
.plugin-actions { display: flex; flex-wrap: wrap; gap: 0.5rem; }
.schema-content { max-height: 50vh; overflow-y: auto; }
.schema-section { margin-bottom: 1rem; }
.schema-section h4 { font-size: 0.95rem; margin-bottom: 0.5rem; color: var(--text-secondary); }
.schema-empty { color: var(--text-muted); font-size: 0.9rem; }
.schema-tags { display: flex; flex-wrap: wrap; gap: 0.35rem; }
.schema-desc { font-size: 0.9rem; color: var(--text-secondary); margin: 0; padding: 0.5rem 0.75rem; background: var(--el-fill-color-light); border-radius: 8px; }
.font-mono { font-family: var(--font-mono); }
.empty-block { grid-column: 1 / -1; padding: 3rem; }
</style>
