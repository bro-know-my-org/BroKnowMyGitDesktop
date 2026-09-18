import { defineConfig, loadEnv } from "vite";
import vue from "@vitejs/plugin-vue";
import { readdir, readFile } from "node:fs/promises";
import { fileURLToPath } from "node:url";

// @ts-expect-error process is a nodejs global
const host = process.env.TAURI_DEV_HOST;

// https://vite.dev/config/
export default defineConfig(async ({ mode }) => {
  const environment = loadEnv(mode, ".", "");
  const performanceProfilingEnabled = mode === "development" || environment.VITE_BKM_PERF === "true";

  return {
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
