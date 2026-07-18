import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";

// Tauri drives this dev server, so the port is fixed and failing is better than
// silently moving to another one the Rust side is not pointed at.
export default defineConfig({
  plugins: [react()],
  clearScreen: false,
  server: {
    port: 1420,
    strictPort: true,
  },
  build: {
    // The webview is always current WebKit, so there is no old-browser target to
    // down-level for.
    target: "safari18",
    sourcemap: false,
  },
});
