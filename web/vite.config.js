import { defineConfig } from 'vite'
import vue from '@vitejs/plugin-vue'

const BACKEND_PORT = process.env.GATEWAY_PORT || '4000'

// https://vite.dev/config/
export default defineConfig({
  plugins: [vue()],
  build: {
    outDir: 'dist',
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
