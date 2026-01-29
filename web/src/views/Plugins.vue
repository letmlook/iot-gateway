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


const pluginSearch = ref('')

const plugins = computed(() => {
  const list = activeTab.value === 'south'
    ? southPlugins.value.map(([name, desc, ver]) => ({ name, description: desc, version: ver, kind: 'south' }))
    : northPlugins.value.map(([name, desc, ver]) => ({ name, description: desc, version: ver, kind: 'north' }))
  if (!pluginSearch.value.trim()) return list
  const k = pluginSearch.value.trim().toLowerCase()
  return list.filter(p => (p.name && p.name.toLowerCase().includes(k)) || (p.description && p.description.toLowerCase().includes(k)))
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

function goToConfigSchema(plugin) {
  router.push(`/plugins/schema/${plugin.kind}/${encodeURIComponent(plugin.name)}`)
}

function goToTagSchema(plugin) {
  router.push({ path: `/plugins/schema/${plugin.kind}/${encodeURIComponent(plugin.name)}`, query: { type: 'tag' } })
}

function goCreateNode(plugin) {
  router.push(plugin.kind === 'south' ? '/south/new' : '/north/new')
}

function goAddPlugin() {
  router.push(activeTab.value === 'south' ? '/south/new' : '/north/new')
}

function goToNode(node) {
  router.push(node.kind === 'south' ? `/south/${node.id}` : `/north/${node.id}`)
}

onMounted(loadData)
</script>

<template>
  <div class="page-container">
    <div class="page-header plugins-header">
      <h2 class="page-title">插件</h2>
      <div class="header-toolbar">
        <el-select v-model="activeTab" style="width: 120px" class="mr-1">
          <el-option label="南向插件" value="south" />
          <el-option label="北向插件" value="north" />
        </el-select>
        <el-input v-model="pluginSearch" placeholder="请输入搜索名称" clearable style="width: 200px" class="mr-1" />
        <el-button type="primary" :icon="Plus" @click="goAddPlugin">+ 添加插件</el-button>
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
          <!-- 表格视图（对标 Neuron） -->
          <el-table :data="plugins" size="default" stripe class="plugins-table">
            <el-table-column prop="name" label="名称" min-width="120" />
            <el-table-column label="插件类型" width="120">
              <template #default="{ row }">
                <el-tag size="small" :type="row.kind === 'south' ? 'success' : 'primary'">
                  {{ row.kind === 'south' ? '南向设备' : '北向应用' }}
                </el-tag>
              </template>
            </el-table-column>
            <el-table-column label="插件类别" width="100">
              <template #default>System</template>
            </el-table-column>
            <el-table-column prop="version" label="插件版本" width="100">
              <template #default="{ row }">v{{ row.version || '-' }}</template>
            </el-table-column>
            <el-table-column prop="description" label="描述" min-width="280" show-overflow-tooltip />
            <el-table-column label="操作" width="240" fixed="right">
              <template #default="{ row }">
                <el-button type="primary" link size="small" :icon="Document" @click="goToConfigSchema(row)">Schema</el-button>
                <el-button v-if="row.kind === 'south'" type="primary" link size="small" :icon="CollectionTag" @click="goToTagSchema(row)">标签</el-button>
                <el-button type="primary" link size="small" :icon="Plus" @click="goCreateNode(row)">
                  {{ row.kind === 'south' ? '创建设备' : '创建应用' }}
                </el-button>
              </template>
            </el-table-column>
          </el-table>
          <el-empty v-if="!plugins.length" description="暂无插件" class="empty-block" />
          <div class="plugins-grid" style="display: none">
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
                  <el-button size="small" :icon="Document" @click="goToConfigSchema(p)">配置 Schema</el-button>
                  <el-button v-if="p.kind === 'south'" size="small" :icon="CollectionTag" @click="goToTagSchema(p)">标签 Schema</el-button>
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
          <el-table :data="plugins" size="default" stripe class="plugins-table">
            <el-table-column prop="name" label="名称" min-width="120" />
            <el-table-column label="插件类型" width="120">
              <template #default="{ row }">
                <el-tag size="small" type="primary">北向应用</el-tag>
              </template>
            </el-table-column>
            <el-table-column label="插件类别" width="100">
              <template #default>System</template>
            </el-table-column>
            <el-table-column prop="version" label="插件版本" width="100">
              <template #default="{ row }">v{{ row.version || '-' }}</template>
            </el-table-column>
            <el-table-column prop="description" label="描述" min-width="280" show-overflow-tooltip />
            <el-table-column label="操作" width="200" fixed="right">
              <template #default="{ row }">
                <el-button type="primary" link size="small" :icon="Document" @click="goToConfigSchema(row)">Schema</el-button>
                <el-button type="primary" link size="small" :icon="Plus" @click="goCreateNode(row)">创建应用</el-button>
              </template>
            </el-table-column>
          </el-table>
          <el-empty v-if="!plugins.length" description="暂无北向插件" class="empty-block" />
          <div class="plugins-grid" style="display: none">
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
                  <el-button size="small" :icon="Document" @click="goToConfigSchema(p)">配置 Schema</el-button>
                  <el-button v-if="p.kind === 'south'" size="small" :icon="CollectionTag" @click="goToTagSchema(p)">标签 Schema</el-button>
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
.font-mono { font-family: var(--font-mono); }
.empty-block { grid-column: 1 / -1; padding: 3rem; }
</style>
