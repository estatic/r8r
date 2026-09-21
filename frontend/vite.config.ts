import { defineConfig } from 'vitest/config'
import vue from '@vitejs/plugin-vue'

export default defineConfig({
  plugins: [vue()],
  server: {
    proxy: {
      '/rest': 'http://localhost:3000',
      '/webhook': 'http://localhost:3000',
      // Vite's string-shorthand proxy entries above do not forward
      // WebSocket upgrade requests -- ws: true is required, or
      // useLiveExecutionSocket's connection silently fails to reach the
      // backend and live execution status never streams in dev mode.
      '/ws': {
        target: 'ws://localhost:3000',
        ws: true,
      },
    },
  },
  test: {
    environment: 'jsdom',
    setupFiles: ['./src/test-setup.ts'],
  },
})
