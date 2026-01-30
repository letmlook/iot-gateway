<script setup>
import { ref, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { getErrorMessage } from '../i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import { CopyDocument, Key, CircleCheck, CircleClose, Upload, Delete } from '@element-plus/icons-vue'
import { api } from '../api.js'

const { t } = useI18n()

const loading = ref(true)
const uploading = ref(false)
const resetting = ref(false)
const error = ref('')
const machineId = ref('')
const licenseStatus = ref(null) // { hasLicense, features }
const fileInputRef = ref(null)

async function loadLicenseInfo() {
  loading.value = true
  error.value = ''
  try {
    const [midRes, statusRes] = await Promise.all([
      api.licenseMachineId().catch(() => null),
      api.licenseStatus().catch(() => null),
    ])
    machineId.value = midRes?.machineId ?? ''
    licenseStatus.value = statusRes ?? null
  } catch (e) {
    error.value = t('license.loadFailed') + getErrorMessage(t, e)
  } finally {
    loading.value = false
  }
}

async function copyMachineId() {
  if (!machineId.value) return
  try {
    await navigator.clipboard.writeText(machineId.value)
    ElMessage.success(t('license.copySuccess'))
  } catch {
    ElMessage.error(t('common.error'))
  }
}

function triggerFileSelect() {
  fileInputRef.value?.click()
}

async function handleFileSelect(event) {
  const file = event.target.files?.[0]
  if (!file) return
  await uploadLicenseFile(file)
  // 清空 input 以便同一文件可再次选择
  event.target.value = ''
}

async function uploadLicenseFile(file) {
  uploading.value = true
  error.value = ''
  try {
    const formData = new FormData()
    formData.append('file', file)
    await api.uploadLicense(formData)
    ElMessage.success(t('license.uploadSuccess'))
    // 重新加载授权状态
    await loadLicenseInfo()
  } catch (e) {
    error.value = t('license.uploadFailed') + getErrorMessage(t, e)
    ElMessage.error(t('license.uploadFailed') + getErrorMessage(t, e))
  } finally {
    uploading.value = false
  }
}

async function confirmResetLicense() {
  try {
    await ElMessageBox.confirm(
      t('license.resetWarning'),
      t('license.resetTitle'),
      {
        type: 'warning',
        confirmButtonText: t('license.confirmReset'),
        cancelButtonText: t('common.cancel'),
        confirmButtonClass: 'el-button--danger',
        dangerouslyUseHTMLString: true,
      }
    )
  } catch {
    return // 用户取消
  }
  await doResetLicense()
}

async function doResetLicense() {
  resetting.value = true
  error.value = ''
  try {
    await api.resetLicense()
    ElMessage.success(t('license.resetSuccess'))
    await loadLicenseInfo()
  } catch (e) {
    error.value = t('license.resetFailed') + getErrorMessage(t, e)
    ElMessage.error(t('license.resetFailed') + getErrorMessage(t, e))
  } finally {
    resetting.value = false
  }
}

function getUsageStatus() {
  if (!licenseStatus.value?.maxTags) return ''
  const percentage = (licenseStatus.value.usedTags ?? 0) / licenseStatus.value.maxTags * 100
  if (percentage >= 100) return 'exception'
  if (percentage >= 80) return 'warning'
  return 'success'
}

onMounted(loadLicenseInfo)
</script>

<template>
  <div class="page-container license-page">
    <div class="license-body">
      <div class="page-header license-header">
        <p class="header-desc">{{ t('license.desc') }}</p>
      </div>

      <el-alert v-if="error" type="error" :title="error" show-icon class="mb-2" />

      <div v-loading="loading" class="license-content">
        <!-- 机器码 -->
        <div class="license-section">
          <div class="section-title">
            <el-icon><Key /></el-icon>
            <span>{{ t('license.machineId') }}</span>
          </div>
          <p class="section-hint">{{ t('license.machineIdHint') }}</p>
          <div class="machine-id-row">
            <el-input
              :model-value="machineId"
              readonly
              type="textarea"
              :rows="2"
              class="machine-id-input"
            />
            <el-button type="primary" :icon="CopyDocument" @click="copyMachineId" :disabled="!machineId">
              {{ t('license.copyMachineId') }}
            </el-button>
          </div>
        </div>

        <!-- 授权状态 -->
        <div class="license-section">
          <div class="section-title">
            <el-icon v-if="licenseStatus?.hasLicense"><CircleCheck /></el-icon>
            <el-icon v-else><CircleClose /></el-icon>
            <span>{{ t('license.status') }}</span>
          </div>
          <div class="status-row">
            <div class="status-badge" :class="{ licensed: licenseStatus?.hasLicense }">
              <span v-if="licenseStatus?.hasLicense">{{ t('license.hasLicense') }}</span>
              <span v-else>{{ t('license.noLicense') }}</span>
            </div>
            <el-button
              v-if="licenseStatus?.hasLicense"
              type="danger"
              plain
              :icon="Delete"
              :loading="resetting"
              @click="confirmResetLicense"
            >
              {{ t('license.reset') }}
            </el-button>
          </div>
        </div>

        <!-- 点位数限制 -->
        <div class="license-section" v-if="licenseStatus">
          <div class="section-title">{{ t('license.tagLimit') }}</div>
          <div class="tag-usage">
            <div class="usage-info">
              <span class="usage-label">{{ t('license.usedTags') }}:</span>
              <span class="usage-value">{{ licenseStatus.usedTags ?? 0 }}</span>
              <span class="usage-separator">/</span>
              <span class="usage-value max">{{ licenseStatus.maxTags ?? t('license.unlimited') }}</span>
            </div>
            <el-progress
              v-if="licenseStatus.maxTags"
              :percentage="Math.min(100, Math.round((licenseStatus.usedTags ?? 0) / licenseStatus.maxTags * 100))"
              :status="getUsageStatus()"
              :stroke-width="10"
              class="usage-progress"
            />
            <p v-else class="text-muted usage-hint">{{ t('license.noTagLimit') }}</p>
          </div>
        </div>

        <!-- 已开通功能 -->
        <div class="license-section" v-if="licenseStatus">
          <div class="section-title">{{ t('license.features') }}</div>
          <div v-if="licenseStatus.features?.length" class="features-list">
            <el-tag v-for="f in licenseStatus.features" :key="f" class="feature-tag">{{ f }}</el-tag>
          </div>
          <p v-else class="text-muted">{{ t('license.noFeatures') }}</p>
        </div>

        <!-- 上传授权文件 -->
        <div class="license-section">
          <div class="section-title">
            <el-icon><Upload /></el-icon>
            <span>{{ t('license.upload') }}</span>
          </div>
          <p class="section-hint">{{ t('license.uploadHint') }}</p>
          <div class="upload-row">
            <input
              ref="fileInputRef"
              type="file"
              accept=".dat"
              style="display: none"
              @change="handleFileSelect"
            />
            <el-button
              type="primary"
              :icon="Upload"
              :loading="uploading"
              @click="triggerFileSelect"
            >
              {{ t('license.selectFile') }}
            </el-button>
          </div>
        </div>

        <!-- 激活说明 -->
        <div class="license-section instructions">
          <div class="section-title">{{ t('license.instructions') }}</div>
          <ol class="instructions-list">
            <li>{{ t('license.instructionsStep1') }}</li>
            <li>{{ t('license.instructionsStep2') }}</li>
            <li>{{ t('license.instructionsStep3') }}</li>
          </ol>
        </div>
      </div>
    </div>
  </div>
</template>

<style scoped>
.license-page {
  padding: 0;
}
.license-body {
  max-width: 720px;
}
.license-header {
  margin-bottom: 1rem;
}
.license-header .header-desc {
  color: var(--el-text-color-secondary);
  margin: 0;
}
.license-content {
  margin-bottom: 1.5rem;
}
.license-section {
  margin-bottom: 1.5rem;
}
.license-section:last-child {
  margin-bottom: 0;
}
.section-title {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  font-weight: 600;
  margin-bottom: 0.5rem;
}
.section-title .el-icon {
  font-size: 1.1rem;
}
.section-hint {
  color: var(--el-text-color-secondary);
  font-size: 0.9rem;
  margin: 0 0 0.75rem 0;
}
.machine-id-row,
.upload-row,
.status-row {
  display: flex;
  gap: 0.75rem;
  align-items: flex-start;
}
.machine-id-input {
  flex: 1;
}
.machine-id-input :deep(textarea) {
  font-family: ui-monospace, monospace;
  font-size: 0.85rem;
}
.status-badge {
  display: inline-block;
  padding: 0.35rem 0.75rem;
  border-radius: 6px;
  font-weight: 500;
  background: var(--el-fill-color-light);
  color: var(--el-text-color-secondary);
}
.status-badge.licensed {
  background: var(--el-color-success-light-9);
  color: var(--el-color-success-dark-2);
}
.features-list {
  display: flex;
  flex-wrap: wrap;
  gap: 0.5rem;
}
.feature-tag {
  margin: 0;
}
.text-muted {
  color: var(--el-text-color-secondary);
  margin: 0;
  font-size: 0.9rem;
}
.tag-usage {
  margin-top: 0.5rem;
}
.usage-info {
  display: flex;
  align-items: center;
  gap: 0.5rem;
  font-size: 1rem;
  margin-bottom: 0.5rem;
}
.usage-label {
  color: var(--el-text-color-secondary);
}
.usage-value {
  font-weight: 600;
  font-family: ui-monospace, monospace;
}
.usage-value.max {
  color: var(--el-color-primary);
}
.usage-separator {
  color: var(--el-text-color-secondary);
}
.usage-progress {
  max-width: 300px;
}
.usage-hint {
  margin-top: 0.25rem;
}
.instructions .section-title {
  margin-bottom: 0.5rem;
}
.instructions-list {
  margin: 0;
  padding-left: 1.25rem;
  color: var(--el-text-color-regular);
  font-size: 0.9rem;
  line-height: 1.7;
}
.instructions-list li {
  margin-bottom: 0.25rem;
}
.mb-2 {
  margin-bottom: 0.75rem;
}
</style>
