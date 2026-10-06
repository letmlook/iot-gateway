<script setup>
import { ref, onMounted } from 'vue'
import { useI18n } from 'vue-i18n'
import { getErrorMessage } from '../i18n'
import { ElMessage, ElMessageBox } from 'element-plus'
import { api } from '../api.js'
import PageHeader from '../components/PageHeader.vue'

const { t } = useI18n()

const loading = ref(true)
const error = ref('')
const users = ref([])
const showDialog = ref(false)
const dialogLoading = ref(false)
const isEdit = ref(false)
const formUser = ref({ username: '', password: '', role: 'operator', tenant: 'default' })
const editId = ref('')
const showPasswordDialog = ref(false)
const passwordUserId = ref('')
const newPassword = ref('')
const passwordDialogLoading = ref(false)

// 租户（形态 B）：域下拉数据源 + 租户管理区块
const tenants = ref([])
const newTenant = ref({ id: '', name: '' })
const tenantLoading = ref(false)

const roleOptions = [
  { value: 'admin', labelKey: 'users.roleAdmin' },
  { value: 'operator', labelKey: 'users.roleOperator' },
  { value: 'viewer', labelKey: 'users.roleViewer' },
]

async function loadUsers() {
  loading.value = true
  error.value = ''
  try {
    const res = await api.users()
    users.value = res?.users ?? []
  } catch (e) {
    error.value = getErrorMessage(t, e)
  } finally {
    loading.value = false
  }
}

async function loadTenants() {
  try {
    const res = await api.tenants()
    tenants.value = res?.tenants ?? []
  } catch {
    // 非 Admin 拿不到租户列表：域下拉退化为自由输入不需要——保持空列表即可
    tenants.value = []
  }
}

function openAdd() {
  isEdit.value = false
  editId.value = ''
  formUser.value = { username: '', password: '', role: 'operator', tenant: 'default' }
  showDialog.value = true
}

function openEdit(row) {
  isEdit.value = true
  editId.value = row.id
  formUser.value = { username: row.username, password: '', role: row.role, tenant: row.tenant_id || 'default' }
  showDialog.value = true
}

async function saveUser() {
  const { username, password, role, tenant } = formUser.value
  if (!username?.trim()) {
    ElMessage.warning(t('users.usernameRequired'))
    return
  }
  if (!isEdit.value && !password?.trim()) {
    ElMessage.warning(t('users.passwordRequired'))
    return
  }
  dialogLoading.value = true
  try {
    if (isEdit.value) {
      await api.updateUser(editId.value, { username: username.trim(), role, tenant })
      ElMessage.success(t('users.updateSuccess'))
    } else {
      await api.createUser({ username: username.trim(), password: password || '', role: role || 'operator', tenant })
      ElMessage.success(t('users.createSuccess'))
    }
    showDialog.value = false
    await loadUsers()
  } catch (e) {
    ElMessage.error(getErrorMessage(t, e))
  } finally {
    dialogLoading.value = false
  }
}

async function doDelete(row) {
  try {
    await ElMessageBox.confirm(
      t('users.deleteConfirm', { name: row.username }),
      t('common.confirmDelete'),
      { type: 'warning', confirmButtonText: t('common.confirm'), cancelButtonText: t('common.cancel') }
    )
  } catch {
    return
  }
  try {
    await api.deleteUser(row.id)
    ElMessage.success(t('users.deleteSuccess'))
    await loadUsers()
  } catch (e) {
    ElMessage.error(getErrorMessage(t, e))
  }
}

function openChangePassword(row) {
  passwordUserId.value = row.id
  newPassword.value = ''
  showPasswordDialog.value = true
}

async function submitPassword() {
  if (!newPassword.value?.trim()) {
    ElMessage.warning(t('users.passwordRequired'))
    return
  }
  passwordDialogLoading.value = true
  try {
    await api.changePassword(passwordUserId.value, newPassword.value)
    ElMessage.success(t('users.passwordChanged'))
    showPasswordDialog.value = false
  } catch (e) {
    ElMessage.error(getErrorMessage(t, e))
  } finally {
    passwordDialogLoading.value = false
  }
}

async function doCreateTenant() {
  const id = newTenant.value.id?.trim()
  const name = newTenant.value.name?.trim() || id
  if (!id) {
    ElMessage.warning(t('users.tenantIdRequired'))
    return
  }
  tenantLoading.value = true
  try {
    await api.createTenant({ id, name })
    ElMessage.success(t('users.tenantCreated'))
    newTenant.value = { id: '', name: '' }
    await loadTenants()
  } catch (e) {
    ElMessage.error(getErrorMessage(t, e))
  } finally {
    tenantLoading.value = false
  }
}

async function doDeleteTenant(row) {
  try {
    await ElMessageBox.confirm(
      t('users.tenantDeleteConfirm', { name: row.id }),
      t('common.confirmDelete'),
      { type: 'warning', confirmButtonText: t('common.confirm'), cancelButtonText: t('common.cancel') }
    )
  } catch {
    return
  }
  try {
    await api.deleteTenant(row.id)
    ElMessage.success(t('users.tenantDeleted'))
    await Promise.all([loadTenants(), loadUsers()])
  } catch (e) {
    ElMessage.error(getErrorMessage(t, e))
  }
}

function roleLabel(role) {
  const opt = roleOptions.find(o => o.value === role)
  return opt ? t(opt.labelKey) : role
}

function formatDateTime(dateStr) {
  if (!dateStr) return '-'
  try {
    let date
    // 如果是纯数字字符串（Unix 时间戳秒数），转换为毫秒
    if (/^\d+$/.test(dateStr)) {
      date = new Date(parseInt(dateStr, 10) * 1000)
    } else {
      date = new Date(dateStr)
    }
    if (isNaN(date.getTime())) return dateStr
    const year = date.getFullYear()
    const month = String(date.getMonth() + 1).padStart(2, '0')
    const day = String(date.getDate()).padStart(2, '0')
    const hours = String(date.getHours()).padStart(2, '0')
    const minutes = String(date.getMinutes()).padStart(2, '0')
    const seconds = String(date.getSeconds()).padStart(2, '0')
    return `${year}-${month}-${day} ${hours}:${minutes}:${seconds}`
  } catch {
    return dateStr
  }
}

onMounted(() => {
  loadUsers()
  loadTenants()
})
</script>

<template>
  <div class="page-container users-page">
    <PageHeader :title="t('users.title')" :subtitle="t('users.desc')" />

    <el-alert v-if="error" type="error" :title="error" show-icon class="mb-2" />

    <div v-loading="loading" class="users-content">
      <div class="users-toolbar">
        <el-button type="primary" @click="openAdd">{{ t('users.addUser') }}</el-button>
      </div>

      <el-table :data="users" stripe>
        <el-table-column prop="username" :label="t('login.username')" min-width="100" />
        <el-table-column prop="role" :label="t('users.role')" width="100">
          <template #default="{ row }">{{ roleLabel(row.role) }}</template>
        </el-table-column>
        <el-table-column prop="tenant_id" :label="t('users.tenant')" min-width="100">
          <template #default="{ row }">
            <el-tag size="small" type="info">{{ row.tenant_id || 'default' }}</el-tag>
          </template>
        </el-table-column>
        <el-table-column prop="created_at" :label="t('users.createdAt')" min-width="140">
          <template #default="{ row }">{{ formatDateTime(row.created_at) }}</template>
        </el-table-column>
        <el-table-column :label="t('common.operation')" width="200" fixed="right">
          <template #default="{ row }">
            <el-button link type="primary" size="small" @click="openEdit(row)">{{ t('common.detail') }}</el-button>
            <el-button link type="primary" size="small" @click="openChangePassword(row)">{{ t('users.changePassword') }}</el-button>
            <el-button link type="danger" size="small" @click="doDelete(row)">{{ t('common.delete') }}</el-button>
          </template>
        </el-table-column>
      </el-table>

      <el-empty v-if="!loading && users.length === 0" :description="t('users.noUsers')" />
    </div>

    <!-- 租户管理（Admin；非 Admin 拿不到租户列表，区块自动隐藏） -->
    <el-card v-if="tenants.length > 0" shadow="never" class="tenant-card">
      <template #header>{{ t('users.tenantManage') }}</template>
      <div class="tenant-toolbar">
        <el-input
          v-model="newTenant.id"
          :placeholder="t('users.tenantIdPlaceholder')"
          style="width: 180px"
          maxlength="64"
        />
        <el-input
          v-model="newTenant.name"
          :placeholder="t('users.tenantNamePlaceholder')"
          style="width: 180px"
        />
        <el-button type="primary" :loading="tenantLoading" @click="doCreateTenant">
          {{ t('users.tenantCreate') }}
        </el-button>
      </div>
      <el-table :data="tenants" stripe size="small">
        <el-table-column prop="id" :label="t('users.tenantId')" min-width="120" />
        <el-table-column prop="name" :label="t('users.tenantName')" min-width="140" />
        <el-table-column prop="nodes_count" :label="t('users.tenantNodes')" width="100" />
        <el-table-column prop="users_count" :label="t('users.tenantUsers')" width="100" />
        <el-table-column :label="t('common.operation')" width="100">
          <template #default="{ row }">
            <el-button
              link
              type="danger"
              size="small"
              :disabled="row.id === 'default'"
              @click="doDeleteTenant(row)"
            >
              {{ t('common.delete') }}
            </el-button>
          </template>
        </el-table-column>
      </el-table>
    </el-card>

    <!-- 新增/编辑 -->
    <el-dialog
      v-model="showDialog"
      :title="isEdit ? t('users.editUser') : t('users.addUser')"
      width="400px"
      :close-on-click-modal="false"
      @closed="dialogLoading = false"
    >
      <el-form :model="formUser" label-width="80px">
        <el-form-item :label="t('login.username')" required>
          <el-input v-model="formUser.username" :placeholder="t('login.usernamePlaceholder')" :disabled="isEdit" />
        </el-form-item>
        <el-form-item v-if="!isEdit" :label="t('login.password')" required>
          <el-input v-model="formUser.password" type="password" :placeholder="t('login.passwordPlaceholder')" show-password />
        </el-form-item>
        <el-form-item :label="t('users.role')">
          <el-select v-model="formUser.role" style="width: 100%">
            <el-option
              v-for="opt in roleOptions"
              :key="opt.value"
              :value="opt.value"
              :label="t(opt.labelKey)"
            />
          </el-select>
        </el-form-item>
        <el-form-item :label="t('users.tenant')">
          <el-select v-model="formUser.tenant" style="width: 100%">
            <el-option
              v-for="tn in tenants"
              :key="tn.id"
              :value="tn.id"
              :label="`${tn.id} (${tn.name})`"
            />
            <el-option v-if="tenants.length === 0" value="default" label="default" />
          </el-select>
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="showDialog = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="dialogLoading" @click="saveUser">{{ t('common.save') }}</el-button>
      </template>
    </el-dialog>

    <!-- 修改密码 -->
    <el-dialog
      v-model="showPasswordDialog"
      :title="t('users.changePassword')"
      width="400px"
      :close-on-click-modal="false"
      @closed="passwordDialogLoading = false"
    >
      <el-form label-width="100px">
        <el-form-item :label="t('login.password')" required>
          <el-input v-model="newPassword" type="password" :placeholder="t('login.passwordPlaceholder')" show-password />
        </el-form-item>
      </el-form>
      <template #footer>
        <el-button @click="showPasswordDialog = false">{{ t('common.cancel') }}</el-button>
        <el-button type="primary" :loading="passwordDialogLoading" @click="submitPassword">{{ t('common.save') }}</el-button>
      </template>
    </el-dialog>
  </div>
</template>

<style scoped>
.users-toolbar {
  display: flex;
  justify-content: flex-end;
  margin-bottom: 1rem;
}
.tenant-card {
  margin-top: 1rem;
}
.tenant-toolbar {
  display: flex;
  gap: 0.5rem;
  margin-bottom: 0.75rem;
}
</style>
