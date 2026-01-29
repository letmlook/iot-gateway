<script setup>
import { ref, inject, onMounted, computed, watch } from 'vue'
import { useRouter, useRoute } from 'vue-router'
import { useI18n } from 'vue-i18n'
import { ElMessage } from 'element-plus'
import { ArrowLeft } from '@element-plus/icons-vue'
import { api } from '../api.js'
import NodeConfigForm from '../components/NodeConfigForm.vue'

const router = useRouter()
const route = useRoute()
const { t } = useI18n()

const kind = computed(() => route.path.startsWith('/north') ? 'north' : 'south')
const southPlugins = inject('southPlugins', ref([]))
const northPlugins = inject('northPlugins', ref([]))

const pluginList = computed(() => kind.value === 'south' ? southPlugins.value : northPlugins.value)
const pluginOptions = computed(() =>
  pluginList.value.map(p => ({ name: p[0], description: p[1], version: p[2] }))
)

const createForm = ref({ name: '', plugin_name: '', config: '{}' })
const configFromSchema = ref({})
const useConfigFormMode = ref(false)
const loading = ref(false)
const error = ref('')
const northPlaceholder = '{"host":"broker.emqx.io","port":1883}'
const southPlaceholder = '{}'

async function loadSchemaForPlugin(pluginName) {
  if (!pluginName) {
    useConfigFormMode.value = false
    return
  }
  try {
    const schema = kind.value === 'south'
      ? await api.pluginSouthSchema(pluginName)
      : await api.pluginNorthSchema(pluginName)
    useConfigFormMode.value = !!(schema && schema.params && schema.params.length > 0)
  } catch {
    useConfigFormMode.value = false
  }
}

async function loadCopyFrom() {
  const copyId = route.query.copyFrom
  if (!copyId) return
  try {
    const node = await api.node(copyId)
    const setting = await api.nodeSetting(copyId)
    if (node) createForm.value.name = (node.name || '') + '-副本'
    createForm.value.plugin_name = node?.plugin_name || ''
    createForm.value.config = JSON.stringify(setting?.config || {}, null, 2)
    configFromSchema.value = { ...(setting?.config || {}) }
    await loadSchemaForPlugin(createForm.value.plugin_name)
  } catch (e) {
    console.error('加载复制配置失败', e)
  }
}

async function submit() {
  let config = {}
  if (useConfigFormMode.value) {
    config = { ...configFromSchema.value }
  } else {
    try {
      config = JSON.parse(createForm.value.config || '{}')
    } catch {
      ElMessage.error(t('createNode.configMustBeJson'))
      return
    }
  }
  loading.value = true
  error.value = ''
  try {
    await api.createNode({
      name: createForm.value.name,
      kind: kind.value,
      plugin_name: createForm.value.plugin_name,
      config
    })
    ElMessage.success(t('createNode.createSuccess'))
    router.push(kind.value === 'south' ? '/south' : '/north')
  } catch (e) {
    error.value = t('createNode.createFailed') + e.message
  } finally {
    loading.value = false
  }
}

function goBack() {
  router.push(kind.value === 'south' ? '/south' : '/north')
}

watch(() => createForm.value.plugin_name, (name) => loadSchemaForPlugin(name))
onMounted(async () => {
  createForm.value.plugin_name = pluginOptions.value[0]?.name || ''
  await loadSchemaForPlugin(createForm.value.plugin_name)
  await loadCopyFrom()
})
</script>

<template>
  <div class="page-container create-page">
    <div class="page-header">
      <el-button :icon="ArrowLeft" @click="goBack">{{ t('createNode.back') }}</el-button>
      <h2 class="page-title">{{ kind === 'south' ? t('createNode.addSouth') : t('createNode.addNorth') }}</h2>
    </div>
    <el-alert v-if="error" type="error" :title="error" closable show-icon @close="error = ''" class="mb-2" />
    <el-card class="form-card">
      <el-form :model="createForm" label-width="100px" label-position="top">
        <el-form-item :label="t('createNode.nodeName')" required>
          <el-input v-model="createForm.name" :placeholder="kind === 'south' ? t('createNode.namePlaceholderSouth') : t('createNode.namePlaceholderNorth')" clearable />
        </el-form-item>
        <el-form-item :label="kind === 'south' ? t('createNode.pluginLabel') : t('createNode.appPluginLabel')" required>
          <el-select v-model="createForm.plugin_name" :placeholder="t('createNode.selectPlugin')" style="width: 100%">
            <el-option
              v-for="p in pluginOptions"
              :key="p.name"
              :label="p.name + (p.version ? ` (v${p.version})` : '')"
              :value="p.name"
            />
          </el-select>
          <div v-if="pluginOptions.find(pp => pp.name === createForm.plugin_name)?.description" class="form-hint">
            {{ pluginOptions.find(pp => pp.name === createForm.plugin_name)?.description }}
          </div>
        </el-form-item>
        <el-form-item :label="t('createNode.nodeConfig')">
          <template v-if="!createForm.plugin_name">
            <el-input
              v-model="createForm.config"
              type="textarea"
              :rows="6"
              :placeholder="t('schema.selectPluginFirst')"
              class="font-mono"
              disabled
            />
          </template>
          <template v-else-if="useConfigFormMode">
            <NodeConfigForm
              :plugin-name="createForm.plugin_name"
              :kind="kind"
              :model-value="configFromSchema"
              @update:model-value="configFromSchema = $event"
            />
          </template>
          <template v-else>
            <div class="config-fallback-hint">{{ t('schema.noSchemaHint') }}</div>
            <el-input
              v-model="createForm.config"
              type="textarea"
              :rows="6"
              :placeholder="kind === 'north' ? northPlaceholder : southPlaceholder"
              class="font-mono"
            />
          </template>
        </el-form-item>
        <el-form-item>
          <el-button @click="goBack">{{ t('createNode.cancel') }}</el-button>
          <el-button type="primary" :loading="loading" :disabled="!createForm.name || !createForm.plugin_name" @click="submit">
            {{ t('createNode.create') }}
          </el-button>
        </el-form-item>
      </el-form>
    </el-card>
  </div>
</template>

<style scoped>
.create-page { max-width: 640px; }
.page-header { display: flex; align-items: center; gap: 1rem; margin-bottom: 1rem; }
.page-title { margin: 0; font-size: 1.25rem; }
.form-card { padding: 1.5rem; }
.mb-2 { margin-bottom: 1rem; }
.form-hint { font-size: 0.8rem; color: var(--text-muted); margin-top: 0.25rem; }
.config-fallback-hint { font-size: 0.8rem; color: var(--el-text-color-secondary); margin-bottom: 0.5rem; }
.font-mono { font-family: var(--font-mono); }
</style>
