import { defineConfig, loadEnv } from "vite";
import vue from "@vitejs/plugin-vue";
import { readdir, readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async ({ mode, command, isPreview }) => {
  const environment = loadEnv(mode, ".", "");
  const performanceProfilingEnabled = mode === "development" || environment.VITE_BKM_PERF === "true";
  const servingDev = command === "serve" && !isPreview;

  return {
    // Leave existing immutable WebView entries under the old optimizer URL.
    cacheDir: servingDev ? "node_modules/.vite-bkm" : undefined,
    plugins: [
      vue(),
      {
        name: "pdf-offline-assets",
        async generateBundle() {
          for (const directory of ["cmaps", "standard_fonts", "wasm"]) {
            const root = new URL(`./node_modules/pdfjs-dist/${directory}/`, import.meta.url);
            const entries = (await readdir(root, { withFileTypes: true })).filter((entry) => entry.isFile());
            await Promise.all(entries.map(async (entry) => {
              this.emitFile({ type: "asset", fileName: `pdf-assets/${directory}/${entry.name}`,
                source: await readFile(fileURLToPath(new URL(entry.name, root))) });
            }));
          }
        },
      },
      ...(performanceProfilingEnabled
        ? [{
            name: "bkm-performance-bootstrap",
            transformIndexHtml: {
              order: "pre" as const,
              handler: () => [{
                tag: "script",
                children: "if(typeof performance!==\"undefined\"&&typeof performance.mark===\"function\"){performance.mark(\"bkm:bootstrap\")}",
                injectTo: "head-prepend" as const,
              }],
            },
          }]
        : []),
    ],

    // Vite options tailored for Tauri development and only applied in `tauri dev` or `tauri build`
    //
    // 1. prevent Vite from obscuring rust errors
    clearScreen: false,
    // 2. tauri expects a fixed port, fail if that port is not available
    server: {
      // A prebundled entry can keep its URL while its shared chunk URLs change.
      headers: servingDev ? { "Cache-Control": "no-cache" } : undefined,
      port: 51420,
      strictPort: true,
      host: host || false,
      hmr: host
        ? {
            protocol: "ws",
            host,
            port: 51421,
          }
        : undefined,
      watch: {
        // 3. tell Vite to ignore watching `src-tauri`
        ignored: ["**/src-tauri/**"],
      },
    },
  };
});
