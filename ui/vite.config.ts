import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vite";

// Tauri expects a fixed dev port and must not lose the Rust compiler's output.
export default defineConfig({
  plugins: [svelte()],
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  // The app and the splash window the shell opens while the backend starts.
  // Assets stay files: the CSP allows 'self' but not data: URLs for images and fonts.
  build: {
    target: "es2022",
    assetsInlineLimit: 0,
    rollupOptions: { input: { main: "index.html", splash: "splash.html" } },
  },
});
