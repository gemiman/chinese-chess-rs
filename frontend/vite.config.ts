import { defineConfig } from 'vite'
import react from '@vitejs/plugin-react-swc'

export default defineConfig({
  plugins: [react()],

  // 素材是单一真相源：直接把仓库根的 assets/ 作为静态目录。
  // 于是 /board/*、/pieces/*、/tokens/* 都能直接访问，构建时也会原样复制进 dist，
  // 不需要在 frontend/ 里放副本。
  //
  // 路径相对本文件所在目录（frontend/）解析 —— 刻意不用 node:path，
  // 这样配置文件不依赖 @types/node。
  publicDir: '../assets',

  server: {
    port: 5173,
    // 开发时前端跑在 5173，API 由 xq-bridge 提供：cargo run -p xq-bridge -- --dev
    proxy: {
      '/api': { target: 'http://127.0.0.1:8848', changeOrigin: true },
    },
  },

  build: {
    outDir: 'dist',
    emptyOutDir: true,
    sourcemap: true,
  },
})
