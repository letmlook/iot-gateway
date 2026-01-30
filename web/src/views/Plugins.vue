<script setup>
import { ref, onMounted, computed } from 'vue'
import { useRouter } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { ElMessage } from 'element-plus'
import { Plus } from '@element-plus/icons-vue'
import { api } from '../api.js'

const router = useRouter()
const { t } = useI18n()

const southPlugins = ref([])
const northPlugins = ref([])
const nodes = ref([])
const loading = ref(false)
const error = ref('')
const activeTab = ref('south')


const pluginSearch = ref('')

const { locale } = useI18n()
// API 返回 { name, name_zh?, name_en?, description?, description_zh?, description_en?, version }
function pluginDisplayName(p) {
  if (!p) return ''
  return (locale.value === 'zh' ? p.name_zh : p.name_en) || p.name || ''
}
function pluginDisplayDesc(p) {
  if (!p) return ''
  return (locale.value === 'zh' ? p.description_zh : p.description_en) || p.description || ''
}
const plugins = computed(() => {
  const list = (activeTab.value === 'south' ? southPlugins.value : northPlugins.value).map(p => ({
    ...p,
    kind: activeTab.value === 'south' ? 'south' : 'north'
  }))
  if (!pluginSearch.value.trim()) return list
  const k = pluginSearch.value.trim().toLowerCase()
  const nameOrDesc = (p) => (pluginDisplayName(p) || p.name || '') + ' ' + (pluginDisplayDesc(p) || p.description || '')
  return list.filter(p => nameOrDesc(p).toLowerCase().includes(k))
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
      <h2 class="page-title">{{ t('plugins.title') }}</h2>
      <div class="header-toolbar">
        <el-select v-model="activeTab" style="width: 120px" class="mr-1">
          <el-option :label="t('plugins.southPlugins')" value="south" />
          <el-option :label="t('plugins.northPlugins')" value="north" />
        </el-select>
        <el-input v-model="pluginSearch" :placeholder="t('plugins.searchPlaceholder')" clearable style="width: 200px" class="mr-1" />
        <el-button type="primary" :icon="Plus" @click="goAddPlugin">{{ t('plugins.addPlugin') }}</el-button>
      </div>
    </div>

    <el-alert v-if="error" type="error" :title="error" closable show-icon @close="error = ''" class="mb-2" />

    <el-skeleton v-if="loading" :rows="6" animated />

    <template v-else>
      <el-tabs v-model="activeTab" class="plugins-tabs">
        <el-tab-pane name="south">
          <template #label>
            <span class="tab-label"><span class="dot south" /> {{ t('plugins.southPlugins') }} <el-tag size="small" type="info" class="ml-1">{{ southPlugins.length }}</el-tag></span>
          </template>
          <!-- 表格视图（对标 Neuron） -->
          <el-table :data="plugins" size="default" stripe class="plugins-table">
            <el-table-column :label="t('common.name')" min-width="120">
              <template #default="{ row }">{{ pluginDisplayName(row) || row.name }}</template>
            </el-table-column>
            <el-table-column :label="t('plugins.pluginType')" width="120">
              <template #default="{ row }">
                <el-tag size="small" :type="row.kind === 'south' ? 'success' : 'primary'">
                  {{ row.kind === 'south' ? t('plugins.southDevice') : t('plugins.northApp') }}
                </el-tag>
              </template>
            </el-table-column>
            <el-table-column :label="t('plugins.pluginCategory')" width="100">
              <template #default>System</template>
            </el-table-column>
            <el-table-column prop="version" :label="t('plugins.version')" width="100">
              <template #default="{ row }">v{{ row.version || '-' }}</template>
            </el-table-column>
            <el-table-column :label="t('common.description')" min-width="280" show-overflow-tooltip>
              <template #default="{ row }">{{ pluginDisplayDesc(row) || row.description || '-' }}</template>
            </el-table-column>
            <el-table-column :label="t('common.operation')" width="140" fixed="right">
              <template #default="{ row }">
                <el-button type="primary" link size="small" :icon="Plus" @click="goCreateNode(row)">
                  {{ row.kind === 'south' ? t('plugins.createDevice') : t('plugins.createApp') }}
                </el-button>
              </template>
            </el-table-column>
          </el-table>
          <el-empty v-if="!plugins.length" :description="t('plugins.noPlugins')" class="empty-block" />
          <div class="plugins-grid" style="display: none">
            <el-card v-for="p in plugins" :key="p.name + p.kind" class="plugin-card" :class="p.kind" shadow="hover">
              <template #header>
                <div class="plugin-header">
                  <div class="plugin-meta">
                    <span class="plugin-name">{{ pluginDisplayName(p) || p.name }}</span>
                    <el-tag v-if="p.version" size="small" type="info">v{{ p.version }}</el-tag>
                    <el-tag :type="p.kind === 'south' ? 'success' : ''" size="small" effect="plain">
                      {{ p.kind === 'south' ? '南向' : '北向' }}
                    </el-tag>
                  </div>
                </div>
              </template>
              <p v-if="pluginDisplayDesc(p) || p.description" class="plugin-desc">{{ pluginDisplayDesc(p) || p.description }}</p>
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
            <span class="tab-label"><span class="dot north" /> {{ t('plugins.northPlugins') }} <el-tag size="small" type="info" class="ml-1">{{ northPlugins.length }}</el-tag></span>
          </template>
          <el-table :data="plugins" size="default" stripe class="plugins-table">
            <el-table-column :label="t('common.name')" min-width="120">
              <template #default="{ row }">{{ pluginDisplayName(row) || row.name }}</template>
            </el-table-column>
            <el-table-column :label="t('plugins.pluginType')" width="120">
              <template #default="{ row }">
                <el-tag size="small" type="primary">{{ t('plugins.northApp') }}</el-tag>
              </template>
            </el-table-column>
            <el-table-column :label="t('plugins.pluginCategory')" width="100">
              <template #default>System</template>
            </el-table-column>
            <el-table-column prop="version" :label="t('plugins.version')" width="100">
              <template #default="{ row }">v{{ row.version || '-' }}</template>
            </el-table-column>
            <el-table-column :label="t('common.description')" min-width="280" show-overflow-tooltip>
              <template #default="{ row }">{{ pluginDisplayDesc(row) || row.description || '-' }}</template>
            </el-table-column>
            <el-table-column :label="t('common.operation')" width="120" fixed="right">
              <template #default="{ row }">
                <el-button type="primary" link size="small" :icon="Plus" @click="goCreateNode(row)">{{ t('plugins.createApp') }}</el-button>
              </template>
            </el-table-column>
          </el-table>
          <el-empty v-if="!plugins.length" :description="t('plugins.noPlugins')" class="empty-block" />
          <div class="plugins-grid" style="display: none">
            <el-card v-for="p in plugins" :key="p.name + p.kind" class="plugin-card" :class="p.kind" shadow="hover">
              <template #header>
                <div class="plugin-header">
                  <div class="plugin-meta">
                    <span class="plugin-name">{{ pluginDisplayName(p) || p.name }}</span>
                    <el-tag v-if="p.version" size="small" type="info">v{{ p.version }}</el-tag>
                    <el-tag :type="p.kind === 'south' ? 'success' : ''" size="small" effect="plain">
                      {{ p.kind === 'south' ? '南向' : '北向' }}
                    </el-tag>
                  </div>
                </div>
              </template>
              <p v-if="pluginDisplayDesc(p) || p.description" class="plugin-desc">{{ pluginDisplayDesc(p) || p.description }}</p>
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
