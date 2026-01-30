<script setup>
/**
 * 根据插件 config_schema 自动生成节点配置表单。
 * 支持 Int / String / Bool 类型，必填/可选、默认值、min/max/regex 校验。
 */
import { ref, watch, computed, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { getErrorMessage } from '../i18n'
import { api } from '../api.js'

const { t, locale } = useI18n()

const props = defineProps({
  /** 插件名称，如 sim / mqtt */
  pluginName: { type: String, default: '' },
  /** south | north */
  kind: { type: String, default: 'south' },
  /** 当前配置对象，用于回填表单 */
  modelValue: { type: Object, default: () => ({}) }
})

const emit = defineEmits(['update:modelValue'])

const schema = ref(null)
const schemaLoading = ref(false)
const schemaError = ref('')

/** 表单项值：{ [param.name]: value } */
const formValues = ref({})

const hasSchema = computed(() => schema.value && schema.value.params && schema.value.params.length > 0)

const params = computed(() => schema.value?.params || [])

/** 按当前语言取参数展示标签：name_zh/name_en，无则用 name */
function paramLabel(p) {
  if (!p) return ''
  const isZh = locale.value === 'zh'
  if (isZh && p.name_zh) return p.name_zh
  if (!isZh && p.name_en) return p.name_en
  return p.name || ''
}

/** 按当前语言取参数展示描述：description_zh/description_en，无则用 description 或 name */
function paramDesc(p) {
  if (!p) return ''
  const isZh = locale.value === 'zh'
  if (isZh && p.description_zh) return p.description_zh
  if (!isZh && p.description_en) return p.description_en
  return p.description || p.name || ''
}

function getParamType(p) {
  const t = (p.ty ?? p.type ?? 'string').toString().toLowerCase()
  if (t === 'int' || t === 'integer') return 'int'
  if (t === 'bool' || t === 'boolean') return 'bool'
  return 'string'
}

function isRequired(p) {
  const a = (p.attribute ?? '').toString().toLowerCase()
  return a === 'required'
}

function getDefaultValue(p) {
  if (p.default !== undefined && p.default !== null) return p.default
  const t = getParamType(p)
  if (t === 'int') return undefined
  if (t === 'bool') return false
  return ''
}

function buildConfig() {
  const config = {}
  for (const p of params.value) {
    const key = p.name
    let val = formValues.value[key]
    if (val === undefined) val = getDefaultValue(p)
    if (val !== undefined && val !== '') {
      config[key] = val
    }
  }
  return config
}

function notifyChange() {
  emit('update:modelValue', buildConfig())
}

function initFormValues() {
  const next = {}
  for (const p of params.value) {
    const key = p.name
    const fromModel = props.modelValue && props.modelValue[key]
    if (fromModel !== undefined && fromModel !== null) {
      next[key] = fromModel
    } else {
      next[key] = getDefaultValue(p)
    }
  }
  formValues.value = next
  notifyChange()
}

async function loadSchema() {
  if (!props.pluginName) {
    schema.value = null
    schemaError.value = ''
    formValues.value = {}
    return
  }
  schemaLoading.value = true
  schemaError.value = ''
  try {
    if (props.kind === 'south') {
      schema.value = await api.pluginSouthSchema(props.pluginName)
    } else {
      schema.value = await api.pluginNorthSchema(props.pluginName)
    }
    initFormValues()
  } catch (e) {
    schema.value = null
    schemaError.value = getErrorMessage(t, e) || t('schema.loadFailed')
    formValues.value = {}
    emit('update:modelValue', {})
  } finally {
    schemaLoading.value = false
  }
}

function onFieldChange() {
  notifyChange()
}

watch(() => [props.pluginName, props.kind], () => {
  loadSchema()
}, { immediate: false })

watch(() => props.modelValue, (newVal) => {
  if (!hasSchema.value || !newVal) return
  const next = { ...formValues.value }
  for (const p of params.value) {
    const key = p.name
    if (newVal[key] !== undefined) next[key] = newVal[key]
  }
  formValues.value = next
}, { deep: true })

onMounted(() => {
  if (props.pluginName) loadSchema()
})
</script>

<template>
  <div class="node-config-form">
    <el-skeleton v-if="schemaLoading" :rows="3" animated />

    <el-alert
      v-else-if="schemaError"
      type="warning"
      :title="schemaError"
      show-icon
      class="mb-2"
    />

    <template v-else-if="hasSchema">
      <el-form label-position="top" class="config-form">
        <el-form-item
          v-for="p in params"
          :key="p.name"
          :label="paramLabel(p)"
          :required="isRequired(p)"
        >
          <template v-if="getParamType(p) === 'int'">
            <el-input-number
              :model-value="formValues[p.name]"
              :min="p.valid?.min ?? undefined"
              :max="p.valid?.max ?? undefined"
              :placeholder="paramDesc(p)"
              style="width: 100%"
              @update:model-value="formValues[p.name] = $event; onFieldChange()"
            />
          </template>
          <template v-else-if="getParamType(p) === 'bool'">
            <div class="bool-row">
              <el-switch
                :model-value="!!formValues[p.name]"
                @update:model-value="formValues[p.name] = $event; onFieldChange()"
              />
              <span v-if="paramDesc(p)" class="param-desc inline">{{ paramDesc(p) }}</span>
            </div>
          </template>
          <template v-else>
            <el-input
              :model-value="formValues[p.name]"
              :type="(p.name || '').toLowerCase().includes('password') ? 'password' : 'text'"
              :placeholder="paramDesc(p)"
              :maxlength="p.valid?.length"
              show-password
              clearable
              @update:model-value="formValues[p.name] = $event; onFieldChange()"
            />
          </template>
          <div v-if="paramDesc(p) && getParamType(p) !== 'bool'" class="param-desc">
            {{ paramDesc(p) }}
          </div>
        </el-form-item>
      </el-form>
    </template>

    <div v-else class="no-schema-hint">
      <span v-if="pluginName">{{ t('schema.noSchemaHint') }}</span>
      <span v-else>{{ t('schema.selectPluginFirst') }}</span>
    </div>
  </div>
</template>

<style scoped>
.node-config-form { min-height: 60px; }
.mb-2 { margin-bottom: 0.75rem; }
.param-desc { font-size: 0.75rem; color: var(--el-text-color-secondary); margin-top: 0.25rem; }
.no-schema-hint { font-size: 0.9rem; color: var(--el-text-color-secondary); padding: 0.5rem 0; }
.config-form :deep(.el-form-item) { margin-bottom: 1rem; }
.bool-row { display: flex; align-items: center; gap: 0.5rem; }
.param-desc.inline { margin-top: 0; }
</style>
