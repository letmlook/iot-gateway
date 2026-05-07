<script setup>
import { ref, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { getErrorMessage } from '../i18n'
import { ElMessage } from 'element-plus'
import { Download, Document } from '@element-plus/icons-vue'
import { api } from '../api.js'
import PageHeader from '../components/PageHeader.vue'

const { t } = useI18n()

const activeTab = ref('manage')
const loading = ref(false)
const error = ref('')
const logLevel = ref('info')
const logUploadEnabled = ref(false)
const savingConfig = ref(false)

const logLevelOptions = [
  { value: 'trace', label: 'Trace' },
  { value: 'debug', label: 'Debug' },
  { value: 'info', label: 'Info' },
  { value: 'warn', label: 'Warn' },
  { value: 'error', label: 'Error' },
]

async function loadLogConfig() {
  loading.value = true
  error.value = ''
  try {
    // 尝试加载日志配置（如果后端支持）
    const config = await api.getLogConfig?.().catch(() => null)
    if (config) {
      logLevel.value = config.level || 'info'
      logUploadEnabled.value = config.upload_enabled || false
    }
  } catch (e) {
    error.value = t('logs.loadFailed') + getErrorMessage(t, e)
  } finally {
    loading.value = false
  }
}

async function downloadSystemLog() {
  try {
    const { blob, filename } = await api.downloadLog('system')
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = filename || 'gateway-system.log'
    a.click()
    URL.revokeObjectURL(url)
    ElMessage.success(t('logs.downloadSuccess'))
  } catch (e) {
    ElMessage.error(t('logs.downloadFailed') + getErrorMessage(t, e))
  }
}

async function downloadDriverLog() {
  try {
    const { blob, filename } = await api.downloadLog('driver')
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = filename || 'gateway-driver.log'
    a.click()
    URL.revokeObjectURL(url)
    ElMessage.success(t('logs.downloadSuccess'))
  } catch (e) {
    ElMessage.error(t('logs.downloadFailed') + getErrorMessage(t, e))
  }
}

async function downloadAllLogs() {
  try {
    const { blob, filename } = await api.downloadLog('all')
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = filename || 'gateway-logs.zip'
    a.click()
    URL.revokeObjectURL(url)
    ElMessage.success(t('logs.downloadSuccess'))
  } catch (e) {
    ElMessage.error(t('logs.downloadFailed') + getErrorMessage(t, e))
  }
}

async function saveLogConfig() {
  savingConfig.value = true
  try {
    await api.setLogConfig?.({ level: logLevel.value, upload_enabled: logUploadEnabled.value })
    ElMessage.success(t('common.save') + ' ' + t('logs.configSaved'))
  } catch (e) {
    ElMessage.error(t('logs.saveFailed') + getErrorMessage(t, e))
  } finally {
    savingConfig.value = false
  }
}

onMounted(loadLogConfig)
</script>

<template>
  <div class="page-container logs-page">
    <PageHeader :title="t('logs.title')" />
    <div class="logs-body">
      <el-tabs v-model="activeTab" class="logs-tabs">
        <el-tab-pane :label="t('logs.tabManage')" name="manage">
          <!-- 日志下载 -->
          <div class="logs-section">
            <h3 class="section-title">{{ t('logs.download') }}</h3>
            <div class="download-buttons">
              <el-button :icon="Download" @click="downloadSystemLog">
                {{ t('logs.downloadSystemLog') }}
              </el-button>
              <el-button :icon="Download" @click="downloadDriverLog">
                {{ t('logs.downloadDriverLog') }}
              </el-button>
              <el-button :icon="Download" @click="downloadAllLogs">
                {{ t('logs.downloadAllLogs') }}
              </el-button>
            </div>
          </div>

          <!-- 日志配置 -->
          <div class="logs-section">
            <h3 class="section-title">{{ t('logs.config') }}</h3>
            <el-form label-width="140px" class="log-config-form">
              <el-form-item :label="t('logs.logLevel')">
                <el-select v-model="logLevel" style="width: 200px">
                  <el-option
                    v-for="opt in logLevelOptions"
                    :key="opt.value"
                    :value="opt.value"
                    :label="opt.label"
                  />
                </el-select>
              </el-form-item>
              <el-form-item :label="t('logs.enableUpload')">
                <el-switch v-model="logUploadEnabled" />
                <span class="form-hint">{{ t('logs.uploadHint') }}</span>
              </el-form-item>
              <el-form-item>
                <el-button type="primary" :loading="savingConfig" @click="saveLogConfig">
                  {{ t('common.save') }}
                </el-button>
              </el-form-item>
            </el-form>
          </div>
        </el-tab-pane>

        <el-tab-pane :label="t('logs.tabMonitor')" name="monitor">
          <div class="logs-section">
            <el-empty :description="t('logs.monitorNotAvailable')" />
          </div>
        </el-tab-pane>
      </el-tabs>
    </div>
  </div>
</template>

<style scoped>
.logs-page {
  padding: 0;
}
.logs-body {
  max-width: 900px;
}
.logs-tabs {
  margin-top: 0.5rem;
}
.logs-section {
  margin-bottom: 2rem;
}
.section-title {
  font-size: 1rem;
  font-weight: 600;
  margin: 0 0 1rem 0;
  color: var(--el-text-color-primary);
}
.download-buttons {
  display: flex;
  flex-wrap: wrap;
  gap: 0.75rem;
}
.log-config-form {
  max-width: 500px;
}
.form-hint {
  margin-left: 0.75rem;
  color: var(--el-text-color-secondary);
  font-size: 0.85rem;
}
</style>
