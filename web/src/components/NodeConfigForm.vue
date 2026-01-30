<script setup>
/**
 * 根据插件 config_schema 自动生成节点配置表单。
 * 按 schema 的 type 显示对应控件：int -> 数字框，bool -> 开关，select -> 下拉，file -> 上传，string -> 文本框（含密码）。
 * 依赖逻辑：depends_on / depends_value / depends_values 控制控件显隐，依赖字段变化时联动更新。
 */
import { ref, watch, computed, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { QuestionFilled } from '@element-plus/icons-vue'
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

/** 从 schema 取参数类型（后端可能为 type 或 ty），返回统一控件类型 */
function getParamType(p) {
  const t = (p.ty ?? p.type ?? 'string').toString().toLowerCase()
  if (t === 'int' || t === 'integer') return 'int'
  if (t === 'bool' || t === 'boolean') return 'bool'
  if (t === 'file') return 'file'
  if (t === 'select') return 'select'
  return 'string'
}

/** 取依赖字段当前有效值：有填用填值，否则用该参数默认值（用于初始与未填时的联动） */
function getEffectiveValue(paramName) {
  if (formValues.value[paramName] !== undefined && formValues.value[paramName] !== null) {
    return formValues.value[paramName]
  }
  const param = params.value.find((x) => x.name === paramName)
  return param ? getDefaultValue(param) : undefined
}

/** 宽松相等：用于依赖比较，使 0 与 "0"、1 与 "1" 等一致 */
function looseEqual(a, b) {
  if (a === b) return true
  if (a == b) return true // eslint-disable-line eqeqeq
  if (typeof a === 'number' && typeof b === 'number' && Number.isNaN(a) && Number.isNaN(b)) return true
  return false
}

/** 字段依赖联动：仅当 depends_on 对应值等于 depends_value 或在 depends_values 中时显示 */
function isParamVisible(p) {
  if (!p.depends_on) return true
  const depVal = getEffectiveValue(p.depends_on)
  if (p.depends_value !== undefined && p.depends_value !== null) {
    return looseEqual(depVal, p.depends_value)
  }
  if (p.depends_values && Array.isArray(p.depends_values)) {
    return p.depends_values.some((v) => looseEqual(depVal, v))
  }
  return false
}

/** 下拉选项展示列表：{ value, label }，按当前语言取 label */
function optionList(p) {
  const opts = p.options || []
  const isZh = locale.value === 'zh'
  return opts.map((o) => {
    let label = o.value
    if (isZh && o.label_zh) label = o.label_zh
    else if (!isZh && o.label_en) label = o.label_en
    else if (o.label) label = o.label
    else if (typeof o.value === 'string') label = o.value
    else if (typeof o.value === 'number' || typeof o.value === 'boolean') label = String(o.value)
    return { value: o.value, label }
  })
}

/** select 类型：选项不超过 2 个用单选按钮，大于 2 个用下拉 */
function useRadioForSelect(p) {
  const opts = p.options || []
  return opts.length > 0 && opts.length <= 2
}

function isRequired(p) {
  const a = (p.attribute ?? '').toString().toLowerCase()
  return a === 'required'
}

/** 仅参数名为 password 时视为密码框（需密码输入框 + 可切换显隐） */
function isPasswordParam(p) {
  return (p.name || '').toLowerCase().includes('password')
}

function getDefaultValue(p) {
  if (p.default !== undefined && p.default !== null) return p.default
  const t = getParamType(p)
  if (t === 'int') return undefined
  if (t === 'bool') return false
  if (t === 'file') return ''
  if (t === 'select') {
    const opts = p.options || []
    return opts.length ? opts[0].value : ''
  }
  return ''
}

function buildConfig() {
  const config = {}
  for (const p of params.value) {
    if (!isParamVisible(p)) continue
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

/** 文件类型：当前路径对应的展示列表（仅用于 el-upload 展示） */
function fileListFor(p) {
  const path = formValues.value[p.name]
  if (!path || typeof path !== 'string') return []
  const name = path.replace(/^.*[/\\]/, '') || path
  return [{ name, path, uid: path }]
}

/** 自定义上传：调用接口保存文件，成功后写入路径 */
async function handleFileUpload(opt, p) {
  try {
    const path = await api.uploadConfigFile(opt.file)
    formValues.value[p.name] = path
    onFieldChange()
    opt.onSuccess({ path })
  } catch (e) {
    opt.onError(e)
  }
}

function clearFile(p) {
  formValues.value[p.name] = ''
  onFieldChange()
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
        <!-- 按 schema type 渲染对应控件；v-show 依赖 isParamVisible(p)，依赖字段变化时联动显隐 -->
        <el-form-item
          v-for="p in params"
          :key="p.name"
          v-show="isParamVisible(p)"
          :required="isRequired(p)"
        >
          <template #label>
            <span class="label-text">{{ paramLabel(p) }}</span>
            <el-tooltip v-if="paramDesc(p)" placement="top" :content="paramDesc(p)">
              <el-icon class="label-tip-icon"><QuestionFilled /></el-icon>
            </el-tooltip>
          </template>
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
            </div>
          </template>
          <template v-else-if="getParamType(p) === 'select'">
            <el-radio-group
              v-if="useRadioForSelect(p)"
              :model-value="formValues[p.name]"
              class="select-radio-group"
              @update:model-value="formValues[p.name] = $event; onFieldChange()"
            >
              <el-radio
                v-for="opt in optionList(p)"
                :key="String(opt.value)"
                :value="opt.value"
              >
                {{ opt.label }}
              </el-radio>
            </el-radio-group>
            <el-select
              v-else
              :model-value="formValues[p.name]"
              :placeholder="paramDesc(p)"
              style="width: 100%"
              clearable
              @update:model-value="formValues[p.name] = $event; onFieldChange()"
            >
              <el-option
                v-for="opt in optionList(p)"
                :key="String(opt.value)"
                :label="opt.label"
                :value="opt.value"
              />
            </el-select>
          </template>
          <template v-else-if="getParamType(p) === 'file'">
            <el-upload
              :file-list="fileListFor(p)"
              :limit="1"
              :auto-upload="true"
              :http-request="(opt) => handleFileUpload(opt, p)"
              @remove="clearFile(p)"
            >
              <el-button type="primary" size="small">{{ t('schema.uploadFile') }}</el-button>
            </el-upload>
            <div v-if="formValues[p.name]" class="file-path-hint">{{ formValues[p.name] }}</div>
          </template>
          <template v-else>
            <el-input
              :model-value="formValues[p.name]"
              :type="isPasswordParam(p) ? 'password' : 'text'"
              :placeholder="paramDesc(p)"
              :maxlength="p.valid?.length"
              :show-password="isPasswordParam(p)"
              clearable
              @update:model-value="formValues[p.name] = $event; onFieldChange()"
            />
          </template>
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
.node-config-form { min-height: 60px; width: 100%; }
.mb-2 { margin-bottom: 0.75rem; }
.no-schema-hint { font-size: 0.9rem; color: var(--el-text-color-secondary); padding: 0.5rem 0; }
/* 自适应网格：窄屏单列，宽屏多列 */
.config-form {
  display: grid;
  grid-template-columns: repeat(auto-fill, minmax(min(100%, 300px), 1fr));
  gap: 1rem 1.25rem;
}
.config-form :deep(.el-form-item) {
  margin-bottom: 0;
  min-width: 0;
}
.config-form :deep(.el-form-item__label) {
  display: inline-flex;
  align-items: center;
  gap: 0.25rem;
}
.label-text { font-weight: inherit; }
.label-tip-icon {
  font-size: 0.9rem;
  color: var(--el-text-color-secondary);
  cursor: help;
}
.bool-row { display: flex; align-items: center; gap: 0.5rem; }
.select-radio-group { display: flex; flex-wrap: wrap; gap: 0.5rem 1rem; }
.file-path-hint { font-size: 0.75rem; color: var(--el-text-color-secondary); margin-top: 0.25rem; word-break: break-all; }
</style>
