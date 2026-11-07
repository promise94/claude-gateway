// 基于 create-tauri-app 4.7.5 官方 react-ts 模板渲染产物。
// 与原模板的差异：defineConfig 改从 vitest/config 导入，并追加 test 段（D12 工程化门禁）。
import { defineConfig } from "vitest/config";
import react from "@vitejs/plugin-react";
// @ts-expect-error type error without @types/node package
import process from "node:process";
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(() => ({
  // Tauri 自定义协议下绝对路径 /assets 容易取不到，用相对路径
  base: "./",
  plugins: [react()],

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
  test: {
    // forks pool 需要起子进程，在受限环境里会被拦；threads 是官方支持的等价方式
    pool: "threads",
    // 并行起多个 worker 时偶发「60s 内无响应」，单个 worker 换来可重复的门禁
    maxWorkers: 1,
    environment: "jsdom",
    setupFiles: ["./src/test/setup.ts"],
    include: ["src/**/*.{test,spec}.{ts,tsx}"],
  },
}));
