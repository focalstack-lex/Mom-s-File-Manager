import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";

// Vite options tailored for Tauri development: a fixed port (tauri.conf.json
// devUrl), no screen clearing so Rust errors stay visible, and src-tauri kept
// out of the watcher. The `test` block configures vitest.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1430,
    strictPort: true,
    watch: { ignored: ["**/src-tauri/**"] },
  },
  test: {
    include: ["src/tests/**/*.test.ts"],
    environment: "node",
  },
});
