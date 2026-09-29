import { defineConfig } from "vite";

// Tauri expects a fixed dev port and must not trigger reloads on Rust rebuilds.
export default defineConfig({
  clearScreen: false,
  server: { port: 1420, strictPort: true, watch: { ignored: ["**/src-tauri/**"] } },
});
