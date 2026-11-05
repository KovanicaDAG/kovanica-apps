import { defineConfig } from "vite";

// Dev server for the Tauri webview. Port 5175 stays clear of apps/desktop (5173)
// and apps/console-desktop (5174).
export default defineConfig({
  clearScreen: false,
  server: {
    port: 5175,
    strictPort: true,
  },
  envPrefix: ["VITE_", "TAURI_ENV_"],
  build: {
    outDir: "dist",
    sourcemap: false,
    target: "es2021",
  },
});
