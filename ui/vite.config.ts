import { svelte } from "@sveltejs/vite-plugin-svelte";
import { defineConfig } from "vite";

// Tauri expects a fixed dev port and must not lose the Rust compiler's output.
export default defineConfig({
  plugins: [svelte()],
  define: { __APP_VERSION__: JSON.stringify(process.env.npm_package_version ?? "") },
  clearScreen: false,
  server: { port: 1420, strictPort: true },
  // The app and the splash window the shell opens while the backend starts.
  build: { target: "es2022", rollupOptions: { input: { main: "index.html", splash: "splash.html" } } },
});
