import { defineConfig } from "vite";

export default defineConfig({
  // Extension pages resolve assets relative to the extension root. Keep the
  // generated popup references relative so dist/popup.html can load dist/*.js.
  base: "./",
  build: {
    outDir: "dist",
    rollupOptions: {
      input: {
        background: "background.ts",
        content: "content.ts",
        popup: "popup.html",
      },
      output: {
        entryFileNames: "[name].js",
        chunkFileNames: "[name].js",
        assetFileNames: "[name][extname]",
      },
    },
  },
  clearScreen: false,
});
