import { defineConfig, loadEnv } from "vite";
import react from "@vitejs/plugin-react";

export default defineConfig(({ mode }) => {
  const env = loadEnv(mode, process.cwd(), "");

  return {
    plugins: [react()],
    server: {
      port: Number(env.VITE_DEV_PORT || 5173),
      host: env.VITE_DEV_HOST || "127.0.0.1",
      proxy: env.VITE_API_PROXY
        ? {
            "/api": {
              target: env.VITE_API_PROXY,
              changeOrigin: true,
            },
          }
        : undefined,
    },
  };
});
