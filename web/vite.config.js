import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

// 后端默认端口见 gateway-server/src/config.rs 的 default_port()
const BACKEND_PORT = process.env.GATEWAY_PORT || '3000'

// https://vite.dev/config/
export default defineConfig({
  plugins: [vue()],
  build: {
    outDir: 'dist',
    rollupOptions: {
      output: {
        // 拆包：主入口只留业务代码，第三方库按「改动频率」分组，
        // 升级 Element Plus 不会让业务 chunk 缓存失效。
        manualChunks(id) {
          if (!id.includes('node_modules')) return undefined
          if (id.includes('element-plus') || id.includes('@element-plus')) {
            return 'element-plus'
          }
          if (id.includes('vue-i18n') || id.includes('@intlify')) return 'vue-i18n'
          if (id.includes('vue-router')) return 'vue-router'
          if (id.includes('/vue/') || id.includes('@vue/')) return 'vue'
          return 'vendor'
        },
      },
    },
  },
  server: {
    proxy: {
      '/api': {
        target: `http://127.0.0.1:${BACKEND_PORT}`,
        changeOrigin: true,
      },
    },
  },
})
