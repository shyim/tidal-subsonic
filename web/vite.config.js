import { defineConfig } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import { viteSingleFile } from "vite-plugin-singlefile";

// The Rust backend embeds `web/dist/index.html` via include_str!, so the build
// must inline all JS/CSS into a single self-contained HTML file.
export default defineConfig({
  plugins: [svelte(), viteSingleFile()],
  build: {
    outDir: "dist",
    emptyOutDir: true,
    // Belt-and-suspenders with viteSingleFile: inline everything.
    assetsInlineLimit: 100000000,
    cssCodeSplit: false,
    reportCompressedSize: false,
  },
  server: {
    port: 5173,
    // During `pnpm dev`, proxy the JSON API to the running Rust server so the
    // cookie session flow works end-to-end against the real backend.
    proxy: {
      "/api": {
        target: "http://localhost:4533",
        changeOrigin: true,
      },
    },
  },
});
