import { createApp } from 'vue'
import { createPinia } from 'pinia'
import './style.css'
import App from './App.vue'
import router from './router.js'
import { i18n } from './i18n/index.js'

// Element Plus 按需引入：模板组件由 vite.config.js 的 unplugin-vue-components +
// ElementPlusResolver 自动注册并按需携带样式；el-config-provider（App.vue）提供 locale。
// 以下是不经模板编译、必须手动引入的部分：
// 1) 命令式 API 的样式（ElMessage / ElMessageBox 在各视图显式 import）
import 'element-plus/es/components/message/style/css'
import 'element-plus/es/components/message-box/style/css'
// 2) v-loading 指令（License.vue / Users.vue 使用）
import { ElLoadingDirective } from 'element-plus'
import 'element-plus/es/components/loading/style/css'

const app = createApp(App)
const pinia = createPinia()

app.use(pinia)
app.use(i18n)
app.use(router)
app.directive('loading', ElLoadingDirective)

// 等路由完成首次导航再挂载，避免白屏/转圈（redirect + beforeEach 已就绪）
router.isReady().then(() => {
  app.mount('#app')
}).catch((err) => {
  console.error('[router.isReady]', err)
  app.mount('#app')
})
