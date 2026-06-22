import path from "path";
import { defineConfig } from "vite";
import react from "@vitejs/plugin-react";
/// <reference types="vitest" />

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;
// @ts-expect-error process is a nodejs global
const isPro = Boolean(process.env.VITE_PRO);

// https://vite.dev/config/
export default defineConfig(async () => ({
  plugins: [react()],
  test: {
    environment: "node",
    include: ["src/**/*.test.ts"],
  },

  resolve: {
    alias: {
      // `@pro` resolves to the real Pro barrel in Pro builds, or to no-op stubs
      // in the free build.  When `src/pro/` is physically deleted (open-source
      // release), the stub path is all that exists — the free build still works.
      "@pro": isPro
        ? path.resolve(__dirname, "src/pro/index.ts")
        : path.resolve(__dirname, "src/pro-stub/index.ts"),
    },
  },

  // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
  //
  // 1. prevent Vite from obscuring rust errors
  clearScreen: false,
  // 2. tauri expects a fixed port, fail if that port is not available
  server: {
    port: 1420,
    strictPort: true,
    host: host || false,
    hmr: host
      ? {
          protocol: "ws",
          host,
          port: 1421,
        }
      : undefined,
    watch: {
      // 3. tell Vite to ignore watching `src-tauri`
      ignored: ["**/src-tauri/**"],
    },
  },
}));
