import { defineConfig } from "vite";

// Tauri serves the dev server on a fixed port and loads the built files from dist/.
export default defineConfig({
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  build: { target: "es2022", outDir: "dist", chunkSizeWarningLimit: 1000 },
});
