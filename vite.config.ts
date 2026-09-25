import { defineConfig } from "vite"
import react from "@vitejs/plugin-react"

// @ts-expect-error process is a Node.js global.
let host = process.env.TAURI_DEV_HOST

export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1422,
    strictPort: true,
    host: host || false,
    hmr: host ? { protocol: "ws", host, port: 1423 } : undefined,
  },
})

