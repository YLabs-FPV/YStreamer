import { fileURLToPath, URL } from "node:url";
import { defineConfig, loadEnv } from "vite";
import { svelte } from "@sveltejs/vite-plugin-svelte";
import tailwindcss from "@tailwindcss/vite";

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), "");
  const PI_HOST = env.PI_HOST ?? "127.0.0.1";
  const PI_PORT = env.PI_PORT ?? "80";
  const target = `http://${PI_HOST}:${PI_PORT}`;

  return {
    plugins: [svelte(), tailwindcss()],
    resolve: {
      // Keep in sync with "paths" in tsconfig.app.json
      alias: { "@": fileURLToPath(new URL("./src", import.meta.url)) },
    },
    server: {
      host: true,
      // In production the Pi serves the built bundle itself, so these are
      // same-origin; the proxy only bridges the dev server
      proxy: {
        "/api": { target, changeOrigin: true, ws: true },
        "/health": { target, changeOrigin: true },
      },
    },
  };
});
