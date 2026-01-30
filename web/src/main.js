import { createApp } from 'vue'
import ElementPlus from 'element-plus'
import zhCn from 'element-plus/es/locale/lang/zh-cn.mjs'
import 'element-plus/dist/index.css'
import './style.css'
import App from './App.vue'
import router from './router.js'
import { i18n } from './i18n/index.js'

const app = createApp(App)
app.use(i18n)
app.use(router)
app.use(ElementPlus, { locale: zhCn })

// 等路由完成首次导航再挂载，避免白屏/转圈（redirect + beforeEach 已就绪）
router.isReady().then(() => {
  app.mount('#app')
}).catch((err) => {
  console.error('[router.isReady]', err)
  app.mount('#app')
})
