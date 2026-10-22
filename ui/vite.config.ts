import path from "node:path";
import tailwindcss from "@tailwindcss/vite";
import react from "@vitejs/plugin-react";
import { defineConfig } from "vite-plus";

// One config for the whole Vite+ toolchain: dev server/build (Vite),
// tests (Vitest), lint (Oxlint) and format (Oxfmt).
// Tauri serves the built files; in dev it loads this server (see
// crates/ui/tauri.conf.json devUrl).
export default defineConfig({
  plugins: [react(), tailwindcss()],
  resolve: { alias: { "@": path.resolve(__dirname, "./src") } },
  clearScreen: false,
  server: { port: 5173, strictPort: true },
  // WebView2 is evergreen Chromium; no need to transpile for old browsers.
  build: { target: "chrome120", outDir: "dist", emptyOutDir: true },
  test: { environment: "node", include: ["src/**/*.test.ts"] },
  lint: {
    plugins: ["react", "typescript"],
    // shadcn components are generated and owned upstream-style.
    ignorePatterns: ["dist/**", "src/components/ui/**"],
  },
  fmt: { printWidth: 120 },
});
