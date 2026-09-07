import { defineConfig } from "vite";

export default defineConfig({
  root: "trail",
  base: "/trail/",
  build: {
    outDir: "dist",
    emptyOutDir: false,
    sourcemap: false,
    rollupOptions: {
      output: {
        entryFileNames: "assets/trail.js",
        chunkFileNames: "assets/[name].js",
        assetFileNames: "assets/trail.[ext]",
      },
    },
  },
});
