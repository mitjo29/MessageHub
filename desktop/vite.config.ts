import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri 2 expects the dev server at a fixed port and strictPort = true
// so it can inject the webview at startup.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
    host: false,
    watch: {
      ignored: ["**/src-tauri/**"],
    },
  },
  envPrefix: ["VITE_", "TAURI_ENV_*"],
  build: {
    target: "es2021",
    minify: false,
    sourcemap: true,
  },
});
