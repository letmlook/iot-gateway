<script setup>
import { ref } from 'vue'
import { useI18n } from 'vue-i18n'
import { getErrorMessage } from '../i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import { Download, Upload } from '@element-plus/icons-vue'
import { api } from '../api.js'

const { t } = useI18n()

const error = ref('')

// 备份恢复
const showRestoreModal = ref(false)
const restoreLoading = ref(false)
const restoreFile = ref(null)
const restorePassword = ref('')

async function doBackup() {
  error.value = ''
  try {
    const { blob, filename } = await api.backup()
    const url = URL.createObjectURL(blob)
    const a = document.createElement('a')
    a.href = url
    a.download = filename
    a.click()
    URL.revokeObjectURL(url)
    ElMessage.success(t('system.backupSuccess'))
  } catch (e) {
    error.value = t('system.backupFailed') + getErrorMessage(t, e)
  }
}

function openRestoreModal() {
  restoreFile.value = null
  restorePassword.value = ''
  showRestoreModal.value = true
}

function onRestoreFileChange(uploadFile) {
  restoreFile.value = uploadFile?.raw ?? null
}

async function doRestore() {
  if (!restoreFile.value) {
    ElMessage.warning(t('system.selectBackupFile'))
    return
  }
  try {
    await ElMessageBox.confirm(t('system.restoreConfirm'), t('system.restoreTitle'), {
      type: 'warning',
      confirmButtonText: t('system.restore'),
      cancelButtonText: t('common.cancel'),
    })
  } catch {
    return
  }
  restoreLoading.value = true
  error.value = ''
  try {
    const formData = new FormData()
    formData.append('file', restoreFile.value)
    if (restorePassword.value) formData.append('password', restorePassword.value)
    await api.restore(formData)
    ElMessage.success(t('system.restoreSuccess'))
    showRestoreModal.value = false
  } catch (e) {
    error.value = t('system.restoreFailed') + getErrorMessage(t, e)
  } finally {
    restoreLoading.value = false
  }
}
</script>

<template>
  <div class="page-container sysconfig-page">
    <div class="sysconfig-body">
      <el-alert v-if="error" type="error" :title="error" closable show-icon @close="error = ''" class="mb-2" />

      <!-- 备份与恢复 -->
      <div class="config-section">
        <h3 class="section-title">{{ t('sysConfig.backupRestore') }}</h3>
        <p class="section-desc">{{ t('sysConfig.backupRestoreDesc') }}</p>
        <div class="backup-buttons">
          <el-button type="primary" :icon="Download" @click="doBackup">
            {{ t('system.backup') }}
          </el-button>
          <el-button :icon="Upload" @click="openRestoreModal">
            {{ t('system.restore') }}
          </el-button>
        </div>
      </div>

      <!-- 恢复对话框 -->
      <el-dialog v-model="showRestoreModal" :title="t('system.restoreTitle')" width="480px" destroy-on-close>
        <p class="restore-hint">{{ t('system.restoreHint') }}</p>
        <el-upload
          :auto-upload="false"
          :show-file-list="true"
          :limit="1"
          accept=".bin"
          @change="onRestoreFileChange"
          @remove="restoreFile = null"
        >
          <el-button type="default" size="small">{{ t('system.selectBackupFile') }}</el-button>
        </el-upload>
        <el-input v-model="restorePassword" type="password" :placeholder="t('system.restorePasswordPlaceholder')" class="mt-2" show-password clearable />
        <template #footer>
          <el-button @click="showRestoreModal = false">{{ t('common.cancel') }}</el-button>
          <el-button type="primary" :icon="Upload" :loading="restoreLoading" @click="doRestore">{{ t('system.restore') }}</el-button>
        </template>
      </el-dialog>
    </div>
  </div>
</template>

<style scoped>
.sysconfig-page {
  padding: 0;
}
.sysconfig-body {
  max-width: 720px;
}
.config-section {
  margin-bottom: 2rem;
}
.section-title {
  font-size: 1rem;
  font-weight: 600;
  margin: 0 0 0.5rem 0;
  color: var(--el-text-color-primary);
}
.section-desc {
  color: var(--el-text-color-secondary);
  font-size: 0.9rem;
  margin: 0 0 1rem 0;
}
.backup-buttons {
  display: flex;
  gap: 0.75rem;
}
.restore-hint {
  font-size: 0.9rem;
  color: var(--text-muted);
  margin-bottom: 0.5rem;
}
.mb-2 {
  margin-bottom: 1rem;
}
.mt-2 {
  margin-top: 0.5rem;
}
</style>
